//! Tests for the Claude reader, over a fake capture directory built from fixtures.
//!
//! The payload fixtures are sanitised copies of a real status-line payload: the numbers
//! are the ones Claude Code actually handed the status line, every path and identifier is
//! a placeholder. See `docs/pinned-internal-formats.md`.

use super::*;
use crate::limits::WindowState;
use crate::testutil::TempDir;

/// The real payload, as captured on the maintainer's machine: `seven_day` only, because
/// the five-hour window had reset and Claude Code drops a window once it has.
const REAL: &str = include_str!("../../../../fixtures/claude/statusline-payload.json");
/// The same payload with a live five-hour window beside the weekly one.
const BOTH: &str = include_str!("../../../../fixtures/claude/statusline-payload-both-windows.json");
/// The same payload with no `rate_limits` at all: not a Pro/Max subscriber, or before the
/// session's first API response.
const NONE: &str =
    include_str!("../../../../fixtures/claude/statusline-payload-no-rate-limits.json");

/// Wrap a payload the way `nazar-statusline` writes it.
fn envelope(payload: &str, captured_at: &str) -> String {
    format!(
        "{{\n  \"schemaVersion\": {CAPTURE_SCHEMA_VERSION},\n  \"updatedAt\": \
         \"{captured_at}\",\n  \"wrapper\": \"nazar-statusline/0.1.0\",\n  \"sessionId\": \
         \"session-under-test\",\n  \"payload\": {payload}\n}}\n"
    )
}

/// Write a capture file into `dir`.
fn plant(dir: &Path, session: &str, payload: &str, captured_at: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(format!("{session}.json"));
    std::fs::write(&path, envelope(payload, captured_at)).unwrap();
    path
}

fn reading(payload: &str, captured_at: &str) -> Reading {
    parse_capture(&envelope(payload, captured_at)).expect("the envelope must parse")
}

// ------------------------------------------------------------------ parsing a capture

#[test]
fn the_real_payload_yields_the_weekly_window_and_nothing_else() {
    let reading = reading(REAL, "2026-09-07T06:40:00Z");

    assert!(reading.has_rate_limits);
    assert_eq!(reading.captured_at.as_deref(), Some("2026-09-07T06:40:00Z"));
    assert_eq!(
        reading.five_hour, None,
        "this payload had no five-hour window"
    );

    let seven_day = reading.seven_day.unwrap();
    assert_eq!(seven_day.used_percentage, 31.0);
    assert_eq!(seven_day.resets_at.as_deref(), Some("2026-09-12T02:00:00Z"));

    // Neither `model.id` nor `version` names a subscription, so no plan is derived.
    assert_eq!(reading.plan, None);
}

#[test]
fn both_windows_come_through_when_both_are_reported() {
    let reading = reading(BOTH, "2026-09-07T06:40:00Z");

    let five_hour = reading.five_hour.unwrap();
    assert_eq!(five_hour.used_percentage, 12.0);
    assert_eq!(five_hour.resets_at.as_deref(), Some("2026-09-07T08:00:00Z"));

    let seven_day = reading.seven_day.unwrap();
    assert_eq!(seven_day.used_percentage, 31.0);
    assert_eq!(seven_day.resets_at.as_deref(), Some("2026-09-12T02:00:00Z"));
}

#[test]
fn a_payload_without_rate_limits_is_recognised_as_such() {
    let reading = reading(NONE, "2026-09-07T06:40:00Z");
    assert!(!reading.has_rate_limits);
    assert!(!reading.is_usable());
    assert_eq!(reading.five_hour, None);
    assert_eq!(reading.seven_day, None);
}

#[test]
fn a_file_that_is_not_a_capture_is_skipped_rather_than_read_as_empty() {
    for text in [
        "",
        "{",
        "not json at all",
        "[1,2,3]",
        r#"{"schemaVersion":1}"#,             // an envelope with no payload
        r#"{"payload":"a string"}"#,          // a payload that is not an object
        r#"{"session_id":"x","cwd":"/tmp"}"#, // a bare payload, not an envelope
    ] {
        assert_eq!(parse_capture(text), None, "{text:?} must not parse");
    }
}

#[test]
fn a_percentage_that_is_not_a_number_is_not_a_window() {
    for bad in [r#""31""#, "null", "true", "[31]", r#"{"value":31}"#] {
        let payload = format!(
            r#"{{"rate_limits":{{"five_hour":{{"used_percentage":{bad}}},
               "seven_day":{{"used_percentage":9}}}}}}"#
        );
        let reading = reading(&payload, "2026-09-07T06:40:00Z");
        assert_eq!(
            reading.five_hour, None,
            "used_percentage {bad} must not be read"
        );
        assert_eq!(reading.seven_day.unwrap().used_percentage, 9.0);
    }
}

#[test]
fn milliseconds_and_string_resets_are_both_understood() {
    let millis = r#"{"rate_limits":{"five_hour":{"used_percentage":1,
        "resets_at":1788768000000}}}"#;
    assert_eq!(
        reading(millis, "2026-09-07T06:40:00Z")
            .five_hour
            .unwrap()
            .resets_at
            .as_deref(),
        Some("2026-09-07T08:00:00Z")
    );

    let iso = r#"{"rate_limits":{"five_hour":{"used_percentage":1,
        "resets_at":"2026-09-07T08:00:00Z"}}}"#;
    assert_eq!(
        reading(iso, "2026-09-07T06:40:00Z")
            .five_hour
            .unwrap()
            .resets_at
            .as_deref(),
        Some("2026-09-07T08:00:00Z")
    );

    let prose = r#"{"rate_limits":{"five_hour":{"used_percentage":1,
        "resets_at":"in about five hours"}}}"#;
    assert_eq!(
        reading(prose, "2026-09-07T06:40:00Z")
            .five_hour
            .unwrap()
            .resets_at,
        None
    );
}

#[test]
fn unknown_keys_are_ignored_at_every_level() {
    let payload = r#"{"brand_new_top":[1,2,3],"model":{"id":"claude-fable-5-1"},
        "rate_limits":{"brand_new_limits":"whatever",
        "five_hour":{"used_percentage":3,"brand_new_window":true}}}"#;
    let reading = reading(payload, "2026-09-07T06:40:00Z");
    assert_eq!(reading.five_hour.unwrap().used_percentage, 3.0);
    assert!(reading.has_rate_limits);
}

/// The one that matters: every string in the payload is a sentinel, and none of it may
/// reach the reading or the provider block built from it.
#[test]
fn nothing_but_the_allow_listed_values_leaves_the_reader() {
    const SENTINEL: &str = "SENTINEL-do-not-leak-4c1e";
    let payload = format!(
        r#"{{
          "session_id": "{SENTINEL}",
          "session_name": "{SENTINEL}",
          "transcript_path": "{SENTINEL}",
          "scratchpad_dir": "{SENTINEL}",
          "cwd": "{SENTINEL}",
          "prompt_id": "{SENTINEL}",
          "model": {{"id": "{SENTINEL}", "display_name": "{SENTINEL}"}},
          "effort": {{"level": "{SENTINEL}"}},
          "version": "{SENTINEL}",
          "output_style": {{"name": "{SENTINEL}"}},
          "workspace": {{
            "current_dir": "{SENTINEL}",
            "project_dir": "{SENTINEL}",
            "added_dirs": ["{SENTINEL}"],
            "repo": {{"host": "{SENTINEL}", "owner": "{SENTINEL}", "name": "{SENTINEL}"}}
          }},
          "cost": {{"total_cost_usd": 9.6, "total_duration_ms": 7051994}},
          "context_window": {{"used_percentage": 16, "context_window_size": 1000000}},
          "prompt_cache": {{"ttl": "{SENTINEL}", "last_miss_cause": "{SENTINEL}"}},
          "plan": "{SENTINEL} with spaces",
          "rate_limits": {{
            "note": "{SENTINEL}",
            "five_hour": {{"used_percentage": 12, "resets_at": 1788768000, "label": "{SENTINEL}"}},
            "seven_day": {{"used_percentage": 31, "resets_at": 1789178400, "label": "{SENTINEL}"}}
          }}
        }}"#
    );

    let reading = reading(&payload, "2026-09-07T06:40:00Z");

    // The numbers still come through: an allow-list is not a wall.
    assert_eq!(reading.five_hour.as_ref().unwrap().used_percentage, 12.0);
    assert_eq!(reading.seven_day.as_ref().unwrap().used_percentage, 31.0);
    // The plan failed its shape check, so it did not come through.
    assert_eq!(reading.plan, None);

    let rendered = format!("{reading:?}");
    assert!(
        !rendered.contains("SENTINEL"),
        "the reader leaked payload text: {rendered}"
    );

    let block = readable(&reading, "2026-09-07T06:40:00Z");
    let serialised = serde_json::to_string(&block).unwrap();
    assert!(
        !serialised.contains("SENTINEL"),
        "the provider block leaked payload text: {serialised}"
    );
}

// --------------------------------------------------------------- the provider block

#[test]
fn a_missing_directory_is_unconfigured_not_a_zero() {
    let dir = TempDir::new("claude-missing");
    let mut reader = ClaudeReader::new(dir.join("statusline"));
    let provider = reader.refresh();

    assert!(!provider.configured);
    assert!(provider.windows.is_empty());
    assert_eq!(provider.source, None);
}

#[test]
fn an_empty_directory_is_unconfigured_too() {
    let dir = TempDir::new("claude-empty");
    std::fs::create_dir_all(dir.join("statusline")).unwrap();
    let mut reader = ClaudeReader::new(dir.join("statusline"));
    assert!(!reader.refresh().configured);
}

#[test]
fn the_chain_file_alone_is_not_a_capture() {
    let dir = TempDir::new("claude-chain-only");
    let captures = dir.join("statusline");
    std::fs::create_dir_all(&captures).unwrap();
    std::fs::write(
        captures.join(CHAIN_FILE_NAME),
        r#"{"schemaVersion":1,"previous":{"type":"command","command":"npx ccstatusline@latest"}}"#,
    )
    .unwrap();

    let mut reader = ClaudeReader::new(&captures);
    let provider = reader.refresh();
    assert!(
        !provider.configured,
        "the wrapper's own state file must not be read as a capture"
    );
}

#[test]
fn a_real_capture_becomes_the_provider_block_the_contract_describes() {
    let dir = TempDir::new("claude-real");
    let captures = dir.join("statusline");
    plant(&captures, "session-a", BOTH, "2026-09-07T06:40:00Z");

    let mut reader = ClaudeReader::new(&captures);
    let provider = reader.refresh();

    assert!(provider.configured);
    assert_eq!(provider.source, Some(Source::Statusline));
    assert_eq!(provider.source_at.as_deref(), Some("2026-09-07T06:40:00Z"));
    assert_eq!(provider.plan, None);
    assert_eq!(provider.binding.as_deref(), Some(WINDOW_SEVEN_DAY));

    let five_hour = &provider.windows[WINDOW_FIVE_HOUR];
    assert_eq!(five_hour.percent, Some(12.0));
    assert_eq!(five_hour.window_minutes, Some(300));
    assert_eq!(five_hour.resets_at.as_deref(), Some("2026-09-07T08:00:00Z"));
    assert_eq!(five_hour.state, WindowState::Ok);

    let seven_day = &provider.windows[WINDOW_SEVEN_DAY];
    assert_eq!(seven_day.percent, Some(31.0));
    assert_eq!(seven_day.window_minutes, Some(10080));
    assert_eq!(seven_day.resets_at.as_deref(), Some("2026-09-12T02:00:00Z"));
    assert_eq!(seven_day.state, WindowState::Ok);
}

#[test]
fn a_window_the_payload_did_not_carry_is_absent_rather_than_zero() {
    let dir = TempDir::new("claude-one-window");
    let captures = dir.join("statusline");
    plant(&captures, "session-a", REAL, "2026-09-07T06:40:00Z");

    let provider = ClaudeReader::new(&captures).refresh();
    assert!(provider.configured);
    assert_eq!(provider.windows.len(), 1);
    assert!(!provider.windows.contains_key(WINDOW_FIVE_HOUR));
    assert_eq!(provider.binding.as_deref(), Some(WINDOW_SEVEN_DAY));
}

#[test]
fn a_payload_without_rate_limits_names_both_windows_with_no_percentage() {
    let dir = TempDir::new("claude-no-limits");
    let captures = dir.join("statusline");
    plant(&captures, "session-a", NONE, "2026-09-07T06:40:00Z");

    let provider = ClaudeReader::new(&captures).refresh();

    assert!(
        provider.configured,
        "the wrapper is installed; we just cannot see numbers"
    );
    assert_eq!(provider.binding, None);
    for key in [WINDOW_FIVE_HOUR, WINDOW_SEVEN_DAY] {
        let window = &provider.windows[key];
        assert_eq!(window.state, WindowState::Error);
        assert_eq!(
            window.percent, None,
            "{key} must not carry an invented number"
        );
        assert!(
            window
                .error
                .as_deref()
                .is_some_and(|why| why.contains("rate_limits"))
        );
    }
    assert_eq!(provider.windows[WINDOW_FIVE_HOUR].window_minutes, Some(300));
    assert_eq!(
        provider.windows[WINDOW_SEVEN_DAY].window_minutes,
        Some(10080)
    );
}

#[test]
fn unreadable_captures_report_rather_than_guess() {
    let dir = TempDir::new("claude-garbage");
    let captures = dir.join("statusline");
    std::fs::create_dir_all(&captures).unwrap();
    std::fs::write(captures.join("session-a.json"), "half a doc").unwrap();

    let provider = ClaudeReader::new(&captures).refresh();
    assert!(provider.configured);
    for key in [WINDOW_FIVE_HOUR, WINDOW_SEVEN_DAY] {
        assert_eq!(provider.windows[key].state, WindowState::Error);
        assert_eq!(provider.windows[key].percent, None);
    }
}

#[test]
fn the_newest_of_three_concurrent_sessions_wins() {
    let dir = TempDir::new("claude-three");
    let captures = dir.join("statusline");

    let older = r#"{"rate_limits":{"seven_day":{"used_percentage":10,"resets_at":1789178400}}}"#;
    let newest = r#"{"rate_limits":{"seven_day":{"used_percentage":31,"resets_at":1789178400}}}"#;
    let middle = r#"{"rate_limits":{"seven_day":{"used_percentage":20,"resets_at":1789178400}}}"#;

    plant(&captures, "session-a", older, "2026-09-07T06:00:00Z");
    plant(&captures, "session-c", newest, "2026-09-07T06:40:00Z");
    plant(&captures, "session-b", middle, "2026-09-07T06:20:00Z");

    let provider = ClaudeReader::new(&captures).refresh();
    assert_eq!(provider.windows[WINDOW_SEVEN_DAY].percent, Some(31.0));
    assert_eq!(provider.source_at.as_deref(), Some("2026-09-07T06:40:00Z"));
}

#[test]
fn a_capture_without_a_stamp_falls_back_to_the_file_time() {
    let dir = TempDir::new("claude-no-stamp");
    let captures = dir.join("statusline");
    std::fs::create_dir_all(&captures).unwrap();
    std::fs::write(
        captures.join("session-a.json"),
        r#"{"schemaVersion":1,"payload":{"rate_limits":{"seven_day":{"used_percentage":31}}}}"#,
    )
    .unwrap();

    let provider = ClaudeReader::new(&captures).refresh();
    assert_eq!(provider.windows[WINDOW_SEVEN_DAY].percent, Some(31.0));
    let source_at = provider
        .source_at
        .expect("a capture always gets a source time");
    assert!(
        crate::timefmt::sanitize_timestamp(&source_at).is_some(),
        "got {source_at}"
    );
}

#[test]
fn the_binding_window_is_the_highest_and_ties_go_to_the_shorter_one() {
    let dir = TempDir::new("claude-binding");
    let captures = dir.join("statusline");

    let five_higher = r#"{"rate_limits":{"five_hour":{"used_percentage":80},"seven_day":{"used_percentage":31}}}"#;
    plant(&captures, "session-a", five_higher, "2026-09-07T06:00:00Z");
    assert_eq!(
        ClaudeReader::new(&captures).refresh().binding.as_deref(),
        Some(WINDOW_FIVE_HOUR)
    );

    let tied = r#"{"rate_limits":{"five_hour":{"used_percentage":31},"seven_day":{"used_percentage":31}}}"#;
    plant(&captures, "session-a", tied, "2026-09-07T06:01:00Z");
    assert_eq!(
        ClaudeReader::new(&captures).refresh().binding.as_deref(),
        Some(WINDOW_FIVE_HOUR),
        "a tie goes to the window you hit first"
    );
}

#[test]
fn a_plan_name_is_taken_only_when_the_payload_says_plan() {
    let with_plan = r#"{"plan":"max_20x","rate_limits":{"seven_day":{"used_percentage":31}}}"#;
    assert_eq!(
        reading(with_plan, "2026-09-07T06:40:00Z").plan.as_deref(),
        Some("max_20x")
    );

    let model_only = r#"{"model":{"id":"claude-fable-5-1","display_name":"Fable 5.1"},
        "version":"2.1.263","rate_limits":{"seven_day":{"used_percentage":31}}}"#;
    assert_eq!(
        reading(model_only, "2026-09-07T06:40:00Z").plan,
        None,
        "a model id is not a subscription and must not be guessed into one"
    );
}

#[test]
fn the_reader_survives_a_directory_full_of_junk() {
    let dir = TempDir::new("claude-junk");
    let captures = dir.join("statusline");
    std::fs::create_dir_all(&captures).unwrap();
    std::fs::write(captures.join("notes.txt"), "not a capture").unwrap();
    std::fs::create_dir_all(captures.join("a-directory.json")).unwrap();
    plant(&captures, "session-a", BOTH, "2026-09-07T06:40:00Z");

    let provider = ClaudeReader::new(&captures).refresh();
    assert_eq!(provider.windows[WINDOW_SEVEN_DAY].percent, Some(31.0));
}
