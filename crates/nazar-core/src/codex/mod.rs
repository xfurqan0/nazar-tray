//! Reading Codex's quota out of its own session logs.
//!
//! Codex is handed its rate limits by the server on every response and writes them
//! straight into the session log it is already keeping. That is the whole reason
//! nazar-tray needs no token and no network for this provider: the numbers are already on
//! disk, in a file the user's own tool wrote, and reading them is reading a log.
//!
//! ```text
//! $CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ISO>-<uuid>[_<uuid>].jsonl
//!   └─ {"timestamp":…,"type":"event_msg","payload":{"type":"token_count",
//!         "rate_limits":{"plan_type":"plus",
//!            "primary":  {"used_percent":54.0,"window_minutes":300,  "resets_at":…},
//!            "secondary":{"used_percent":70.0,"window_minutes":10080,"resets_at":…}}}}
//! ```
//!
//! `CODEX_HOME` defaults to `~/.codex`. Codex writes a quota line dozens of times per
//! session (13 in the 118-line log this module's fixtures came from), so the newest line
//! of the newest log is never far behind the truth while a session is running, and is the
//! last thing the server said when one is not.
//!
//! ## What this module will not do
//!
//! It opens rollout logs and nothing else. Codex keeps a sign-in file next to them; this
//! reader has no code path that names it, and `tests/hygiene.rs` greps the whole
//! workspace to keep it that way. `docs/pinned-internal-formats.md` lists the files that
//! are off limits by name, in a document rather than in the source, so that the grep
//! stays sound. It also never copies text out of a rollout line: a line holds the user's
//! prompt, the model's reasoning and the output of every command it ran, and the seven
//! values named in [`parse`] are the only things that leave it.
//!
//! ## What it will not invent
//!
//! A window whose percentage could not be read has no percentage. Not zero — the audit's
//! first red finding was a prototype that drew a reassuring blue `0` when it had failed to
//! read anything at all. The binding window is the highest percentage of the windows that
//! were read, computed here, never taken from a flag: Codex's payload has no such flag,
//! and the prototype inventing one is why its own note called the 52 % window binding
//! while a 70 % window sat next to it.
//!
//! ## What it will not keep pretending
//!
//! A rollout log is the last thing the server said, and Codex only ever says anything while
//! it is running. On a machine nobody has opened Codex on for two days the newest log still
//! parses perfectly and still reports `70 %` — for a window that reset yesterday. That is
//! the reading being **out of date**, not the file being unreadable, and the contract has a
//! word for it: a window whose `resets_at` is more than [`RESET_GRACE_SECONDS`] behind the
//! current instant is written `state: "stale"`, keeps its percentage, and stops counting as
//! current for anything downstream — no threshold notification, and a panel that draws it
//! the way it draws every other stale window. See [`readable`].
//!
//! The grace period is what keeps that from firing on a window that has *just* turned over:
//! the log is appended a few seconds after the reset with the new period's numbers, and a
//! reading in that gap is late rather than wrong. **This rule is the Codex reader's alone.**
//! Claude Code's `five_hour` window crosses its own reset every day while a session is open
//! and is re-reported seconds later, so the same rule there would blink a perfectly live
//! window grey once a day.

pub mod locate;
pub mod parse;
pub mod tail;
pub mod zst;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::Result;
use crate::limits::{Provider, Source, Window, WindowState};
use crate::paths::home_dir;
use crate::timefmt::{now_rfc3339, rfc3339_from_system_time, unix_seconds_from_rfc3339};

use locate::MAX_FILES_OPENED;
use parse::{Outcome, Quota};

/// Key of the five-hour window in `limits.json`, named after the field in the log.
pub const WINDOW_PRIMARY: &str = "primary";
/// Key of the weekly window in `limits.json`, named after the field in the log.
pub const WINDOW_SECONDARY: &str = "secondary";

/// Length of Codex's `primary` window. Constant across all 326 lines observed.
pub const PRIMARY_WINDOW_MINUTES: u32 = 300;
/// Length of Codex's `secondary` window. Constant across all 326 lines observed.
pub const SECONDARY_WINDOW_MINUTES: u32 = 10_080;

/// Environment variable that moves Codex's home directory.
pub const CODEX_HOME_VAR: &str = "CODEX_HOME";

/// How far past its own reset a window may be and still count as current.
///
/// Five minutes. A window that has just turned over is re-reported within seconds of the
/// next thing Codex does, so the honest reading of a reset that passed thirty seconds ago
/// is "the log has not caught up yet" rather than "these numbers are from last week". A
/// reset that passed **an hour** ago means nobody has run Codex since, and the percentage
/// standing on it is about a period that has ended.
pub const RESET_GRACE_SECONDS: i64 = 5 * 60;

/// `$CODEX_HOME`, or `~/.codex` when it is not set.
///
/// Fails only when neither `CODEX_HOME` nor a home directory can be found, which is a
/// state to report rather than guess around.
pub fn codex_home() -> Result<PathBuf> {
    let variable = std::env::var_os(CODEX_HOME_VAR);
    match resolve_home(variable.as_deref()) {
        Some(home) => Ok(home),
        None => Ok(home_dir()?.join(".codex")),
    }
}

/// The override half of [`codex_home`], separated so it can be tested.
///
/// Changing the process environment from a test is unsound once anything else in the
/// program reads it, and a test suite runs in threads. Splitting the decision out means
/// the rule is checked directly instead of by mutating a global.
fn resolve_home(variable: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    // An empty variable is not a directory. Treating it as one would point the reader at
    // the process's working directory, which is nobody's Codex home.
    variable
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// What a refresh found.
#[derive(Debug, Clone, PartialEq)]
enum Status {
    /// Codex's home directory is not on this machine.
    NoHome,
    /// The home directory is there but holds no session log.
    NoLogs,
    /// Every rollout here is compressed and not one of them could be decoded.
    ///
    /// A state of its own rather than a kind of [`Status::NoLogs`], because it is the
    /// opposite sentence: a machine nobody has opened Codex on for a fortnight has a
    /// **full** `sessions/` tree of `.jsonl.zst`, and answering "no rollout log" there
    /// tells the user they have never run Codex.
    ///
    /// Since T-WP26 this is a narrow state rather than the normal one. These files are read
    /// now ([`zst`]), so reaching here means the decoder refused every one of them — a
    /// truncated archive, bit rot, a frame this decoder does not implement, or a format
    /// that has moved. The sentence says exactly that and no more.
    CompressedOnly,
    /// Logs are there, but none of the ones we opened carried a usable quota line.
    NoQuotaLine,
    /// A reading, with the time the source produced it.
    Read(Box<Quota>, String),
}

/// A stateful reader over one Codex home directory.
///
/// Holds a byte offset into the log it is following so that a poll costs a directory
/// listing and the bytes that arrived since last time, not the whole file. Keeps the last
/// reading it managed, so a session that has ended still reports the last thing the
/// server said rather than nothing — with `sourceAt` saying how old that is. Deciding
/// when "old" becomes "stale" is the state model's job (WP3), not this module's.
#[derive(Debug)]
pub struct CodexReader {
    home: PathBuf,
    tail: Option<tail::Tail>,
    latest: Option<(Quota, String)>,
    /// Candidate set the widened scan last ran over, so it does not run again over an
    /// unchanged set of logs. Without this a machine whose newest logs genuinely hold no
    /// quota line would read five files end to end on every poll, for ever.
    scanned: Option<Vec<(PathBuf, SystemTime)>>,
    /// Compressed candidates the last widened scan could not decode.
    ///
    /// Kept beside [`CodexReader::scanned`] and for the same reason: the scan runs once per
    /// candidate set, so the question "could these files be opened at all" is answered once
    /// and then remembered, rather than re-asked on a poll that deliberately does no work.
    undecodable: usize,
    warnings: u64,
}

impl CodexReader {
    /// A reader over an explicit Codex home directory.
    #[must_use]
    pub fn new(home: impl Into<PathBuf>) -> Self {
        CodexReader {
            home: home.into(),
            tail: None,
            latest: None,
            scanned: None,
            undecodable: 0,
            warnings: 0,
        }
    }

    /// A reader over `$CODEX_HOME`, or `~/.codex`.
    pub fn discover() -> Result<Self> {
        Ok(CodexReader::new(codex_home()?))
    }

    /// The directory this reader is watching.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The rollout log this reader is currently following, once it has found one.
    ///
    /// The refresh loop watches it, and the directory it sits in, so that an appended quota
    /// line is noticed within five seconds instead of at the next minute. Nothing else needs
    /// this: it is a fact about the reader's own progress, not about the numbers.
    #[must_use]
    pub fn following(&self) -> Option<&Path> {
        self.tail.as_ref().map(tail::Tail::path)
    }

    /// How many lines have been skipped for being malformed since the reader was made.
    ///
    /// A steady rise here means the log format has moved and the pinned notes in
    /// `docs/pinned-internal-formats.md` need revisiting.
    #[must_use]
    pub fn warnings(&self) -> u64 {
        self.warnings
    }

    /// Read the newest quota and render it as a `limits.json` provider block, for `now`.
    ///
    /// `now` is RFC 3339, and it is a parameter rather than a clock reading because it is
    /// the one thing here that is not on disk: whether a window is past its reset is a
    /// question about the current instant, and the refresh loop already holds the answer
    /// (`crate::refresh::sources::Reader::read`). A `now` this function cannot parse leaves
    /// every window exactly as the log reported it, which is the same rule the rest of the
    /// crate follows when it cannot read a time.
    #[must_use]
    pub fn refresh_at(&mut self, now: &str) -> Provider {
        provider(&self.status(), now)
    }

    /// [`CodexReader::refresh_at`] against the system clock, for a caller that has none.
    ///
    /// The one-shot `--print` path, which reads the clock once and exits. Everything that
    /// runs in a loop passes its own instant in.
    #[must_use]
    pub fn refresh(&mut self) -> Provider {
        self.refresh_at(&now_rfc3339())
    }

    /// Look for the newest usable quota line, updating the reader's state.
    fn status(&mut self) -> Status {
        if !self.home.is_dir() {
            // Codex was uninstalled, or never installed. Anything we remember is about a
            // machine that no longer exists.
            self.tail = None;
            self.latest = None;
            self.scanned = None;
            self.undecodable = 0;
            return Status::NoHome;
        }

        let found = locate::find_rollouts(&self.home, MAX_FILES_OPENED);
        let candidates = found.candidates;
        let Some(newest) = candidates.first() else {
            self.tail = None;
            return Status::NoLogs;
        };

        // Follow the newest log incrementally; start over when a newer one appears. Only a
        // plain log is followed: an archive is a log Codex has not touched for seven days,
        // so there is nothing for a tail to wait for, and a zstd stream has no offset for
        // one to hold anyway. When the newest candidate is an archive the tail is put down
        // entirely and the widened scan below reads the candidates in order, which is the
        // order that puts the archive first.
        if newest.compressed {
            self.tail = None;
        } else {
            let following = self
                .tail
                .as_ref()
                .is_some_and(|tail| tail.path() == newest.path);
            if !following {
                self.tail = Some(tail::Tail::new(&newest.path, tail::INITIAL_WINDOW));
            }
            if let Some(reading) = self.poll_current(newest.modified) {
                self.latest = Some(reading);
            }
        }

        if self.latest.is_none() {
            // Nothing yet, from this reader's whole life. Widen the search once per
            // candidate set: the tail window covers the last quarter megabyte, and a log
            // can hold its only quota line further back, or hold none at all (three of
            // eighteen on the maintainer's machine are stubs from sessions that ended
            // immediately).
            let fingerprint: Vec<(PathBuf, SystemTime)> = candidates
                .iter()
                .map(|candidate| (candidate.path.clone(), candidate.modified))
                .collect();
            if self.scanned.as_ref() != Some(&fingerprint) {
                let found = self.scan_candidates(&candidates);
                self.scanned = Some(fingerprint);
                self.latest = found;
            }
        }

        match &self.latest {
            Some((quota, source_at)) => Status::Read(Box::new(quota.clone()), source_at.clone()),
            // "No quota line" is a claim about what the logs said, so it may only be made
            // about logs that were read. When every candidate is an archive and the decoder
            // refused them, nothing here has read a line at all.
            None if self.undecodable > 0 && candidates.iter().all(|one| one.compressed) => {
                Status::CompressedOnly
            }
            None => Status::NoQuotaLine,
        }
    }

    /// Read whatever arrived in the log we are following.
    fn poll_current(&mut self, modified: SystemTime) -> Option<(Quota, String)> {
        let polled = self.tail.as_mut()?.poll(parse::NEEDLE);
        let batch = match polled {
            Ok(batch) => batch,
            Err(_) => {
                // The file went away between the listing and the read: a session ended
                // and something tidied up. Next poll re-lists and picks the new newest.
                self.warnings += 1;
                self.tail = None;
                return None;
            }
        };
        self.warnings += batch.dropped;
        newest_usable(&batch.lines, modified, &mut self.warnings)
    }

    /// Widen the search: one full pass over each candidate, newest first.
    ///
    /// Opens at most [`MAX_FILES_OPENED`] distinct files in total, counting the one the
    /// tail already has open.
    fn scan_candidates(&mut self, candidates: &[locate::Candidate]) -> Option<(Quota, String)> {
        self.undecodable = 0;
        for candidate in candidates.iter().take(MAX_FILES_OPENED) {
            let lines = if candidate.compressed {
                match self.decode(&candidate.path) {
                    Some(lines) => lines,
                    None => continue,
                }
            } else {
                let mut pass = tail::Tail::new(&candidate.path, tail::FULL_WINDOW);
                let Ok(batch) = pass.poll(parse::NEEDLE) else {
                    self.warnings += 1;
                    continue;
                };
                self.warnings += batch.dropped;
                batch.lines
            };
            if let Some(reading) = newest_usable(&lines, candidate.modified, &mut self.warnings) {
                return Some(reading);
            }
        }
        None
    }

    /// Decode one archived rollout, keeping the lines that could hold a quota.
    ///
    /// The filter is the same needle the tail applies, applied to the same bytes, so a log
    /// read through the decoder and the same log read plain reach [`newest_usable`] with
    /// the same lines in the same order. That is the property the fixture pair exists to
    /// hold: `rollout-sample.jsonl` and `rollout-sample.jsonl.zst` are one file.
    fn decode(&mut self, path: &Path) -> Option<Vec<String>> {
        let mut lines = Vec::new();
        let mut warnings = 0u64;
        let decoded = zst::read_lines(path, &mut |line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() || !tail::contains(line, parse::NEEDLE) {
                return;
            }
            match std::str::from_utf8(line) {
                Ok(text) => lines.push(text.to_owned()),
                // A log with bytes that are not UTF-8 is a log we do not understand.
                // Counted, never guessed at.
                Err(_) => warnings += 1,
            }
        });
        self.warnings += warnings;
        match decoded {
            Ok(decoded) => {
                self.warnings += decoded.dropped;
                Some(lines)
            }
            Err(_) => {
                self.warnings += 1;
                self.undecodable += 1;
                None
            }
        }
    }
}

/// The newest usable quota among `lines`, with the time the source produced it.
///
/// "Usable" excludes a quota line whose windows are all null — Codex writes those for a
/// second limit family — so a null line arriving last does not erase a good reading from
/// a few lines earlier.
fn newest_usable(
    lines: &[String],
    modified: SystemTime,
    warnings: &mut u64,
) -> Option<(Quota, String)> {
    let mut found: Option<Quota> = None;
    for line in lines {
        match parse::parse_line(line) {
            Outcome::Quota(quota) => found = Some(*quota),
            Outcome::Malformed => *warnings += 1,
            Outcome::Empty | Outcome::Other => {}
        }
    }
    let quota = found?;
    // The line's own timestamp is what the source stamped on the numbers. Without one,
    // the file's modification time is the closest honest answer.
    let source_at = quota
        .source_at
        .clone()
        .unwrap_or_else(|| rfc3339_from_system_time(modified));
    Some((quota, source_at))
}

/// Render a status as the `providers.codex` block of `limits.json`, as of `now`.
fn provider(status: &Status, now: &str) -> Provider {
    match status {
        Status::NoHome => Provider {
            configured: false,
            ..Provider::default()
        },
        Status::NoLogs => unreadable("no rollout log in the Codex session directory"),
        Status::CompressedOnly => {
            unreadable("every rollout here is zstd-compressed and none of them could be decoded")
        }
        Status::NoQuotaLine => unreadable(&format!(
            "no rate_limits line in the newest {MAX_FILES_OPENED} rollouts"
        )),
        Status::Read(quota, source_at) => readable(quota, source_at, now),
    }
}

/// A provider whose files are present but told us nothing.
///
/// Both windows are named, because a consumer that draws two rows should keep drawing
/// two rows, and both are `error` with **no percentage at all**. The window lengths are
/// the ones Codex uses, which is structure rather than a reading: they were the same on
/// every one of the 652 windows observed, and they let a panel label the rows correctly
/// while it says it does not know the numbers.
fn unreadable(reason: &str) -> Provider {
    let mut windows = BTreeMap::new();
    windows.insert(
        WINDOW_PRIMARY.to_owned(),
        Window::error(reason).with_window_minutes(PRIMARY_WINDOW_MINUTES),
    );
    windows.insert(
        WINDOW_SECONDARY.to_owned(),
        Window::error(reason).with_window_minutes(SECONDARY_WINDOW_MINUTES),
    );
    Provider {
        configured: true,
        source: Some(Source::Rollout),
        windows,
        ..Provider::default()
    }
}

/// A provider built from a reading, as of `now`.
///
/// The percentages are the source's; the only judgement made here is the one the source
/// cannot make about itself — whether the period the numbers belong to is still running.
/// A window past its reset by more than [`RESET_GRACE_SECONDS`] is `stale`: it **keeps its
/// percentage**, because that is still the last thing the server said and rule 2 of the
/// contract is about not inventing numbers rather than about hiding them, and it stops
/// being current, because a consumer that draws it as a live reading is telling the user
/// they have used 70 % of a week that ended yesterday.
fn readable(quota: &Quota, source_at: &str, now: &str) -> Provider {
    let now_seconds = unix_seconds_from_rfc3339(now);
    let mut windows = BTreeMap::new();
    for (key, window) in [
        (WINDOW_PRIMARY, quota.primary.as_ref()),
        (WINDOW_SECONDARY, quota.secondary.as_ref()),
    ] {
        // A window the source did not report is absent, not an error and not a zero.
        // "There is no weekly window in this payload" and "the weekly window is at 0 %"
        // are different sentences and the file must not conflate them.
        let Some(reported) = window else { continue };
        let mut rendered = Window::ok(reported.used_percent);
        if let Some(minutes) = reported.window_minutes {
            rendered = rendered.with_window_minutes(minutes);
        }
        if let Some(resets_at) = &reported.resets_at {
            rendered = rendered.with_resets_at(resets_at.as_str());
            if past_its_reset(resets_at, now_seconds) {
                rendered.state = WindowState::Stale;
                rendered.error = Some(format!(
                    "the window reset at {resets_at} and Codex has written nothing since"
                ));
            }
        }
        windows.insert(key.to_owned(), rendered);
    }

    Provider {
        configured: true,
        plan: quota.plan.clone(),
        source: Some(Source::Rollout),
        source_at: Some(source_at.to_owned()),
        binding: binding(&windows),
        windows,
        ..Provider::default()
    }
}

/// Whether a reset is far enough behind `now` that the reading standing on it is over.
///
/// Strictly more than [`RESET_GRACE_SECONDS`], so a reset exactly five minutes old is still
/// within the grace period rather than one second outside it — an inclusive boundary in the
/// other direction would make the constant's name a lie by a second.
///
/// Answers `false` when either side is not a timestamp. That is the crate's standing rule
/// for a time it cannot read: a `now` the caller could not produce, or a `resets_at` a
/// future Codex spells some other way, is not evidence that a window has expired, and
/// greying out a live reading because of it would be the worse of the two mistakes.
fn past_its_reset(resets_at: &str, now_seconds: Option<i64>) -> bool {
    let (Some(resets), Some(now)) = (unix_seconds_from_rfc3339(resets_at), now_seconds) else {
        return false;
    };
    now - resets > RESET_GRACE_SECONDS
}

/// Key of the window with the highest percentage.
///
/// The provider's binding window is a computed fact, never a flag: the Codex payload has
/// no field that says which window constrains you, and the retired prototype inventing
/// one (`active: key == "primary_window"`) is finding B04 of the code audit — it showed a
/// 52 % window in bold as "the one limiting you" while a 70 % window sat beside it.
///
/// Ties go to `primary`, the shorter window: when the five-hour and the weekly window
/// are equally full, the five-hour one is what you hit first. That rule is the same for
/// every provider, so it lives in [`crate::state::binding`] and this is a name for it.
fn binding(windows: &BTreeMap<String, Window>) -> Option<String> {
    crate::state::binding(windows)
}

#[cfg(test)]
mod tests;
