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

/// The instant these tests are run at.
///
/// Fixed, and passed in explicitly, because half of what the reader decides is now a
/// comparison against the current time: the fixtures' windows reset on 2026-09-07, so a
/// suite that read the real clock would turn from green to `stale` on its own on the eighth
/// and stay that way for ever. This instant sits before both fixture resets (`03:17:24Z`
/// and `12:24:52Z` in the log, `03:17:00Z` and `12:24:00Z` once the reader has floored
/// them to the minute), which is what makes the happy path the happy path.
const NOW: &str = "2026-09-07T00:00:00Z";

/// Two days after both fixture resets: the machine of somebody who has not opened Codex
/// since, where every window's period has ended and the log still says otherwise.
const LATER: &str = "2026-09-09T00:00:00Z";

/// Write `contents` to `<home>/sessions/<date>/rollout-<label>.jsonl`.
fn plant(home: &Path, date: &str, label: &str, contents: &str) -> PathBuf {
    let dir = locate::sessions_dir(home).join(date.replace('/', std::path::MAIN_SEPARATOR_STR));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("rollout-{label}.jsonl"));
    std::fs::write(&path, contents).unwrap();
    path
}

/// Write a few bytes to `<home>/sessions/<date>/rollout-<label>.jsonl.zst`.
///
/// The bytes are deliberately **not** a zstd archive. Nothing in this crate opens one, and
/// a fixture that held a real archive would be pinning a decompressor this reader does not
/// have — what is being tested is that the name is recognised and the file left alone.
fn plant_compressed(home: &Path, date: &str, label: &str) -> PathBuf {
    let dir = locate::sessions_dir(home).join(date.replace('/', std::path::MAIN_SEPARATOR_STR));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("rollout-{label}.jsonl.zst"));
    std::fs::write(&path, b"not an archive, and never opened").unwrap();
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

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
    assert!(provider.configured);
    assert_eq!(provider.windows[WINDOW_SECONDARY].percent, Some(70.0));

    // And a reader pointed somewhere else does not see it.
    let elsewhere = TempDir::new("codex-override-empty");
    assert!(
        !CodexReader::new(elsewhere.join("absent"))
            .refresh_at(NOW)
            .configured
    );
}

// ---------------------------------------------------------------- absent provider

#[test]
fn an_absent_codex_home_is_unconfigured() {
    let dir = TempDir::new("codex-absent");
    let mut reader = CodexReader::new(dir.join("no-codex-here"));
    let provider = reader.refresh_at(NOW);

    assert!(!provider.configured);
    assert!(provider.windows.is_empty());
    assert_eq!(provider.binding, None);
    assert_eq!(provider.plan, None);
}

#[test]
fn a_codex_home_without_logs_is_configured_but_unreadable() {
    let dir = TempDir::new("codex-no-logs");
    std::fs::create_dir_all(locate::sessions_dir(&dir.path)).unwrap();

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);

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

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);

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

// ---------------------------------------------------------------- compressed logs

#[test]
fn a_tree_of_plain_logs_reads_exactly_as_it_did() {
    // The first of the three states, kept as a test of its own so that the other two are
    // read against something rather than against memory.
    let dir = TempDir::new("codex-plain-only");
    plant(&dir.path, "2026/09/07", "session", SAMPLE);

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);

    assert!(provider.windows[WINDOW_PRIMARY].percent.is_some());
    assert_eq!(provider.windows[WINDOW_PRIMARY].error, None);
}

#[test]
fn a_compressed_log_beside_a_plain_one_changes_nothing() {
    let dir = TempDir::new("codex-mixed");
    let plain = plant(&dir.path, "2026/09/07", "session", SAMPLE);
    let compressed = plant_compressed(&dir.path, "2026/09/07", "cold");
    // The compressed one is the newer file, which is the case that would have mattered had
    // it been a candidate: the walk sorts by modification time.
    age(&plain, 600);

    let mut reader = CodexReader::new(&dir.path);
    let provider = reader.refresh_at(NOW);

    assert!(
        provider.windows[WINDOW_PRIMARY].percent.is_some(),
        "the plain log is still the reading"
    );
    assert_eq!(provider.windows[WINDOW_PRIMARY].error, None);
    assert_eq!(
        reader.following(),
        Some(plain.as_path()),
        "the reader must follow the log it can read"
    );
    assert_ne!(reader.following(), Some(compressed.as_path()));
}

#[test]
fn a_tree_of_only_compressed_logs_says_so_instead_of_claiming_there_are_none() {
    // The scenario the risk report described: Codex's compression flag is on, nobody has
    // opened Codex for more than a week, every rollout is cold. `sessions/` is full.
    let dir = TempDir::new("codex-compressed-only");
    plant_compressed(&dir.path, "2026/09/07", "cold-one");
    plant_compressed(&dir.path, "2026/09/06", "cold-two");

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);

    assert!(provider.configured, "Codex is installed and has been used");
    assert_eq!(provider.windows.len(), 2);
    for key in [WINDOW_PRIMARY, WINDOW_SECONDARY] {
        let error = provider.windows[key].error.clone().unwrap();
        assert!(
            error.contains("zstd-compressed"),
            "the reason must name the compression: got {error}"
        );
        assert!(
            !error.contains("no rollout log"),
            "a full sessions directory is not an empty one: got {error}"
        );
        assert_eq!(provider.windows[key].percent, None);
        assert_eq!(provider.windows[key].state, WindowState::Error);
    }
}

// ---------------------------------------------------------------- the happy path

#[test]
fn the_real_sample_maps_to_the_contract() {
    let dir = TempDir::new("codex-sample");
    plant(&dir.path, "2026/09/07", "session", SAMPLE);

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);

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
    assert_eq!(primary.resets_at.as_deref(), Some("2026-09-07T03:17:00Z"));
    assert_eq!(primary.state, WindowState::Ok);

    let secondary = &provider.windows[WINDOW_SECONDARY];
    assert_eq!(secondary.percent, Some(70.0));
    assert_eq!(secondary.window_minutes, Some(SECONDARY_WINDOW_MINUTES));
    assert_eq!(secondary.resets_at.as_deref(), Some("2026-09-07T12:24:00Z"));
    assert_eq!(secondary.state, WindowState::Ok);
}

#[test]
fn the_provider_block_serialises_the_way_the_contract_document_says() {
    let dir = TempDir::new("codex-serialise");
    plant(&dir.path, "2026/09/07", "session", SAMPLE);

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
    let json = serde_json::to_string_pretty(&provider).unwrap();

    for expected in [
        "\"configured\": true",
        "\"plan\": \"plus\"",
        "\"source\": \"rollout\"",
        "\"binding\": \"secondary\"",
        "\"percent\": 54",
        "\"windowMinutes\": 300",
        "\"resetsAt\": \"2026-09-07T12:24:00Z\"",
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

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
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

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
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

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
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

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
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
    let provider = reader.refresh_at(NOW);

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

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
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

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
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

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
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
        CodexReader::new(&dir.path)
            .refresh_at(NOW)
            .binding
            .as_deref(),
        Some(WINDOW_SECONDARY)
    );

    std::fs::write(&path, line_with(91.0, 70.0, "2026-09-07T09:01:00.000Z")).unwrap();
    assert_eq!(
        CodexReader::new(&dir.path)
            .refresh_at(NOW)
            .binding
            .as_deref(),
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
        CodexReader::new(&dir.path)
            .refresh_at(NOW)
            .binding
            .as_deref(),
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

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
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
    assert_eq!(
        reader.refresh_at(NOW).windows[WINDOW_PRIMARY].percent,
        Some(10.0)
    );

    append(&path, &line_with(30.0, 40.0, "2026-09-07T09:05:00.000Z"));
    let provider = reader.refresh_at(NOW);
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
    let first = reader.refresh_at(NOW);
    // Nothing appended, and a line that is not a quota line.
    append(&path, NO_QUOTA);
    let second = reader.refresh_at(NOW);

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
    assert_eq!(
        reader.refresh_at(NOW).windows[WINDOW_PRIMARY].percent,
        Some(10.0)
    );

    let second = plant(
        &dir.path,
        "2026/09/07",
        "bbb-second",
        &line_with(80.0, 20.0, "2026-09-07T09:30:00.000Z"),
    );
    age(&second, 1);

    assert_eq!(
        reader.refresh_at(NOW).windows[WINDOW_PRIMARY].percent,
        Some(80.0)
    );
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
    assert_eq!(
        reader.refresh_at(NOW).windows[WINDOW_PRIMARY].percent,
        Some(11.0)
    );

    // Same name, shorter file: a new session reused the path.
    std::fs::write(&path, line_with(2.0, 3.0, "2026-09-07T10:00:00.000Z")).unwrap();
    assert_eq!(
        reader.refresh_at(NOW).windows[WINDOW_PRIMARY].percent,
        Some(2.0)
    );
}

#[test]
fn a_log_written_with_crlf_reads_the_same() {
    let dir = TempDir::new("codex-crlf");
    let contents = SAMPLE.replace('\n', "\r\n");
    plant(&dir.path, "2026/09/07", "session", &contents);

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
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
    let provider = reader.refresh_at(NOW);
    assert_eq!(
        provider.windows[WINDOW_PRIMARY].percent, None,
        "half a line must not become a reading"
    );

    append(&path, tail_bytes);
    assert_eq!(
        reader.refresh_at(NOW).windows[WINDOW_PRIMARY].percent,
        Some(77.0)
    );
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

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
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

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
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

    let provider = CodexReader::new(&dir.path).refresh_at(NOW);
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
            resets_at: Some("2026-09-07T13:17:00Z".to_owned()),
        }),
        secondary: Some(parse::QuotaWindow {
            used_percent: 70.5,
            window_minutes: Some(10080),
            resets_at: None,
        }),
    };

    let provider = readable(&quota, "2026-09-07T09:00:00Z", "2026-09-07T09:00:00Z");
    assert_eq!(provider.binding.as_deref(), Some(WINDOW_SECONDARY));
    assert_eq!(provider.windows[WINDOW_SECONDARY].percent, Some(70.5));
    assert_eq!(provider.windows[WINDOW_SECONDARY].resets_at, None);
    assert_eq!(provider.windows[WINDOW_SECONDARY].state, WindowState::Ok);

    // Serialising an unrounded percentage keeps it unrounded: rounding is a display
    // decision, and the contract says so.
    let json = serde_json::to_string(&provider).unwrap();
    assert!(json.contains("70.5"), "got {json}");
}

// ---------------------------------------------------------------- a window past its reset

#[test]
fn a_window_whose_reset_has_passed_is_stale_and_keeps_its_percentage() {
    // The maintainer's own machine on 2026-09-08: Codex last used on the sixth, the newest
    // rollout log still parsing perfectly, and `secondary` reported as `percent 70,
    // resetsAt 2026-09-07T12:24:52Z, state ok` — a confident number for a week that had
    // ended the day before.
    let dir = TempDir::new("codex-expired");
    plant(&dir.path, "2026/09/07", "session", SAMPLE);

    let provider = CodexReader::new(&dir.path).refresh_at(LATER);

    for key in [WINDOW_PRIMARY, WINDOW_SECONDARY] {
        let window = &provider.windows[key];
        assert_eq!(window.state, WindowState::Stale, "{key} should be stale");
        assert!(
            window.percent.is_some(),
            "{key} lost its percentage; stale is not unknown"
        );
        let reason = window.error.clone().unwrap_or_default();
        assert!(
            reason.contains("reset"),
            "{key} says nothing useful: {reason}"
        );
    }

    assert_eq!(provider.windows[WINDOW_PRIMARY].percent, Some(54.0));
    assert_eq!(provider.windows[WINDOW_SECONDARY].percent, Some(70.0));
    assert_eq!(
        provider.binding.as_deref(),
        Some(WINDOW_SECONDARY),
        "a stale window is still the fullest one; the panel greys it, it does not vanish"
    );
    assert!(provider.configured, "the provider is installed either way");
}

#[test]
fn the_grace_period_is_five_minutes_and_it_is_a_boundary_not_a_feeling() {
    // Both fixture windows reset at 03:17:00Z and 12:24:00Z once floored; the primary is
    // the one this walks across. A window that has *just* turned over is a log that has
    // not caught up, not a reading from last week.
    let dir = TempDir::new("codex-grace");
    plant(&dir.path, "2026/09/07", "session", SAMPLE);

    let state_at = |now: &str| {
        CodexReader::new(&dir.path).refresh_at(now).windows[WINDOW_PRIMARY]
            .state
            .clone()
    };

    assert_eq!(state_at("2026-09-07T03:16:59Z"), WindowState::Ok, "before");
    assert_eq!(state_at("2026-09-07T03:17:00Z"), WindowState::Ok, "on it");
    assert_eq!(
        state_at("2026-09-07T03:21:59Z"),
        WindowState::Ok,
        "a second inside the grace period"
    );
    assert_eq!(
        state_at("2026-09-07T03:22:00Z"),
        WindowState::Ok,
        "exactly five minutes past is still within a five-minute grace"
    );
    assert_eq!(
        state_at("2026-09-07T03:22:01Z"),
        WindowState::Stale,
        "a second past the grace period is a second too old"
    );
    assert_eq!(RESET_GRACE_SECONDS, 300);
}

#[test]
fn one_window_can_be_stale_while_the_other_is_current() {
    // The ordinary shape of a Codex machine at four in the morning: the five-hour window
    // has turned over and nothing has been run since, the weekly one has days left.
    let dir = TempDir::new("codex-half-stale");
    plant(&dir.path, "2026/09/07", "session", SAMPLE);

    let provider = CodexReader::new(&dir.path).refresh_at("2026-09-07T06:00:00Z");

    assert_eq!(provider.windows[WINDOW_PRIMARY].state, WindowState::Stale);
    assert_eq!(provider.windows[WINDOW_SECONDARY].state, WindowState::Ok);
    assert_eq!(provider.windows[WINDOW_PRIMARY].percent, Some(54.0));
}

#[test]
fn a_window_with_no_reset_is_never_stale_and_neither_is_one_read_at_an_unreadable_now() {
    // Two ways not to know, and the same answer to both: a time nobody can read is not
    // evidence that a window has expired.
    let dir = TempDir::new("codex-no-reset");
    plant(
        &dir.path,
        "2026/09/07",
        "session",
        "{\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":7.5}}}}\n",
    );
    assert_eq!(
        CodexReader::new(&dir.path).refresh_at(LATER).windows[WINDOW_PRIMARY].state,
        WindowState::Ok,
        "no reset to be past"
    );

    let dated = TempDir::new("codex-bad-now");
    plant(&dated.path, "2026/09/07", "session", SAMPLE);
    for now in ["", "whenever you like", "2026-13-45T99:99:99Z"] {
        assert_eq!(
            CodexReader::new(&dated.path).refresh_at(now).windows[WINDOW_PRIMARY].state,
            WindowState::Ok,
            "a `now` of {now:?} must not condemn a window"
        );
    }
}

#[test]
fn an_unreadable_provider_is_error_rather_than_stale_however_old_it_is() {
    // The two states are not degrees of the same thing. `stale` carries a number nobody
    // should act on; `error` carries no number at all, and no passage of time turns one
    // into the other.
    let dir = TempDir::new("codex-stale-vs-error");
    plant(&dir.path, "2026/09/07", "stub", NO_QUOTA);

    let provider = CodexReader::new(&dir.path).refresh_at(LATER);
    for window in provider.windows.values() {
        assert_eq!(window.state, WindowState::Error);
        assert_eq!(window.percent, None);
    }
}

#[test]
fn the_reset_a_stale_window_names_is_a_time_and_nothing_from_the_machine() {
    // The reason text is the one new string this reader writes, and it goes into a file
    // that is meant to be safe to paste into a bug report.
    let dir = TempDir::new("codex-stale-reason");
    plant(&dir.path, "2026/09/07", "session", SAMPLE);

    let provider = CodexReader::new(&dir.path).refresh_at(LATER);
    let reason = provider.windows[WINDOW_SECONDARY].error.clone().unwrap();

    assert!(reason.contains("2026-09-07T12:24:00Z"), "got {reason}");
    let directory = dir.path.to_string_lossy().into_owned();
    assert!(
        !reason.contains(&directory),
        "leaked a local path: {reason}"
    );
    assert!(
        !reason.contains(".codex"),
        "leaked a path fragment: {reason}"
    );
    assert!(reason.len() < 120, "the reason is a sentence, not a report");
}

#[test]
fn the_clock_reading_entry_point_agrees_with_the_one_that_is_told_the_time() {
    // `refresh` is `refresh_at(now)` with the system clock, and the one-shot `--print`
    // path is the only caller that needs it. Fixtures from 2026-09-07 are long past on any
    // machine running this, so the two must agree that they are stale.
    let dir = TempDir::new("codex-system-clock");
    plant(&dir.path, "2026/09/07", "session", SAMPLE);

    let provider = CodexReader::new(&dir.path).refresh();
    assert_eq!(provider.windows[WINDOW_SECONDARY].percent, Some(70.0));
    assert_eq!(
        provider.windows[WINDOW_SECONDARY].state,
        WindowState::Stale,
        "these fixtures reset in September 2026; no clock this runs on is before that"
    );
}
