//! The state model: what the measured numbers mean *right now*.
//!
//! `limits.json` stores what a source reported and nothing else. Everything a display
//! actually wants — which window is binding, how long until it resets, how old the reading
//! is, how alarming it is — changes with the clock rather than with the data, so none of it
//! is stored. It is worked out here, at read time, from the document and an instant.
//!
//! ```text
//!   Snapshot (measured)                    now + rules
//!   percent 70, resetsAt 12:24Z   ──────────────┬──────────▶  SnapshotView (derived)
//!   sourceAt 11:58Z                             │              binding   secondary
//!                                               │              remaining 26 min
//!                                               │              age       6 min
//!                                               │              freshness aging
//!                                               ▼              severity  warn
//!                                    thresholds 60/85/100
//!                                    freshness  5 min / 45 min
//! ```
//!
//! Four rules shape this module, and each is a finding from the audit of the retired
//! prototype:
//!
//! 1. **Unknown is a value.** A window with no percentage is [`Severity::Unknown`], never
//!    [`Severity::Ok`] and never a `0` (finding B03). The same goes for a reading with no
//!    `sourceAt`: its freshness is [`Freshness::Unknown`], not "fresh".
//! 2. **The binding window is computed, never flagged.** The highest percentage of a
//!    provider's windows; ties go to the shorter window, because that is the one you hit
//!    first (findings B04 and B16).
//! 3. **One staleness threshold, in one place.** The prototype had three displays and three
//!    different answers to "is this old" (finding B14). Here there is one rule, it lives in
//!    the settings, and every consumer reads it from the same place.
//! 4. **Reset maths happens on instants, not on calendars.** A countdown is the difference
//!    between two UTC instants, which is the same number in every time zone. Local time is
//!    a rendering decision and it belongs at the very end, in the panel.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::limits::{Limits, Provider, Providers, Window, WindowState};
use crate::timefmt::unix_seconds_from_rfc3339;

/// Percentage at or above which a window is [`Severity::Warn`].
pub const DEFAULT_WARN: f64 = 60.0;
/// Percentage at or above which a window is [`Severity::Critical`].
pub const DEFAULT_CRITICAL: f64 = 85.0;
/// Percentage at or above which a window is [`Severity::Exhausted`].
pub const DEFAULT_EXHAUSTED: f64 = 100.0;

/// Minutes below which a reading is [`Freshness::Fresh`].
pub const DEFAULT_FRESH_MINUTES: u32 = 5;
/// Minutes below which a reading is [`Freshness::Aging`], and above which it is stale.
pub const DEFAULT_AGING_MINUTES: u32 = 45;

/// How alarming one window is.
///
/// Ordered from calm to alarming so that a display can take the worst of several with
/// `max`, and `Unknown` sorts *below* `Ok` on purpose: "I do not know" must never win a
/// comparison against a real reading and colour the icon on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// No percentage was read. Rendered as unknown — never as `0 %`.
    Unknown,
    /// Below the warning threshold.
    Ok,
    /// At or above the warning threshold.
    Warn,
    /// At or above the critical threshold.
    Critical,
    /// At or above 100 %: the window is spent.
    Exhausted,
}

/// How old a provider's reading is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Freshness {
    /// No `sourceAt`, so the age cannot be worked out at all.
    Unknown,
    /// Read within the fresh window.
    Fresh,
    /// Older than fresh, younger than stale.
    Aging,
    /// Older than the stale threshold.
    Stale,
}

/// Percentages at which a window changes severity.
///
/// Held as `f64` because `percent` is, and compared with `>=` so that exactly 60 is a
/// warning and exactly 85 is critical. Evaluated from the most severe threshold down, so a
/// settings file whose thresholds are out of order still produces a sensible answer rather
/// than a surprising one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thresholds {
    /// Amber. `60` by default.
    pub warn: f64,
    /// Red. `85` by default.
    pub critical: f64,
    /// Spent. `100` by default.
    pub exhausted: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            warn: DEFAULT_WARN,
            critical: DEFAULT_CRITICAL,
            exhausted: DEFAULT_EXHAUSTED,
        }
    }
}

impl Thresholds {
    /// Severity of a percentage that may not exist.
    ///
    /// `None` — and a value that is not a finite number, which no source should produce but
    /// a damaged file can — is [`Severity::Unknown`]. That is the whole point of the type:
    /// the audit's first red finding was a display that answered "unknown" with a
    /// reassuring blue zero.
    #[must_use]
    pub fn severity(&self, percent: Option<f64>) -> Severity {
        let Some(percent) = percent.filter(|value| value.is_finite()) else {
            return Severity::Unknown;
        };
        if percent >= self.exhausted {
            Severity::Exhausted
        } else if percent >= self.critical {
            Severity::Critical
        } else if percent >= self.warn {
            Severity::Warn
        } else {
            Severity::Ok
        }
    }
}

/// When a reading stops counting as fresh, and when it starts counting as stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreshnessRules {
    /// Minutes a reading stays [`Freshness::Fresh`]. `5` by default.
    pub fresh_minutes: u32,
    /// Minutes a reading stays [`Freshness::Aging`]. `45` by default, which is the number
    /// `docs/PROJECT.md` has always named.
    pub aging_minutes: u32,
}

impl Default for FreshnessRules {
    fn default() -> Self {
        FreshnessRules {
            fresh_minutes: DEFAULT_FRESH_MINUTES,
            aging_minutes: DEFAULT_AGING_MINUTES,
        }
    }
}

impl FreshnessRules {
    /// Classify an age in milliseconds.
    ///
    /// A negative age — a source stamped in the future, which a machine whose clock has
    /// just been corrected really does produce — counts as fresh rather than as an error.
    /// It is not information the user can act on, and refusing to show a number because two
    /// clocks disagree by a second would be worse than showing it.
    #[must_use]
    pub fn classify(&self, age_ms: Option<i64>) -> Freshness {
        let Some(age_ms) = age_ms else {
            return Freshness::Unknown;
        };
        if age_ms <= i64::from(self.fresh_minutes) * 60_000 {
            Freshness::Fresh
        } else if age_ms <= i64::from(self.aging_minutes) * 60_000 {
            Freshness::Aging
        } else {
            Freshness::Stale
        }
    }
}

/// Everything the derived view needs that is not in the document.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    /// Percentages at which a window changes severity.
    pub thresholds: Thresholds,
    /// When a reading stops being fresh.
    pub freshness: FreshnessRules,
}

/// The measured document, held in memory by the one process that refreshes it.
///
/// Deliberately a thin wrapper rather than a second copy of the data: what makes it a
/// *snapshot* is that nothing derived is stored in it, so it can be handed to a display
/// once and asked a different question every second.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    limits: Limits,
}

impl Snapshot {
    /// A snapshot over a document.
    #[must_use]
    pub fn new(limits: Limits) -> Self {
        Snapshot { limits }
    }

    /// A snapshot with both providers unconfigured — what the tray holds before its first
    /// refresh has finished.
    #[must_use]
    pub fn empty(now: impl Into<String>) -> Self {
        Snapshot::new(Limits::new(now))
    }

    /// When the document was last written.
    #[must_use]
    pub fn updated_at(&self) -> &str {
        &self.limits.updated_at
    }

    /// The measured providers.
    #[must_use]
    pub fn providers(&self) -> &Providers {
        &self.limits.providers
    }

    /// The document itself, for the writer.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Take the document out of the snapshot.
    #[must_use]
    pub fn into_limits(self) -> Limits {
        self.limits
    }

    /// Everything a display needs, worked out for the instant `now`.
    ///
    /// `now` is RFC 3339; a reading it cannot parse leaves every derived time as `None`
    /// rather than as a guess, which is the same rule the rest of this crate follows.
    #[must_use]
    pub fn view(&self, now: &str, rules: &Rules) -> SnapshotView {
        let now_seconds = unix_seconds_from_rfc3339(now);
        let mut providers = Vec::new();
        for (name, provider) in [
            ("claude", &self.limits.providers.claude),
            ("codex", &self.limits.providers.codex),
        ] {
            providers.push(provider_view(name, provider, now_seconds, rules));
        }
        SnapshotView {
            updated_at: self.limits.updated_at.clone(),
            now: now.to_owned(),
            providers,
        }
    }
}

/// The whole derived view, as the panel receives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotView {
    /// `updatedAt` from the document: when the numbers last changed.
    pub updated_at: String,
    /// The instant everything below was derived for.
    pub now: String,
    /// Both providers, always, in a stable order.
    pub providers: Vec<ProviderView>,
}

impl SnapshotView {
    /// The most alarming severity across every provider.
    ///
    /// What the tray icon shows once WP4 draws it. [`Severity::Unknown`] sorts lowest, so a
    /// provider nobody could read never decides the colour on its own — but it is also what
    /// comes back when nothing at all could be read, which is exactly the grey "?" state.
    #[must_use]
    pub fn severity(&self) -> Severity {
        self.providers
            .iter()
            .map(|provider| provider.severity)
            .max()
            .unwrap_or(Severity::Unknown)
    }
}

/// One provider, derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    /// `claude` or `codex`.
    pub name: String,
    /// `false` when the provider's files are not on this machine.
    pub configured: bool,
    /// Plan name, as the source reported it.
    pub plan: Option<String>,
    /// Where the numbers came from.
    pub source: Option<String>,
    /// When the source produced them.
    pub source_at: Option<String>,
    /// Key of the binding window: the highest percentage of this provider's windows.
    pub binding: Option<String>,
    /// Milliseconds between `sourceAt` and `now`. Negative when the source is ahead.
    pub age_ms: Option<i64>,
    /// [`age_ms`](ProviderView::age_ms) classified.
    pub freshness: Freshness,
    /// Severity of the binding window, which is the one that constrains the user.
    pub severity: Severity,
    /// Every window, shortest first.
    pub windows: Vec<WindowView>,
}

/// One window, derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowView {
    /// Window key: `five_hour`, `seven_day`, `seven_day_fable`, `primary`, `secondary`, …
    pub key: String,
    /// Percentage used, unrounded and as reported. **Absent when it is unknown.**
    pub percent: Option<f64>,
    /// Window length in minutes.
    pub window_minutes: Option<u32>,
    /// When the window resets, RFC 3339 in UTC.
    pub resets_at: Option<String>,
    /// Milliseconds until the reset. **Negative when the reset is already due**, which is
    /// what a reading taken before a long sleep looks like; the retired prototype printed
    /// "now" for ever instead (finding S7).
    pub remaining_ms: Option<i64>,
    /// `ok`, `stale`, `error`, or whatever a newer writer put there.
    pub state: String,
    /// Short reason the window is stale or in error.
    pub error: Option<String>,
    /// Model a weekly window is scoped to.
    pub model: Option<String>,
    /// Whether the window came from the opt-in detailed-windows mode.
    pub detailed: bool,
    /// Whether this is the provider's binding window.
    pub binding: bool,
    /// [`percent`](WindowView::percent) against the thresholds.
    pub severity: Severity,
}

/// Key of the window with the highest percentage.
///
/// The one implementation of the rule, shared by both readers and by the view. It is
/// computed rather than taken from a flag: neither source provides one, and the retired
/// prototype inventing one is finding B04 of the audit — it showed a 52 % window in bold as
/// "the one limiting you" while a 70 % window sat beside it.
///
/// * A window with **no percentage never binds.** "I could not read it" is not a candidate
///   for "the number that constrains you", so a provider whose windows are all unknown has
///   no binding window at all.
/// * **Ties go to the shorter window**, then to the smaller key. When the five-hour and the
///   weekly window are equally full, the five-hour one is what you hit first; and a window
///   with no stated length sorts last among equals, because it is the one we know least
///   about.
#[must_use]
pub fn binding(windows: &BTreeMap<String, Window>) -> Option<String> {
    let length = |window: &Window| window.window_minutes.unwrap_or(u32::MAX);

    windows
        .iter()
        .filter_map(|(key, window)| {
            window
                .percent
                .filter(|percent| percent.is_finite())
                .map(|percent| (key, window, percent))
        })
        .max_by(|left, right| {
            left.2
                .total_cmp(&right.2)
                // The tie-breaks are written the other way round on purpose: the comparator
                // answers "is left greater", and among equal percentages the *smaller*
                // window length, then the *smaller* key, is what should win.
                .then_with(|| length(right.1).cmp(&length(left.1)))
                .then_with(|| right.0.cmp(left.0))
        })
        .map(|(key, _, _)| key.clone())
}

/// Derive one provider.
fn provider_view(
    name: &str,
    provider: &Provider,
    now_seconds: Option<i64>,
    rules: &Rules,
) -> ProviderView {
    let age_ms = provider
        .source_at
        .as_deref()
        .and_then(unix_seconds_from_rfc3339)
        .zip(now_seconds)
        .map(|(source, now)| (now - source) * 1000);

    // Recomputed here rather than trusted from the file: a document written by an older
    // build, or by hand, may carry a `binding` that no longer matches its own numbers.
    let binding_key = binding(&provider.windows);

    let mut windows: Vec<WindowView> = provider
        .windows
        .iter()
        .map(|(key, window)| WindowView {
            key: key.clone(),
            percent: window.percent,
            window_minutes: window.window_minutes,
            resets_at: window.resets_at.clone(),
            remaining_ms: window
                .resets_at
                .as_deref()
                .and_then(unix_seconds_from_rfc3339)
                .zip(now_seconds)
                .map(|(resets, now)| (resets - now) * 1000),
            state: state_name(&window.state),
            error: window.error.clone(),
            model: window.model.clone(),
            detailed: window.detailed.unwrap_or(false),
            binding: binding_key.as_deref() == Some(key.as_str()),
            severity: rules.thresholds.severity(window.percent),
        })
        .collect();
    // Shortest window first, then by key: a panel that draws them in this order draws the
    // five-hour row above the weekly one on both providers without knowing either.
    windows.sort_by(|left, right| {
        left.window_minutes
            .unwrap_or(u32::MAX)
            .cmp(&right.window_minutes.unwrap_or(u32::MAX))
            .then_with(|| left.key.cmp(&right.key))
    });

    let severity = windows
        .iter()
        .find(|window| window.binding)
        .map_or(Severity::Unknown, |window| window.severity);

    ProviderView {
        name: name.to_owned(),
        configured: provider.configured,
        plan: provider.plan.clone(),
        source: provider.source.clone().map(String::from),
        source_at: provider.source_at.clone(),
        binding: binding_key,
        age_ms,
        freshness: rules.freshness.classify(age_ms),
        severity,
        windows,
    }
}

/// The window state as the file spells it, including a value this build does not know.
fn state_name(state: &WindowState) -> String {
    String::from(state.clone())
}

#[cfg(test)]
mod tests;
