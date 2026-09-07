//! Turning the usage endpoint's answer into `limits.json` windows.
//!
//! The shape, as the retired prototype read it and as the live check confirmed:
//!
//! ```json
//! {"limits":[
//!    {"kind":"session",       "percent":12, "resets_at":"…", "is_active":true},
//!    {"kind":"weekly_all",    "percent":18, "resets_at":"…"},
//!    {"kind":"weekly_scoped", "percent":23, "resets_at":"…",
//!     "scope":{"model":{"display_name":"Fable"}}},
//!    {"kind":"opus",          "percent":5,  "resets_at":"…"}],
//!  "five_hour":{"utilization":12,"resets_at":"…"},
//!  "seven_day":{"utilization":18,"resets_at":"…"}}
//! ```
//!
//! | `kind` | Window key | `windowMinutes` |
//! |---|---|---|
//! | `session` | `five_hour` | 300 |
//! | `weekly_all` | `seven_day` | 10080 |
//! | `weekly_scoped` | `seven_day_<model>` | 10080 |
//! | `opus` | `seven_day_opus` | 10080 |
//! | anything else | *skipped* | — |
//!
//! `five_hour` and `seven_day` at the top level are the older shape, and are read only
//! when `limits[]` produced nothing — they carry `utilization` where the array carries
//! `percent`.
//!
//! ## What is not read
//!
//! **`is_active`.** It is tempting, because it looks like the answer to "which window is
//! binding". It is not the answer this product gives: the binding window is the highest
//! percentage across every window, computed here and never taken from a flag. The
//! prototype trusted a flag it had invented for Codex and drew the wrong window in bold
//! (audit finding B04), and read `is_active` for Claude while its tray used a maximum, so
//! its three displays disagreed (B16). One rule, computed, everywhere.
//!
//! **`extra_usage`.** Real, and interesting — whether paid overflow usage is switched on.
//! The `limits.json` contract has no field for it and is not growing one in WP2b.
//!
//! An unknown `kind` is skipped rather than guessed at. A window whose meaning is not
//! known cannot be given a length, and a window without a length is worse than absent.

use std::collections::BTreeMap;

use serde_json::Value;

use super::error::DetailedError;
use crate::claude::{FIVE_HOUR_WINDOW_MINUTES, SEVEN_DAY_WINDOW_MINUTES, resets_at_value};
use crate::limits::Window;
use crate::timefmt::sanitize_plan;

/// Prefix every weekly window key shares.
const WEEKLY_PREFIX: &str = "seven_day";

/// Key for a scoped weekly window whose model name yielded nothing usable.
const UNNAMED_SCOPED: &str = "seven_day_scoped";

/// Longest slug taken from a model's display name.
const MAX_SLUG_LEN: usize = 32;

/// Longest model display name copied into `limits.json`.
const MAX_MODEL_NAME_LEN: usize = 48;

/// Most windows accepted from one answer, so a strange body cannot grow the document.
const MAX_WINDOWS: usize = 24;

/// What one answer yielded.
#[derive(Debug, Clone, PartialEq)]
pub struct Mapped {
    /// The plan name, normalised from the sign-in's hints. `None` when neither said
    /// anything usable — the contract forbids inventing one.
    pub plan: Option<String>,
    /// The windows, keyed the way `limits.json` keys them.
    pub windows: BTreeMap<String, Window>,
}

/// Read one response body.
///
/// `tier` and `subscription` are the two hints out of the sign-in file; the plan name is
/// theirs, not the response's, which does not carry one.
pub fn map_response(
    body: &str,
    tier: Option<&str>,
    subscription: Option<&str>,
) -> std::result::Result<Mapped, DetailedError> {
    let document: Value =
        serde_json::from_str(body).map_err(|_| DetailedError::UnexpectedShape {
            expected: "a JSON object",
        })?;
    let object = document.as_object().ok_or(DetailedError::UnexpectedShape {
        expected: "a JSON object",
    })?;

    let mut windows = BTreeMap::new();
    if let Some(limits) = object.get("limits").and_then(Value::as_array) {
        for entry in limits.iter().take(MAX_WINDOWS) {
            if let Some((key, window)) = read_entry(entry, &windows) {
                windows.insert(key, window);
            }
        }
    }

    // The older shape, and the reason the prototype did not go blank the day the array
    // was missing. Read only when the array gave nothing, never merged with it.
    if windows.is_empty() {
        for (key, minutes, field) in [
            (
                crate::claude::WINDOW_FIVE_HOUR,
                FIVE_HOUR_WINDOW_MINUTES,
                "five_hour",
            ),
            (
                crate::claude::WINDOW_SEVEN_DAY,
                SEVEN_DAY_WINDOW_MINUTES,
                "seven_day",
            ),
        ] {
            let Some(entry) = object.get(field) else {
                continue;
            };
            let Some(percent) = percent_of(entry) else {
                continue;
            };
            windows.insert(key.to_owned(), window(percent, minutes, entry));
        }
    }

    if windows.is_empty() {
        return Err(DetailedError::UnexpectedShape {
            expected: "limits[] with a readable percentage, or five_hour/seven_day",
        });
    }

    Ok(Mapped {
        plan: normalise_plan(tier, subscription),
        windows,
    })
}

/// One entry of `limits[]`, as a key and a window.
///
/// `taken` is what has been mapped so far, so that two scoped windows for the same model
/// do not silently become one.
fn read_entry(entry: &Value, taken: &BTreeMap<String, Window>) -> Option<(String, Window)> {
    let object = entry.as_object()?;
    let kind = object.get("kind")?.as_str()?;
    let percent = percent_of(entry)?;

    match kind {
        "session" => Some((
            crate::claude::WINDOW_FIVE_HOUR.to_owned(),
            window(percent, FIVE_HOUR_WINDOW_MINUTES, entry),
        )),
        "weekly_all" => Some((
            crate::claude::WINDOW_SEVEN_DAY.to_owned(),
            window(percent, SEVEN_DAY_WINDOW_MINUTES, entry),
        )),
        "weekly_scoped" => {
            let name = object
                .get("scope")
                .and_then(|scope| scope.get("model"))
                .and_then(|model| model.get("display_name"))
                .and_then(Value::as_str)
                .and_then(model_name);
            let key = unique(
                match name.as_deref().and_then(slug) {
                    Some(slug) => format!("{WEEKLY_PREFIX}_{slug}"),
                    None => UNNAMED_SCOPED.to_owned(),
                },
                taken,
            );
            let mut mapped = window(percent, SEVEN_DAY_WINDOW_MINUTES, entry);
            if let Some(name) = name {
                mapped = mapped.with_model(name);
            }
            Some((key, mapped))
        }
        // The older name for the model-scoped weekly cap, from before `weekly_scoped`
        // existed. It names its model in the kind rather than in a scope.
        "opus" => Some((
            format!("{WEEKLY_PREFIX}_opus"),
            window(percent, SEVEN_DAY_WINDOW_MINUTES, entry).with_model("Opus"),
        )),
        _ => None,
    }
}

/// A key nothing has claimed yet.
fn unique(key: String, taken: &BTreeMap<String, Window>) -> String {
    if !taken.contains_key(&key) {
        return key;
    }
    for suffix in 2..=9u32 {
        let candidate = format!("{key}_{suffix}");
        if !taken.contains_key(&candidate) {
            return candidate;
        }
    }
    key
}

/// Build a window, marked as coming from the opt-in mode.
///
/// **Every** window the endpoint produced carries `detailed: true`, not only the
/// model-scoped ones: the flag says where a number came from, and the merge can replace
/// `five_hour` and `seven_day` with fresher passive readings, at which point they stop
/// being detailed and the flag goes with them.
fn window(percent: f64, minutes: u32, entry: &Value) -> Window {
    let mut mapped = Window::ok(percent).with_window_minutes(minutes);
    if let Some(resets_at) = entry.get("resets_at").and_then(resets_at_value) {
        mapped = mapped.with_resets_at(resets_at);
    }
    mapped.detailed = Some(true);
    mapped
}

/// `percent`, or `utilization` on the older shape.
///
/// Outside `0..=100` the value is not a percentage of anything this build understands, so
/// the window is dropped rather than clamped. Clamping would invent a number; the contract
/// would rather say nothing.
fn percent_of(entry: &Value) -> Option<f64> {
    let value = entry
        .get("percent")
        .or_else(|| entry.get("utilization"))?
        .as_f64()?;
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        return None;
    }
    Some(value)
}

/// A model's display name, if it is a name.
///
/// Bounded, no control characters, no path separators, and at least one letter or digit.
/// It is written into `limits.json`, which is meant to be safe to paste into a bug report,
/// so a field that has been repurposed to hold something else is dropped rather than
/// forwarded — the same rule `crate::timefmt::sanitize_plan` applies to plan names, minus
/// the identifier shape, because "Fable 5.1" is a legitimate display name.
#[must_use]
pub fn model_name(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > MAX_MODEL_NAME_LEN {
        return None;
    }
    if value
        .chars()
        .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return None;
    }
    if !value.chars().any(char::is_alphanumeric) {
        return None;
    }
    Some(value.to_owned())
}

/// A window-key fragment from a display name: `Fable 5.1` becomes `fable_5_1`.
///
/// ASCII only, because the key is read by other programs and by people typing it into a
/// filter. A name with no ASCII alphanumerics in it yields nothing, and the caller falls
/// back to a key that says "scoped" without claiming which model.
#[must_use]
pub fn slug(name: &str) -> Option<String> {
    let mut out = String::new();
    let mut pending_separator = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !out.is_empty() {
                out.push('_');
            }
            out.push(character.to_ascii_lowercase());
            pending_separator = false;
            if out.len() >= MAX_SLUG_LEN {
                break;
            }
        } else {
            pending_separator = true;
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// The plan name, from the sign-in's two hints.
///
/// `rateLimitTier` is the specific one (`default_claude_max_20x`) and is tried first;
/// `subscriptionType` is the coarse one (`max`, `pro`) and is the fallback. A tier this
/// build does not recognise is passed through as written, provided it looks like an
/// identifier, because a new tier name is more useful in the file than a shrug.
#[must_use]
pub fn normalise_plan(tier: Option<&str>, subscription: Option<&str>) -> Option<String> {
    if let Some(tier) = tier {
        let lowered = tier.to_ascii_lowercase();
        for (needle, name) in [("max_20x", "max_20x"), ("max_5x", "max_5x")] {
            if lowered.contains(needle) {
                return Some(name.to_owned());
            }
        }
        // After the two Max tiers, so that a future `max_pro` cannot be read as `pro`.
        if lowered.contains("pro") {
            return Some("pro".to_owned());
        }
        if let Some(raw) = sanitize_plan(tier) {
            return Some(raw);
        }
    }
    subscription.and_then(sanitize_plan)
}
