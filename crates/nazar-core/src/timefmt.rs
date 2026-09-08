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

/// Read a source's integer timestamp, in seconds or milliseconds, as Unix **seconds**.
#[must_use]
pub fn unix_seconds_auto(value: i64) -> i64 {
    if value.abs() >= MILLISECONDS_THRESHOLD {
        value.div_euclid(1000)
    } else {
        value
    }
}

/// Read a source's integer timestamp, in seconds or milliseconds, as RFC 3339 UTC.
#[must_use]
pub fn rfc3339_from_unix_auto(value: i64) -> String {
    rfc3339_from_unix_seconds(unix_seconds_auto(value))
}

/// The whole minute an instant falls in: seconds rounded **down**, never up.
///
/// Down rather than to the nearest, because a reset is a deadline: the minute it is
/// reported in is the minute it is still counting down through, and rounding `12:24:52`
/// up to `12:25:00` would show eight seconds of quota that are not there.
///
/// `div_euclid` rather than `/`, so that an instant before 1970 floors the same way one
/// after it does instead of rounding towards zero and landing a minute late.
#[must_use]
pub fn floor_to_minute(seconds: i64) -> i64 {
    seconds.div_euclid(60) * 60
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

/// Read an RFC 3339 timestamp back as Unix seconds.
///
/// The inverse of [`rfc3339_from_unix_seconds`], and the reason it exists is age: "did
/// this number arrive in the last quarter of an hour" cannot be answered by comparing two
/// strings. Fractional seconds are read and discarded — a window's age is not measured in
/// milliseconds — and a numeric offset is applied, so a timestamp another program wrote
/// with `+03:00` names the same instant here as it does there.
///
/// Returns `None` for anything [`sanitize_timestamp`] would reject, and for a date the
/// calendar does not have.
#[must_use]
pub fn unix_seconds_from_rfc3339(text: &str) -> Option<i64> {
    let text = sanitize_timestamp(text)?;
    let bytes = text.as_bytes();
    let number = |from: usize, to: usize| text.get(from..to)?.parse::<i64>().ok();

    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let days = days_from_civil(year, month as u32, day as u32);
    // The conversion accepts a day the month does not have (31 April becomes 1 May), so
    // the round trip is the validity check.
    if civil_from_days(days) != (year, month as u32, day as u32) {
        return None;
    }
    let mut seconds = days * 86_400 + hour * 3600 + minute * 60 + second;

    // A zone that is not `Z` is `±HH:MM`, and the last six characters are all of it.
    if !(text.ends_with('Z') || text.ends_with('z')) {
        let sign_at = text.len() - 6;
        let sign = match bytes[sign_at] {
            b'+' => 1,
            b'-' => -1,
            _ => return None,
        };
        let offset_hours = number(sign_at + 1, sign_at + 3)?;
        let offset_minutes = number(sign_at + 4, sign_at + 6)?;
        if offset_hours > 23 || offset_minutes > 59 {
            return None;
        }
        seconds -= sign * (offset_hours * 3600 + offset_minutes * 60);
    }
    Some(seconds)
}

/// Rewrite any RFC 3339 timestamp as the contract's form: UTC, whole seconds, `…Z`.
///
/// Rule 6 of `docs/limits-contract.md` is that every timestamp in `limits.json` ends in a
/// `Z`, so that consumers never have to work out an offset and never see two spellings of
/// the same instant. A source that writes Unix seconds gets that for free from
/// [`rfc3339_from_unix_seconds`]. A source that writes text does not: the usage endpoint
/// answers `2026-09-07T13:10:00.130195+00:00`, which is the same instant, is legal
/// RFC 3339, and is neither of the two things the contract promises.
///
/// So a string goes round the loop — read to seconds, written back out — which converts
/// the offset and drops sub-second precision nobody is counting down to. A string that
/// does not survive the trip is not a timestamp and yields nothing.
#[must_use]
pub fn rfc3339_utc(text: &str) -> Option<String> {
    unix_seconds_from_rfc3339(text).map(rfc3339_from_unix_seconds)
}

/// Seconds from `earlier` to `later`, when both are RFC 3339.
///
/// `None` when either side is not a timestamp; negative when `later` is the earlier one,
/// which happens on a machine whose clock went backwards and is not this function's
/// problem to hide.
#[must_use]
pub fn seconds_between(earlier: &str, later: &str) -> Option<i64> {
    Some(unix_seconds_from_rfc3339(later)? - unix_seconds_from_rfc3339(earlier)?)
}

/// A civil `(year, month, day)` to days since 1970-01-01.
///
/// Howard Hinnant's `days_from_civil`, the exact inverse of [`civil_from_days`]. It
/// accepts a day number the month does not have (31 April), so the caller checks the
/// round trip when that matters.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
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
    fn a_second_hand_is_rounded_down_to_the_minute_it_is_in() {
        // The live jitter this exists for: the usage endpoint answered `02:00:00Z` and
        // `01:59:59Z` for the same weekly reset, one refresh apart.
        let round = |text: &str| {
            rfc3339_from_unix_seconds(floor_to_minute(unix_seconds_from_rfc3339(text).unwrap()))
        };
        assert_eq!(round("2026-09-12T02:00:00Z"), "2026-09-12T02:00:00Z");
        assert_eq!(round("2026-09-12T01:59:59Z"), "2026-09-12T01:59:00Z");
        assert_eq!(round("2026-09-12T01:59:00Z"), "2026-09-12T01:59:00Z");

        // Down, never up, and on both sides of the epoch.
        assert_eq!(floor_to_minute(0), 0);
        assert_eq!(floor_to_minute(59), 0);
        assert_eq!(floor_to_minute(60), 60);
        assert_eq!(floor_to_minute(-1), -60);
        assert_eq!(floor_to_minute(-60), -60);
        assert_eq!(unix_seconds_auto(1_788_751_044_000), 1_788_751_044);
        assert_eq!(unix_seconds_auto(1_788_751_044), 1_788_751_044);
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
    fn timestamps_read_back_as_the_seconds_they_were_written_from() {
        for seconds in [
            0,
            1,
            951_782_400,    // 2000-02-29
            1_709_164_800,  // 2024-02-29
            1_788_751_044,  // the observed Codex five-hour reset
            1_788_783_892,  // and the weekly one
            -2_203_977_600, // 1900-02-28
            4_102_444_800,  // 2100-01-01
        ] {
            let text = rfc3339_from_unix_seconds(seconds);
            assert_eq!(
                unix_seconds_from_rfc3339(&text),
                Some(seconds),
                "{text} did not read back"
            );
        }
    }

    #[test]
    fn an_offset_names_the_same_instant_as_the_utc_form() {
        // 2026-09-07T03:17:24Z is 06:17:24 in Riyadh and 22:17:24 the previous day in
        // New York. All three have to be the same number of seconds.
        assert_eq!(
            unix_seconds_from_rfc3339("2026-09-07T03:17:24Z"),
            Some(1_788_751_044)
        );
        assert_eq!(
            unix_seconds_from_rfc3339("2026-09-07T06:17:24+03:00"),
            Some(1_788_751_044)
        );
        assert_eq!(
            unix_seconds_from_rfc3339("2026-09-06T23:17:24-04:00"),
            Some(1_788_751_044)
        );
        // Fractional seconds are read and dropped, not rejected.
        assert_eq!(
            unix_seconds_from_rfc3339("2026-09-07T03:17:24.987Z"),
            Some(1_788_751_044)
        );
    }

    #[test]
    fn a_date_the_calendar_does_not_have_is_rejected() {
        for value in [
            "2026-02-30T00:00:00Z",
            "2025-02-29T00:00:00Z", // 2025 is not a leap year
            "2026-04-31T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-00-10T00:00:00Z",
            "2026-09-00T00:00:00Z",
            "2026-09-07T24:00:00Z",
            "2026-09-07T00:60:00Z",
            "not a timestamp",
        ] {
            assert_eq!(
                unix_seconds_from_rfc3339(value),
                None,
                "should have rejected {value}"
            );
        }
        // A leap second is a real instant that a source may report; it is read, not refused.
        assert!(unix_seconds_from_rfc3339("2016-12-31T23:59:60Z").is_some());
    }

    /// The three timestamps observed live on 2026-09-07, and the one spelling the
    /// contract accepts for all of them.
    #[test]
    fn every_source_is_rewritten_into_the_contracts_one_spelling() {
        // The usage endpoint: microseconds and a numeric offset, not a `Z` in sight.
        assert_eq!(
            rfc3339_utc("2026-09-07T13:10:00.130195+00:00").as_deref(),
            Some("2026-09-07T13:10:00Z")
        );
        assert_eq!(
            rfc3339_utc("2026-09-12T02:00:00.130216+00:00").as_deref(),
            Some("2026-09-12T02:00:00Z")
        );
        // A real offset is converted, not truncated.
        assert_eq!(
            rfc3339_utc("2026-09-07T16:10:00+03:00").as_deref(),
            Some("2026-09-07T13:10:00Z")
        );
        // Already right, and left alone.
        assert_eq!(
            rfc3339_utc("2026-09-07T13:10:00Z").as_deref(),
            Some("2026-09-07T13:10:00Z")
        );
        // And the status line's own encoding lands in the same place.
        assert_eq!(
            rfc3339_from_unix_auto(1_788_786_600),
            "2026-09-07T13:10:00Z"
        );

        assert_eq!(rfc3339_utc("2026-02-30T00:00:00Z"), None);
        assert_eq!(rfc3339_utc("whenever you like"), None);
    }

    #[test]
    fn the_gap_between_two_timestamps_is_signed_seconds() {
        assert_eq!(
            seconds_between("2026-09-07T03:00:00Z", "2026-09-07T03:15:00Z"),
            Some(900)
        );
        assert_eq!(
            seconds_between("2026-09-07T03:15:00Z", "2026-09-07T03:00:00Z"),
            Some(-900),
            "a clock that went backwards is reported, not hidden"
        );
        assert_eq!(seconds_between("nonsense", "2026-09-07T03:00:00Z"), None);
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
