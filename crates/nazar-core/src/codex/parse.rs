//! Pulling the quota numbers out of a rollout line, and leaving everything else behind.
//!
//! A rollout line is a whole turn of a Codex session: the prompt, the model's reasoning,
//! every shell command it ran and everything those commands printed. The quota lives in a
//! corner of it:
//!
//! ```text
//! {"timestamp":"…","ordinal":409,"type":"event_msg",
//!  "payload":{"type":"token_count","info":{…},"rate_limits":{
//!     "plan_type":"plus",
//!     "primary":  {"used_percent":54.0,"window_minutes":300,  "resets_at":1788751044},
//!     "secondary":{"used_percent":70.0,"window_minutes":10080,"resets_at":1788783892}}}}
//! ```
//!
//! This module is an **allow-list**, not a filter. It names the seven values it wants and
//! constructs a [`Quota`] out of them; nothing else in the line is copied anywhere, and
//! the two strings that do come through ([`Quota::plan`] and [`Quota::source_at`]) are
//! checked against a shape before they are kept. That is what the leak test proves: a
//! line whose every text field holds a sentinel produces a `Quota` that does not contain
//! the sentinel anywhere.
//!
//! Verified against 329 quota lines in 18 real logs (2026-08-27 to 2026-09-07,
//! `cli_version` 0.153.4). See `docs/pinned-internal-formats.md`.

use serde_json::Value;

use crate::timefmt::{
    floor_to_minute, rfc3339_from_unix_seconds, sanitize_plan, sanitize_timestamp,
    unix_seconds_auto, unix_seconds_from_rfc3339,
};

/// The byte string a line must contain before it is worth parsing.
pub const NEEDLE: &[u8] = b"\"rate_limits\"";

/// The quota numbers taken from one rollout line. Everything else is dropped.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Quota {
    /// `rate_limits.plan_type`, if it looks like a plan name. `plus`, `pro`, `max_20x`.
    pub plan: Option<String>,
    /// The line's `timestamp`, if it looks like a timestamp. RFC 3339 as written.
    pub source_at: Option<String>,
    /// The five-hour window.
    pub primary: Option<QuotaWindow>,
    /// The weekly window.
    pub secondary: Option<QuotaWindow>,
}

impl Quota {
    /// `true` when at least one window carried a percentage.
    ///
    /// A quota line with neither is not a usable reading: Codex writes such lines for a
    /// second limit family (`limit_id` `premium` on the maintainer's machine) whose two
    /// windows are both `null`. Treating one as a reading would report "no windows" on a
    /// machine that has perfectly good numbers a few lines earlier.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        self.primary.is_some() || self.secondary.is_some()
    }
}

/// One window's numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaWindow {
    /// `used_percent`, as reported. Never rounded here; rounding is a display decision.
    pub used_percent: f64,
    /// `window_minutes`. `300` for the five-hour window, `10080` for the weekly one.
    /// Absent when the source omitted it — the window is still kept.
    pub window_minutes: Option<u32>,
    /// `resets_at`, already turned into RFC 3339 text. Absent when the source omitted it.
    pub resets_at: Option<String>,
}

/// What one line turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A quota line with at least one window that carried a percentage.
    Quota(Box<Quota>),
    /// A quota line whose windows were all absent or null. Nothing to report, and not a
    /// fault either — the reader keeps looking further back.
    Empty,
    /// Not a quota line.
    Other,
    /// The line is not JSON, or not the shape a rollout line has. Counted as a warning.
    Malformed,
}

/// Read one rollout line.
///
/// Never panics and never returns anything the source did not report. A line that is not
/// JSON, a `rate_limits` that is not an object, a `used_percent` that is not a number:
/// each of these produces [`Outcome::Malformed`] or a missing window, never a zero.
#[must_use]
pub fn parse_line(line: &str) -> Outcome {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Outcome::Malformed;
    };
    let Some(object) = value.as_object() else {
        return Outcome::Malformed;
    };
    let Some(payload) = object.get("payload").and_then(Value::as_object) else {
        // No payload at all: either a line shape we do not know, or truncated JSON that
        // happened to parse. Not a quota line; not worth a warning either way.
        return Outcome::Other;
    };
    let Some(limits) = payload.get("rate_limits") else {
        return Outcome::Other;
    };
    let Some(limits) = limits.as_object() else {
        // The key is there but it is not an object. That is the format changing under
        // us, which is exactly what a warning count is for.
        return Outcome::Malformed;
    };

    let quota = Quota {
        plan: limits
            .get("plan_type")
            .and_then(Value::as_str)
            .and_then(sanitize_plan),
        source_at: object
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(sanitize_timestamp),
        primary: limits.get("primary").and_then(window),
        secondary: limits.get("secondary").and_then(window),
    };

    if quota.is_usable() {
        Outcome::Quota(Box::new(quota))
    } else {
        Outcome::Empty
    }
}

/// Read one window object, or nothing.
///
/// A window with no usable `used_percent` is no window. The contract's second rule is
/// that an unknown percentage is absent rather than zero, and the cheapest place to obey
/// it is here, where the alternative would be to default a missing number.
fn window(value: &Value) -> Option<QuotaWindow> {
    let object = value.as_object()?;
    // `as_f64` accepts both `54` and `54.0`; a string or a null is not a percentage.
    let used_percent = object.get("used_percent")?.as_f64()?;
    if !used_percent.is_finite() {
        return None;
    }

    let window_minutes = object
        .get("window_minutes")
        .and_then(Value::as_u64)
        .and_then(|minutes| u32::try_from(minutes).ok())
        .filter(|minutes| *minutes > 0);

    let resets_at = object.get("resets_at").and_then(resets_at);

    Some(QuotaWindow {
        used_percent,
        window_minutes,
        resets_at,
    })
}

/// Read `resets_at` as RFC 3339 text, **rounded down to the whole minute**.
///
/// Codex writes Unix seconds (verified over 652 windows). An integer too large to be
/// seconds is read as milliseconds; a string is accepted only if it already looks like a
/// timestamp, so a future format change degrades rather than breaks.
///
/// ## Why the minute, on this side too
///
/// T-WP9 flattened the Claude reader to the minute because the usage endpoint reported the
/// same weekly reset one second apart on alternating refreshes and the notification state
/// machine read every flip as a new week. Codex has not been seen to jitter — but its
/// resets are written to the second (`2026-09-09T02:31:59Z`), and leaving one provider at
/// second precision while the other is at minute precision means `limits.json` carries two
/// spellings of the same kind of value, two panels rendering the same countdown differently,
/// and one reader that is one bug fix behind the other. Nothing downstream consumes
/// sub-minute precision: the panel counts down in minutes and [`crate::alerts`] only asks
/// whether two readings name the same period. So both sources go through the same gate,
/// [`floor_to_minute`], and produce the same text for the same instant.
///
/// **Down, never to the nearest**: a reset is a deadline, and rounding `12:24:52` up to
/// `12:25:00` would show eight seconds of quota that are not there.
fn resets_at(value: &Value) -> Option<String> {
    let seconds = if let Some(seconds) = value.as_i64() {
        unix_seconds_auto(seconds)
    } else if let Some(seconds) = value.as_f64() {
        if !seconds.is_finite() || seconds.abs() >= 9e18 {
            return None;
        }
        unix_seconds_auto(seconds as i64)
    } else {
        // A string is read back to seconds and written out again, which converts an offset
        // and drops fractions, so a source that switches to text lands in the same spelling
        // as one that writes numbers. `unix_seconds_from_rfc3339` is `sanitize_timestamp`
        // plus arithmetic, so the shape check is still the first thing that happens.
        unix_seconds_from_rfc3339(value.as_str()?)?
    };
    Some(rfc3339_from_unix_seconds(floor_to_minute(seconds)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The newest quota line of the maintainer's newest log, byte for byte apart from the
    /// text fields, which the fixture generator replaced.
    const REAL: &str = include_str!("../../../../fixtures/codex/rollout-sample.jsonl");
    /// The `limit_id: "premium"` variant, whose two windows are both `null`.
    const PREMIUM: &str = include_str!("../../../../fixtures/codex/rollout-premium-null.jsonl");
    /// Real non-quota entries: session metadata, settings, tool calls.
    const OTHER: &str = include_str!("../../../../fixtures/codex/rollout-no-rate-limits.jsonl");

    fn quota(line: &str) -> Quota {
        match parse_line(line) {
            Outcome::Quota(quota) => *quota,
            other => panic!("expected a quota line, got {other:?}"),
        }
    }

    #[test]
    fn the_real_newest_line_parses_to_the_numbers_the_endpoint_agreed_with() {
        let last = REAL.lines().next_back().unwrap();
        let quota = quota(last);

        assert_eq!(quota.plan.as_deref(), Some("plus"));
        assert_eq!(quota.source_at.as_deref(), Some("2026-09-06T22:55:05.479Z"));

        let primary = quota.primary.unwrap();
        assert_eq!(primary.used_percent, 54.0);
        assert_eq!(primary.window_minutes, Some(300));
        assert_eq!(primary.resets_at.as_deref(), Some("2026-09-07T03:17:00Z"));

        let secondary = quota.secondary.unwrap();
        assert_eq!(secondary.used_percent, 70.0);
        assert_eq!(secondary.window_minutes, Some(10080));
        assert_eq!(secondary.resets_at.as_deref(), Some("2026-09-07T12:24:00Z"));
    }

    #[test]
    fn every_line_of_the_real_sample_parses() {
        let mut seen = 0;
        for line in REAL.lines() {
            assert!(matches!(parse_line(line), Outcome::Quota(_)), "{line:.80}");
            seen += 1;
        }
        assert_eq!(seen, 13, "the fixture is the 13 quota lines of one session");
    }

    #[test]
    fn the_premium_variant_is_empty_rather_than_zero() {
        for line in PREMIUM.lines() {
            assert_eq!(
                parse_line(line),
                Outcome::Empty,
                "null windows must not become a reading"
            );
        }
    }

    #[test]
    fn real_non_quota_lines_are_recognised_as_such() {
        for line in OTHER.lines() {
            assert_eq!(parse_line(line), Outcome::Other, "{line:.80}");
        }
    }

    #[test]
    fn an_integer_percentage_is_as_good_as_a_float() {
        let line = r#"{"timestamp":"2026-09-06T22:55:05Z","payload":{"rate_limits":
            {"plan_type":"plus","primary":{"used_percent":54,"window_minutes":300,
            "resets_at":1788751044}}}}"#;
        let quota = quota(line);
        assert_eq!(quota.primary.unwrap().used_percent, 54.0);
        assert_eq!(quota.secondary, None);
    }

    #[test]
    fn a_missing_secondary_leaves_primary_alone() {
        let line = r#"{"payload":{"rate_limits":{"primary":{"used_percent":12.5}}}}"#;
        let quota = quota(line);
        assert_eq!(quota.primary.unwrap().used_percent, 12.5);
        assert_eq!(quota.secondary, None);
        assert_eq!(quota.plan, None);
        assert_eq!(quota.source_at, None);
    }

    #[test]
    fn a_null_secondary_is_the_same_as_a_missing_one() {
        let line = r#"{"payload":{"rate_limits":{"primary":{"used_percent":1},"secondary":null}}}"#;
        assert_eq!(quota(line).secondary, None);
    }

    #[test]
    fn a_window_without_minutes_is_kept_without_minutes() {
        let line = r#"{"payload":{"rate_limits":{"primary":{"used_percent":40,
            "resets_at":1788751044}}}}"#;
        let primary = quota(line).primary.unwrap();
        assert_eq!(primary.used_percent, 40.0);
        assert_eq!(primary.window_minutes, None);
        assert_eq!(primary.resets_at.as_deref(), Some("2026-09-07T03:17:00Z"));
    }

    #[test]
    fn a_window_without_a_reset_is_kept_without_a_reset() {
        let line = r#"{"payload":{"rate_limits":{"primary":{"used_percent":40,
            "window_minutes":300}}}}"#;
        let primary = quota(line).primary.unwrap();
        assert_eq!(primary.resets_at, None);
        assert_eq!(primary.window_minutes, Some(300));
    }

    #[test]
    fn unknown_keys_are_ignored_at_every_level() {
        let line = r#"{"timestamp":"2026-09-06T22:55:05Z","ordinal":1,"type":"event_msg",
            "brand_new_top_level":{"a":1},
            "payload":{"type":"token_count","brand_new_payload":[1,2,3],
            "rate_limits":{"plan_type":"plus","brand_new_limits":"whatever",
            "primary":{"used_percent":3,"window_minutes":300,"brand_new_window":true}}}}"#;
        let quota = quota(line);
        assert_eq!(quota.plan.as_deref(), Some("plus"));
        assert_eq!(quota.primary.unwrap().used_percent, 3.0);
    }

    #[test]
    fn a_percentage_that_is_not_a_number_is_not_a_window() {
        for bad in [r#""54""#, "null", "true", "[54]", r#"{"value":54}"#] {
            let line = format!(
                r#"{{"payload":{{"rate_limits":{{"primary":{{"used_percent":{bad}}},
                "secondary":{{"used_percent":9}}}}}}}}"#
            );
            let quota = quota(&line);
            assert_eq!(quota.primary, None, "used_percent {bad} must not be read");
            assert_eq!(quota.secondary.unwrap().used_percent, 9.0);
        }
    }

    #[test]
    fn malformed_json_is_reported_never_panicked_on() {
        for bad in [
            "",
            "{",
            "not json at all",
            r#"{"payload":{"rate_limits":"a string"}}"#,
            r#"{"payload":{"rate_limits":[1,2,3]}}"#,
            "[1,2,3]",
            "\"just a string\"",
            "12345",
        ] {
            let outcome = parse_line(bad);
            assert!(
                matches!(outcome, Outcome::Malformed | Outcome::Other),
                "{bad:?} produced {outcome:?}"
            );
            assert!(
                !matches!(outcome, Outcome::Quota(_)),
                "{bad:?} must not become a reading"
            );
        }
    }

    #[test]
    fn a_deeply_nested_line_does_not_blow_the_stack() {
        // serde_json has its own recursion limit; the point is that hitting it is a
        // parse error we report, not a crash that takes the tray with it.
        let deep = format!("{}{}", "[".repeat(2000), "]".repeat(2000));
        assert!(matches!(
            parse_line(&deep),
            Outcome::Malformed | Outcome::Other
        ));
    }

    #[test]
    fn milliseconds_and_string_resets_are_both_understood() {
        let millis = r#"{"payload":{"rate_limits":{"primary":{"used_percent":1,
            "resets_at":1788751044000}}}}"#;
        assert_eq!(
            quota(millis).primary.unwrap().resets_at.as_deref(),
            Some("2026-09-07T03:17:00Z")
        );

        let iso = r#"{"payload":{"rate_limits":{"primary":{"used_percent":1,
            "resets_at":"2026-09-07T03:17:24Z"}}}}"#;
        assert_eq!(
            quota(iso).primary.unwrap().resets_at.as_deref(),
            Some("2026-09-07T03:17:00Z")
        );

        let prose = r#"{"payload":{"rate_limits":{"primary":{"used_percent":1,
            "resets_at":"in about five hours"}}}}"#;
        assert_eq!(quota(prose).primary.unwrap().resets_at, None);
    }

    /// The one that matters: a line in which **every** text field is a sentinel, and the
    /// parser's whole output must not contain it. `plan_type` and `timestamp` are the
    /// only strings allowed out, and both must pass a shape check first — so a sentinel
    /// placed in them is dropped too, and this test can put one in every string there is.
    #[test]
    fn nothing_but_the_allow_listed_values_leaves_the_parser() {
        const SENTINEL: &str = "SENTINEL-do-not-leak-2f8a";
        let line = format!(
            r#"{{
              "timestamp": "{SENTINEL}",
              "type": "{SENTINEL}",
              "ordinal": 409,
              "git": {{"branch": "{SENTINEL}", "commit": "{SENTINEL}"}},
              "payload": {{
                "type": "{SENTINEL}",
                "cwd": "{SENTINEL}",
                "session_id": "{SENTINEL}",
                "prompt": "{SENTINEL}",
                "message": [{{"text": "{SENTINEL}"}}],
                "command": ["{SENTINEL}", "{SENTINEL}"],
                "aggregated_output": "{SENTINEL}",
                "info": {{
                  "model_context_window": 272000,
                  "total_token_usage": {{"total_tokens": 1068314}},
                  "note": "{SENTINEL}"
                }},
                "thread_token_usage": {{"total_tokens": 1068314, "label": "{SENTINEL}"}},
                "rate_limits": {{
                  "limit_id": "{SENTINEL}",
                  "limit_name": "{SENTINEL}",
                  "plan_type": "{SENTINEL} with spaces",
                  "credits": {{"balance": "{SENTINEL}", "has_credits": false}},
                  "individual_limit": "{SENTINEL}",
                  "rate_limit_reached_type": "{SENTINEL}",
                  "primary": {{
                    "used_percent": 54.0,
                    "window_minutes": 300,
                    "resets_at": 1788751044,
                    "label": "{SENTINEL}"
                  }},
                  "secondary": {{
                    "used_percent": 70.0,
                    "window_minutes": 10080,
                    "resets_at": 1788783892,
                    "label": "{SENTINEL}"
                  }}
                }}
              }}
            }}"#
        );

        let quota = quota(&line);

        // The numbers still come through: the allow-list is an allow-list, not a wall.
        assert_eq!(quota.primary.as_ref().unwrap().used_percent, 54.0);
        assert_eq!(quota.secondary.as_ref().unwrap().used_percent, 70.0);
        assert_eq!(
            quota.primary.as_ref().unwrap().resets_at.as_deref(),
            Some("2026-09-07T03:17:00Z")
        );
        // Both strings failed their shape check, so neither came through.
        assert_eq!(quota.plan, None);
        assert_eq!(quota.source_at, None);

        let rendered = format!("{quota:?}");
        assert!(
            !rendered.contains("SENTINEL"),
            "the parser leaked source text: {rendered}"
        );
    }
}
