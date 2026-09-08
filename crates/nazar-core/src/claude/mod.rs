//! Reading Claude Code's quota out of the status-line captures.
//!
//! Claude Code hands its status-line command a JSON payload on every refresh, and that
//! payload carries the two quota windows the server last reported. `nazar-statusline` — a
//! separate binary in this repository — is installed as that command, writes the payload
//! whole to `~/.nazar/statusline/<session_id>.json`, and then runs whatever command the
//! user had before. This module is the other end: it reads those capture files and turns
//! the numbers into the `providers.claude` block of `limits.json`.
//!
//! ```text
//! ~/.nazar/statusline/
//!   ├─ chain.json                     the wrapper's own state; never a capture
//!   ├─ <session-id>.json  ─┐
//!   └─ <session-id>.json  ─┴─ {"schemaVersion":1,"updatedAt":"…Z","payload":{ … }}
//!                                                                      │
//!        payload.rate_limits.five_hour {used_percentage, resets_at} ────┘
//!        payload.rate_limits.seven_day {used_percentage, resets_at}
//! ```
//!
//! ## Why a directory and not a file
//!
//! Three concurrent Claude Code sessions each refresh their own status line. A wrapper
//! that wrote one fixed path would have them overwrite each other — the mistake the data
//! layer audit found in the prototype that inspired this. So the wrapper keys the file by
//! `session_id`, and this reader takes the newest one.
//!
//! ## What this module will not do
//!
//! A capture file holds the whole payload, and the payload holds paths: `cwd`,
//! `transcript_path`, the workspace and its repository. **None of it leaves this module.**
//! Four values are read — two percentages and two reset times — plus a plan name if a
//! future payload ever carries one, and that goes through the same shape check the Codex
//! reader uses. A leak test puts a sentinel in every string of a capture and fails if it
//! reaches the output.
//!
//! ## What it will not invent
//!
//! `rate_limits` is present only for Pro and Max subscribers, and only after the session's
//! first API response; Claude Code also drops a window once its `resets_at` has passed.
//! Each of those is "we do not know", and each produces a window with **no percentage** —
//! never a reassuring `0`. See `docs/statusline-wrapper.md`.

#[cfg(feature = "detailed-windows")]
pub mod detailed;
#[cfg(feature = "detailed-windows")]
pub mod merge;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::error::Result;
use crate::limits::{Provider, Source, Window};
use crate::paths::statusline_dir;
use crate::timefmt::{
    floor_to_minute, rfc3339_from_system_time, rfc3339_from_unix_seconds, sanitize_plan,
    sanitize_timestamp, unix_seconds_auto, unix_seconds_from_rfc3339,
};

/// Key of the five-hour window in `limits.json`.
pub const WINDOW_FIVE_HOUR: &str = "five_hour";
/// Key of the global weekly window in `limits.json`.
pub const WINDOW_SEVEN_DAY: &str = "seven_day";

/// Length of Claude Code's `five_hour` window, written so consumers need no provider
/// -specific logic. Structure, not a reading: the field names the window itself.
pub const FIVE_HOUR_WINDOW_MINUTES: u32 = 300;
/// Length of Claude Code's `seven_day` window.
pub const SEVEN_DAY_WINDOW_MINUTES: u32 = 10_080;

/// Version of the capture envelope this build writes and understands.
pub const CAPTURE_SCHEMA_VERSION: u64 = 1;

/// The wrapper's own state file. It sits beside the captures and is not one.
pub const CHAIN_FILE_NAME: &str = "chain.json";

/// How many capture files a refresh opens, newest by modification time first.
///
/// One file per Claude Code session, and the wrapper prunes the old ones, so the
/// directory holds a handful. The cap is here so that a directory nobody pruned — a
/// machine where the wrapper was removed by hand, say — costs a bounded read.
pub const MAX_CAPTURES_READ: usize = 16;

/// Reason a window has no percentage, when the payload carried no `rate_limits` at all.
const NO_RATE_LIMITS: &str = "the status-line payload carried no rate_limits (Pro/Max only, and only after the \
     session's first API response)";

/// One window as the payload reported it.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadingWindow {
    /// `used_percentage`, as reported. Never rounded here; rounding is a display decision.
    pub used_percentage: f64,
    /// `resets_at`, already turned into RFC 3339 UTC. Absent when the source omitted it.
    pub resets_at: Option<String>,
}

/// Everything one capture file yields. Four numbers and, one day perhaps, a plan name.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reading {
    /// A plan name, if a future payload ever carries one. See [`derive_plan`].
    pub plan: Option<String>,
    /// The envelope's `updatedAt`: when the wrapper captured the payload.
    pub captured_at: Option<String>,
    /// `rate_limits.five_hour`.
    pub five_hour: Option<ReadingWindow>,
    /// `rate_limits.seven_day`.
    pub seven_day: Option<ReadingWindow>,
    /// Whether `rate_limits` was there at all. `false` and no windows is a documented
    /// state (not a subscriber, or before the first response); `true` and no windows is
    /// a payload whose windows had all reset.
    pub has_rate_limits: bool,
}

impl Reading {
    /// `true` when at least one window carried a percentage.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        self.five_hour.is_some() || self.seven_day.is_some()
    }
}

/// What a refresh found.
#[derive(Debug, Clone, PartialEq)]
enum Status {
    /// No capture directory, or nothing in it. The wrapper is not installed here.
    NotConfigured,
    /// Capture files are there and none of them parsed.
    Unreadable,
    /// A capture, and the time the wrapper wrote it.
    Read(Box<Reading>, String),
}

/// A reader over one capture directory.
///
/// Stateless between refreshes on purpose: the files are tiny, there are a handful of
/// them, and a status line rewrites its capture every few seconds, so there is nothing an
/// offset would save. The Codex reader tails a growing multi-megabyte log; this one reads
/// two kilobytes.
#[derive(Debug, Clone)]
pub struct ClaudeReader {
    dir: PathBuf,
}

impl ClaudeReader {
    /// A reader over an explicit capture directory.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        ClaudeReader { dir: dir.into() }
    }

    /// A reader over `~/.nazar/statusline` (`NAZAR_HOME` moves it).
    pub fn discover() -> Result<Self> {
        Ok(ClaudeReader::new(statusline_dir()?))
    }

    /// The directory this reader is watching.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Read the newest capture and render it as a `limits.json` provider block.
    #[must_use]
    pub fn refresh(&mut self) -> Provider {
        provider(&self.status())
    }

    /// Look for the newest usable capture.
    fn status(&self) -> Status {
        let mut candidates = capture_files(&self.dir);
        if candidates.is_empty() {
            return Status::NotConfigured;
        }
        // Newest by modification time first, then read at most a handful of them.
        candidates.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
        candidates.truncate(MAX_CAPTURES_READ);

        let mut best: Option<(Reading, String)> = None;
        for (path, modified) in candidates {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(reading) = parse_capture(&text) else {
                continue;
            };
            // The envelope's own stamp beats the file's, which a copy or a restore can
            // move. Both are RFC 3339 in UTC, so they compare as text.
            let at = reading
                .captured_at
                .clone()
                .unwrap_or_else(|| rfc3339_from_system_time(modified));
            if best.as_ref().is_none_or(|(_, seen)| at > *seen) {
                best = Some((reading, at));
            }
        }

        match best {
            Some((reading, at)) => Status::Read(Box::new(reading), at),
            None => Status::Unreadable,
        }
    }
}

/// Every `*.json` in `dir` that is a capture, with its modification time.
///
/// `chain.json` is the wrapper's own state and is skipped by name. The atomic writer's
/// temporary files have no `.json` extension, so the filter drops those too.
fn capture_files(dir: &Path) -> Vec<(PathBuf, SystemTime)> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some(CHAIN_FILE_NAME) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        found.push((path, modified));
    }
    found
}

/// Read one capture file.
///
/// An allow-list, not a filter: it names the values it wants and builds a [`Reading`] out
/// of them. Nothing else in the payload is copied anywhere. Returns `None` when the text
/// is not a capture envelope at all, so a stray file in the directory is skipped rather
/// than reported as an empty reading.
#[must_use]
pub fn parse_capture(text: &str) -> Option<Reading> {
    let value: Value = serde_json::from_str(text).ok()?;
    let envelope = value.as_object()?;
    let payload = envelope.get("payload")?.as_object()?;

    let limits = payload.get("rate_limits").and_then(Value::as_object);

    Some(Reading {
        plan: derive_plan(payload),
        captured_at: envelope
            .get("updatedAt")
            .and_then(Value::as_str)
            .and_then(sanitize_timestamp),
        five_hour: limits
            .and_then(|limits| limits.get("five_hour"))
            .and_then(window),
        seven_day: limits
            .and_then(|limits| limits.get("seven_day"))
            .and_then(window),
        has_rate_limits: limits.is_some(),
    })
}

/// A plan name, if the payload names one.
///
/// Today it never does. `model.id` (`claude-fable-5-1`) and `version` (`2.1.263`) say
/// which model and which build, not which subscription, and deriving `max_20x` from
/// either would be a guess — rule 2 of the contract forbids exactly that. So this looks
/// only for a field that says "plan" outright, and returns nothing on every payload shape
/// observed so far. The opt-in detailed-windows mode (WP2b) is where a real plan name
/// comes from.
#[must_use]
pub fn derive_plan(payload: &serde_json::Map<String, Value>) -> Option<String> {
    for key in ["plan", "plan_type", "subscription_type"] {
        if let Some(plan) = payload
            .get(key)
            .and_then(Value::as_str)
            .and_then(sanitize_plan)
        {
            return Some(plan);
        }
    }
    payload
        .get("rate_limits")
        .and_then(Value::as_object)
        .and_then(|limits| limits.get("plan_type"))
        .and_then(Value::as_str)
        .and_then(sanitize_plan)
}

/// Read one window object, or nothing.
///
/// A window with no usable `used_percentage` is no window: an unknown percentage is
/// absent, never zero.
fn window(value: &Value) -> Option<ReadingWindow> {
    let object = value.as_object()?;
    let used_percentage = object.get("used_percentage")?.as_f64()?;
    if !used_percentage.is_finite() {
        return None;
    }
    Some(ReadingWindow {
        used_percentage,
        resets_at: object.get("resets_at").and_then(resets_at_value),
    })
}

/// Read `resets_at` as RFC 3339 text, in the one spelling the contract allows, **rounded
/// down to the whole minute**.
///
/// The two sources disagree about how to write an instant, which is exactly why this is
/// one function rather than two:
///
/// * the **status-line payload** writes Unix seconds (`1788768000`), the same as Codex;
/// * the **usage endpoint** writes text with microseconds and a numeric offset
///   (`2026-09-07T13:10:00.130195+00:00`), observed live on 2026-09-07.
///
/// Both come out as `2026-09-07T13:10:00Z`. An integer too large to be seconds is read as
/// milliseconds and a string that is not a timestamp yields nothing, so a format change
/// degrades rather than breaks.
///
/// ## Why the minute and not the second
///
/// The usage endpoint does not report a stable reset instant. On 2026-09-08 it answered
/// **`2026-09-12T02:00:00Z` and `2026-09-12T01:59:59Z` for the same weekly window**, one
/// refresh apart, and went on alternating between the two for hours. One second of jitter
/// is noise from whatever computes that field on the far side, and **nothing downstream of
/// here has a consumer for sub-minute precision**: the panel draws a countdown in minutes,
/// and [`crate::alerts`] only asks whether two readings name the same period. Flooring both
/// sources to the minute makes them produce the same text for the same instant, and makes
/// the endpoint produce the same text twice running for a reset that has not moved.
///
/// This is the cheap half of the fix. The other half is in [`crate::alerts`], which
/// compares two `resetsAt` values with a tolerance rather than as strings — so a source
/// that jitters by more than a minute, or one that starts jittering tomorrow in a way this
/// rounding does not flatten, still cannot be mistaken for a new week.
pub(crate) fn resets_at_value(value: &Value) -> Option<String> {
    let seconds = if let Some(seconds) = value.as_i64() {
        unix_seconds_auto(seconds)
    } else if let Some(seconds) = value.as_f64() {
        if !seconds.is_finite() || seconds.abs() >= 9e18 {
            return None;
        }
        unix_seconds_auto(seconds as i64)
    } else {
        unix_seconds_from_rfc3339(value.as_str()?)?
    };
    Some(rfc3339_from_unix_seconds(floor_to_minute(seconds)))
}

/// Render a status as the `providers.claude` block of `limits.json`.
fn provider(status: &Status) -> Provider {
    match status {
        // No capture at all: the wrapper is not installed on this machine. The key stays,
        // the windows do not — "not set up" is not "at zero".
        Status::NotConfigured => Provider {
            configured: false,
            ..Provider::default()
        },
        Status::Unreadable => unreadable("no readable capture in the status-line directory"),
        Status::Read(reading, captured_at) => readable(reading, captured_at),
    }
}

/// A provider whose capture files are present but told us nothing.
fn unreadable(reason: &str) -> Provider {
    Provider {
        configured: true,
        source: Some(Source::Statusline),
        windows: error_windows(reason),
        ..Provider::default()
    }
}

/// Both windows, named, with no percentage and a reason.
///
/// Named because a panel that draws two rows should keep drawing two rows while it says
/// it does not know the numbers, and the window lengths are structure rather than a
/// reading.
fn error_windows(reason: &str) -> BTreeMap<String, Window> {
    let mut windows = BTreeMap::new();
    windows.insert(
        WINDOW_FIVE_HOUR.to_owned(),
        Window::error(reason).with_window_minutes(FIVE_HOUR_WINDOW_MINUTES),
    );
    windows.insert(
        WINDOW_SEVEN_DAY.to_owned(),
        Window::error(reason).with_window_minutes(SEVEN_DAY_WINDOW_MINUTES),
    );
    windows
}

/// A provider built from a capture.
fn readable(reading: &Reading, captured_at: &str) -> Provider {
    // A payload with no `rate_limits` block is the documented "we cannot know" case: the
    // user is not on Pro or Max, or the session has not had its first API response yet.
    // Both windows say so, and neither carries a number.
    let windows = if reading.has_rate_limits {
        let mut windows = BTreeMap::new();
        for (key, minutes, reported) in [
            (
                WINDOW_FIVE_HOUR,
                FIVE_HOUR_WINDOW_MINUTES,
                reading.five_hour.as_ref(),
            ),
            (
                WINDOW_SEVEN_DAY,
                SEVEN_DAY_WINDOW_MINUTES,
                reading.seven_day.as_ref(),
            ),
        ] {
            // A window the payload did not carry is absent, not an error and not a zero.
            // Claude Code drops a window once it has reset, and "there is no five-hour
            // window in this payload" is a different sentence from "it is at 0 %".
            let Some(reported) = reported else { continue };
            let mut rendered = Window::ok(reported.used_percentage).with_window_minutes(minutes);
            if let Some(resets_at) = &reported.resets_at {
                rendered = rendered.with_resets_at(resets_at.as_str());
            }
            windows.insert(key.to_owned(), rendered);
        }
        windows
    } else {
        error_windows(NO_RATE_LIMITS)
    };

    Provider {
        configured: true,
        plan: reading.plan.clone(),
        source: Some(Source::Statusline),
        source_at: Some(captured_at.to_owned()),
        binding: binding(&windows),
        windows,
        ..Provider::default()
    }
}

/// Key of the window with the highest percentage, over **every** window there is.
///
/// Computed, never taken from a flag — the status-line payload has none, the usage
/// endpoint's `is_active` is not read, and the retired prototype inventing one is finding
/// B04 of the code audit. Written to iterate the map rather than a fixed pair of keys,
/// because in detailed mode the provider carries model-scoped weeklies too and the one
/// that constrains a Fable-heavy Max user is exactly the one a fixed list would miss.
///
/// Ties are broken by the shorter window first and then by key, so the answer is the same
/// on every run: when two windows are equally full, the one that resets sooner is the one
/// you hit first.
///
/// One implementation, in [`crate::state::binding`], shared with the Codex reader and with
/// the derived view — because three displays that each computed it themselves is finding
/// B16 of the audit.
pub(crate) fn binding(windows: &BTreeMap<String, Window>) -> Option<String> {
    crate::state::binding(windows)
}

#[cfg(test)]
mod tests;
