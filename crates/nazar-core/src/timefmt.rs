//! Turning instants into RFC 3339 text, and refusing to turn anything else into it.
//!
//! Two jobs, both small and both deliberately dependency-free.
//!
//! **Formatting.** Codex reports a window's reset as a Unix timestamp in seconds. The
//! `limits.json` contract wants RFC 3339 text. The conversion here is the civil-from-days
//! algorithm and it produces **UTC** (`…Z`), not a local offset. That is a decision, not
//! an oversight: the standard library has no time-zone database and no way to ask the
//! operating system for the current offset, so a local offset would cost either a new
//! runtime dependency or hand-written platform code with daylight-saving edge cases. An
//! RFC 3339 timestamp in UTC names exactly the same instant as one with an offset, every
//! consumer of `limits.json` can render it in local time (the panel is JavaScript, where
//! that is one call), and a countdown — which is what the tray actually shows — is
//! offset-independent. See `docs/pinned-internal-formats.md`.
//!
//! **Refusing.** [`sanitize_timestamp`] and [`sanitize_plan`] are the allow-list that
//! keeps arbitrary text from a session log out of `limits.json`. The Codex reader copies
//! exactly two strings out of a rollout line, and both go through here first, so a field
//! that has been repurposed to hold prose cannot become a field in a file that is meant
//! to be safe to paste into a bug report.

use std::time::{SystemTime, UNIX_EPOCH};

/// Longest plan name accepted from a source. Real values are `plus`, `pro`, `max_20x`.
const MAX_PLAN_LEN: usize = 64;
/// Longest timestamp accepted from a source. `2026-09-06T22:55:05.479123456+03:00` is 35.
const MAX_TIMESTAMP_LEN: usize = 40;

/// Seconds above which an integer timestamp is read as milliseconds instead.
///
/// `10^12` seconds is the year 33658; `10^12` milliseconds is 2001. No source will ever
/// mean the former, so the boundary is unambiguous. Codex writes seconds today
/// (verified); this is here so a switch to milliseconds degrades into a correct reading
/// rather than a reset date forty thousand years away.
const MILLISECONDS_THRESHOLD: i64 = 1_000_000_000_000;

/// Format Unix seconds as RFC 3339 in UTC: `2026-09-07T03:17:24Z`.
#[must_use]
pub fn rfc3339_from_unix_seconds(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let time_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60,
    );
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Read a source's integer timestamp, in seconds or milliseconds, as RFC 3339 UTC.
#[must_use]
pub fn rfc3339_from_unix_auto(value: i64) -> String {
    if value.abs() >= MILLISECONDS_THRESHOLD {
        rfc3339_from_unix_seconds(value.div_euclid(1000))
    } else {
        rfc3339_from_unix_seconds(value)
    }
}

/// The current instant as RFC 3339 in UTC.
#[must_use]
pub fn now_rfc3339() -> String {
    let seconds = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        // A clock set before 1970 is a real machine state, not a reason to panic.
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
    };
    rfc3339_from_unix_seconds(seconds)
}

/// A file's modification time as RFC 3339 in UTC, when the filesystem reports one.
#[must_use]
pub fn rfc3339_from_system_time(at: SystemTime) -> String {
    let seconds = match at.duration_since(UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
    };
    rfc3339_from_unix_seconds(seconds)
}

/// Accept a timestamp string from a source, or reject it.
///
/// The shape has to look like RFC 3339 — `NNNN-NN-NNT…` with a zone marker — and the
/// character set is closed. Anything else is dropped rather than copied through, so a
/// `timestamp` field that turns out to hold something else cannot leak into the contract.
#[must_use]
pub fn sanitize_timestamp(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() < 20 || value.len() > MAX_TIMESTAMP_LEN {
        return None;
    }
    if !value.is_ascii() {
        return None;
    }
    let bytes = value.as_bytes();
    let digits_then = |index: usize, count: usize, next: u8| {
        bytes.len() > index + count
            && bytes[index..index + count].iter().all(u8::is_ascii_digit)
            && bytes[index + count] == next
    };
    // NNNN-NN-NNTNN:NN:NN…
    if !(digits_then(0, 4, b'-') && digits_then(5, 2, b'-') && digits_then(8, 2, b'T')) {
        return None;
    }
    if !(digits_then(11, 2, b':') && digits_then(14, 2, b':')) {
        return None;
    }
    if !bytes[17..19].iter().all(u8::is_ascii_digit) {
        return None;
    }
    // A zone is required: without one the instant is ambiguous and the contract's
    // consumers would each guess differently.
    let zone_ok = value.ends_with('Z')
        || value.ends_with('z')
        || matches!(bytes.get(value.len() - 6), Some(b'+' | b'-'));
    if !zone_ok {
        return None;
    }
    if !bytes[19..].iter().all(|byte| {
        byte.is_ascii_digit() || matches!(byte, b'.' | b'+' | b'-' | b':' | b'Z' | b'z')
    }) {
        return None;
    }
    Some(value.to_owned())
}

/// Accept a plan name from a source, or reject it.
///
/// A plan name is an identifier: short, ASCII, no spaces. Anything else is dropped, so
/// the field cannot become a channel for text out of a session log.
#[must_use]
pub fn sanitize_plan(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_PLAN_LEN {
        return None;
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return None;
    }
    Some(value.to_owned())
}

/// Days since 1970-01-01 to a civil `(year, month, day)`.
///
/// Howard Hinnant's `civil_from_days`, which is exact for the proleptic Gregorian
/// calendar over the whole range of `i64` days and needs no tables.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = (month_prime + if month_prime < 10 { 3 } else { -9 }) as u32;
    (year + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_and_its_neighbours() {
        assert_eq!(rfc3339_from_unix_seconds(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_from_unix_seconds(1), "1970-01-01T00:00:01Z");
        assert_eq!(rfc3339_from_unix_seconds(-1), "1969-12-31T23:59:59Z");
        assert_eq!(rfc3339_from_unix_seconds(86_399), "1970-01-01T23:59:59Z");
        assert_eq!(rfc3339_from_unix_seconds(86_400), "1970-01-02T00:00:00Z");
    }

    /// The two values the maintainer's newest rollout log actually carried, checked
    /// against the reset times the official endpoint reported for the same windows.
    #[test]
    fn the_observed_codex_reset_values() {
        assert_eq!(
            rfc3339_from_unix_seconds(1_788_751_044),
            "2026-09-07T03:17:24Z"
        );
        assert_eq!(
            rfc3339_from_unix_seconds(1_788_783_892),
            "2026-09-07T12:24:52Z"
        );
    }

    #[test]
    fn leap_days_and_century_rules() {
        assert_eq!(
            rfc3339_from_unix_seconds(951_782_400),
            "2000-02-29T00:00:00Z"
        );
        assert_eq!(
            rfc3339_from_unix_seconds(1_709_164_800),
            "2024-02-29T00:00:00Z"
        );
        // 1900 was not a leap year: 1900-03-01 is the day after 1900-02-28.
        assert_eq!(
            rfc3339_from_unix_seconds(-2_203_977_600),
            "1900-02-28T00:00:00Z"
        );
        assert_eq!(
            rfc3339_from_unix_seconds(-2_203_891_200),
            "1900-03-01T00:00:00Z"
        );
    }

    #[test]
    fn milliseconds_are_recognised_by_magnitude() {
        assert_eq!(
            rfc3339_from_unix_auto(1_788_751_044),
            "2026-09-07T03:17:24Z"
        );
        assert_eq!(
            rfc3339_from_unix_auto(1_788_751_044_000),
            "2026-09-07T03:17:24Z"
        );
    }

    #[test]
    fn extreme_values_do_not_panic() {
        for value in [i64::MIN, i64::MIN + 1, i64::MAX, -1, 0] {
            let text = rfc3339_from_unix_seconds(value);
            assert!(text.ends_with('Z'), "got {text}");
        }
    }

    #[test]
    fn now_looks_like_a_timestamp() {
        let now = now_rfc3339();
        assert_eq!(sanitize_timestamp(&now).as_deref(), Some(now.as_str()));
    }

    #[test]
    fn timestamps_that_are_accepted() {
        for value in [
            "2026-09-06T22:55:05.479Z",
            "2026-09-06T22:55:05Z",
            "2026-09-06T22:55:05+03:00",
            "2026-09-06T22:55:05.479123-05:30",
        ] {
            assert_eq!(
                sanitize_timestamp(value).as_deref(),
                Some(value),
                "should have accepted {value}"
            );
        }
    }

    #[test]
    fn timestamps_that_are_rejected() {
        for value in [
            "",
            "not a timestamp at all",
            "2026-09-06T22:55:05",           // no zone
            "2026-09-06 22:55:05Z",          // no T
            "2026-09-06T22:55:05Z and more", // trailing prose
            "SENTINEL-2026-09-06T22:55:05Z",
            "2026-09-06T22:55:05Z\u{1F600}",
        ] {
            assert_eq!(
                sanitize_timestamp(value),
                None,
                "should have rejected {value:?}"
            );
        }
    }

    #[test]
    fn plan_names_are_identifiers_or_nothing() {
        assert_eq!(sanitize_plan("plus").as_deref(), Some("plus"));
        assert_eq!(sanitize_plan("max_20x").as_deref(), Some("max_20x"));
        assert_eq!(sanitize_plan("pro-2.5").as_deref(), Some("pro-2.5"));
        assert_eq!(sanitize_plan("  plus  ").as_deref(), Some("plus"));

        assert_eq!(sanitize_plan(""), None);
        assert_eq!(sanitize_plan("a plan with spaces"), None);
        assert_eq!(sanitize_plan("SENTINEL prompt text"), None);
        assert_eq!(sanitize_plan(&"x".repeat(MAX_PLAN_LEN + 1)), None);
        assert_eq!(sanitize_plan("C:\\Users\\someone"), None);
    }

    #[test]
    fn a_year_of_days_round_trips() {
        // Every day of 2024 (a leap year) formats to a distinct, ordered date.
        let start = 1_704_067_200; // 2024-01-01T00:00:00Z
        let mut previous = String::new();
        for day in 0..366 {
            let text = rfc3339_from_unix_seconds(start + day * 86_400);
            assert!(text > previous, "{text} should sort after {previous}");
            assert!(text.starts_with("2024-"), "got {text}");
            previous = text;
        }
        assert_eq!(
            rfc3339_from_unix_seconds(start + 366 * 86_400),
            "2025-01-01T00:00:00Z"
        );
    }
}
