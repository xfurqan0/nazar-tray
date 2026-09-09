//! The readers, run against payloads this machine actually produced.
//!
//! Every other test in this crate is written against a world its author imagined. That is
//! most of what a test suite should be — it is how a boundary case gets exercised at all —
//! but it has one failure mode that no amount of it can fix: **the author's idea of the
//! payload is the same idea in the test and in the code**, so a field that is not shaped
//! the way both of them assume is a case neither of them has. The fixtures under
//! `crates/nazar-core/fixtures/captured/` are the other half of that. They are real
//! `~/.nazar` files, copied off a running installation on 2026-09-08, and the only thing
//! changed in them is identity:
//!
//! | Kept, exactly | Replaced |
//! |---|---|
//! | every number: percentages, resets, token counts, costs, cache statistics | session and prompt identifiers → fixed synthetic UUIDs |
//! | every date and time, to the second | paths → `C:\work\project` and a synthetic Claude home |
//! | the envelope, the key order, the nulls, the empty objects | the session's name → `session-a`; the repository → an example one |
//!
//! So the shapes here are not a guess: `context_window` really does come through with four
//! nulls in it before a session's first API response, a payload really does drop
//! `five_hour` once that window has reset rather than reporting it at zero, and the weekly
//! reset really was reported one second apart on two consecutive captures. Each of those
//! was a *sentence in a document* before it was a fixture, and a sentence in a document is
//! exactly the kind of claim that goes quietly out of date.
//!
//! `tests/hygiene.rs` greps this directory on every run for anything that identifies the
//! machine it came from, so adding a capture is safe as long as the sanitising is.

use std::collections::BTreeMap;

use crate::alerts::{AlertRules, Alerts};
use crate::claude::{self, ClaudeReader};
use crate::limits::{Limits, Provider, Source, WindowState};
use crate::state::{Rules, Snapshot};
use crate::testutil::TempDir;

/// A live session with both windows: `five_hour` at 15 %, `seven_day` at 74 %.
const BOTH_WINDOWS: &str = include_str!("../fixtures/captured/statusline-both-windows.json");
/// A session whose five-hour window had reset: Claude Code stops reporting it entirely.
const WEEKLY_ONLY: &str = include_str!("../fixtures/captured/statusline-weekly-only.json");
/// A session before its first API response: no `rate_limits`, and a `context_window` of nulls.
const NO_RATE_LIMITS: &str = include_str!("../fixtures/captured/statusline-no-rate-limits.json");
/// The weekly reset as it was captured on the minute…
const RESET_ON_THE_MINUTE: &str =
    include_str!("../fixtures/captured/statusline-weekly-reset-02-00-00.json");
/// …and one second under it, one refresh later. Thirty-two toasts came out of this pair.
const RESET_A_SECOND_UNDER: &str =
    include_str!("../fixtures/captured/statusline-weekly-reset-01-59-59.json");
/// The document the tray wrote from all of the above.
///
/// Named `limits-captured.json` rather than `limits.json` because `.gitignore` refuses the
/// latter by name anywhere in the tree — "a real `limits.json` is this machine's usage data,
/// not a fixture" — and that rule is worth more than the filename. This *is* that machine's
/// usage data; what makes it committable is that usage figures are not identity, which is the
/// same line `tests/hygiene.rs` has drawn since WP1.
const LIMITS: &str = include_str!("../fixtures/captured/limits-captured.json");

/// An instant inside every window these captures report, so nothing is expired.
const NOW: &str = "2026-09-08T22:30:00Z";

/// Write a capture into a directory the Claude reader will read.
fn plant(dir: &TempDir, name: &str, capture: &str) {
    std::fs::write(dir.join(name), capture).unwrap();
}

/// The provider block one capture produces, on its own.
fn provider_from(label: &str, capture: &str) -> Provider {
    let dir = TempDir::new(label);
    plant(&dir, "00000000-0000-4000-8000-00000000c0de.json", capture);
    ClaudeReader::new(&dir.path).refresh()
}

// ------------------------------------------------------------------ the payload shapes

#[test]
fn the_captured_payload_parses_to_the_numbers_the_status_line_was_showing() {
    let reading = claude::parse_capture(BOTH_WINDOWS).expect("a real capture must parse");

    assert!(reading.has_rate_limits);
    assert_eq!(reading.captured_at.as_deref(), Some("2026-09-08T22:21:45Z"));

    let five_hour = reading.five_hour.clone().expect("five_hour was reported");
    assert_eq!(five_hour.used_percentage, 15.0);
    assert_eq!(five_hour.resets_at.as_deref(), Some("2026-09-09T00:50:00Z"));

    let seven_day = reading.seven_day.clone().expect("seven_day was reported");
    assert_eq!(seven_day.used_percentage, 74.0);
    assert_eq!(seven_day.resets_at.as_deref(), Some("2026-09-12T02:00:00Z"));

    // Nothing else in the payload comes out. The percentages that are *not* quota — the
    // context window's 21 %, the cache's hit ratio — are the ones a careless reader would
    // pick up, and they are all over this file.
    assert_eq!(reading.plan, None, "no payload has ever named a plan");
    let rendered = format!("{reading:?}");
    for absent in ["0.982", "206267", "work", "example-owner", "session-a"] {
        assert!(!rendered.contains(absent), "{absent} leaked: {rendered}");
    }
}

#[test]
fn a_window_claude_code_has_stopped_reporting_is_absent_rather_than_zero() {
    // The real shape of a capture taken after the five-hour window reset: `rate_limits` is
    // there, `seven_day` is in it, and `five_hour` is simply not. A reader that defaulted a
    // missing number would show a fresh five-hour window at 0 % — reassuring, and invented.
    let reading = claude::parse_capture(WEEKLY_ONLY).expect("a real capture must parse");

    assert!(reading.has_rate_limits);
    assert_eq!(reading.five_hour, None);
    assert_eq!(reading.seven_day.unwrap().used_percentage, 42.0);

    let provider = provider_from("captured-weekly-only", WEEKLY_ONLY);
    assert!(provider.configured);
    assert!(
        !provider.windows.contains_key(claude::WINDOW_FIVE_HOUR),
        "a window the payload did not carry is absent from the block, not a zero in it"
    );
    assert_eq!(provider.windows.len(), 1);
    assert_eq!(
        provider.windows[claude::WINDOW_SEVEN_DAY].percent,
        Some(42.0)
    );
    assert_eq!(
        provider.windows[claude::WINDOW_SEVEN_DAY].state,
        WindowState::Ok
    );
    assert_eq!(provider.binding.as_deref(), Some(claude::WINDOW_SEVEN_DAY));
}

#[test]
fn a_session_before_its_first_response_reports_nothing_and_says_why() {
    // Captured from a session that had just started: no `rate_limits` key at all, and a
    // `context_window` whose `used_percentage` is `null`. Two ways for a percentage to be
    // missing, in one file, both of them real.
    assert!(NO_RATE_LIMITS.contains("\"used_percentage\": null"));

    let reading = claude::parse_capture(NO_RATE_LIMITS).expect("a real capture must parse");
    assert!(!reading.has_rate_limits);
    assert!(!reading.is_usable());

    let provider = provider_from("captured-no-limits", NO_RATE_LIMITS);
    assert!(provider.configured, "the wrapper is installed and running");
    assert_eq!(provider.binding, None);
    for window in provider.windows.values() {
        assert_eq!(window.percent, None);
        assert_eq!(window.state, WindowState::Error);
        assert!(window.error.is_some());
    }
}

// ------------------------------------------------------------------ the jitter, captured

#[test]
fn the_two_spellings_of_one_reset_are_a_minute_apart_and_still_one_period() {
    // The pair T-WP9 was written for, as two files rather than as a sentence in a log
    // entry: the same weekly reset, reported as `…02:00:00Z` and `…01:59:59Z` one refresh
    // apart. Flooring to the minute does **not** flatten this one — one second under the
    // minute floors to the minute before — which is exactly why the fix has a second half.
    let on_the_minute = provider_from("captured-jitter-a", RESET_ON_THE_MINUTE);
    let a_second_under = provider_from("captured-jitter-b", RESET_A_SECOND_UNDER);

    assert_eq!(
        on_the_minute.windows[claude::WINDOW_SEVEN_DAY]
            .resets_at
            .as_deref(),
        Some("2026-09-12T02:00:00Z")
    );
    assert_eq!(
        a_second_under.windows[claude::WINDOW_SEVEN_DAY]
            .resets_at
            .as_deref(),
        Some("2026-09-12T01:59:00Z"),
        "one second under the minute floors to the minute before it, not to the same one"
    );

    // And the half of the fix that carries this case: one toast, then silence.
    let mut alerts = Alerts::in_memory();
    let fired: Vec<f64> = alerts
        .evaluate(&view_of(on_the_minute.clone()), &AlertRules::default())
        .iter()
        .map(|alert| alert.threshold)
        .collect();
    assert_eq!(fired, vec![60.0], "74 % arrives above the warning line");

    for _ in 0..4 {
        assert!(
            alerts
                .evaluate(&view_of(a_second_under.clone()), &AlertRules::default())
                .is_empty(),
            "a second of jitter is not a new week"
        );
        assert!(
            alerts
                .evaluate(&view_of(on_the_minute.clone()), &AlertRules::default())
                .is_empty(),
            "and neither is jittering back"
        );
    }
}

/// The derived view of one Claude provider block, at [`NOW`].
fn view_of(claude: Provider) -> crate::state::SnapshotView {
    let mut limits = Limits::new(NOW.to_owned());
    limits.providers.claude = claude;
    Snapshot::new(limits).view(NOW, &Rules::default())
}

// ------------------------------------------------------------------ the document itself

#[test]
fn the_captured_document_is_a_contract_document_and_round_trips() {
    let limits = Limits::from_json(LIMITS).expect("a real limits.json must parse");

    assert_eq!(limits.schema_version, crate::limits::SCHEMA_VERSION);
    assert_eq!(limits.updated_at, "2026-09-09T00:49:15Z");
    assert_eq!(
        limits.to_json().unwrap(),
        LIMITS,
        "a document this build wrote must survive being read and written back, byte for byte"
    );

    let claude = &limits.providers.claude;
    assert_eq!(claude.source, Some(Source::Endpoint));
    assert_eq!(claude.plan.as_deref(), Some("max_20x"));
    assert_eq!(claude.binding.as_deref(), Some("seven_day"));
    assert_eq!(claude.windows["seven_day"].percent, Some(78.0));
    assert_eq!(
        claude.windows["seven_day_fable"].model.as_deref(),
        Some("Fable"),
        "the model-scoped weekly the passive path cannot see"
    );
    // The jitter, in the document the tray actually wrote: the endpoint's weekly reset is
    // `02:00:00Z` on some refreshes and `01:59:59Z` on others, and the second of those is
    // what was on disk when this was copied — floored to the minute *before*, which is why
    // flooring alone was never the whole of the T-WP9 fix.
    assert_eq!(
        claude.windows["seven_day"].resets_at.as_deref(),
        Some("2026-09-12T01:59:00Z")
    );

    let codex = &limits.providers.codex;
    assert_eq!(codex.source, Some(Source::Rollout));
    assert_eq!(codex.plan.as_deref(), Some("plus"));
    assert_eq!(codex.binding.as_deref(), Some("primary"));

    // The one thing this build would write differently. The capture is from before the
    // Codex reader was put through the same minute-flooring gate as the Claude one, so its
    // resets still carry seconds; reading such a document back is unaffected, which is what
    // rule 4 of the contract promises and what this line is here to keep true.
    assert_eq!(
        codex.windows["primary"].resets_at.as_deref(),
        Some("2026-09-09T02:31:59Z")
    );
}

#[test]
fn the_derived_view_of_the_captured_document_says_what_the_panel_showed() {
    // The tray drew this at 00:49 on the ninth: Claude binding on its weekly window at
    // 78 %, Codex on its five-hour window at 60 %, and one of the two providers stale
    // because nothing had written a rollout log since the evening before.
    let limits = Limits::from_json(LIMITS).unwrap();
    let view = Snapshot::new(limits).view("2026-09-09T00:50:00Z", &Rules::default());

    let by_name: BTreeMap<&str, &crate::state::ProviderView> = view
        .providers
        .iter()
        .map(|provider| (provider.name.as_str(), provider))
        .collect();

    let claude = by_name["claude"];
    assert_eq!(claude.binding.as_deref(), Some("seven_day"));
    assert_eq!(
        claude.severity,
        crate::state::Severity::Warn,
        "78 % is warn"
    );
    assert_eq!(claude.freshness, crate::state::Freshness::Fresh);

    let codex = by_name["codex"];
    assert_eq!(codex.binding.as_deref(), Some("primary"));
    assert_eq!(codex.severity, crate::state::Severity::Warn, "60 % is warn");
    // Written at 21:50 the evening before and read at 00:50: three hours old.
    assert_eq!(codex.freshness, crate::state::Freshness::Stale);

    // And the reason the Codex reader's reset rule is the Codex reader's alone, in a
    // document rather than in an argument: **Claude's five-hour window was fifteen seconds
    // past its own reset at the moment the tray wrote this file**, because that window
    // renews while the session is open and is re-reported on the next refresh. A reader
    // that greyed out a window the moment its reset passed would have blinked this one
    // grey — a live window, on a machine actively in use.
    let five_hour = claude
        .windows
        .iter()
        .find(|window| window.key == "five_hour")
        .expect("the five-hour window is in the document");
    assert_eq!(five_hour.resets_at.as_deref(), Some("2026-09-09T00:49:00Z"));
    assert!(
        five_hour
            .remaining_ms
            .is_some_and(|remaining| remaining < 0),
        "this window's reset was already behind the instant the file was written"
    );
    assert_eq!(five_hour.state, "ok", "and the reader left it alone");

    // Every other window in the file is still counting down.
    for provider in &view.providers {
        for window in &provider.windows {
            if window.key == "five_hour" && provider.name == "claude" {
                continue;
            }
            let remaining = window.remaining_ms.expect("every window has a reset");
            assert!(
                remaining > 0,
                "{}/{} was already past its reset",
                provider.name,
                window.key
            );
        }
    }
}

// ------------------------------------------------------------------ the whole directory

#[test]
fn the_newest_capture_in_a_directory_full_of_them_is_the_one_that_counts() {
    // What the reader actually faces: several sessions, each rewriting its own file, and
    // one of them with no numbers in it at all. The envelope's `updatedAt` decides, not
    // the filename and not the modification time, which a copy or a restore can move.
    let dir = TempDir::new("captured-directory");
    plant(&dir, "a.json", NO_RATE_LIMITS); // 2026-09-08T21:15:44Z
    plant(&dir, "b.json", WEEKLY_ONLY); // 2026-09-08T02:38:25Z
    plant(&dir, "c.json", BOTH_WINDOWS); // 2026-09-08T22:21:45Z
    plant(&dir, "chain.json", "{\"schemaVersion\":1}");

    let provider = ClaudeReader::new(&dir.path).refresh();
    assert_eq!(provider.source, Some(Source::Statusline));
    assert_eq!(provider.source_at.as_deref(), Some("2026-09-08T22:21:45Z"));
    assert_eq!(
        provider.windows[claude::WINDOW_SEVEN_DAY].percent,
        Some(74.0)
    );
    assert_eq!(
        provider.windows[claude::WINDOW_FIVE_HOUR].percent,
        Some(15.0)
    );
    assert_eq!(provider.binding.as_deref(), Some(claude::WINDOW_SEVEN_DAY));
}

#[test]
fn nothing_from_a_captured_payload_reaches_the_contract() {
    // The leak test, run over real payloads rather than a constructed one. A capture is
    // made of paths, names and a repository; four numbers are allowed out of it.
    let dir = TempDir::new("captured-leak");
    for (name, capture) in [
        ("a.json", BOTH_WINDOWS),
        ("b.json", WEEKLY_ONLY),
        ("c.json", NO_RATE_LIMITS),
    ] {
        plant(&dir, name, capture);
    }

    let provider = ClaudeReader::new(&dir.path).refresh();
    let json = serde_json::to_string(&provider).unwrap();

    for absent in [
        "work",
        "project",
        "claude-home",
        "example-owner",
        "example-repo",
        "session-a",
        "00000000-0000-4000-8000",
        "scratchpad",
        "transcript",
    ] {
        assert!(!json.contains(absent), "{absent} reached the file: {json}");
    }
}
