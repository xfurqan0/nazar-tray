//! Threshold notifications: who gets told what, and exactly once.
//!
//! `docs/PROJECT.md` section 1 puts this second on the list of things nobody else ships —
//! *"being warned at 85 % before hitting the wall is the reason a quota tray exists"* — and
//! the acceptance criterion for WP5 is one sentence: **the toast appears exactly once when
//! you cross 85 %, and it survives sleep and wake.** Everything in this module is that
//! sentence taken apart.
//!
//! ```text
//!   percent   0 ─────── 60 ────────── 85 ──── 100
//!                       │              │        │
//!   80 → 86             │  crossed ────┘        │      one toast
//!   86 → 86             │  already fired        │      silence
//!   restart at 86       │  alerts.json says so  │      silence
//!   asleep 9 h, 40 → 91 │  crossed ─────────────┘      one toast, the highest
//!   reset, 86 again     │  the keys are cleared │      one toast
//! ```
//!
//! Five rules, and each one is a way this goes wrong if it is left out.
//!
//! 1. **Edge-triggered, not level-triggered.** A toast fires when a window *crosses* a
//!    threshold — `previous < T ≤ current` — not while it is above one. A level test would
//!    fire every sixty seconds for the rest of the week.
//! 2. **Once per threshold per reset period.** The key is
//!    `(provider, window, threshold, resetsAt)` and it is written to
//!    `%APPDATA%\nazar\alerts.json`, so **restarting the tray does not re-fire**. That file
//!    is the difference between a warning and a nuisance, and it is why this module has a
//!    disk format at all.
//! 3. **The first observation counts.** A tray started when the weekly window is already at
//!    91 % has no previous reading, and the honest thing is to say so once and then be
//!    quiet — not to wait for a crossing that has already happened. So "no previous
//!    reading" is treated as "below", and rule 2 is what stops it repeating.
//! 4. **A reset clears the window's keys, and its memory** — but a `resetsAt` that *moved*
//!    is not the same thing as a `resetsAt` that **renewed**. A new period clears the fired
//!    set and the remembered percentage, otherwise a window that reset from 86 % straight
//!    back to 86 % would look like no crossing at all, which is exactly the case the
//!    `--demo-cross` run in `docs/PROJECT.md` exists to catch. So the question "is this the
//!    same period" has to be answered by `same_period` rather than by comparing two
//!    strings, because a source can spell the same instant two ways one refresh apart:
//!
//!    > **The bug this rule was rewritten for.** On **2026-09-08**, between 19:16 and
//!    > 21:43, the maintainer's machine showed **32 identical toasts** — *Claude Code ·
//!    > weekly window 60 %* — one every five minutes, most of them twice. The Anthropic
//!    > usage endpoint was reporting the same weekly reset as `2026-09-12T02:00:00Z` and
//!    > `2026-09-12T01:59:59Z` on alternating refreshes. **One second.** String equality
//!    > read every flip as a new week, cleared `fired`, dropped `previous`, and re-fired 60
//!    > as a first observation (rule 3); the doubles were the swing landing in both
//!    > directions inside one evaluation pair. `alerts.json` was rewritten every five
//!    > minutes for two and a half hours.
//!
//!    A period that genuinely renews moves its reset **forward by a whole window**. So two
//!    `resetsAt` values name the same period when they are less than half a window apart —
//!    or, when the window's length is not known, less than an hour. Text that will not
//!    parse falls back to the old string comparison, because there is nothing else to
//!    compare. `crate::claude::resets_at_value` rounds the endpoint's answer down to the
//!    minute as well, which flattens this particular source; the tolerance here is what
//!    holds when the next source jitters by more than that.
//! 5. **Unknown never notifies.** A window with no percentage, or one in `error`, produces
//!    nothing and *forgets* what it last saw, so the reading that comes after it is a first
//!    observation rather than a continuation of a number nobody can vouch for. Finding B03
//!    of the audit, arriving through a new door.
//!
//! **One toast per crossing, naming the most severe threshold reached.** A window that goes
//! from 10 % to 91 % in one step crosses 60 and 85 together; two toasts stacked on top of
//! each other say less than one that says 85 %. Both thresholds are marked as fired — 60
//! will not fire later in the same period — and only the higher one is shown. The lower
//! ones are *consumed*, not skipped.
//!
//! **Quiet hours suppress the toast and keep the key** — and so does the notifications
//! switch being off, which reaches this module as the same flag. [`Alert::suppressed`] says
//! a crossing happened and was not shown; the caller still colours the tray icon, because
//! the icon is the state and the toast is the interruption. A suppressed alert is not shown
//! later either: that is what suppressing means, and a three-in-the-morning warning
//! delivered at eight is about a number that has moved on. It also means that switching the
//! notifications back on is quiet rather than a burst of catching up.
//!
//! Nothing here reads a clock, opens a socket or knows what a notification is. It takes a
//! derived view and the rules, and answers with a list. [`crate::state`] is the same shape
//! of idea: the numbers are somebody else's, the meaning is worked out here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::atomic;
use crate::error::{Error, Result};
use crate::state::{SnapshotView, Thresholds, WindowView};
use crate::timefmt::unix_seconds_from_rfc3339;

/// Version of the `alerts.json` document this build writes.
pub const ALERTS_SCHEMA_VERSION: u32 = 1;

/// How long after a window's reset its record is swept up.
///
/// A window whose reset was two days ago is not coming back under that key: either the
/// source stopped reporting it or the whole provider is gone. Keeping the record would
/// grow the file for ever, and dropping it earlier would risk forgetting a window that a
/// temporarily unreadable provider is about to report again.
pub const RECORD_MAX_AGE_SECONDS: i64 = 48 * 3600;

/// Two percentages that are the same number.
///
/// Thresholds make a round trip through JSON on every write, and `60.0` comes back as
/// exactly `60.0` — but a settings file written by hand can say `85.00000000000001`, and a
/// toast that fires twice because of the fifteenth decimal place would be a very silly bug.
const EPSILON: f64 = 1e-9;

fn same(left: f64, right: f64) -> bool {
    (left - right).abs() < EPSILON
}

/// How far two `resetsAt` values may sit apart and still be one period, when the window's
/// length is not known.
///
/// An hour: long enough to swallow any jitter a source has been seen to produce, and far
/// short of the shortest window anything here reports (five hours), so a real renewal of
/// even the shortest window is still unambiguously a new period.
const UNKNOWN_WINDOW_TOLERANCE_SECONDS: i64 = 3600;

/// Whether two `resetsAt` readings name the same reset period.
///
/// Rule 4's comparison, and the answer to the toast storm in this module's header. The
/// order matters:
///
/// 1. **The same text is the same period**, including two `None`s — a window that has never
///    carried a `resetsAt` is one period as far as this file is concerned.
/// 2. **One side missing is not.** A window that had a reset and now has none is telling us
///    something changed, and the safe reading of "something changed" is a new period.
/// 3. **Two instants close together are.** Less than half a window apart when the length is
///    known, less than [`UNKNOWN_WINDOW_TOLERANCE_SECONDS`] when it is not. Half a window,
///    because a period that renews moves its reset forward by a whole one: the gap is
///    either a rounding wobble or the length of the window, never anything in between.
/// 4. **Text that will not parse falls back to step 1**, which is where this rule was
///    before: with nothing to subtract there is nothing to be tolerant with.
fn same_period(left: Option<&str>, right: Option<&str>, window_minutes: Option<u32>) -> bool {
    if left == right {
        return true;
    }
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    let (Some(left), Some(right)) = (
        unix_seconds_from_rfc3339(left),
        unix_seconds_from_rfc3339(right),
    ) else {
        return false;
    };
    let tolerance = window_minutes
        .filter(|minutes| *minutes > 0)
        .map_or(UNKNOWN_WINDOW_TOLERANCE_SECONDS, |minutes| {
            i64::from(minutes) * 60 / 2
        });
    left.abs_diff(right) < tolerance.unsigned_abs()
}

/// What one window has already been warned about, as `alerts.json` stores it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowRecord {
    /// The reset this record belongs to. A different one means a new period.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    /// Thresholds already consumed in this period, ascending.
    #[serde(default)]
    pub fired: Vec<f64>,
    /// Keys a newer build wrote. Preserved verbatim, like everywhere else.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

/// The whole `alerts.json` document.
///
/// Keyed `"<provider>/<window>"` — `claude/seven_day_fable`, `codex/secondary`. Neither a
/// provider name nor a window key contains a slash in the `limits.json` contract, so the
/// two halves can always be told apart again by a human reading the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertLog {
    /// Document version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// One record per window that has fired something.
    #[serde(default)]
    pub windows: BTreeMap<String, WindowRecord>,
    /// Keys a newer build wrote.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

fn default_schema_version() -> u32 {
    ALERTS_SCHEMA_VERSION
}

impl Default for AlertLog {
    fn default() -> Self {
        AlertLog {
            schema_version: ALERTS_SCHEMA_VERSION,
            windows: BTreeMap::new(),
            extra: Map::new(),
        }
    }
}

impl AlertLog {
    /// Parse a document.
    pub fn from_json(text: &str) -> Result<Self> {
        Ok(serde_json::from_str(text)?)
    }

    /// Render it the way it is written: pretty, with a trailing newline.
    pub fn to_json(&self) -> Result<String> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }
}

/// Everything the state machine needs that is not in the view.
///
/// The default is the shipped thresholds with nothing suppressed, which is what a machine
/// nobody has configured evaluates a crossing against.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AlertRules {
    /// The percentages a crossing is measured against. `60 / 85 / 100` by default.
    pub thresholds: Thresholds,
    /// Whether a crossing found now is **not to be shown**.
    ///
    /// Two settings arrive here as one flag, because they mean the same thing to this
    /// module: the notifications switch being off, and the local clock being inside the
    /// user's quiet hours. Either way the crossing still happens, is still recorded and
    /// still colours the tray icon; only the interruption is withheld.
    ///
    /// A `bool` rather than the settings themselves, because "what time is it *locally*" is
    /// a question the standard library cannot answer and this crate deliberately does not
    /// try to (`docs/pinned-internal-formats.md`, "Times are written in UTC"). The caller
    /// knows; [`crate::config::Config::alert_rules`] does the arithmetic once it has looked.
    pub quiet: bool,
}

impl AlertRules {
    /// The thresholds, ascending, deduplicated, and with the nonsense removed.
    ///
    /// Sorted here rather than trusted from the settings: [`crate::config::Config::validate`]
    /// refuses to *save* thresholds that are out of order, but a file edited by hand has
    /// never been through it, and a ladder that is not ascending would let the higher toast
    /// fire before the lower one had been consumed.
    #[must_use]
    pub fn ladder(&self) -> Vec<f64> {
        let mut steps: Vec<f64> = [
            self.thresholds.warn,
            self.thresholds.critical,
            self.thresholds.exhausted,
        ]
        .into_iter()
        .filter(|value| value.is_finite() && *value > 0.0)
        .collect();
        steps.sort_by(f64::total_cmp);
        steps.dedup_by(|left, right| same(*left, *right));
        steps
    }
}

/// A crossing that just happened.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    /// `claude` or `codex`.
    pub provider: String,
    /// Window key: `five_hour`, `seven_day_fable`, `secondary`, …
    pub window: String,
    /// Length of the window in minutes, for a display that names it by its length.
    pub window_minutes: Option<u32>,
    /// Model a weekly window is scoped to, when the detailed mode read one.
    pub model: Option<String>,
    /// The threshold that was crossed — the highest of them, when several were.
    pub threshold: f64,
    /// The percentage that crossed it.
    pub percent: f64,
    /// Milliseconds until the window resets, when the source said.
    pub remaining_ms: Option<i64>,
    /// Whether quiet hours mean this one is not to be shown.
    ///
    /// It still happened, it is still recorded, and the tray icon still changes colour.
    /// Only the interruption is withheld.
    pub suppressed: bool,
}

/// The state machine, plus the file it remembers itself in.
///
/// One instance per process, owned by whatever shows the notifications. It is not `Sync`
/// by accident of its fields but by intent: two of these evaluating the same view would
/// both decide a threshold had been crossed, which is the double toast this whole module
/// exists to prevent.
#[derive(Debug, Default)]
pub struct Alerts {
    log: AlertLog,
    /// The last percentage seen for each window and the reset it belonged to.
    ///
    /// **In memory only, and that is deliberate.** Persisting it would make a restart look
    /// like a continuation, and the whole of rule 3 is that it is not one: after a restart
    /// there is no previous reading, and the fired set is what keeps the first observation
    /// from repeating a toast the user has already had.
    seen: BTreeMap<String, (f64, Option<String>)>,
    path: Option<PathBuf>,
    dirty: bool,
}

impl Alerts {
    /// A state machine that remembers nothing between runs. The tests' way in.
    #[must_use]
    pub fn in_memory() -> Self {
        Alerts::default()
    }

    /// A state machine backed by a file.
    ///
    /// A file that is missing is an empty log — nobody has crossed anything yet. A file
    /// that is **damaged** is also an empty log, and unlike `config.json` that is the right
    /// answer: this file is ours rather than the user's, nothing in it can be recovered by
    /// hand, and refusing to start the notifications because of it would trade a lost
    /// bookkeeping entry for no warnings at all. The cost of the wrong guess here is one
    /// extra toast; the cost of the other wrong guess is silence.
    #[must_use]
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let log = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| AlertLog::from_json(&text).ok())
            .unwrap_or_default();
        Alerts {
            log,
            seen: BTreeMap::new(),
            path: Some(path),
            dirty: false,
        }
    }

    /// The state machine at [`crate::paths::alerts_path`].
    #[must_use]
    pub fn discover() -> Self {
        match crate::paths::alerts_path() {
            Ok(path) => Alerts::open(path),
            // No home directory: warn, but remember nothing between runs.
            Err(_) => Alerts::in_memory(),
        }
    }

    /// The document, for a test or a diagnostic view.
    #[must_use]
    pub fn log(&self) -> &AlertLog {
        &self.log
    }

    /// Whether there is anything to write.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Write the log, if it has changed and there is somewhere to write it.
    ///
    /// Atomically, like everything else this product writes: a reader — which today is only
    /// the next launch of this same application — sees the old file or the new one.
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let Some(path) = self.path.as_deref() else {
            self.dirty = false;
            return Ok(());
        };
        let text = self.log.to_json()?;
        atomic::write_bytes(path, text.as_bytes())?;
        self.dirty = false;
        Ok(())
    }

    /// Read a log from an explicit path, reporting what went wrong.
    ///
    /// [`Alerts::open`] swallows a damaged file on purpose; this is for the test that
    /// proves the file it wrote is the file it can read.
    pub fn read(path: &Path) -> Result<AlertLog> {
        match std::fs::read_to_string(path) {
            Ok(text) => AlertLog::from_json(&text).map_err(|error| match error {
                Error::Json { source, .. } => Error::json(path, source),
                other => other,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AlertLog::default()),
            Err(source) => Err(Error::io(path, source)),
        }
    }

    /// Look at a derived view and say what has just been crossed.
    ///
    /// Called after **every** refresh, not only after one that changed the document: the
    /// first pass of a tray that has just started has to be able to say "you are already at
    /// 91 %" even when the file on disk already said so.
    ///
    /// The returned alerts are in the view's own order — Claude before Codex, shortest
    /// window first — so a caller showing several of them shows them in the order the panel
    /// draws them.
    pub fn evaluate(&mut self, view: &SnapshotView, rules: &AlertRules) -> Vec<Alert> {
        let ladder = rules.ladder();
        let mut alerts = Vec::new();
        let mut live: Vec<String> = Vec::new();

        for provider in &view.providers {
            for window in &provider.windows {
                let key = format!("{}/{}", provider.name, window.key);
                live.push(key.clone());
                if let Some(alert) = self.evaluate_window(&key, &provider.name, window, &ladder) {
                    alerts.push(Alert {
                        suppressed: rules.quiet,
                        ..alert
                    });
                }
            }
        }

        self.sweep(&live, &view.now);
        alerts
    }

    /// One window. `None` when nothing was crossed.
    fn evaluate_window(
        &mut self,
        key: &str,
        provider: &str,
        window: &WindowView,
        ladder: &[f64],
    ) -> Option<Alert> {
        // Rule 5. An unknown window says nothing and forgets what it saw, so that the
        // reading after it is a first observation rather than a continuation.
        let percent = match window.percent.filter(|value| value.is_finite()) {
            Some(percent) if window.state != "error" => percent,
            _ => {
                self.seen.remove(key);
                return None;
            }
        };

        let reset = window.resets_at.as_deref();
        // Both halves of rule 4 ask the same question of the same window, so they ask it
        // the same way: a second of jitter in `resetsAt` is not a new week.
        let is_same_period =
            |stored: Option<&str>| same_period(stored, reset, window.window_minutes);

        // Rule 4, the in-memory half: a reading from before the reset is not a previous
        // reading, it is a reading of a different week.
        let previous = self
            .seen
            .get(key)
            .filter(|(_, seen_reset)| is_same_period(seen_reset.as_deref()))
            .map(|(percent, _)| *percent);

        // Rule 2 and rule 4, the on-disk half.
        let already: &[f64] = match self.log.windows.get(key) {
            Some(record) if is_same_period(record.resets_at.as_deref()) => &record.fired,
            _ => &[],
        };

        let crossed: Vec<f64> = ladder
            .iter()
            .copied()
            .filter(|step| percent + EPSILON >= *step)
            .filter(|step| !already.iter().any(|fired| same(*fired, *step)))
            .filter(|step| previous.is_none_or(|before| before + EPSILON < *step))
            .collect();

        self.seen
            .insert(key.to_owned(), (percent, window.resets_at.clone()));

        if crossed.is_empty() {
            // Nothing fired, but the period may still have turned over: a record from the
            // week before is bookkeeping for a week nobody is in any more. A record whose
            // `resetsAt` merely wobbled is left exactly as it is — rewriting it here is what
            // rewrote `alerts.json` every five minutes for two and a half hours.
            if let Some(record) = self.log.windows.get(key)
                && !is_same_period(record.resets_at.as_deref())
            {
                self.log.windows.remove(key);
                self.dirty = true;
            }
            return None;
        }

        let record = self.log.windows.entry(key.to_owned()).or_default();
        if !is_same_period(record.resets_at.as_deref()) {
            record.fired.clear();
        }
        // The newest spelling wins, whether or not the period turned over. The record is
        // allowed to drift with the source because nothing compares it exactly any more.
        record.resets_at = window.resets_at.clone();
        record.fired.extend(crossed.iter().copied());
        record.fired.sort_by(f64::total_cmp);
        record.fired.dedup_by(|left, right| same(*left, *right));
        self.dirty = true;

        // Every crossed threshold is consumed; the most severe one is the one shown.
        let threshold = *crossed.last()?;
        Some(Alert {
            provider: provider.to_owned(),
            window: window.key.clone(),
            window_minutes: window.window_minutes,
            model: window.model.clone(),
            threshold,
            percent,
            remaining_ms: window.remaining_ms,
            suppressed: false,
        })
    }

    /// Drop records for windows nobody reports any more.
    ///
    /// Only the ones whose reset is comfortably past: a provider that could not be read for
    /// one refresh must not have its bookkeeping thrown away, because the next reading
    /// would then look like a first observation and re-fire a toast the user has had.
    fn sweep(&mut self, live: &[String], now: &str) {
        let Some(now) = unix_seconds_from_rfc3339(now) else {
            return;
        };
        let stale: Vec<String> = self
            .log
            .windows
            .iter()
            .filter(|(key, _)| !live.contains(key))
            .filter(|(_, record)| {
                record
                    .resets_at
                    .as_deref()
                    .and_then(unix_seconds_from_rfc3339)
                    .is_some_and(|resets| now - resets > RECORD_MAX_AGE_SECONDS)
            })
            .map(|(key, _)| key.clone())
            .collect();
        if !stale.is_empty() {
            for key in stale {
                self.log.windows.remove(&key);
            }
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests;
