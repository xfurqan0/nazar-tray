//! The readers the loop owns, behind one small trait.
//!
//! There is exactly one thread that reads anything, and it is the refresh loop's. Every
//! reader in this crate is stateful — the Codex reader holds a byte offset into the log it
//! is following, the opt-in mode holds its backoff and its last good numbers — and none of
//! them is safe to call from two places at once. Putting them all behind [`Reader`] and
//! giving the loop the only handle is what makes that a property of the design rather than
//! a rule people remember.
//!
//! ```text
//!            ┌── CodexSource   ── rollout-*.jsonl ──▶ providers.codex
//!   Loop ──▶ │
//!            └── ClaudeSource  ── statusline captures ─┐
//!                               (+ the opt-in endpoint) ├─ merge ─▶ providers.claude
//!                                                       ┘
//! ```
//!
//! A reader that fails does not fail the refresh: it returns a provider block that says so,
//! in the contract's own vocabulary (`state: "error"`, no percentage), and the other reader
//! is untouched. That is finding B11 of the audit — "not set up" and "could not read" are
//! different sentences and the retired prototype said the second when it meant the first.

use std::path::PathBuf;
use std::time::Duration;

use crate::claude::ClaudeReader;
use crate::clock::Clock;
use crate::codex::CodexReader;
use crate::limits::{Limits, Provider};

/// How long the opt-in detailed-windows mode waits between requests.
///
/// The endpoint is undocumented and rate-limited, and its own backoff already pushes a
/// failure away; this is the floor under a *success*, so that a machine where everything is
/// working still asks twelve times an hour rather than sixty. The passive path, which costs
/// nothing but a file read, keeps running at the full tick.
pub const DETAILED_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// One provider's reader, as the refresh loop sees it.
pub trait Reader: Send {
    /// The provider key this reader fills in: `claude` or `codex`.
    fn key(&self) -> &'static str;

    /// Read the provider block for the instant `now`.
    ///
    /// Infallible on purpose. Everything that can go wrong is already part of the contract —
    /// a provider that is not on this machine, a window that could not be read — so there is
    /// no error for a caller to handle that the document does not already express better.
    fn read(&mut self, clock: &dyn Clock, now: &str) -> Provider;

    /// How many times this reader has had to skip something it did not understand.
    ///
    /// A number that keeps climbing means a format has moved under us.
    fn warnings(&self) -> u64 {
        0
    }

    /// Files and directories whose changes should trigger a refresh.
    fn watched(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// Every reader the loop owns.
#[derive(Default)]
pub struct ReaderSet {
    readers: Vec<Box<dyn Reader>>,
}

impl std::fmt::Debug for ReaderSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReaderSet")
            .field(
                "readers",
                &self
                    .readers
                    .iter()
                    .map(|reader| reader.key())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl ReaderSet {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        ReaderSet::default()
    }

    /// Add a reader.
    #[must_use]
    pub fn with(mut self, reader: Box<dyn Reader>) -> Self {
        self.readers.push(reader);
        self
    }

    /// The readers this build ships, pointed at this machine and at these settings.
    ///
    /// Two things in the settings decide what is built, and in both cases "off" means the
    /// reader **does not exist** rather than that its answer is thrown away:
    ///
    /// * `config.providers` switches a whole provider off. Somebody who does not use Codex
    ///   should not have a program listing their session directory every five seconds to
    ///   find out; with the switch off, nothing here opens `~/.codex` at all.
    /// * `config.detailedWindows` switches the opt-in endpoint mode on. With it false the
    ///   mode is not built, so no path in it can run; see [`crate::claude::detailed`].
    #[must_use]
    pub fn discover(config: &crate::config::Config) -> Self {
        let mut set = ReaderSet::new();
        if config.providers.codex
            && let Ok(reader) = CodexReader::discover()
        {
            set.readers.push(Box::new(CodexSource::new(reader)));
        }
        if config.providers.claude
            && let Ok(reader) = ClaudeReader::discover()
        {
            #[cfg(feature = "detailed-windows")]
            set.readers
                .push(Box::new(ClaudeSource::new(reader, config.detailed_windows)));
            #[cfg(not(feature = "detailed-windows"))]
            set.readers.push(Box::new(ClaudeSource::new(reader)));
        }
        set
    }

    /// The provider keys this set will fill in.
    #[must_use]
    pub fn keys(&self) -> Vec<&'static str> {
        self.readers.iter().map(|reader| reader.key()).collect()
    }

    /// Build the whole document for `now`.
    ///
    /// Every provider key the contract knows about is present whatever happened, because
    /// [`Limits::new`] puts both there unconfigured before any reader runs. A reader for a
    /// provider this build does not have a field for lands in `providers.extra`, which is
    /// how a third provider arrives in v2 without a schema change.
    pub fn read(&mut self, clock: &dyn Clock, now: &str) -> Limits {
        let mut limits = Limits::new(now);
        for reader in &mut self.readers {
            let key = reader.key();
            let provider = reader.read(clock, now);
            match key {
                "claude" => limits.providers.claude = provider,
                "codex" => limits.providers.codex = provider,
                other => {
                    if let Ok(value) = serde_json::to_value(&provider) {
                        limits.providers.extra.insert(other.to_owned(), value);
                    }
                }
            }
        }
        limits
    }

    /// Each reader's warning counter, by key.
    #[must_use]
    pub fn warnings(&self) -> Vec<(String, u64)> {
        self.readers
            .iter()
            .map(|reader| (reader.key().to_owned(), reader.warnings()))
            .collect()
    }

    /// Everything the readers would like watched.
    #[must_use]
    pub fn watched(&self) -> Vec<PathBuf> {
        self.readers
            .iter()
            .flat_map(|reader| reader.watched())
            .collect()
    }

    /// How many readers there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.readers.len()
    }

    /// Whether there are no readers at all, which is what a machine with no home directory
    /// looks like.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.readers.is_empty()
    }
}

/// Codex's rollout logs.
#[derive(Debug)]
pub struct CodexSource {
    reader: CodexReader,
}

impl CodexSource {
    /// Wrap a reader.
    #[must_use]
    pub fn new(reader: CodexReader) -> Self {
        CodexSource { reader }
    }
}

impl Reader for CodexSource {
    fn key(&self) -> &'static str {
        "codex"
    }

    fn read(&mut self, _clock: &dyn Clock, now: &str) -> Provider {
        // `now` is not decoration here. A rollout log is written only while Codex is
        // running, so on a machine nobody has opened it on for two days the newest log
        // still parses to a confident percentage — for a window that reset yesterday. The
        // reader compares each window's reset against this instant and marks the expired
        // ones stale rather than reporting them as current.
        self.reader.refresh_at(now)
    }

    fn warnings(&self) -> u64 {
        self.reader.warnings()
    }

    fn watched(&self) -> Vec<PathBuf> {
        // The log being followed, and the directory it is in. The first notices an append
        // to this session; the second notices tomorrow's session starting.
        let mut paths = Vec::new();
        if let Some(log) = self.reader.following() {
            paths.push(log.to_path_buf());
            if let Some(parent) = log.parent() {
                paths.push(parent.to_path_buf());
            }
        }
        paths
    }
}

/// Claude Code's status-line captures, and the opt-in endpoint when it is on.
pub struct ClaudeSource {
    passive: ClaudeReader,
    blind_reads: u64,
    #[cfg(feature = "detailed-windows")]
    detailed: crate::claude::detailed::DetailedWindows,
    #[cfg(feature = "detailed-windows")]
    last_reading: Option<crate::claude::detailed::Reading>,
    #[cfg(feature = "detailed-windows")]
    last_asked_ms: Option<u64>,
    #[cfg(feature = "detailed-windows")]
    refusals: u64,
}

impl std::fmt::Debug for ClaudeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeSource")
            .field("dir", &self.passive.dir())
            .finish_non_exhaustive()
    }
}

impl ClaudeSource {
    /// The passive reader alone.
    #[cfg(not(feature = "detailed-windows"))]
    #[must_use]
    pub fn new(passive: ClaudeReader) -> Self {
        ClaudeSource {
            passive,
            blind_reads: 0,
        }
    }

    /// The passive reader, plus the opt-in mode when `detailed` is on.
    #[cfg(feature = "detailed-windows")]
    #[must_use]
    pub fn new(passive: ClaudeReader, detailed: bool) -> Self {
        ClaudeSource {
            passive,
            blind_reads: 0,
            detailed: crate::claude::detailed::DetailedWindows::from_config(detailed),
            last_reading: None,
            last_asked_ms: None,
            refusals: 0,
        }
    }

    /// Whether the last read produced no percentage at all.
    fn blind(provider: &Provider) -> bool {
        provider.configured
            && !provider
                .windows
                .values()
                .any(|window| window.percent.is_some())
    }
}

impl Reader for ClaudeSource {
    fn key(&self) -> &'static str {
        "claude"
    }

    #[cfg(not(feature = "detailed-windows"))]
    fn read(&mut self, _clock: &dyn Clock, _now: &str) -> Provider {
        let provider = self.passive.refresh();
        if ClaudeSource::blind(&provider) {
            self.blind_reads += 1;
        }
        provider
    }

    #[cfg(feature = "detailed-windows")]
    fn read(&mut self, clock: &dyn Clock, now: &str) -> Provider {
        let passive = self.passive.refresh();

        // The endpoint is asked at most every five minutes, on top of its own backoff. The
        // captures are free to read and are re-read on every refresh.
        if self.detailed.is_enabled() {
            let now_ms = clock.monotonic_millis();
            let due = self.last_asked_ms.is_none_or(|last| {
                now_ms.saturating_sub(last) >= DETAILED_INTERVAL.as_millis() as u64
            });
            if due {
                self.last_asked_ms = Some(now_ms);
                if let Some(outcome) = self.detailed.refresh(clock) {
                    if outcome.error.is_some() {
                        self.refusals += 1;
                    }
                    if outcome.reading.is_some() {
                        self.last_reading = outcome.reading;
                    }
                }
            }
        }

        let merged = crate::claude::merge::merge(passive, self.last_reading.as_ref(), now);
        if ClaudeSource::blind(&merged) {
            self.blind_reads += 1;
        }
        merged
    }

    fn warnings(&self) -> u64 {
        #[cfg(feature = "detailed-windows")]
        {
            self.blind_reads + self.refusals
        }
        #[cfg(not(feature = "detailed-windows"))]
        {
            self.blind_reads
        }
    }

    fn watched(&self) -> Vec<PathBuf> {
        vec![self.passive.dir().to_path_buf()]
    }
}
