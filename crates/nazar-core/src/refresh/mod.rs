//! The refresh loop: one thread, all the readers, and the only writer.
//!
//! `docs/PROJECT.md` decided this in the research phase, and the audit is the argument.
//! The retired prototype scheduled its refresh with the operating system — a Windows
//! scheduled task, a VBS launcher and a PowerShell wrapper, with `launchd` and `systemd`
//! waiting to be written for the other two platforms — and every one of the observed
//! failures came from there: two refreshers racing each other four seconds apart (B01), a
//! lock nobody in the chain took, a missed run replayed the instant a laptop woke with a
//! sign-in that had expired overnight (B34, S1), and a scheduler reporting success for
//! three hours while the data went nowhere (B17).
//!
//! Inside the process there is one loop, it owns the readers, and it is the only thing that
//! writes `limits.json`. Nothing has to agree with anything else.
//!
//! ```text
//!            startup ─┐
//!          60 s tick ─┤
//!   file change (5 s) ├─▶ debounce 250 ms ─▶ read every provider ─▶ compare
//!    panel or relaunch┤                                                │
//!     clock jump >2×  ─┘                       unchanged ◀────────────┴──▶ changed
//!                                                  │                          │
//!                                       nothing is written          stamp updatedAt,
//!                                       and nothing is emitted      write atomically,
//!                                                                   tell the panel
//! ```
//!
//! The loop is split so that all of the timing can be tested without waiting for any of it:
//! [`schedule`] is pure arithmetic over an injected clock, [`watch`] is the five-second
//! look at the filesystem, [`sources`] holds the readers, and [`Engine`] is the part that
//! joins them up. [`spawn`] is thirty lines of thread and channel around `Engine::tick`,
//! and the tests drive `tick` directly.

pub mod schedule;
pub mod sources;
pub mod watch;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::atomic;
use crate::clock::Clock;
use crate::error::Result;
use crate::limits::Limits;
use crate::lock::{Heartbeat, LimitsLock};
use crate::state::{Rules, Snapshot};
use crate::timefmt::unix_seconds_from_rfc3339;
use crate::writer::{LimitsWriter, Written, canonical_form};

pub use schedule::{Cause, Schedule};
pub use sources::{ClaudeSource, CodexSource, Reader, ReaderSet};
pub use watch::WatchSet;

/// How old a request file may be and still be acted on.
///
/// A second launch writes one and expects the running tray to open its panel within a few
/// seconds. One left behind by a crash is not an instruction, it is litter, and popping a
/// panel open days later because of it would be a small haunting.
pub const REQUEST_MAX_AGE: Duration = Duration::from_secs(60);

/// Something the loop wants the application to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A pass finished, whether or not anything moved.
    ///
    /// Emitted **before** [`Event::SnapshotChanged`] on a pass that changed something, and
    /// on its own on a pass that did not. The threshold notifications listen for this one
    /// rather than for the change, because the first pass after a restart has to be able to
    /// say "you are already at 91 %" even when the file on disk said so all along — and
    /// because a pass that read nothing new is still the pass that follows a wake-up.
    Refreshed,
    /// The document changed. The panel should re-read the snapshot.
    SnapshotChanged,
    /// Somebody asked for the panel: a second launch of the application.
    ShowRequested,
}

/// What one pass of the loop did.
#[derive(Debug, Clone, PartialEq)]
pub struct Cycle {
    /// Why a refresh happened, or `None` when none did.
    pub cause: Option<Cause>,
    /// Whether the document differed from the last one.
    pub changed: bool,
    /// What reached the disk.
    pub written: Written,
    /// How long the loop may sleep before the next pass.
    pub sleep_for: Duration,
}

/// Counters a diagnostic view can show without anything having to be logged.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Warnings {
    /// Per-reader counters, by provider key.
    pub readers: BTreeMap<String, u64>,
    /// Refreshes that produced a document.
    pub refreshes: u64,
    /// Times the document actually reached the disk.
    pub writes: u64,
    /// Times a write failed. The loop carries on and tries again next time.
    pub write_failures: u64,
    /// Times the advisory lock could not be renewed.
    pub lock_failures: u64,
    /// Whether this process is a reader rather than the writer.
    pub read_only: bool,
    /// The last failure, as a sentence. Never file contents; see [`crate::error`].
    pub last_error: Option<String>,
}

/// The refresh loop, minus the thread.
///
/// Everything happens in [`Engine::tick`], which is called with a clock and returns how
/// long it would like to sleep. A test calls it in a row with a clock it moves by hand; the
/// application calls it from [`spawn`].
pub struct Engine {
    readers: ReaderSet,
    schedule: Schedule,
    watch: WatchSet,
    rules: Rules,
    snapshot: Arc<Mutex<Snapshot>>,
    last_canonical: Option<String>,
    writer: Option<LimitsWriter>,
    lock: Option<LimitsLock>,
    request: Option<PathBuf>,
    listener: Option<Box<dyn Fn(Event) + Send>>,
    warnings: Warnings,
    published: Arc<Mutex<Warnings>>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("readers", &self.readers)
            .field("writing", &self.writer.is_some())
            .field("watching", &self.watch.targets().len())
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// An engine over `limits_path`.
    ///
    /// With a `lock`, this process is the writer. Without one it is a reader: it still
    /// refreshes, still keeps a snapshot and still tells the panel about it, but nothing it
    /// does reaches `limits.json`. That is what a second instance does, and what
    /// `nazar-tray --print --write` does when the tray is already running.
    ///
    /// Whatever is already in the file becomes the starting snapshot, so a tray that has
    /// just started shows the last known numbers instead of a blank panel, and a restart
    /// over an unchanged document writes nothing.
    #[must_use]
    pub fn new(
        readers: ReaderSet,
        limits_path: impl Into<PathBuf>,
        lock: Option<LimitsLock>,
        rules: Rules,
        now: &str,
    ) -> Self {
        let path = limits_path.into();
        let existing = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| Limits::from_json(&text).ok());
        let last_canonical = existing
            .as_ref()
            .and_then(|limits| canonical_form(limits).ok());
        let snapshot = existing.map_or_else(|| Snapshot::empty(now), Snapshot::new);
        let writing = lock.is_some();

        Engine {
            readers,
            schedule: Schedule::default(),
            watch: WatchSet::new(),
            rules,
            snapshot: Arc::new(Mutex::new(snapshot)),
            last_canonical,
            writer: writing.then(|| LimitsWriter::adopting(&path)),
            lock,
            request: None,
            listener: None,
            warnings: Warnings {
                read_only: !writing,
                ..Warnings::default()
            },
            published: Arc::new(Mutex::new(Warnings {
                read_only: !writing,
                ..Warnings::default()
            })),
        }
    }

    /// Watch a path for "somebody launched the application again".
    #[must_use]
    pub fn with_request_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.request = Some(path.into());
        self
    }

    /// Hand events to a listener. Called on the loop's own thread.
    #[must_use]
    pub fn on_event(mut self, listener: impl Fn(Event) + Send + 'static) -> Self {
        self.listener = Some(Box::new(listener));
        self
    }

    /// Replace the timing, for a test that does not want to wait a minute per tick.
    #[must_use]
    pub fn with_schedule(mut self, schedule: Schedule) -> Self {
        self.schedule = schedule;
        self
    }

    /// The shared snapshot. Cloned into the application so the panel can read it.
    #[must_use]
    pub fn snapshot(&self) -> Arc<Mutex<Snapshot>> {
        Arc::clone(&self.snapshot)
    }

    /// The rules this engine derives with.
    #[must_use]
    pub fn rules(&self) -> Rules {
        self.rules
    }

    /// The counters.
    #[must_use]
    pub fn warnings(&self) -> Warnings {
        let mut warnings = self.warnings.clone();
        warnings.readers = self.readers.warnings().into_iter().collect();
        warnings
    }

    /// The counters, shared with whoever wants to watch them.
    ///
    /// The engine is moved onto its own thread by [`spawn`], so this is how a panel or a
    /// diagnostic view reads the numbers while the loop is running. Refreshed at the end of
    /// every refresh.
    #[must_use]
    pub fn diagnostics(&self) -> Arc<Mutex<Warnings>> {
        Arc::clone(&self.published)
    }

    /// Whether this process is the writer.
    #[must_use]
    pub fn is_writing(&self) -> bool {
        self.writer.is_some()
    }

    /// The paths currently watched.
    #[must_use]
    pub fn watched(&self) -> &[PathBuf] {
        self.watch.targets()
    }

    /// Ask for a refresh at the next pass.
    pub fn request_refresh(&mut self, clock: &dyn Clock) {
        self.schedule
            .signal(clock.monotonic_millis(), Cause::Requested);
    }

    /// Replace the readers and the rules, and refresh at once.
    ///
    /// What a settings change does. Switching a provider off has to mean the reader is
    /// **gone** rather than that its answer is ignored (`docs/PROJECT.md` WP5), and the only
    /// place a reader can be dropped is here, on the thread that owns it — which is why this
    /// arrives as a command rather than as a lock somebody else can take.
    ///
    /// The watch list is cleared with the readers: a directory nobody reads any more is a
    /// directory nobody should be listing every five seconds.
    pub fn apply(&mut self, readers: ReaderSet, rules: Rules, clock: &dyn Clock) {
        self.readers = readers;
        self.rules = rules;
        self.watch.set_targets(self.readers.watched());
        self.schedule
            .signal(clock.monotonic_millis(), Cause::Requested);
    }

    /// One pass of the loop.
    pub fn tick(&mut self, clock: &dyn Clock) -> Cycle {
        let now_ms = clock.monotonic_millis();
        let now = clock.now_rfc3339();

        let observation = self
            .schedule
            .observe(now_ms, crate::clock::wall_seconds(clock));
        if observation.woke {
            self.schedule.signal(now_ms, Cause::Woke);
        }
        if observation.scan {
            if self.take_request(&now) {
                self.emit(Event::ShowRequested);
                self.schedule.signal(now_ms, Cause::Requested);
            }
            if self.watch.scan() {
                self.schedule.signal(now_ms, Cause::FileChange);
            }
        }

        let cause = self.schedule.due(now_ms);
        let (changed, written) = match cause {
            Some(_) => self.refresh(clock, &now),
            None => (false, Written::Unchanged),
        };

        Cycle {
            cause,
            changed,
            written,
            sleep_for: self.schedule.sleep_for(now_ms),
        }
    }

    /// Read every provider, decide whether anything moved, and act on the answer.
    fn refresh(&mut self, clock: &dyn Clock, now: &str) -> (bool, Written) {
        let mut limits = self.readers.read(clock, now);
        // The readers learn what to watch by reading: which rollout log is newest today is
        // not knowable until one has been opened.
        self.watch.set_targets(self.readers.watched());
        self.warnings.refreshes += 1;

        let canonical = canonical_form(&limits).ok();
        let changed = match (self.last_canonical.as_deref(), canonical.as_deref()) {
            (Some(previous), Some(current)) => previous != current,
            // Nothing to compare against, or a document that would not serialise: treat it
            // as a change, so the first refresh always lands and a failure is never mistaken
            // for "nothing happened".
            _ => true,
        };

        // `updatedAt` moves with the content. An unchanged document keeps the stamp it
        // earned, in memory as well as on disk, so the two can never disagree.
        limits.updated_at = if changed {
            now.to_owned()
        } else {
            self.snapshot_updated_at()
        };

        // Say we are still here, and find out whether we still may write, **before**
        // writing. A holder that was evicted while its machine slept must not get one more
        // document in over the top of the instance that replaced it.
        self.beat(now);

        let mut written = Written::Unchanged;
        if changed {
            match self.writer.as_mut() {
                Some(writer) => match writer.write_if_changed(&limits, now) {
                    Ok(result) => {
                        if result.changed() {
                            self.warnings.writes += 1;
                        }
                        written = result;
                        self.last_canonical = canonical;
                    }
                    Err(error) => {
                        // A full disk is not a reason to stop reading. The document stays
                        // in memory, the counter goes up, and the next refresh tries again
                        // because `last_canonical` was deliberately not moved.
                        self.warnings.write_failures += 1;
                        self.warnings.last_error = Some(error.to_string());
                    }
                },
                None => self.last_canonical = canonical,
            }
        }

        self.store(Snapshot::new(limits));
        *self
            .published
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = self.warnings();
        // Every pass, then the news. A listener that only cares about news filters; one
        // that has to look at every reading — the notifications do — would otherwise have
        // to poll, and a poll would be a second schedule disagreeing with this one.
        self.emit(Event::Refreshed);
        if changed {
            self.emit(Event::SnapshotChanged);
        }
        (changed, written)
    }

    /// Say the writer is still here, and notice if it has been replaced.
    fn beat(&mut self, now: &str) {
        let Some(lock) = self.lock.as_mut() else {
            return;
        };
        match lock.heartbeat(now) {
            Ok(Heartbeat::Held) => {}
            Ok(Heartbeat::Lost) => {
                // Somebody else is the writer now. Stop writing at once: two writers is the
                // one thing this whole mechanism exists to prevent.
                self.lock = None;
                self.writer = None;
                self.warnings.read_only = true;
                self.warnings.last_error =
                    Some("the advisory lock was taken over by another instance".to_owned());
            }
            Err(error) => {
                self.warnings.lock_failures += 1;
                self.warnings.last_error = Some(error.to_string());
            }
        }
    }

    /// Whether somebody asked for the panel since the last look.
    ///
    /// The file is removed either way: an old one is litter and a fresh one has been acted
    /// on, and leaving either behind would make the next launch ambiguous.
    fn take_request(&self, now: &str) -> bool {
        let Some(path) = self.request.as_deref() else {
            return false;
        };
        let Ok(metadata) = std::fs::metadata(path) else {
            return false;
        };
        let fresh = metadata
            .modified()
            .ok()
            .zip(unix_seconds_from_rfc3339(now))
            .map(|(modified, now)| now - crate::clock::system_time_seconds(modified))
            .is_none_or(|age| age <= REQUEST_MAX_AGE.as_secs() as i64);
        let _ = std::fs::remove_file(path);
        fresh
    }

    fn snapshot_updated_at(&self) -> String {
        self.snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .updated_at()
            .to_owned()
    }

    fn store(&self, snapshot: Snapshot) {
        *self
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = snapshot;
    }

    fn emit(&self, event: Event) {
        if let Some(listener) = &self.listener {
            listener(event);
        }
    }
}

/// Leave a marker asking the running instance to show its panel.
///
/// Written atomically, so the running instance never sees half of it, and written with the
/// current time inside for a human reading `~/.nazar` by hand. Only the file's existence and
/// its modification time are ever read.
pub fn place_request(path: &Path, now: &str) -> Result<()> {
    let body = format!("{{\n  \"requestedAt\": \"{now}\"\n}}\n");
    atomic::write_bytes(path, body.as_bytes())
}

/// What the loop's owner can tell it to do.
pub enum Command {
    /// Refresh at the next pass.
    RefreshNow,
    /// Read different providers from now on, and refresh.
    Reconfigure(Box<(ReaderSet, Rules)>),
    /// Finish and give the engine back.
    Stop,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::RefreshNow => f.write_str("RefreshNow"),
            Command::Reconfigure(_) => f.write_str("Reconfigure"),
            Command::Stop => f.write_str("Stop"),
        }
    }
}

/// A running loop.
#[derive(Debug)]
pub struct LoopHandle {
    commands: Sender<Command>,
    thread: Option<JoinHandle<Engine>>,
}

impl LoopHandle {
    /// Ask for a refresh. `false` when the loop has already stopped.
    pub fn refresh_now(&self) -> bool {
        self.commands.send(Command::RefreshNow).is_ok()
    }

    /// Hand the loop a new set of readers and rules. `false` when it has already stopped.
    ///
    /// The refresh that follows is immediate rather than at the next tick: a user who has
    /// just switched a provider on is looking at the panel, and a minute of nothing would
    /// read as the switch not working.
    pub fn reconfigure(&self, readers: ReaderSet, rules: Rules) -> bool {
        self.commands
            .send(Command::Reconfigure(Box::new((readers, rules))))
            .is_ok()
    }

    /// Stop the loop and take the engine back.
    ///
    /// Returns `None` only when the loop's thread panicked, which would have been a bug in
    /// a reader — the loop itself has no failure path that panics.
    pub fn stop(mut self) -> Option<Engine> {
        let _ = self.commands.send(Command::Stop);
        self.thread.take().and_then(|thread| thread.join().ok())
    }
}

impl Drop for LoopHandle {
    fn drop(&mut self) {
        // Dropping the sender is enough on its own — the loop treats a disconnected channel
        // as a stop — but saying so explicitly means the thread wakes now rather than at the
        // end of its current sleep.
        let _ = self.commands.send(Command::Stop);
    }
}

/// Run an engine on its own thread.
///
/// The thread sleeps for exactly as long as the schedule asks and wakes early for a
/// command, so an idle tray costs one wake-up every five seconds and no busy waiting.
pub fn spawn(engine: Engine, clock: Arc<dyn Clock>) -> std::io::Result<LoopHandle> {
    let (commands, orders) = channel();
    let thread = std::thread::Builder::new()
        .name("nazar-refresh".to_owned())
        .spawn(move || run(engine, &*clock, &orders))?;
    Ok(LoopHandle {
        commands,
        thread: Some(thread),
    })
}

/// The loop body, separated so it reads as the short thing it is.
fn run(mut engine: Engine, clock: &dyn Clock, orders: &Receiver<Command>) -> Engine {
    loop {
        let cycle = engine.tick(clock);
        match orders.recv_timeout(cycle.sleep_for) {
            Ok(Command::RefreshNow) => engine.request_refresh(clock),
            Ok(Command::Reconfigure(change)) => {
                let (readers, rules) = *change;
                engine.apply(readers, rules, clock);
            }
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return engine,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
mod tests;
