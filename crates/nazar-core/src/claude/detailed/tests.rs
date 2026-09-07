//! The detailed-windows mode, tested against a socket on this machine and nothing else.
//!
//! No test here reaches the network, reads the real `~/.claude`, or writes anything
//! outside a temporary directory it made itself. The endpoint is [`super::mock`], a
//! forty-line HTTP server bound to `127.0.0.1:0`.
//!
//! The one test that matters most is [`the_token_reaches_the_header_and_nowhere_else`]:
//! it runs the whole flow with a sentinel where the token goes, and then goes looking for
//! that sentinel in every file, every error string and every `Debug` rendering it can
//! reach. It is the acceptance criterion for WP2b written as code.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::json;

use super::backoff::{Backoff, Clock, MAX_DELAY};
use super::mock::{Canned, MockServer};
use super::*;
use crate::claude::merge::merge;
use crate::limits::{Limits, Provider, Source, Window, WindowState};
use crate::testutil::TempDir;

// ---------------------------------------------------------------- helpers --

/// A clock the test moves by hand.
struct TestClock {
    millis: AtomicU64,
    wall: Mutex<String>,
}

impl TestClock {
    fn new(wall: &str) -> Self {
        TestClock {
            millis: AtomicU64::new(0),
            wall: Mutex::new(wall.to_owned()),
        }
    }

    fn advance(&self, by: Duration) {
        self.millis
            .fetch_add(by.as_millis() as u64, Ordering::Relaxed);
    }

    fn set_wall(&self, wall: &str) {
        *self.wall.lock().unwrap() = wall.to_owned();
    }
}

impl Clock for TestClock {
    fn monotonic_millis(&self) -> u64 {
        self.millis.load(Ordering::Relaxed)
    }

    fn now_rfc3339(&self) -> String {
        self.wall.lock().unwrap().clone()
    }
}

/// A sign-in file with a token and a plan, valid until well after the test clock.
fn write_sign_in(dir: &TempDir, token: &str) -> std::path::PathBuf {
    write_sign_in_with(
        dir,
        token,
        Some(4_102_444_800_000i64),
        "default_claude_max_20x",
    )
}

fn write_sign_in_with(
    dir: &TempDir,
    token: &str,
    expires_at: Option<i64>,
    tier: &str,
) -> std::path::PathBuf {
    let mut oauth = serde_json::Map::new();
    oauth.insert("accessToken".to_owned(), json!(token));
    oauth.insert("refreshToken".to_owned(), json!("refresh-value-never-read"));
    if let Some(expires_at) = expires_at {
        oauth.insert("expiresAt".to_owned(), json!(expires_at));
    }
    oauth.insert("scopes".to_owned(), json!(["user:inference"]));
    oauth.insert("subscriptionType".to_owned(), json!("max"));
    oauth.insert("rateLimitTier".to_owned(), json!(tier));

    let path = dir.join(".credentials.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&json!({ "claudeAiOauth": oauth })).unwrap(),
    )
    .unwrap();
    path
}

/// The response shape the endpoint returns, with the numbers the live check saw.
fn usage_body() -> String {
    json!({
        "limits": [
            {"kind": "session",       "percent": 12, "resets_at": "2026-09-07T14:00:00Z", "is_active": true},
            {"kind": "weekly_all",    "percent": 18, "resets_at": "2026-09-12T02:00:00Z", "is_active": false},
            {"kind": "weekly_scoped", "percent": 23, "resets_at": "2026-09-12T02:00:00Z",
             "scope": {"model": {"display_name": "Fable"}}}
        ],
        "extra_usage": {"is_enabled": false, "utilization": 0}
    })
    .to_string()
}

/// A reader pointed at a mock server and a temporary sign-in file.
fn reader_for(server: &MockServer, credentials: &std::path::Path) -> DetailedReader {
    DetailedReader::new(UsageClient::new(server.url()), credentials)
}

/// A passive block like the one `ClaudeReader::refresh` produces.
fn passive(captured_at: &str, five_hour: f64, seven_day: f64) -> Provider {
    let mut windows = BTreeMap::new();
    windows.insert(
        crate::claude::WINDOW_FIVE_HOUR.to_owned(),
        Window::ok(five_hour).with_window_minutes(300),
    );
    windows.insert(
        crate::claude::WINDOW_SEVEN_DAY.to_owned(),
        Window::ok(seven_day).with_window_minutes(10_080),
    );
    Provider {
        configured: true,
        plan: None,
        source: Some(Source::Statusline),
        source_at: Some(captured_at.to_owned()),
        binding: crate::claude::binding(&windows),
        windows,
        extra: serde_json::Map::new(),
    }
}

// ------------------------------------------------------- the mode's switch --

#[test]
fn off_by_default() {
    assert!(
        !crate::config::Config::default().detailed_windows,
        "the shipped default must be off"
    );
    let mut mode = DetailedWindows::from_config(false);
    assert!(!mode.is_enabled());
    assert_eq!(mode.refresh(&SystemClock), None);
}

/// With the mode off, a sign-in file that cannot be read must not matter.
///
/// The file is a **directory** with the sign-in file's name, so every attempt to open it
/// fails at the operating system rather than in a parser. If anything on the default path
/// went near it, this test would see an error instead of an untouched provider.
#[test]
fn the_sign_in_file_is_not_opened_when_the_mode_is_off() {
    let dir = TempDir::new("detailed-off");
    let poisoned = dir.join(".credentials.json");
    std::fs::create_dir_all(&poisoned).unwrap();
    std::fs::write(
        poisoned.join("not-a-file"),
        "if this is read, the test is wrong",
    )
    .unwrap();

    let mut built = false;
    let mut mode = DetailedWindows::new(false, || {
        built = true;
        Some(DetailedReader::new(
            UsageClient::new("http://127.0.0.1:1/never"),
            &poisoned,
        ))
    });

    assert!(!built, "with the mode off there is not even a reader");
    assert_eq!(mode.refresh(&SystemClock), None);

    let block = passive("2026-09-07T12:00:00Z", 12.0, 18.0);
    let merged = merge(block.clone(), None, "2026-09-07T12:00:05Z");
    assert_eq!(merged, block, "the passive block passes through untouched");
    assert_eq!(merged.source, Some(Source::Statusline));
}

#[test]
fn a_directory_where_the_sign_in_should_be_is_reported_not_panicked_on() {
    let dir = TempDir::new("detailed-poisoned-on");
    let poisoned = dir.join(".credentials.json");
    std::fs::create_dir_all(&poisoned).unwrap();

    let server = MockServer::once(Canned::ok(usage_body()));
    let mut reader = reader_for(&server, &poisoned);
    let outcome = reader.refresh(&TestClock::new("2026-09-07T12:00:00Z"));

    assert_eq!(outcome.error, Some(DetailedError::NotSignedIn));
    assert_eq!(outcome.reading, None);
    assert!(
        server.requests().is_empty(),
        "no sign-in means no request; the endpoint must not be asked for nothing"
    );
}

// ------------------------------------------------------- the status codes --

#[test]
fn two_hundred_produces_the_windows_and_the_plan() {
    let dir = TempDir::new("detailed-200");
    let credentials = write_sign_in(&dir, "token-for-the-mock");
    let server = MockServer::once(Canned::ok(usage_body()));

    let mut reader = reader_for(&server, &credentials);
    let outcome = reader.refresh(&TestClock::new("2026-09-07T12:00:00Z"));

    assert_eq!(outcome.error, None);
    assert!(outcome.is_fresh());
    let reading = outcome.reading.expect("a 200 must produce numbers");
    assert_eq!(reading.plan.as_deref(), Some("max_20x"));
    assert_eq!(reading.fetched_at, "2026-09-07T12:00:00Z");

    let keys: Vec<_> = reading.windows.keys().cloned().collect();
    assert_eq!(keys, ["five_hour", "seven_day", "seven_day_fable"]);
    assert_eq!(reading.windows["seven_day_fable"].percent, Some(23.0));
    assert_eq!(
        reading.windows["seven_day_fable"].model.as_deref(),
        Some("Fable")
    );
    assert_eq!(reading.windows["seven_day_fable"].detailed, Some(true));
    assert_eq!(
        reading.windows["seven_day_fable"].resets_at.as_deref(),
        Some("2026-09-12T02:00:00Z")
    );
    assert_eq!(reading.windows["five_hour"].window_minutes, Some(300));
    assert_eq!(reading.windows["seven_day"].window_minutes, Some(10_080));
}

#[test]
fn the_request_carries_the_three_headers_and_names_this_product() {
    let dir = TempDir::new("detailed-headers");
    let credentials = write_sign_in(&dir, "header-token");
    let server = MockServer::once(Canned::ok(usage_body()));

    let mut reader = reader_for(&server, &credentials);
    reader.refresh(&TestClock::new("2026-09-07T12:00:00Z"));

    let requests = server.requests();
    assert_eq!(requests.len(), 1, "exactly one request per refresh");
    let seen = &requests[0];
    assert!(
        seen.request_line.starts_with("GET "),
        "{}",
        seen.request_line
    );
    assert_eq!(seen.header("authorization"), Some("Bearer header-token"));
    assert_eq!(seen.header("anthropic-beta"), Some("oauth-2025-04-20"));
    assert_eq!(seen.header("accept"), Some("application/json"));
    assert!(
        seen.header("user-agent")
            .is_some_and(|agent| agent.starts_with("nazar-tray/")),
        "got {:?}",
        seen.header("user-agent")
    );
}

#[test]
fn four_oh_one_says_what_to_do_and_keeps_the_last_numbers() {
    let dir = TempDir::new("detailed-401");
    let credentials = write_sign_in(&dir, "token");
    let server = MockServer::start(vec![Canned::ok(usage_body()), Canned::status(401)]);
    let clock = TestClock::new("2026-09-07T12:00:00Z");

    let mut reader = reader_for(&server, &credentials);
    let first = reader.refresh(&clock);
    assert!(first.is_fresh());

    clock.advance(Duration::from_secs(60));
    clock.set_wall("2026-09-07T12:01:00Z");
    let second = reader.refresh(&clock);

    assert_eq!(second.error, Some(DetailedError::Unauthorized));
    assert_eq!(
        second.error.as_ref().unwrap().to_string(),
        "token expired; run Claude Code once to refresh"
    );
    let reading = second.reading.expect("the last good numbers stay");
    assert!(reading.remembered);
    assert_eq!(reading.windows["seven_day_fable"].percent, Some(23.0));
    assert_eq!(
        reading.reason.as_deref(),
        Some("token expired; run Claude Code once to refresh")
    );
}

#[test]
fn four_two_nine_honours_retry_after_and_leaves_the_last_value_stale() {
    let dir = TempDir::new("detailed-429");
    let credentials = write_sign_in(&dir, "token");
    let server = MockServer::start(vec![
        Canned::ok(usage_body()),
        Canned::status(429).header("Retry-After", "120"),
    ]);
    let clock = TestClock::new("2026-09-07T12:00:00Z");

    let mut reader = reader_for(&server, &credentials);
    reader.refresh(&clock);
    clock.advance(Duration::from_secs(60));
    clock.set_wall("2026-09-07T12:01:00Z");
    let outcome = reader.refresh(&clock);

    assert_eq!(
        outcome.error,
        Some(DetailedError::RateLimited {
            retry_after: Some(Duration::from_secs(120))
        })
    );
    let reading = outcome.reading.expect("the last value survives a 429");
    assert!(reading.remembered);
    assert_eq!(reading.windows["seven_day_fable"].percent, Some(23.0));

    // And the window it produces says stale, which is the acceptance criterion's words.
    let merged = merge(
        passive("2026-09-07T11:00:00Z", 5.0, 6.0),
        Some(&reading),
        "2026-09-07T12:01:00Z",
    );
    assert_eq!(merged.windows["seven_day_fable"].state, WindowState::Stale);
    assert_eq!(merged.windows["seven_day_fable"].percent, Some(23.0));

    // The server asked for two minutes, so nothing is attempted for two minutes.
    assert_eq!(
        reader.backoff().remaining(clock.monotonic_millis()),
        Some(Duration::from_secs(120))
    );
}

#[test]
fn a_server_error_is_reported_by_its_number() {
    let dir = TempDir::new("detailed-500");
    let credentials = write_sign_in(&dir, "token");
    let server = MockServer::once(Canned::status(500));

    let mut reader = reader_for(&server, &credentials);
    let outcome = reader.refresh(&TestClock::new("2026-09-07T12:00:00Z"));

    assert_eq!(outcome.error, Some(DetailedError::Status { code: 500 }));
    assert_eq!(
        outcome.error.unwrap().to_string(),
        "the usage endpoint returned HTTP 500"
    );
    assert_eq!(outcome.reading, None, "nothing was ever known to remember");
}

#[test]
fn a_network_failure_is_a_category_not_a_third_partys_sentence() {
    let dir = TempDir::new("detailed-network");
    let credentials = write_sign_in(&dir, "token");
    // Port 1 on loopback: nothing listens there, and the refusal is immediate.
    let mut reader = DetailedReader::new(
        UsageClient::new("http://127.0.0.1:1/api/oauth/usage"),
        &credentials,
    );

    let outcome = reader.refresh(&TestClock::new("2026-09-07T12:00:00Z"));
    let error = outcome.error.expect("an unreachable endpoint is a failure");
    assert!(
        matches!(error, DetailedError::Network { .. }),
        "got {error:?}"
    );
    assert!(
        error.to_string().starts_with("the usage endpoint "),
        "got {error}"
    );
}

#[test]
fn a_body_the_mapper_does_not_know_is_named_without_being_quoted() {
    let dir = TempDir::new("detailed-shape");
    let credentials = write_sign_in(&dir, "token");
    let server = MockServer::once(Canned::ok(
        r#"{"quota":{"used":"a lot"},"note":"a shape from the future"}"#,
    ));

    let mut reader = reader_for(&server, &credentials);
    let outcome = reader.refresh(&TestClock::new("2026-09-07T12:00:00Z"));

    let error = outcome.error.expect("an unreadable 200 is a failure");
    assert!(
        matches!(error, DetailedError::UnexpectedShape { .. }),
        "got {error:?}"
    );
    let message = error.to_string();
    assert!(
        !message.contains("a lot"),
        "the body must not be quoted: {message}"
    );
    assert!(
        !message.contains("a shape from the future"),
        "the body must not be quoted: {message}"
    );
}

#[test]
fn an_error_body_never_reaches_an_error_message() {
    let dir = TempDir::new("detailed-error-body");
    let credentials = write_sign_in(&dir, "token");
    let server = MockServer::once(Canned::status_with_body(
        403,
        r#"{"error":{"message":"forbidden for account acct_1234567890 <someone@example.com>"}}"#,
    ));

    let mut reader = reader_for(&server, &credentials);
    let outcome = reader.refresh(&TestClock::new("2026-09-07T12:00:00Z"));

    let error = outcome.error.unwrap();
    for rendering in [format!("{error}"), format!("{error:?}")] {
        assert!(!rendering.contains("acct_"), "got {rendering}");
        assert!(!rendering.contains("example.com"), "got {rendering}");
        assert!(!rendering.contains("forbidden for"), "got {rendering}");
    }
}

#[test]
fn an_expired_token_costs_no_request_at_all() {
    let dir = TempDir::new("detailed-expired");
    // Expired at 2026-09-07T11:00:00Z, in milliseconds the way Claude Code writes it.
    let credentials = write_sign_in_with(
        &dir,
        "stale-token",
        Some(1_788_778_800_000),
        "default_claude_max_20x",
    );
    let server = MockServer::once(Canned::ok(usage_body()));

    let mut reader = reader_for(&server, &credentials);
    let outcome = reader.refresh(&TestClock::new("2026-09-07T12:00:00Z"));

    assert_eq!(outcome.error, Some(DetailedError::Expired));
    assert_eq!(
        outcome.error.unwrap().to_string(),
        "token expired; run Claude Code once to refresh"
    );
    assert!(
        server.requests().is_empty(),
        "a token that is already expired must not be sent; this is the 401 storm the \
         audit found, prevented rather than retried"
    );
}

#[test]
fn a_sign_in_with_no_expiry_is_not_treated_as_expired() {
    let dir = TempDir::new("detailed-no-expiry");
    let credentials = write_sign_in_with(&dir, "token", None, "default_claude_max_20x");
    let server = MockServer::once(Canned::ok(usage_body()));

    let mut reader = reader_for(&server, &credentials);
    let outcome = reader.refresh(&TestClock::new("2026-09-07T12:00:00Z"));

    assert_eq!(
        outcome.error, None,
        "an unknown expiry is not an expired one"
    );
    assert_eq!(server.requests().len(), 1);
}

// ------------------------------------------------------------- the backoff --

#[test]
fn the_delay_doubles_and_stops_at_half_an_hour() {
    let mut backoff = Backoff::new();
    let mut now = 0u64;
    let expected = [1u64, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 1800, 1800];

    for (attempt, seconds) in expected.iter().enumerate() {
        backoff.failed(now, None);
        assert_eq!(
            backoff.remaining(now),
            Some(Duration::from_secs(*seconds)),
            "attempt {} should wait {seconds} s",
            attempt + 1
        );
        // Wait it out, then fail again.
        now += seconds * 1000;
        assert!(backoff.allows(now), "the wait should be over at {now} ms");
    }
    assert_eq!(MAX_DELAY, Duration::from_secs(1800));
}

#[test]
fn a_success_puts_the_delay_back_to_nothing() {
    let mut backoff = Backoff::new();
    backoff.failed(0, None);
    backoff.failed(0, None);
    backoff.failed(0, None);
    assert_eq!(backoff.failures(), 3);

    backoff.succeeded();
    assert_eq!(backoff.failures(), 0);
    assert!(backoff.allows(0));
    assert_eq!(backoff.remaining(0), None);
}

#[test]
fn the_servers_own_answer_beats_the_doubling_in_both_directions() {
    let mut backoff = Backoff::new();
    // Six failures would be 32 s; the server asks for 2.
    for _ in 0..5 {
        backoff.failed(0, None);
    }
    backoff.failed(0, Some(Duration::from_secs(2)));
    assert_eq!(backoff.remaining(0), Some(Duration::from_secs(2)));

    // And an hour is still capped at the half-hour ceiling.
    backoff.failed(0, Some(Duration::from_secs(3600)));
    assert_eq!(backoff.remaining(0), Some(MAX_DELAY));
}

#[test]
fn nothing_is_asked_while_the_wait_is_running() {
    let dir = TempDir::new("detailed-backoff-gate");
    let credentials = write_sign_in(&dir, "token");
    let server = MockServer::start(vec![Canned::status(500), Canned::ok(usage_body())]);
    let clock = TestClock::new("2026-09-07T12:00:00Z");

    let mut reader = reader_for(&server, &credentials);
    reader.refresh(&clock);
    assert_eq!(server.requests().len(), 1);

    // Half a second later the one-second wait is still running.
    clock.advance(Duration::from_millis(500));
    let waiting = reader.refresh(&clock);
    assert!(
        matches!(waiting.error, Some(DetailedError::BackingOff { .. })),
        "got {:?}",
        waiting.error
    );
    assert_eq!(
        server.requests().len(),
        1,
        "the second refresh must not have reached the endpoint"
    );

    // Past the wait, it tries again.
    clock.advance(Duration::from_millis(600));
    let retried = reader.refresh(&clock);
    assert_eq!(retried.error, None);
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn a_new_sign_in_file_clears_the_wait() {
    let dir = TempDir::new("detailed-refresh-clears");
    let credentials = write_sign_in(&dir, "old-token");
    let server = MockServer::start(vec![Canned::status(401), Canned::ok(usage_body())]);
    let clock = TestClock::new("2026-09-07T12:00:00Z");

    let mut reader = reader_for(&server, &credentials);
    let refused = reader.refresh(&clock);
    assert_eq!(refused.error, Some(DetailedError::Unauthorized));
    assert!(!reader.backoff().allows(clock.monotonic_millis()));

    // Claude Code runs and rewrites the file. The reason for the 401 is gone, so the
    // wait should be too — this is the audit's three-hour 401 storm, ended.
    std::thread::sleep(Duration::from_millis(20));
    write_sign_in(&dir, "fresh-token");

    let retried = reader.refresh(&clock);
    assert_eq!(
        retried.error, None,
        "the refreshed sign-in must be tried at once"
    );
    assert_eq!(
        server.requests().last().unwrap().header("authorization"),
        Some("Bearer fresh-token")
    );
}

#[test]
fn a_different_account_does_not_inherit_the_previous_ones_numbers() {
    let dir = TempDir::new("detailed-account-switch");
    let credentials = write_sign_in_with(&dir, "max-token", None, "default_claude_max_20x");
    let server = MockServer::start(vec![Canned::ok(usage_body()), Canned::status(500)]);
    let clock = TestClock::new("2026-09-07T12:00:00Z");

    let mut reader = reader_for(&server, &credentials);
    assert_eq!(
        reader.refresh(&clock).reading.unwrap().plan.as_deref(),
        Some("max_20x")
    );

    // Sign in as somebody on Pro, then have the endpoint fail.
    std::thread::sleep(Duration::from_millis(20));
    write_sign_in_with(&dir, "pro-token", None, "default_claude_pro");
    clock.advance(Duration::from_secs(60));
    let outcome = reader.refresh(&clock);

    assert_eq!(outcome.error, Some(DetailedError::Status { code: 500 }));
    assert_eq!(
        outcome.reading, None,
        "a Max account's percentages must not be shown under a Pro account's name (B25)"
    );
}

#[test]
fn no_sign_in_at_all_throws_the_remembered_numbers_away() {
    let dir = TempDir::new("detailed-signed-out");
    let credentials = write_sign_in(&dir, "token");
    let server = MockServer::once(Canned::ok(usage_body()));
    let clock = TestClock::new("2026-09-07T12:00:00Z");

    let mut reader = reader_for(&server, &credentials);
    assert!(reader.refresh(&clock).is_fresh());

    std::fs::remove_file(&credentials).unwrap();
    clock.advance(Duration::from_secs(60));
    let outcome = reader.refresh(&clock);

    assert_eq!(outcome.error, Some(DetailedError::NotSignedIn));
    assert_eq!(
        outcome.reading, None,
        "numbers from an account that is no longer signed in are worse than none"
    );
}

// -------------------------------------------------------------- the mapping --

#[test]
fn every_kind_lands_where_the_contract_says() {
    let body = json!({
        "limits": [
            {"kind": "session",       "percent": 4,  "resets_at": "2026-09-07T14:00:00Z"},
            {"kind": "weekly_all",    "percent": 18, "resets_at": "2026-09-12T02:00:00Z"},
            {"kind": "weekly_scoped", "percent": 23, "scope": {"model": {"display_name": "Fable 5.1"}}},
            {"kind": "opus",          "percent": 7},
            {"kind": "something_new", "percent": 99}
        ]
    })
    .to_string();

    let mapped = map_response(&body, None, None).unwrap();
    let keys: Vec<_> = mapped.windows.keys().cloned().collect();
    assert_eq!(
        keys,
        [
            "five_hour",
            "seven_day",
            "seven_day_fable_5_1",
            "seven_day_opus"
        ],
        "an unknown kind is skipped, never guessed at"
    );
    assert_eq!(
        mapped.windows["seven_day_fable_5_1"].model.as_deref(),
        Some("Fable 5.1")
    );
    assert_eq!(
        mapped.windows["seven_day_opus"].model.as_deref(),
        Some("Opus")
    );
    assert_eq!(mapped.windows["seven_day_opus"].percent, Some(7.0));
    assert_eq!(mapped.windows["five_hour"].window_minutes, Some(300));
    for window in mapped.windows.values() {
        assert_eq!(
            window.detailed,
            Some(true),
            "every window from this path is a detailed one"
        );
    }
}

#[test]
fn the_older_top_level_shape_is_read_when_the_array_is_not_there() {
    let body = json!({
        "five_hour": {"utilization": 12, "resets_at": 1_788_768_000i64},
        "seven_day": {"utilization": 31.5, "resets_at": "2026-09-12T02:00:00Z"}
    })
    .to_string();

    let mapped = map_response(&body, None, None).unwrap();
    assert_eq!(mapped.windows["five_hour"].percent, Some(12.0));
    assert_eq!(
        mapped.windows["five_hour"].resets_at.as_deref(),
        Some("2026-09-07T08:00:00Z"),
        "a Unix-seconds reset is read the same way the status line's is"
    );
    assert_eq!(mapped.windows["seven_day"].percent, Some(31.5));
    assert_eq!(mapped.windows.len(), 2);
}

/// The endpoint does not write timestamps the way the contract does, and that is fine.
///
/// Observed live on 2026-09-07: `2026-09-07T13:10:00.130195+00:00`. Legal RFC 3339, the
/// right instant, and neither of the two things `limits.json` promises — a `Z` and whole
/// seconds. The reader rewrites it rather than forwarding it, so a consumer never has to
/// deal with two spellings of the same moment depending on which mode is on.
#[test]
fn the_endpoints_own_timestamp_form_is_rewritten_into_the_contracts() {
    let body = json!({
        "limits": [
            {"kind": "session",    "percent": 2,  "resets_at": "2026-09-07T13:10:00.130195+00:00"},
            {"kind": "weekly_all", "percent": 38, "resets_at": "2026-09-12T02:00:00.130216+00:00"},
            {"kind": "weekly_scoped", "percent": 30, "resets_at": "2026-09-12T02:00:00.130399+00:00",
             "scope": {"model": {"display_name": "Fable"}}}
        ]
    })
    .to_string();

    let mapped = map_response(&body, Some("default_claude_max_20x"), Some("max")).unwrap();
    assert_eq!(
        mapped.windows["five_hour"].resets_at.as_deref(),
        Some("2026-09-07T13:10:00Z")
    );
    for window in mapped.windows.values() {
        let resets_at = window.resets_at.as_deref().unwrap();
        assert!(resets_at.ends_with('Z'), "got {resets_at}");
        assert!(
            !resets_at.contains('.'),
            "sub-second precision has no place in a countdown: {resets_at}"
        );
    }
}

#[test]
fn the_array_wins_over_the_older_shape_and_they_are_never_mixed() {
    let body = json!({
        "limits": [{"kind": "session", "percent": 4}],
        "five_hour": {"utilization": 99},
        "seven_day": {"utilization": 99}
    })
    .to_string();

    let mapped = map_response(&body, None, None).unwrap();
    assert_eq!(mapped.windows.len(), 1);
    assert_eq!(mapped.windows["five_hour"].percent, Some(4.0));
}

#[test]
fn a_percentage_that_is_not_one_drops_its_window() {
    let body = json!({
        "limits": [
            {"kind": "session",    "percent": 101},
            {"kind": "weekly_all", "percent": -1},
            {"kind": "opus",       "percent": "twenty"},
            {"kind": "weekly_scoped", "percent": 23, "scope": {"model": {"display_name": "Fable"}}}
        ]
    })
    .to_string();

    let mapped = map_response(&body, None, None).unwrap();
    assert_eq!(
        mapped.windows.keys().collect::<Vec<_>>(),
        ["seven_day_fable"],
        "a value that is not a percentage is dropped, never clamped into one"
    );
}

#[test]
fn two_scoped_windows_for_the_same_model_do_not_become_one() {
    let body = json!({
        "limits": [
            {"kind": "weekly_scoped", "percent": 23, "scope": {"model": {"display_name": "Fable"}}},
            {"kind": "weekly_scoped", "percent": 41, "scope": {"model": {"display_name": "Fable"}}}
        ]
    })
    .to_string();

    let mapped = map_response(&body, None, None).unwrap();
    assert_eq!(
        mapped.windows.keys().collect::<Vec<_>>(),
        ["seven_day_fable", "seven_day_fable_2"]
    );
    assert_eq!(mapped.windows["seven_day_fable_2"].percent, Some(41.0));
}

#[test]
fn a_scoped_window_with_no_usable_model_name_still_counts() {
    let body = json!({
        "limits": [
            {"kind": "weekly_scoped", "percent": 23, "scope": {"model": {"display_name": "   "}}},
            {"kind": "weekly_scoped", "percent": 30}
        ]
    })
    .to_string();

    let mapped = map_response(&body, None, None).unwrap();
    assert_eq!(
        mapped.windows.keys().collect::<Vec<_>>(),
        ["seven_day_scoped", "seven_day_scoped_2"],
        "the number is real even when the name is not; it is the number that constrains you"
    );
    assert_eq!(mapped.windows["seven_day_scoped"].model, None);
}

#[test]
fn a_model_name_that_is_not_a_name_is_dropped() {
    assert_eq!(map::model_name("Fable").as_deref(), Some("Fable"));
    assert_eq!(map::model_name("Fable 5.1").as_deref(), Some("Fable 5.1"));
    assert_eq!(map::model_name("  Opus  ").as_deref(), Some("Opus"));

    assert_eq!(map::model_name(""), None);
    assert_eq!(map::model_name("   "), None);
    assert_eq!(map::model_name("---"), None);
    assert_eq!(map::model_name("C:\\Users\\someone"), None);
    assert_eq!(map::model_name("/home/someone"), None);
    assert_eq!(map::model_name("a\nname"), None);
    assert_eq!(map::model_name(&"x".repeat(49)), None);
}

#[test]
fn a_slug_is_ascii_lower_case_and_bounded() {
    assert_eq!(map::slug("Fable").as_deref(), Some("fable"));
    assert_eq!(map::slug("Fable 5.1").as_deref(), Some("fable_5_1"));
    assert_eq!(map::slug("  Opus  4.5 ").as_deref(), Some("opus_4_5"));
    assert_eq!(map::slug("Claude — Fable").as_deref(), Some("claude_fable"));
    assert_eq!(map::slug("日本語"), None, "a key nobody can type is no key");
    assert_eq!(map::slug(""), None);
    assert!(map::slug(&"long-name-".repeat(20)).unwrap().len() <= 32);
}

#[test]
fn a_body_that_is_not_the_endpoints_is_refused_rather_than_half_read() {
    for body in [
        "",
        "not json at all",
        "[]",
        "{}",
        r#"{"limits":[]}"#,
        r#"{"limits":[{"kind":"session"}]}"#,
        r#"{"five_hour":{}}"#,
    ] {
        assert!(
            matches!(
                map_response(body, None, None),
                Err(DetailedError::UnexpectedShape { .. })
            ),
            "should have refused {body:?}"
        );
    }
}

// --------------------------------------------------------- the plan naming --

#[test]
fn the_plan_is_normalised_from_the_tier_then_the_subscription() {
    // The two values observed on the maintainer's machine.
    assert_eq!(
        normalise_plan(Some("default_claude_max_20x"), Some("max")).as_deref(),
        Some("max_20x")
    );
    assert_eq!(
        normalise_plan(Some("default_claude_max_5x"), Some("max")).as_deref(),
        Some("max_5x")
    );
    assert_eq!(
        normalise_plan(Some("default_claude_pro"), Some("pro")).as_deref(),
        Some("pro")
    );

    // An unrecognised tier travels as written, provided it looks like an identifier.
    assert_eq!(
        normalise_plan(Some("default_claude_team_2"), None).as_deref(),
        Some("default_claude_team_2")
    );
    // A tier that is prose is dropped, and the subscription answers instead.
    assert_eq!(
        normalise_plan(Some("your plan is: unlimited!"), Some("team")).as_deref(),
        Some("team")
    );
    assert_eq!(normalise_plan(None, Some("max")).as_deref(), Some("max"));
    assert_eq!(normalise_plan(None, None), None);
    assert_eq!(normalise_plan(Some("   "), None), None);
}

#[test]
fn the_offer_is_made_once_and_only_on_max() {
    assert!(should_suggest_detailed(Some("max_20x"), false));
    assert!(should_suggest_detailed(Some("max_5x"), false));

    assert!(
        !should_suggest_detailed(Some("max_20x"), true),
        "asked once"
    );
    assert!(
        !should_suggest_detailed(Some("pro"), false),
        "Pro has no model-scoped weekly window for the mode to reveal"
    );
    assert!(!should_suggest_detailed(Some("plus"), false));
    assert!(
        !should_suggest_detailed(None, false),
        "with the mode off there is no plan to read, and no plan means no offer"
    );
}

// ------------------------------------------------------------ the merge --

#[test]
fn the_endpoint_lays_down_the_block_and_the_binding_window_is_the_scoped_one() {
    let reading = Reading {
        plan: Some("max_20x".to_owned()),
        fetched_at: "2026-09-07T12:00:00Z".to_owned(),
        windows: map_response(&usage_body(), None, None).unwrap().windows,
        remembered: false,
        reason: None,
    };

    let merged = merge(
        passive("2026-09-07T11:59:00Z", 12.0, 18.0),
        Some(&reading),
        "2026-09-07T12:00:10Z",
    );

    assert_eq!(merged.source, Some(Source::Endpoint));
    assert_eq!(merged.source_at.as_deref(), Some("2026-09-07T12:00:00Z"));
    assert_eq!(merged.plan.as_deref(), Some("max_20x"));
    assert_eq!(
        merged.binding.as_deref(),
        Some("seven_day_fable"),
        "23 beats 18 and 12 — the constraint the passive path cannot see"
    );
    for window in merged.windows.values() {
        assert_eq!(window.state, WindowState::Ok);
    }
}

#[test]
fn a_newer_status_line_reading_wins_on_the_two_windows_it_also_reports() {
    let reading = Reading {
        plan: Some("max_20x".to_owned()),
        fetched_at: "2026-09-07T12:00:00Z".to_owned(),
        windows: map_response(&usage_body(), None, None).unwrap().windows,
        remembered: false,
        reason: None,
    };

    // The status line has been running since; its numbers are five minutes newer.
    let merged = merge(
        passive("2026-09-07T12:05:00Z", 25.0, 19.0),
        Some(&reading),
        "2026-09-07T12:05:10Z",
    );

    assert_eq!(merged.windows["five_hour"].percent, Some(25.0));
    assert_eq!(merged.windows["seven_day"].percent, Some(19.0));
    assert_eq!(
        merged.windows["five_hour"].detailed, None,
        "a window the status line replaced is no longer a detailed one"
    );
    assert_eq!(
        merged.windows["seven_day_fable"].percent,
        Some(23.0),
        "the model-scoped window has no passive equivalent, so it stays"
    );
    assert_eq!(merged.source_at.as_deref(), Some("2026-09-07T12:05:00Z"));
    assert_eq!(merged.binding.as_deref(), Some("five_hour"));
}

#[test]
fn an_older_status_line_reading_does_not_win() {
    let reading = Reading {
        plan: None,
        fetched_at: "2026-09-07T12:00:00Z".to_owned(),
        windows: map_response(&usage_body(), None, None).unwrap().windows,
        remembered: false,
        reason: None,
    };
    let merged = merge(
        passive("2026-09-07T11:00:00Z", 99.0, 99.0),
        Some(&reading),
        "2026-09-07T12:00:10Z",
    );
    assert_eq!(merged.windows["five_hour"].percent, Some(12.0));
    assert_eq!(merged.windows["seven_day"].percent, Some(18.0));
}

#[test]
fn a_passive_window_with_no_number_never_replaces_one_that_has_one() {
    let reading = Reading {
        plan: None,
        fetched_at: "2026-09-07T12:00:00Z".to_owned(),
        windows: map_response(&usage_body(), None, None).unwrap().windows,
        remembered: false,
        reason: None,
    };

    let mut block = passive("2026-09-07T12:05:00Z", 25.0, 19.0);
    block.windows.insert(
        "five_hour".to_owned(),
        Window::error("the payload carried no rate_limits").with_window_minutes(300),
    );

    let merged = merge(block, Some(&reading), "2026-09-07T12:05:10Z");
    assert_eq!(
        merged.windows["five_hour"].percent,
        Some(12.0),
        "\"I could not read it\" is not fresher information than a number"
    );
    assert_eq!(merged.windows["seven_day"].percent, Some(19.0));
}

#[test]
fn an_endpoint_reading_that_has_aged_out_says_so() {
    let reading = Reading {
        plan: None,
        fetched_at: "2026-09-07T12:00:00Z".to_owned(),
        windows: map_response(&usage_body(), None, None).unwrap().windows,
        remembered: false,
        reason: None,
    };

    let fresh = merge(Provider::default(), Some(&reading), "2026-09-07T12:14:00Z");
    assert_eq!(fresh.windows["seven_day_fable"].state, WindowState::Ok);

    let old = merge(Provider::default(), Some(&reading), "2026-09-07T12:40:00Z");
    assert_eq!(old.windows["seven_day_fable"].state, WindowState::Stale);
    assert!(
        old.windows["seven_day_fable"]
            .error
            .as_deref()
            .is_some_and(|reason| reason.contains("40 minutes ago")),
        "got {:?}",
        old.windows["seven_day_fable"].error
    );
    assert_eq!(
        old.windows["seven_day_fable"].percent,
        Some(23.0),
        "old and honest beats blank"
    );
}

#[test]
fn the_endpoint_alone_is_enough_to_call_the_provider_configured() {
    let reading = Reading {
        plan: Some("max_20x".to_owned()),
        fetched_at: "2026-09-07T12:00:00Z".to_owned(),
        windows: map_response(&usage_body(), None, None).unwrap().windows,
        remembered: false,
        reason: None,
    };
    // No status-line wrapper on this machine at all.
    let merged = merge(Provider::default(), Some(&reading), "2026-09-07T12:00:10Z");
    assert!(merged.configured);
    assert_eq!(merged.windows.len(), 3);
}

#[test]
fn an_empty_reading_leaves_the_passive_block_alone() {
    let reading = Reading {
        plan: None,
        fetched_at: "2026-09-07T12:00:00Z".to_owned(),
        windows: BTreeMap::new(),
        remembered: true,
        reason: Some("the usage endpoint timed out".to_owned()),
    };
    let block = passive("2026-09-07T11:00:00Z", 12.0, 18.0);
    assert_eq!(
        merge(block.clone(), Some(&reading), "2026-09-07T12:00:10Z"),
        block
    );
}

// --------------------------------------------------------- the leak gates --

/// The acceptance criterion, as a test.
///
/// A sentinel goes in where the token goes, the whole flow runs — sign-in read, request
/// sent, answer mapped, block merged, document written, then a failure whose body echoes
/// the sentinel back — and afterwards the sentinel must be findable in exactly one place:
/// the `Authorization` header the mock server received. Not in a file, not in an error, not
/// in a `Debug` rendering, not in `limits.json`.
#[test]
fn the_token_reaches_the_header_and_nowhere_else() {
    const SENTINEL: &str = "nazar-leak-sentinel-Zx7Q2m4Kp9Rt";

    let nazar_home = TempDir::new("leak-nazar-home");
    let claude_home = TempDir::new("leak-claude-home");
    let credentials = write_sign_in(&claude_home, SENTINEL);

    // The settings the mode is turned on with, written where the real ones would live.
    let config = crate::config::Config {
        detailed_windows: true,
        detailed_suggested: true,
        ..crate::config::Config::default()
    };
    config.write(&nazar_home.join("config.json")).unwrap();

    // A 200, then a 403 whose body echoes the token back at us — the worst case for a
    // client that puts response bodies into error messages.
    let server = MockServer::start(vec![
        Canned::ok(usage_body()),
        Canned::status_with_body(403, format!(r#"{{"error":"bad token {SENTINEL}"}}"#)),
    ]);
    let clock = TestClock::new("2026-09-07T12:00:00Z");

    let mut mode = DetailedWindows::new(true, || {
        Some(DetailedReader::new(
            UsageClient::new(server.url()),
            &credentials,
        ))
    });

    let first = mode.refresh(&clock).expect("the mode is on");
    assert!(first.is_fresh(), "the flow must actually have run");
    clock.advance(Duration::from_secs(60));
    clock.set_wall("2026-09-07T12:01:00Z");
    let second = mode.refresh(&clock).expect("the mode is on");
    assert_eq!(second.error, Some(DetailedError::Unauthorized));

    // Everything the flow produces, written where the real files go.
    let mut limits = Limits::new("2026-09-07T12:01:00Z");
    limits.providers.claude = merge(
        passive("2026-09-07T11:59:00Z", 12.0, 18.0),
        first.reading.as_ref(),
        "2026-09-07T12:01:00Z",
    );
    crate::limits::write_limits(&nazar_home.join("limits.json"), &limits).unwrap();

    // The sentinel did travel: this test would pass trivially if it had not.
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].header("authorization"),
        Some(format!("Bearer {SENTINEL}").as_str())
    );

    // And now: nowhere else.
    let mut renderings = vec![
        limits.to_json().unwrap(),
        format!("{first:?}"),
        format!("{second:?}"),
        format!("{mode:?}"),
        format!("{:?}", limits.providers.claude),
    ];
    for outcome in [&first, &second] {
        if let Some(error) = &outcome.error {
            renderings.push(format!("{error}"));
            renderings.push(format!("{error:?}"));
        }
        if let Some(reading) = &outcome.reading {
            renderings.push(format!("{reading:?}"));
            renderings.push(reading.reason.clone().unwrap_or_default());
        }
    }
    for rendering in &renderings {
        assert!(
            !rendering.contains(SENTINEL),
            "the token reached a rendering:\n{rendering}"
        );
    }

    for path in files_under(&nazar_home) {
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(SENTINEL),
            "the token reached {}",
            path.display()
        );
    }
    assert!(
        files_under(&nazar_home).len() >= 2,
        "the walk found almost nothing, so it is not proving anything"
    );

    // The sign-in file itself is untouched: read, never written.
    let after = std::fs::read_to_string(&credentials).unwrap();
    assert!(
        after.contains(SENTINEL),
        "the sign-in file must be left alone"
    );
}

#[test]
fn a_secret_prints_nothing_and_has_no_display_at_all() {
    let secret = Secret::new("nazar-leak-sentinel-in-a-secret".to_owned());
    assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
    assert!(!format!("{secret:?}").contains("sentinel"));
    assert_eq!(secret.len(), "nazar-leak-sentinel-in-a-secret".len());
    assert!(!secret.is_empty());
    assert_eq!(
        secret.expose_for_one_request(),
        "nazar-leak-sentinel-in-a-secret"
    );

    // A struct that holds one can still derive Debug, and stays silent.
    #[derive(Debug)]
    struct Holder {
        #[allow(dead_code)]
        token: Secret,
    }
    let holder = Holder { token: secret };
    assert!(!format!("{holder:?}").contains("sentinel"), "{holder:?}");
}

#[test]
fn a_sign_in_never_prints_its_token() {
    let dir = TempDir::new("detailed-signin-debug");
    let credentials = write_sign_in(&dir, "nazar-leak-sentinel-in-a-sign-in");
    let sign_in = credentials::read_sign_in(&credentials).unwrap();

    assert!(!format!("{sign_in:?}").contains("sentinel"), "{sign_in:?}");
    assert_eq!(
        sign_in.rate_limit_tier.as_deref(),
        Some("default_claude_max_20x")
    );
    assert_eq!(sign_in.subscription_type.as_deref(), Some("max"));
    assert_eq!(
        sign_in.token.expose_for_one_request(),
        "nazar-leak-sentinel-in-a-sign-in"
    );
}

#[test]
fn a_file_that_is_not_a_sign_in_is_not_one() {
    let dir = TempDir::new("detailed-not-a-signin");
    for (name, contents) in [
        ("empty.json", ""),
        ("not-json.json", "just some text"),
        (
            "no-oauth.json",
            r#"{"somethingElse": {"accessToken": "x"}}"#,
        ),
        ("no-token.json", r#"{"claudeAiOauth": {"expiresAt": 1}}"#),
        (
            "empty-token.json",
            r#"{"claudeAiOauth": {"accessToken": ""}}"#,
        ),
        (
            "token-is-not-a-string.json",
            r#"{"claudeAiOauth": {"accessToken": 12345}}"#,
        ),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        assert_eq!(
            credentials::read_sign_in(&path).err(),
            Some(DetailedError::NotSignedIn),
            "{name} should not have been read as a sign-in"
        );
    }
}

/// Every regular file under `dir`, however deep.
fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}
