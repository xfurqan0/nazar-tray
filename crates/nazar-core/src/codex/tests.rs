//! End-to-end tests for the Codex reader, over a fake `CODEX_HOME` built from fixtures.
//!
//! The fixtures are sanitised copies of the maintainer's real logs: the quota numbers are
//! the ones the server actually reported, every text field is a placeholder. See
//! `docs/pinned-internal-formats.md` for what was kept and why.

use super::*;
use crate::limits::WindowState;
use crate::testutil::TempDir;

/// Thirteen real quota lines from one session, oldest first.
const SAMPLE: &str = include_str!("../../../../fixtures/codex/rollout-sample.jsonl");
/// One real line of the second limit family, whose windows are both `null`.
const PREMIUM: &str = include_str!("../../../../fixtures/codex/rollout-premium-null.jsonl");
/// Six real non-quota entries.
const NO_QUOTA: &str = include_str!("../../../../fixtures/codex/rollout-no-rate-limits.jsonl");
/// Hand-written damage: truncated JSON, wrong types, a stray blank line.
const MALFORMED: &str = include_str!("../../../../fixtures/codex/rollout-malformed.jsonl");

/// Write `contents` to `<home>/sessions/<date>/rollout-<label>.jsonl`.
fn plant(home: &Path, date: &str, label: &str, contents: &str) -> PathBuf {
    let dir = locate::sessions_dir(home).join(date.replace('/', std::path::MAIN_SEPARATOR_STR));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("rollout-{label}.jsonl"));
    std::fs::write(&path, contents).unwrap();
    path
}

/// Stamp a file's modification time, so "newest" is a fact rather than a race.
fn age(path: &Path, seconds: u64) {
    let when = SystemTime::now() - std::time::Duration::from_secs(seconds);
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

fn append(path: &Path, contents: &str) {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(contents.as_bytes())
        .unwrap();
}

// ---------------------------------------------------------------- home discovery

#[test]
fn codex_home_defaults_to_the_dot_codex_directory_under_home() {
    // The variable is read from the real environment, so this only asserts the default
    // when the machine running the tests has not set one.
    let Ok(expected) = home_dir().map(|home| home.join(".codex")) else {
        return;
    };
    if std::env::var_os(CODEX_HOME_VAR).is_none() {
        assert_eq!(codex_home().unwrap(), expected);
    }
    assert_eq!(CODEX_HOME_VAR, "CODEX_HOME");
}

#[test]
fn the_environment_variable_overrides_the_default_and_an_empty_one_does_not() {
    use std::ffi::OsStr;

    assert_eq!(
        resolve_home(Some(OsStr::new("D:\\somewhere\\codex"))),
        Some(PathBuf::from("D:\\somewhere\\codex"))
    );
    assert_eq!(
        resolve_home(Some(OsStr::new("/opt/codex"))),
        Some(PathBuf::from("/opt/codex"))
    );
    assert_eq!(resolve_home(None), None, "unset falls back to ~/.codex");
    assert_eq!(
        resolve_home(Some(OsStr::new(""))),
        None,
        "an empty variable must not resolve to the working directory"
    );
}

#[test]
fn an_overridden_home_is_read_end_to_end() {
    // The same path `CODEX_HOME` takes, without mutating the process environment: a
    // reader built on an explicit directory reads it and nothing else.
    let dir = TempDir::new("codex-override");
    plant(&dir.path, "2026/09/07", "session", SAMPLE);

    let provider = CodexReader::new(&dir.path).refresh();
    assert!(provider.configured);
    assert_eq!(provider.windows[WINDOW_SECONDARY].percent, Some(70.0));

    // And a reader pointed somewhere else does not see it.
    let elsewhere = TempDir::new("codex-override-empty");
    assert!(
        !CodexReader::new(elsewhere.join("absent"))
            .refresh()
            .configured
    );
}

// ---------------------------------------------------------------- absent provider

#[test]
fn an_absent_codex_home_is_unconfigured() {
    let dir = TempDir::new("codex-absent");
    let mut reader = CodexReader::new(dir.join("no-codex-here"));
    let provider = reader.refresh();

    assert!(!provider.configured);
    assert!(provider.windows.is_empty());
    assert_eq!(provider.binding, None);
    assert_eq!(provider.plan, None);
}

#[test]
fn a_codex_home_without_logs_is_configured_but_unreadable() {
    let dir = TempDir::new("codex-no-logs");
    std::fs::create_dir_all(locate::sessions_dir(&dir.path)).unwrap();

    let provider = CodexReader::new(&dir.path).refresh();

    assert!(provider.configured);
    assert_eq!(provider.source, Some(Source::Rollout));
    assert_eq!(provider.binding, None, "nothing was read, so nothing binds");
    assert_eq!(provider.windows.len(), 2);
    for key in [WINDOW_PRIMARY, WINDOW_SECONDARY] {
        let window = &provider.windows[key];
        assert_eq!(window.state, WindowState::Error);
        assert_eq!(window.percent, None, "an unread window has no percentage");
        assert!(window.error.is_some());
    }
}

#[test]
fn logs_without_a_quota_line_report_the_reason_and_no_percentage() {
    let dir = TempDir::new("codex-no-quota");
    plant(&dir.path, "2026/09/07", "stub", NO_QUOTA);

    let provider = CodexReader::new(&dir.path).refresh();

    assert!(provider.configured);
    assert_eq!(provider.windows.len(), 2);
    let error = provider.windows[WINDOW_PRIMARY].error.clone().unwrap();
    assert!(
        error.contains("no rate_limits line in the newest"),
        "got {error}"
    );
    for window in provider.windows.values() {
        assert_eq!(window.percent, None);
        assert_eq!(window.state, WindowState::Error);
    }
}

// ---------------------------------------------------------------- the happy path

#[test]
fn the_real_sample_maps_to_the_contract() {
    let dir = TempDir::new("codex-sample");
    plant(&dir.path, "2026/09/07", "session", SAMPLE);

    let provider = CodexReader::new(&dir.path).refresh();

    assert!(provider.configured);
    assert_eq!(provider.plan.as_deref(), Some("plus"));
    assert_eq!(provider.source, Some(Source::Rollout));
    assert_eq!(
        provider.source_at.as_deref(),
        Some("2026-09-06T22:55:05.479Z")
    );
    assert_eq!(provider.binding.as_deref(), Some(WINDOW_SECONDARY));

    let primary = &provider.windows[WINDOW_PRIMARY];
    assert_eq!(primary.percent, Some(54.0));
    assert_eq!(primary.window_minutes, Some(PRIMARY_WINDOW_MINUTES));
    assert_eq!(primary.resets_at.as_deref(), Some("2026-09-07T03:17:24Z"));
    assert_eq!(primary.state, WindowState::Ok);

    let secondary = &provider.windows[WINDOW_SECONDARY];
    assert_eq!(secondary.percent, Some(70.0));
    assert_eq!(secondary.window_minutes, Some(SECONDARY_WINDOW_MINUTES));
    assert_eq!(secondary.resets_at.as_deref(), Some("2026-09-07T12:24:52Z"));
    assert_eq!(secondary.state, WindowState::Ok);
}

#[test]
fn the_provider_block_serialises_the_way_the_contract_document_says() {
    let dir = TempDir::new("codex-serialise");
    plant(&dir.path, "2026/09/07", "session", SAMPLE);

    let provider = CodexReader::new(&dir.path).refresh();
    let json = serde_json::to_string_pretty(&provider).unwrap();

    for expected in [
        "\"configured\": true",
        "\"plan\": \"plus\"",
        "\"source\": \"rollout\"",
        "\"binding\": \"secondary\"",
        "\"percent\": 54",
        "\"windowMinutes\": 300",
        "\"resetsAt\": \"2026-09-07T12:24:52Z\"",
        "\"state\": \"ok\"",
    ] {
        assert!(json.contains(expected), "missing {expected} in\n{json}");
    }
    assert!(
        !json.contains("rate_limits"),
        "raw source keys must not survive"
    );
    assert!(!json.contains("thread_token_usage"));
    assert!(
        !json.contains("redacted"),
        "no source text may reach the file"
    );
}

#[test]
fn the_newest_log_wins_even_when_an_older_one_has_higher_numbers() {
    let dir = TempDir::new("codex-newest");
    let old = plant(&dir.path, "2026/09/05", "old", SAMPLE);
    let new = plant(
        &dir.path,
        "2026/09/07",
        "new",
        &line_with(3.0, 4.0, "2026-09-07T09:00:00.000Z"),
    );
    age(&old, 90_000);
    age(&new, 5);

    let provider = CodexReader::new(&dir.path).refresh();
    assert_eq!(provider.windows[WINDOW_PRIMARY].percent, Some(3.0));
    assert_eq!(
        provider.source_at.as_deref(),
        Some("2026-09-07T09:00:00.000Z")
    );
}

#[test]
fn a_stub_log_falls_through_to_the_next_candidate() {
    let dir = TempDir::new("codex-stub");
    // Exactly the maintainer's machine: the newest log is a three-line stub from a
    // session that ended before the first response, and the real numbers are one back.
    let real = plant(&dir.path, "2026/09/07", "real", SAMPLE);
    let stub = plant(&dir.path, "2026/09/07", "stub", NO_QUOTA);
    age(&real, 600);
    age(&stub, 5);

    let provider = CodexReader::new(&dir.path).refresh();
    assert_eq!(provider.windows[WINDOW_PRIMARY].percent, Some(54.0));
    assert_eq!(provider.windows[WINDOW_SECONDARY].percent, Some(70.0));
}

#[test]
fn the_search_stops_after_the_cap_even_if_the_answer_is_one_file_further() {
    let dir = TempDir::new("codex-cap");
    // Six stubs newer than the one real log: the cap is five, so the real log is out of
    // reach and the reader says so instead of reading the whole history.
    let real = plant(&dir.path, "2026/09/01", "real", SAMPLE);
    age(&real, 100_000);
    for index in 0..MAX_FILES_OPENED + 1 {
        let stub = plant(&dir.path, "2026/09/07", &format!("stub{index}"), NO_QUOTA);
        age(&stub, (index as u64) + 1);
    }

    let provider = CodexReader::new(&dir.path).refresh();
    assert_eq!(provider.windows[WINDOW_PRIMARY].percent, None);
    assert_eq!(provider.windows[WINDOW_PRIMARY].state, WindowState::Error);
}

// ---------------------------------------------------------------- schema tolerance

/// Build a minimal but realistically shaped quota line.
fn line_with(primary: f64, secondary: f64, timestamp: &str) -> String {
    format!(
        r#"{{"timestamp":"{timestamp}","ordinal":1,"type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"plan_type":"plus","primary":{{"used_percent":{primary},"window_minutes":300,"resets_at":1788751044}},"secondary":{{"used_percent":{secondary},"window_minutes":10080,"resets_at":1788783892}}}}}}}}
"#
    )
}

#[test]
fn a_null_window_line_arriving_last_does_not_erase_the_reading() {
    let dir = TempDir::new("codex-premium");
    // The `limit_id: "premium"` line, whose two windows are null, is the last line of the
    // file. Reading it as "no windows" would blank a perfectly good display.
    plant(
        &dir.path,
        "2026/09/07",
        "session",
        &format!("{SAMPLE}{PREMIUM}"),
    );

    let provider = CodexReader::new(&dir.path).refresh();
    assert_eq!(provider.windows[WINDOW_PRIMARY].percent, Some(54.0));
    assert_eq!(provider.windows[WINDOW_SECONDARY].percent, Some(70.0));
    assert_eq!(
        provider.source_at.as_deref(),
        Some("2026-09-06T22:55:05.479Z")
    );
}

#[test]
fn malformed_lines_are_skipped_and_counted() {
    let dir = TempDir::new("codex-malformed");
    plant(&dir.path, "2026/09/07", "session", MALFORMED);

    let mut reader = CodexReader::new(&dir.path);
    let provider = reader.refresh();

    assert_eq!(
        provider.windows[WINDOW_PRIMARY].percent,
        Some(42.0),
        "the one good line in a damaged file must still be read"
    );
    assert!(
        reader.warnings() >= 3,
        "the damaged lines must be counted, got {}",
        reader.warnings()
    );
}

#[test]
fn a_missing_secondary_leaves_one_window_rather_than_inventing_two() {
    let dir = TempDir::new("codex-one-window");
    plant(
        &dir.path,
        "2026/09/07",
        "session",
        "{\"timestamp\":\"2026-09-07T09:00:00.000Z\",\"payload\":{\"rate_limits\":{\"plan_type\":\"plus\",\"primary\":{\"used_percent\":12,\"window_minutes\":300}}}}\n",
    );

    let provider = CodexReader::new(&dir.path).refresh();
    assert_eq!(provider.windows.len(), 1);
    assert_eq!(provider.windows[WINDOW_PRIMARY].percent, Some(12.0));
    assert_eq!(provider.binding.as_deref(), Some(WINDOW_PRIMARY));
    assert!(!provider.windows.contains_key(WINDOW_SECONDARY));
}

#[test]
fn a_window_without_minutes_keeps_the_window_and_drops_the_field() {
    let dir = TempDir::new("codex-no-minutes");
    plant(
        &dir.path,
        "2026/09/07",
        "session",
        "{\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":7.5}}}}\n",
    );

    let provider = CodexReader::new(&dir.path).refresh();
    let window = &provider.windows[WINDOW_PRIMARY];
    assert_eq!(window.percent, Some(7.5));
    assert_eq!(window.window_minutes, None);
    assert_eq!(window.resets_at, None);
    assert_eq!(window.state, WindowState::Ok);
}

#[test]
fn a_line_without_a_timestamp_falls_back_to_the_file_time() {
    let dir = TempDir::new("codex-no-timestamp");
    let path = plant(
        &dir.path,
        "2026/09/07",
        "session",
        "{\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":7.5}}}}\n",
    );
    age(&path, 0);

    let provider = CodexReader::new(&dir.path).refresh();
    let source_at = provider.source_at.unwrap();
    assert!(
        crate::timefmt::sanitize_timestamp(&source_at).is_some(),
        "the fallback must still be a timestamp, got {source_at}"
    );
}

// ---------------------------------------------------------------- binding window

#[test]
fn the_binding_window_is_the_fullest_one() {
    let dir = TempDir::new("codex-binding");
    let path = plant(
        &dir.path,
        "2026/09/07",
        "session",
        &line_with(54.0, 70.0, "2026-09-07T09:00:00.000Z"),
    );
    assert_eq!(
        CodexReader::new(&dir.path).refresh().binding.as_deref(),
        Some(WINDOW_SECONDARY)
    );

    std::fs::write(&path, line_with(91.0, 70.0, "2026-09-07T09:01:00.000Z")).unwrap();
    assert_eq!(
        CodexReader::new(&dir.path).refresh().binding.as_deref(),
        Some(WINDOW_PRIMARY)
    );
}

#[test]
fn a_tie_binds_the_shorter_window() {
    let dir = TempDir::new("codex-tie");
    plant(
        &dir.path,
        "2026/09/07",
        "session",
        &line_with(61.0, 61.0, "2026-09-07T09:00:00.000Z"),
    );
    assert_eq!(
        CodexReader::new(&dir.path).refresh().binding.as_deref(),
        Some(WINDOW_PRIMARY),
        "equally full windows: the five-hour one is the one you hit first"
    );
}

#[test]
fn a_zero_percent_window_still_counts_as_read() {
    let dir = TempDir::new("codex-zero");
    plant(
        &dir.path,
        "2026/09/07",
        "session",
        &line_with(0.0, 35.0, "2026-09-07T09:00:00.000Z"),
    );

    let provider = CodexReader::new(&dir.path).refresh();
    assert_eq!(
        provider.windows[WINDOW_PRIMARY].percent,
        Some(0.0),
        "a reported zero is a number; only an unread window has none"
    );
    assert_eq!(provider.windows[WINDOW_PRIMARY].state, WindowState::Ok);
    assert_eq!(provider.binding.as_deref(), Some(WINDOW_SECONDARY));
}

// ---------------------------------------------------------------- incremental reads

#[test]
fn a_second_poll_sees_a_line_appended_since_the_first() {
    let dir = TempDir::new("codex-incremental");
    let path = plant(
        &dir.path,
        "2026/09/07",
        "session",
        &line_with(10.0, 20.0, "2026-09-07T09:00:00.000Z"),
    );

    let mut reader = CodexReader::new(&dir.path);
    assert_eq!(reader.refresh().windows[WINDOW_PRIMARY].percent, Some(10.0));

    append(&path, &line_with(30.0, 40.0, "2026-09-07T09:05:00.000Z"));
    let provider = reader.refresh();
    assert_eq!(provider.windows[WINDOW_PRIMARY].percent, Some(30.0));
    assert_eq!(
        provider.source_at.as_deref(),
        Some("2026-09-07T09:05:00.000Z")
    );
}

#[test]
fn a_poll_that_finds_nothing_new_keeps_the_last_reading() {
    let dir = TempDir::new("codex-sticky");
    let path = plant(
        &dir.path,
        "2026/09/07",
        "session",
        &line_with(10.0, 20.0, "2026-09-07T09:00:00.000Z"),
    );

    let mut reader = CodexReader::new(&dir.path);
    let first = reader.refresh();
    // Nothing appended, and a line that is not a quota line.
    append(&path, NO_QUOTA);
    let second = reader.refresh();

    assert_eq!(first.windows, second.windows);
    assert_eq!(first.source_at, second.source_at);
}

#[test]
fn a_new_session_log_takes_over_from_the_one_being_followed() {
    let dir = TempDir::new("codex-handover");
    let first = plant(
        &dir.path,
        "2026/09/07",
        "aaa-first",
        &line_with(10.0, 20.0, "2026-09-07T09:00:00.000Z"),
    );
    age(&first, 60);

    let mut reader = CodexReader::new(&dir.path);
    assert_eq!(reader.refresh().windows[WINDOW_PRIMARY].percent, Some(10.0));

    let second = plant(
        &dir.path,
        "2026/09/07",
        "bbb-second",
        &line_with(80.0, 20.0, "2026-09-07T09:30:00.000Z"),
    );
    age(&second, 1);

    assert_eq!(reader.refresh().windows[WINDOW_PRIMARY].percent, Some(80.0));
}

#[test]
fn a_truncated_log_is_read_again_from_the_start() {
    let dir = TempDir::new("codex-truncate");
    let path = plant(
        &dir.path,
        "2026/09/07",
        "session",
        &format!(
            "{}{}",
            line_with(10.0, 20.0, "2026-09-07T09:00:00.000Z"),
            line_with(11.0, 21.0, "2026-09-07T09:01:00.000Z")
        ),
    );

    let mut reader = CodexReader::new(&dir.path);
    assert_eq!(reader.refresh().windows[WINDOW_PRIMARY].percent, Some(11.0));

    // Same name, shorter file: a new session reused the path.
    std::fs::write(&path, line_with(2.0, 3.0, "2026-09-07T10:00:00.000Z")).unwrap();
    assert_eq!(reader.refresh().windows[WINDOW_PRIMARY].percent, Some(2.0));
}

#[test]
fn a_log_written_with_crlf_reads_the_same() {
    let dir = TempDir::new("codex-crlf");
    let contents = SAMPLE.replace('\n', "\r\n");
    plant(&dir.path, "2026/09/07", "session", &contents);

    let provider = CodexReader::new(&dir.path).refresh();
    assert_eq!(provider.windows[WINDOW_PRIMARY].percent, Some(54.0));
    assert_eq!(provider.windows[WINDOW_SECONDARY].percent, Some(70.0));
}

#[test]
fn a_line_still_being_written_is_read_once_it_is_whole() {
    let dir = TempDir::new("codex-partial");
    let whole = line_with(77.0, 20.0, "2026-09-07T09:00:00.000Z");
    let (head, tail_bytes) = whole.split_at(whole.len() / 2);

    let path = plant(&dir.path, "2026/09/07", "session", head);
    let mut reader = CodexReader::new(&dir.path);
    let provider = reader.refresh();
    assert_eq!(
        provider.windows[WINDOW_PRIMARY].percent, None,
        "half a line must not become a reading"
    );

    append(&path, tail_bytes);
    assert_eq!(reader.refresh().windows[WINDOW_PRIMARY].percent, Some(77.0));
}

#[test]
fn a_quota_line_far_from_the_end_of_a_long_log_is_still_found() {
    let dir = TempDir::new("codex-far");
    // The quota line, then more than the tail window's worth of ordinary entries after
    // it. Only the widened pass can reach it.
    let mut contents = line_with(64.0, 21.0, "2026-09-07T09:00:00.000Z");
    let filler = format!(
        "{{\"timestamp\":\"2026-09-07T09:00:01.000Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"item_completed\",\"pad\":\"{}\"}}}}\n",
        "p".repeat(4096)
    );
    while contents.len() < tail::INITIAL_WINDOW as usize + 8192 {
        contents.push_str(&filler);
    }
    plant(&dir.path, "2026/09/07", "session", &contents);

    let provider = CodexReader::new(&dir.path).refresh();
    assert_eq!(provider.windows[WINDOW_PRIMARY].percent, Some(64.0));
}

// ---------------------------------------------------------------- no leaks

#[test]
fn no_source_text_reaches_the_provider_block() {
    const SENTINEL: &str = "SENTINEL-do-not-leak-9c1d";
    let dir = TempDir::new("codex-leak");
    let line = format!(
        r#"{{"timestamp":"{SENTINEL}","type":"{SENTINEL}","cwd":"{SENTINEL}","payload":{{"type":"{SENTINEL}","session_id":"{SENTINEL}","prompt":"{SENTINEL}","aggregated_output":"{SENTINEL}","thread_token_usage":{{"total_tokens":1,"note":"{SENTINEL}"}},"rate_limits":{{"limit_id":"{SENTINEL}","limit_name":"{SENTINEL}","plan_type":"{SENTINEL} plan","credits":{{"balance":"{SENTINEL}"}},"primary":{{"used_percent":54.0,"window_minutes":300,"resets_at":1788751044,"note":"{SENTINEL}"}},"secondary":{{"used_percent":70.0,"window_minutes":10080,"resets_at":1788783892,"note":"{SENTINEL}"}}}}}}}}
"#
    );
    plant(&dir.path, "2026/09/07", "session", &line);

    let provider = CodexReader::new(&dir.path).refresh();
    assert_eq!(provider.windows[WINDOW_PRIMARY].percent, Some(54.0));

    let json = serde_json::to_string(&provider).unwrap();
    assert!(
        !json.contains("SENTINEL"),
        "leaked into the contract: {json}"
    );
    assert!(!format!("{provider:?}").contains("SENTINEL"));
}

#[test]
fn no_path_from_the_machine_reaches_the_provider_block() {
    let dir = TempDir::new("codex-paths");
    plant(&dir.path, "2026/09/07", "session", NO_QUOTA);

    let provider = CodexReader::new(&dir.path).refresh();
    let json = serde_json::to_string(&provider).unwrap();

    // The error path is the one that historically leaked a path into a tooltip: the
    // prototype put the absolute path of the file it had failed to open into the message
    // it drew on screen, which is audit finding B11.
    let directory = dir.path.to_string_lossy().into_owned();
    assert!(!json.contains(&directory), "leaked a local path: {json}");
    assert!(!json.contains("sessions"), "leaked a path fragment: {json}");
    assert!(!json.contains(".codex"), "leaked a path fragment: {json}");
}

// ---------------------------------------------------------------- pure mapping

#[test]
fn the_mapping_is_a_pure_function_of_the_reading() {
    let quota = Quota {
        plan: Some("plus".to_owned()),
        source_at: Some("2026-09-07T09:00:00Z".to_owned()),
        primary: Some(parse::QuotaWindow {
            used_percent: 54.0,
            window_minutes: Some(300),
            resets_at: Some("2026-09-07T03:17:24Z".to_owned()),
        }),
        secondary: Some(parse::QuotaWindow {
            used_percent: 70.5,
            window_minutes: Some(10080),
            resets_at: None,
        }),
    };

    let provider = readable(&quota, "2026-09-07T09:00:00Z");
    assert_eq!(provider.binding.as_deref(), Some(WINDOW_SECONDARY));
    assert_eq!(provider.windows[WINDOW_SECONDARY].percent, Some(70.5));
    assert_eq!(provider.windows[WINDOW_SECONDARY].resets_at, None);
    assert_eq!(provider.windows[WINDOW_SECONDARY].state, WindowState::Ok);

    // Serialising an unrounded percentage keeps it unrounded: rounding is a display
    // decision, and the contract says so.
    let json = serde_json::to_string(&provider).unwrap();
    assert!(json.contains("70.5"), "got {json}");
}
