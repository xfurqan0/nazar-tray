//! The notification state machine, driven the way the tray drives it.
//!
//! Every test here builds a real `limits.json` document, derives the real view through
//! [`crate::state`], and hands that to [`Alerts::evaluate`] — rather than constructing a
//! `SnapshotView` by hand. The view is the only thing the tray ever passes in, and a test
//! that skipped it would not notice the day a window stopped carrying its `resetsAt`.

use std::collections::BTreeMap;
use std::time::Duration;

use super::*;
use crate::limits::{Limits, Provider, Source, Window};
use crate::state::{Rules, Snapshot};
use crate::testutil::{ManualClock, TempDir};

/// An instant far enough in the future to be obviously synthetic.
const RESET_A: &str = "2026-09-12T02:00:00Z";
/// The next week's reset.
const RESET_B: &str = "2026-09-19T02:00:00Z";
/// [`RESET_A`] as the usage endpoint also spelled it on 2026-09-08, one second earlier and
/// one refresh apart. Thirty-two toasts came out of this string.
const RESET_A_JITTERED: &str = "2026-09-12T01:59:59Z";
const NOW: &str = "2026-09-07T12:00:00Z";

/// A view with one Codex weekly window at `percent`, resetting at `resets_at`.
fn view_at(percent: f64, resets_at: &str, now: &str) -> crate::state::SnapshotView {
    view_of(
        Window::ok(percent)
            .with_window_minutes(10080)
            .with_resets_at(resets_at),
        now,
    )
}

/// A view with one Codex weekly window built by the caller.
fn view_of(window: Window, now: &str) -> crate::state::SnapshotView {
    let mut windows = BTreeMap::new();
    windows.insert("secondary".to_owned(), window);
    let mut limits = Limits::new(now.to_owned());
    limits.providers.codex = Provider {
        configured: true,
        source: Some(Source::Rollout),
        source_at: Some(now.to_owned()),
        windows,
        ..Provider::default()
    };
    Snapshot::new(limits).view(now, &Rules::default())
}

/// The thresholds a crossing is measured against, with quiet hours off.
fn rules() -> AlertRules {
    AlertRules::default()
}

/// The thresholds each alert names, in order.
fn thresholds(alerts: &[Alert]) -> Vec<f64> {
    alerts.iter().map(|alert| alert.threshold).collect()
}

// ------------------------------------------------------------------ edge crossing

#[test]
fn a_crossing_fires_once_and_staying_above_says_nothing() {
    let mut alerts = Alerts::in_memory();

    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(40.0, RESET_A, NOW), &rules())),
        Vec::<f64>::new(),
        "40 % is below every threshold"
    );
    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(86.0, RESET_A, NOW), &rules())),
        vec![85.0],
        "crossing 60 and 85 in one step is one toast, and it names the worse of them"
    );
    for _ in 0..5 {
        assert!(
            alerts
                .evaluate(&view_at(86.0, RESET_A, NOW), &rules())
                .is_empty(),
            "a window that is still above a threshold has not crossed it again"
        );
    }
    assert!(
        alerts
            .evaluate(&view_at(99.0, RESET_A, NOW), &rules())
            .is_empty(),
        "climbing without crossing anything new is silence"
    );
    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(100.0, RESET_A, NOW), &rules())),
        vec![100.0],
        "and the window running out is worth saying"
    );
}

#[test]
fn each_threshold_gets_its_own_toast_when_they_are_crossed_apart() {
    let mut alerts = Alerts::in_memory();

    assert!(
        alerts
            .evaluate(&view_at(10.0, RESET_A, NOW), &rules())
            .is_empty()
    );
    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(61.0, RESET_A, NOW), &rules())),
        vec![60.0]
    );
    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(85.0, RESET_A, NOW), &rules())),
        vec![85.0],
        "exactly 85 is a crossing: the thresholds are compared with >=, like the severities"
    );
}

#[test]
fn the_lower_thresholds_of_a_leap_are_consumed_rather_than_skipped() {
    let mut alerts = Alerts::in_memory();

    // 10 % to 91 % in one refresh: 60 and 85 are both crossed, one toast says 85.
    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(10.0, RESET_A, NOW), &rules())),
        Vec::<f64>::new()
    );
    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(91.0, RESET_A, NOW), &rules())),
        vec![85.0]
    );
    assert_eq!(
        alerts.log().windows["codex/secondary"].fired,
        vec![60.0, 85.0],
        "60 was consumed, so it cannot arrive late once the number settles"
    );
}

#[test]
fn a_number_that_falls_back_and_climbs_again_does_not_repeat_itself() {
    let mut alerts = Alerts::in_memory();

    alerts.evaluate(&view_at(90.0, RESET_A, NOW), &rules());
    // A source correction, not a reset: the same window says a smaller number.
    assert!(
        alerts
            .evaluate(&view_at(10.0, RESET_A, NOW), &rules())
            .is_empty()
    );
    assert!(
        alerts
            .evaluate(&view_at(90.0, RESET_A, NOW), &rules())
            .is_empty(),
        "once per threshold per reset period, whatever the number did in between"
    );
}

// ---------------------------------------------------------------- startup above

#[test]
fn a_tray_started_above_a_threshold_says_so_once() {
    let mut alerts = Alerts::in_memory();

    // No previous reading at all: rule 3.
    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(91.0, RESET_A, NOW), &rules())),
        vec![85.0],
        "arriving at a window that is already spent is worth exactly one sentence"
    );
    assert!(
        alerts
            .evaluate(&view_at(91.0, RESET_A, NOW), &rules())
            .is_empty(),
        "and then silence"
    );
}

#[test]
fn a_restart_does_not_re_fire_what_the_file_remembers() {
    let dir = TempDir::new("alerts-restart");
    let path = dir.join("alerts.json");

    let mut first = Alerts::open(&path);
    assert_eq!(
        thresholds(&first.evaluate(&view_at(86.0, RESET_A, NOW), &rules())),
        vec![85.0]
    );
    first.save().unwrap();
    assert!(
        path.exists(),
        "a crossing has to reach the disk to survive a restart"
    );

    // The whole process goes away and comes back with the window still at 86 %.
    let mut second = Alerts::open(&path);
    assert!(
        second
            .evaluate(&view_at(86.0, RESET_A, NOW), &rules())
            .is_empty(),
        "restarting the tray must not repeat a warning the user has already had"
    );
    assert!(
        !second.is_dirty(),
        "and nothing changed, so nothing is rewritten"
    );
}

#[test]
fn the_log_survives_a_round_trip_and_keeps_keys_it_does_not_know() {
    let dir = TempDir::new("alerts-round-trip");
    let path = dir.join("alerts.json");

    let mut alerts = Alerts::open(&path);
    alerts.evaluate(&view_at(86.0, RESET_A, NOW), &rules());
    alerts.save().unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.ends_with('\n'), "got {text:?}");
    assert!(text.contains("\"codex/secondary\""), "got {text}");
    assert!(text.contains("\"resetsAt\""), "got {text}");

    let read = Alerts::read(&path).unwrap();
    assert_eq!(read.schema_version, ALERTS_SCHEMA_VERSION);
    assert_eq!(read.windows["codex/secondary"].fired, vec![60.0, 85.0]);

    let newer = r#"{ "schemaVersion": 1, "snoozedUntil": "2026-09-08T00:00:00Z",
        "windows": { "codex/secondary": { "resetsAt": "…", "fired": [60], "note": "hi" } } }"#;
    let log = AlertLog::from_json(newer).unwrap();
    assert_eq!(
        log.extra["snoozedUntil"],
        Value::from("2026-09-08T00:00:00Z")
    );
    assert_eq!(
        log.windows["codex/secondary"].extra["note"],
        Value::from("hi")
    );
    assert_eq!(AlertLog::from_json(&log.to_json().unwrap()).unwrap(), log);
}

#[test]
fn a_damaged_log_is_an_empty_log_rather_than_a_refusal_to_warn() {
    let dir = TempDir::new("alerts-damaged");
    let path = dir.join("alerts.json");
    std::fs::write(&path, "{ \"windows\": ").unwrap();

    let mut alerts = Alerts::open(&path);
    assert!(alerts.log().windows.is_empty());
    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(86.0, RESET_A, NOW), &rules())),
        vec![85.0],
        "the cost of the wrong guess here is one extra toast; the other way it is silence"
    );
    assert!(
        Alerts::read(&path).is_err(),
        "and the damage is still reportable"
    );
}

// -------------------------------------------------------------------- the reset

#[test]
fn a_new_reset_clears_the_keys_and_the_memory() {
    let mut alerts = Alerts::in_memory();

    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(86.0, RESET_A, NOW), &rules())),
        vec![85.0]
    );
    // The week turns over and the number is the same: no dip to observe, and the toast
    // still has to fire. This is `--demo-cross`'s fourth step.
    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(86.0, RESET_B, NOW), &rules())),
        vec![85.0],
        "a new resetsAt is a new week; what was said last week does not count"
    );
    assert_eq!(
        alerts.log().windows["codex/secondary"].resets_at.as_deref(),
        Some(RESET_B)
    );
    assert_eq!(
        alerts.log().windows["codex/secondary"].fired,
        vec![60.0, 85.0]
    );
}

#[test]
fn a_reset_that_empties_the_window_drops_the_record_it_left_behind() {
    let mut alerts = Alerts::in_memory();

    alerts.evaluate(&view_at(86.0, RESET_A, NOW), &rules());
    assert!(alerts.log().windows.contains_key("codex/secondary"));

    // The normal shape of a reset: the week turns over and the number goes to nearly zero.
    assert!(
        alerts
            .evaluate(&view_at(2.0, RESET_B, NOW), &rules())
            .is_empty()
    );
    assert!(
        alerts.log().windows.is_empty(),
        "a record for a week nobody is in is bookkeeping nobody needs"
    );
}

#[test]
fn a_record_for_a_window_nobody_reports_is_swept_up_once_its_week_is_gone() {
    let mut alerts = Alerts::in_memory();
    alerts.evaluate(
        &view_at(86.0, "2026-09-01T02:00:00Z", "2026-09-01T01:00:00Z"),
        &rules(),
    );
    assert_eq!(alerts.log().windows.len(), 1);

    // A day later the window is still gone from the view, and its reset is a day past:
    // not yet, because a provider that could not be read for one refresh must keep its
    // bookkeeping or the next reading re-fires.
    let empty = Snapshot::new(Limits::new("2026-09-02T02:30:00Z"))
        .view("2026-09-02T02:30:00Z", &Rules::default());
    alerts.evaluate(&empty, &rules());
    assert_eq!(
        alerts.log().windows.len(),
        1,
        "one day is not long enough to forget"
    );

    let later = "2026-09-04T02:30:00Z";
    let empty = Snapshot::new(Limits::new(later.to_owned())).view(later, &Rules::default());
    alerts.evaluate(&empty, &rules());
    assert!(alerts.log().windows.is_empty(), "three days is");
}

// ------------------------------------------------------ a reset that only looks new

/// The live bug of 2026-09-08, in the shape it arrived in.
///
/// The usage endpoint alternated between `…T02:00:00Z` and `…T01:59:59Z` for the same
/// weekly reset, every five minutes, for two and a half hours. String equality read each
/// flip as a new week: 32 toasts, most of them doubled, and `alerts.json` rewritten every
/// time. One second of jitter is not a renewal.
#[test]
fn a_resets_at_that_jitters_by_a_second_is_the_same_period() {
    let dir = TempDir::new("alerts-jitter");
    let path = dir.join("alerts.json");
    let mut alerts = Alerts::open(&path);

    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(70.0, RESET_A, NOW), &rules())),
        vec![60.0],
        "70 % on arrival is a first observation and worth one toast"
    );
    alerts.save().unwrap();
    let written = std::fs::read_to_string(&path).unwrap();

    assert!(
        alerts
            .evaluate(&view_at(70.0, RESET_A_JITTERED, NOW), &rules())
            .is_empty(),
        "the same reset spelled one second earlier is not a new week"
    );
    assert!(
        alerts
            .evaluate(&view_at(70.0, RESET_A, NOW), &rules())
            .is_empty(),
        "and swinging back is not one either"
    );
    for _ in 0..8 {
        assert!(
            alerts
                .evaluate(&view_at(70.0, RESET_A_JITTERED, NOW), &rules())
                .is_empty()
        );
        assert!(
            alerts
                .evaluate(&view_at(70.0, RESET_A, NOW), &rules())
                .is_empty()
        );
    }

    assert!(
        !alerts.is_dirty(),
        "a wobble is not bookkeeping: nothing about the record changed"
    );
    alerts.save().unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        written,
        "alerts.json was rewritten every five minutes for two and a half hours"
    );
    assert_eq!(alerts.log().windows["codex/secondary"].fired, vec![60.0]);
}

/// The tolerance has to stay narrow enough that rule 4 still works.
#[test]
fn a_reset_that_really_renewed_still_clears_the_keys() {
    let mut alerts = Alerts::in_memory();

    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(70.0, RESET_A, NOW), &rules())),
        vec![60.0]
    );
    assert!(
        alerts
            .evaluate(&view_at(70.0, RESET_A_JITTERED, NOW), &rules())
            .is_empty()
    );
    // A week later, to the second: seven days is a whole window forward, which is what a
    // renewal looks like and what a wobble never does.
    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(70.0, RESET_B, NOW), &rules())),
        vec![60.0],
        "the fired set is cleared by a real reset, jitter or no jitter"
    );
    assert_eq!(
        alerts.log().windows["codex/secondary"].resets_at.as_deref(),
        Some(RESET_B),
        "and the record carries the newest spelling of the reset"
    );
    assert_eq!(alerts.log().windows["codex/secondary"].fired, vec![60.0]);
}

/// A window with no `windowMinutes` has no window to take half of, so the tolerance is a
/// flat hour.
#[test]
fn without_a_window_length_the_tolerance_is_an_hour() {
    let at = |resets_at: &str| view_of(Window::ok(70.0).with_resets_at(resets_at), NOW);

    let mut inside = Alerts::in_memory();
    assert_eq!(
        thresholds(&inside.evaluate(&at("2026-09-12T02:00:00Z"), &rules())),
        vec![60.0]
    );
    assert!(
        inside
            .evaluate(&at("2026-09-12T02:59:00Z"), &rules())
            .is_empty(),
        "fifty-nine minutes is still the same period"
    );

    let mut outside = Alerts::in_memory();
    assert_eq!(
        thresholds(&outside.evaluate(&at("2026-09-12T02:00:00Z"), &rules())),
        vec![60.0]
    );
    assert_eq!(
        thresholds(&outside.evaluate(&at("2026-09-12T03:01:00Z"), &rules())),
        vec![60.0],
        "sixty-one minutes is a different one"
    );
}

/// The half-window rule, and what it does with text it cannot read.
#[test]
fn two_resets_are_one_period_when_they_are_less_than_half_a_window_apart() {
    // A five-hour window tolerates just under two and a half hours.
    assert!(same_period(
        Some("2026-09-12T02:00:00Z"),
        Some("2026-09-12T04:29:00Z"),
        Some(300)
    ));
    assert!(!same_period(
        Some("2026-09-12T02:00:00Z"),
        Some("2026-09-12T04:31:00Z"),
        Some(300)
    ));
    // A weekly one tolerates just under three and a half days, and a real week is two of
    // those.
    assert!(same_period(
        Some(RESET_A),
        Some(RESET_A_JITTERED),
        Some(10080)
    ));
    assert!(!same_period(Some(RESET_A), Some(RESET_B), Some(10080)));

    // Nothing to subtract: back to comparing the strings, which is where this rule was.
    assert!(same_period(Some("whenever"), Some("whenever"), Some(10080)));
    assert!(!same_period(Some("whenever"), Some("later"), Some(10080)));
    assert!(!same_period(Some(RESET_A), Some("whenever"), Some(10080)));
    // A window that has never carried a reset is one period; one that lost its reset is not.
    assert!(same_period(None, None, Some(10080)));
    assert!(!same_period(Some(RESET_A), None, Some(10080)));
    // A nonsense length falls back to the hour rather than to zero tolerance.
    assert!(same_period(Some(RESET_A), Some(RESET_A_JITTERED), Some(0)));
}

/// The restart half of rule 4: the record on disk jitters too.
#[test]
fn a_record_read_from_disk_does_not_re_fire_on_a_jittered_reset() {
    let dir = TempDir::new("alerts-jitter-restart");
    let path = dir.join("alerts.json");
    std::fs::write(
        &path,
        "{\"schemaVersion\":1,\"windows\":{\"codex/secondary\":\
         {\"resetsAt\":\"2026-09-12T02:00:00Z\",\"fired\":[60.0]}}}\n",
    )
    .unwrap();

    // The tray comes back, and the first reading it gets is the other spelling. Before the
    // tolerance this cleared the record and toasted 60 % on every restart.
    let mut alerts = Alerts::open(&path);
    assert!(
        alerts
            .evaluate(&view_at(70.0, RESET_A_JITTERED, NOW), &rules())
            .is_empty(),
        "restarting into the wobble must not repeat a warning the user has already had"
    );
    assert!(!alerts.is_dirty(), "and nothing was rewritten");
    assert_eq!(alerts.log().windows["codex/secondary"].fired, vec![60.0]);
}

// ------------------------------------------------------------------ unknown windows

#[test]
fn a_window_nobody_could_read_never_notifies() {
    let mut alerts = Alerts::in_memory();

    let unreadable = view_of(
        Window::error("no quota line in the newest session log").with_window_minutes(10080),
        NOW,
    );
    assert!(alerts.evaluate(&unreadable, &rules()).is_empty());
    assert!(
        alerts.log().windows.is_empty(),
        "and it leaves nothing behind: there is no key for a number nobody read"
    );
}

#[test]
fn a_window_that_goes_unknown_and_comes_back_is_a_first_observation_again() {
    let mut alerts = Alerts::in_memory();

    assert!(
        alerts
            .evaluate(&view_at(40.0, RESET_A, NOW), &rules())
            .is_empty()
    );
    let unreadable = view_of(
        Window::error("the log moved").with_window_minutes(10080),
        NOW,
    );
    assert!(alerts.evaluate(&unreadable, &rules()).is_empty());

    // Back at 86 %: the reading before the gap is not evidence about this one, so this
    // counts as an arrival above the line rather than as a continuation.
    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(86.0, RESET_A, NOW), &rules())),
        vec![85.0]
    );
}

#[test]
fn a_stale_reading_still_notifies_because_the_number_is_real() {
    let mut alerts = Alerts::in_memory();
    let stale = view_of(
        Window::stale(86.0)
            .with_window_minutes(10080)
            .with_resets_at(RESET_A),
        NOW,
    );
    assert_eq!(
        thresholds(&alerts.evaluate(&stale, &rules())),
        vec![85.0],
        "stale means old, not wrong; error is the state that means nobody read it"
    );
}

// -------------------------------------------------------------------- quiet hours

#[test]
fn quiet_hours_suppress_the_toast_and_still_consume_the_key() {
    let mut alerts = Alerts::in_memory();
    let quiet = AlertRules {
        quiet: true,
        ..AlertRules::default()
    };

    let fired = alerts.evaluate(&view_at(86.0, RESET_A, NOW), &quiet);
    assert_eq!(thresholds(&fired), vec![85.0]);
    assert!(
        fired[0].suppressed,
        "the caller still colours the icon; only the interruption is withheld"
    );
    assert_eq!(
        alerts.log().windows["codex/secondary"].fired,
        vec![60.0, 85.0]
    );

    // Morning. The crossing is not delivered late: it was suppressed, not deferred.
    assert!(
        alerts
            .evaluate(&view_at(86.0, RESET_A, NOW), &rules())
            .is_empty()
    );
}

// ------------------------------------------------------------------ sleep and wake

#[test]
fn a_crossing_that_happened_during_a_sleep_fires_once_when_the_machine_wakes() {
    // The clock the refresh loop uses, moved the way a suspended laptop moves it. Pinned to
    // the same `NOW` the rest of this module evaluates against, so the nine hours below stay
    // on this side of `RESET_A` whatever day the suite runs.
    let clock = ManualClock::at(NOW);
    let dir = TempDir::new("alerts-sleep");
    let path = dir.join("alerts.json");
    let mut alerts = Alerts::open(&path);

    let before = clock.now();
    assert!(
        alerts
            .evaluate(&view_at(40.0, RESET_A, &before), &rules())
            .is_empty()
    );
    alerts.save().unwrap();

    // Nine hours pass on the wall clock and a hundred milliseconds on the monotonic one,
    // which is exactly how `crate::refresh::schedule` recognises a machine that has slept.
    clock.sleep_through(Duration::from_secs(9 * 3600));
    let after = clock.now();
    assert!(
        crate::timefmt::seconds_between(&before, &after).unwrap() >= 9 * 3600,
        "the harness has to actually move the wall clock"
    );

    let woke = alerts.evaluate(&view_at(88.0, RESET_A, &after), &rules());
    assert_eq!(
        thresholds(&woke),
        vec![85.0],
        "the crossing happened while nobody was looking, and it is still worth one toast"
    );

    // The loop's wake refresh is followed by its ordinary sixty-second ticks. None of them
    // may repeat it, and neither may a restart on the same numbers.
    clock.advance(Duration::from_secs(60));
    assert!(
        alerts
            .evaluate(&view_at(88.0, RESET_A, &clock.now()), &rules())
            .is_empty()
    );
    alerts.save().unwrap();

    let mut restarted = Alerts::open(&path);
    assert!(
        restarted
            .evaluate(&view_at(88.0, RESET_A, &clock.now()), &rules())
            .is_empty(),
        "nothing double-fires, whichever way the tray came back"
    );
}

// ------------------------------------------------------------------ the whole ladder

#[test]
fn the_demo_cross_sequence_produces_exactly_the_toasts_the_plan_asks_for() {
    // docs/PROJECT.md's acceptance run: 80 → 86 → 86 → reset → 86.
    let mut alerts = Alerts::in_memory();
    let seen: Vec<Vec<f64>> = [
        (80.0, RESET_A),
        (86.0, RESET_A),
        (86.0, RESET_A),
        (86.0, RESET_B),
    ]
    .into_iter()
    .map(|(percent, reset)| thresholds(&alerts.evaluate(&view_at(percent, reset, NOW), &rules())))
    .collect();

    assert_eq!(
        seen,
        vec![vec![60.0], vec![85.0], vec![], vec![85.0]],
        "one 60 % toast on arrival, one 85 % toast for the crossing, silence, and one \
         more 85 % toast after the reset"
    );
}

#[test]
fn thresholds_out_of_order_in_a_hand_edited_file_still_climb() {
    let mut alerts = Alerts::in_memory();
    let upside_down = AlertRules {
        thresholds: Thresholds {
            warn: 90.0,
            critical: 50.0,
            exhausted: 70.0,
        },
        quiet: false,
    };
    assert_eq!(upside_down.ladder(), vec![50.0, 70.0, 90.0]);

    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(75.0, RESET_A, NOW), &upside_down)),
        vec![70.0],
        "the ladder is sorted before it is climbed, whatever the settings file says"
    );
}

#[test]
fn a_ladder_with_repeats_or_nonsense_in_it_does_not_double_fire() {
    let repeated = AlertRules {
        thresholds: Thresholds {
            warn: 85.0,
            critical: 85.0,
            exhausted: f64::NAN,
        },
        quiet: false,
    };
    assert_eq!(repeated.ladder(), vec![85.0]);

    let mut alerts = Alerts::in_memory();
    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(86.0, RESET_A, NOW), &repeated)),
        vec![85.0]
    );
}

#[test]
fn an_alert_carries_what_a_sentence_about_it_needs() {
    let mut alerts = Alerts::in_memory();
    let mut windows = BTreeMap::new();
    windows.insert(
        "seven_day_fable".to_owned(),
        Window::ok(88.0)
            .with_window_minutes(10080)
            .with_resets_at("2026-09-07T14:10:00Z")
            .with_model("Fable"),
    );
    let mut limits = Limits::new(NOW.to_owned());
    limits.providers.claude = Provider {
        configured: true,
        windows,
        ..Provider::default()
    };
    let view = Snapshot::new(limits).view(NOW, &Rules::default());

    let fired = alerts.evaluate(&view, &rules());
    assert_eq!(fired.len(), 1);
    let alert = &fired[0];
    assert_eq!(alert.provider, "claude");
    assert_eq!(alert.window, "seven_day_fable");
    assert_eq!(alert.window_minutes, Some(10080));
    assert_eq!(alert.model.as_deref(), Some("Fable"));
    assert_eq!(alert.percent, 88.0);
    assert_eq!(
        alert.remaining_ms,
        Some(2 * 3_600_000 + 10 * 60_000),
        "the body says how long is left, so the alert has to carry it"
    );
}

#[test]
fn two_providers_crossing_at_once_are_two_alerts_in_the_views_order() {
    let mut alerts = Alerts::in_memory();

    let mut claude_windows = BTreeMap::new();
    claude_windows.insert(
        "five_hour".to_owned(),
        Window::ok(87.0)
            .with_window_minutes(300)
            .with_resets_at(RESET_A),
    );
    let mut codex_windows = BTreeMap::new();
    codex_windows.insert(
        "secondary".to_owned(),
        Window::ok(92.0)
            .with_window_minutes(10080)
            .with_resets_at(RESET_A),
    );

    let mut limits = Limits::new(NOW.to_owned());
    limits.providers.claude = Provider {
        configured: true,
        windows: claude_windows,
        ..Provider::default()
    };
    limits.providers.codex = Provider {
        configured: true,
        windows: codex_windows,
        ..Provider::default()
    };
    let view = Snapshot::new(limits).view(NOW, &Rules::default());

    let fired = alerts.evaluate(&view, &rules());
    assert_eq!(
        fired
            .iter()
            .map(|alert| format!("{}/{}", alert.provider, alert.window))
            .collect::<Vec<_>>(),
        vec!["claude/five_hour", "codex/secondary"],
        "Claude before Codex, which is the order the panel draws them in"
    );
}

#[test]
fn nothing_is_written_when_nothing_crossed() {
    let dir = TempDir::new("alerts-quiet-disk");
    let path = dir.join("alerts.json");
    let mut alerts = Alerts::open(&path);

    alerts.evaluate(&view_at(10.0, RESET_A, NOW), &rules());
    assert!(!alerts.is_dirty());
    alerts.save().unwrap();
    assert!(
        !path.exists(),
        "a machine that has never crossed a threshold has no alerts.json"
    );
}

// ------------------------------------------------------- the grid over time comparisons

/// Half of the weekly window, in seconds: the tolerance `same_period` uses for `10080`.
const HALF_WEEK: i64 = 10_080 * 60 / 2;
/// Half of the five-hour window.
const HALF_FIVE_HOURS: i64 = 300 * 60 / 2;

/// An instant `offset_seconds` from [`RESET_A`].
///
/// So that a row of the grid below can say "half a window, minus one" instead of a
/// timestamp somebody has to check by hand.
fn at(offset_seconds: i64) -> String {
    let origin = crate::timefmt::unix_seconds_from_rfc3339(RESET_A).unwrap();
    crate::timefmt::rfc3339_from_unix_seconds(origin + offset_seconds)
}

/// The weekly window, walked across `same_period`'s tolerance one named point at a time.
///
/// A grid rather than a generator, and the reason is repeatability. The comparisons this
/// module rests on are a subtraction against a threshold, and every bug either of them has
/// had was **at the threshold**: a second of jitter read as a new week (T-WP9), a reset
/// that had passed read as current (T-WP10). A random property test would have found those
/// eventually and told a different story every run. So the interesting points are named:
/// nothing apart, one second either way, either side of a minute, either side of half a
/// window, and a whole window on. No `proptest` dependency — a table is cheaper to read
/// than a generator and never flakes.
#[test]
fn the_same_period_grid_over_a_weekly_window() {
    let week = Some(10080u32);
    for (offset, expected, why) in [
        (0, true, "the same instant"),
        (1, true, "one second later: the jitter T-WP9 was about"),
        (
            -1,
            true,
            "one second earlier: the same jitter, the other way",
        ),
        (59, true, "just under a minute"),
        (61, true, "just over a minute"),
        (-61, true, "a minute backwards is not a new week either"),
        (3599, true, "an hour is nothing against a week"),
        (3601, true, "and neither is an hour and a second"),
        (
            HALF_WEEK - 1,
            true,
            "half a window minus one: still one period",
        ),
        (
            HALF_WEEK,
            false,
            "half a window exactly: the boundary is exclusive",
        ),
        (HALF_WEEK + 1, false, "past half a window: a different week"),
        (-HALF_WEEK, false, "and the same going backwards"),
        (
            7 * 86_400,
            false,
            "a whole window on: the renewal this rule is for",
        ),
    ] {
        assert_eq!(
            same_period(Some(RESET_A), Some(&at(offset)), week),
            expected,
            "{offset} s apart: {why}"
        );
        // The comparison is symmetric: which of two readings arrived first is not a fact
        // about the period, and a rule that answered differently would fire on alternate
        // polls, which is exactly the shape of the bug it was written for.
        assert_eq!(
            same_period(Some(&at(offset)), Some(RESET_A), week),
            expected,
            "{offset} s apart, the other way round: {why}"
        );
    }
}

/// The same walk for every window length, including the two that mean "no length".
#[test]
fn the_same_period_grid_over_the_shorter_windows_and_none_at_all() {
    for (window, tolerance, name) in [
        (Some(300u32), HALF_FIVE_HOURS, "the five-hour window"),
        (Some(10080u32), HALF_WEEK, "the weekly window"),
        (None, 3600, "a window with no stated length"),
        (Some(0u32), 3600, "a window whose length is a nonsense zero"),
    ] {
        assert!(
            same_period(Some(RESET_A), Some(&at(0)), window),
            "{name}: nothing apart"
        );
        assert!(
            same_period(Some(RESET_A), Some(&at(tolerance - 1)), window),
            "{name}: one second inside the tolerance"
        );
        assert!(
            !same_period(Some(RESET_A), Some(&at(tolerance)), window),
            "{name}: the tolerance itself is already outside"
        );
    }
}

/// The places arithmetic on instants goes wrong on its own.
#[test]
fn the_same_period_grid_where_arithmetic_on_instants_goes_wrong() {
    let week = Some(10080u32);

    // A zone offset names an instant, and two spellings of one instant are one period.
    assert!(same_period(
        Some("2026-09-12T02:00:00Z"),
        Some("2026-09-12T05:00:00+03:00"),
        week
    ));
    assert!(same_period(
        Some("2026-09-12T02:00:00Z"),
        Some("2026-09-11T22:00:00-04:00"),
        week
    ));

    // The night Europe puts its clocks forward. In UTC nothing happens at all, which is
    // the point of storing UTC: 00:59:59Z and 01:00:00Z are one second apart on the
    // 29th of March exactly as they are on any other day.
    assert!(same_period(
        Some("2026-03-29T00:59:59Z"),
        Some("2026-03-29T01:00:00Z"),
        week
    ));
    // And the local hour that does not exist that night is still an hour of real time.
    // Two local times two hours and two minutes apart on their faces — 01:59+01:00 is
    // 00:59Z, 04:01+03:00 is 01:01Z — are two minutes apart in reality, and one period.
    assert!(same_period(
        Some("2026-03-29T01:59:00+01:00"),
        Some("2026-03-29T04:01:00+03:00"),
        week
    ));

    // Before the epoch, where a subtraction that rounded towards zero would go wrong.
    assert!(same_period(
        Some("1969-12-31T23:59:59Z"),
        Some("1970-01-01T00:00:00Z"),
        week
    ));
    assert!(!same_period(
        Some("1969-12-31T23:00:00Z"),
        Some("1970-01-01T00:00:01Z"),
        None
    ));

    // Fractional seconds are below the resolution of every window there is.
    assert!(same_period(
        Some("2026-09-12T02:00:00.999999Z"),
        Some("2026-09-12T02:00:00Z"),
        week
    ));

    // Text that will not parse falls back to comparing the strings, which is where this
    // rule started: with nothing to subtract there is nothing to be tolerant with.
    for (left, right, expected) in [
        ("in about a week", "in about a week", true),
        ("in about a week", "in about two weeks", false),
        ("2026-09-12T02:00:00Z", "soon", false),
        ("", "", true),
        ("2026-02-30T02:00:00Z", "2026-02-30T02:00:01Z", false),
    ] {
        assert_eq!(
            same_period(Some(left), Some(right), week),
            expected,
            "{left:?} against {right:?}"
        );
    }

    // A window that has never carried a reset is one period; one that had a reset and
    // lost it is telling us something changed.
    assert!(same_period(None, None, week));
    assert!(!same_period(Some(RESET_A), None, week));
    assert!(!same_period(None, Some(RESET_A), week));
}

// ------------------------------------------------- rule 6: a period that has ended

#[test]
fn a_window_whose_reset_has_already_passed_never_fires() {
    // The machine T-WP10 came from: Codex last opened two days earlier, the newest rollout
    // log still parsing perfectly, the weekly window still reported at 70 % — for a week
    // that had ended the day before.
    let mut alerts = Alerts::in_memory();
    let expired = view_of(
        Window::stale(86.0)
            .with_window_minutes(10080)
            .with_resets_at("2026-09-07T11:00:00Z"),
        NOW,
    );

    assert!(
        alerts.evaluate(&expired, &rules()).is_empty(),
        "a crossing cannot happen inside a period that is over"
    );
    assert!(
        alerts.log().windows.is_empty(),
        "and nothing is written down about it"
    );
}

#[test]
fn a_reset_one_second_away_still_fires_and_one_second_gone_does_not() {
    // The boundary from both sides, with everything else held still.
    let just_alive = view_of(
        Window::ok(86.0)
            .with_window_minutes(10080)
            .with_resets_at("2026-09-07T12:00:01Z"),
        NOW,
    );
    assert_eq!(
        thresholds(&Alerts::in_memory().evaluate(&just_alive, &rules())),
        vec![85.0],
        "a second of window left is still a window"
    );

    let just_gone = view_of(
        Window::ok(86.0)
            .with_window_minutes(10080)
            .with_resets_at("2026-09-07T11:59:59Z"),
        NOW,
    );
    assert!(
        Alerts::in_memory()
            .evaluate(&just_gone, &rules())
            .is_empty(),
        "a second past the reset is a different period"
    );

    // Exactly on it: a window runs *up to* its reset, and zero remaining is not negative
    // remaining. The contract's own countdown column says the same thing.
    let exactly = view_of(
        Window::ok(86.0)
            .with_window_minutes(10080)
            .with_resets_at(NOW),
        NOW,
    );
    assert_eq!(
        thresholds(&Alerts::in_memory().evaluate(&exactly, &rules())),
        vec![85.0],
        "zero remaining is not negative remaining"
    );
}

#[test]
fn a_window_with_no_reset_at_all_is_not_treated_as_expired() {
    // "I do not know when this resets" is not "this has reset". A source that stopped
    // reporting `resets_at` must not silence every warning the tray has.
    let mut alerts = Alerts::in_memory();
    let no_reset = view_of(Window::ok(86.0).with_window_minutes(10080), NOW);
    assert_eq!(
        thresholds(&alerts.evaluate(&no_reset, &rules())),
        vec![85.0]
    );
}

#[test]
fn the_period_after_an_expired_one_starts_from_nothing() {
    // Rule 6 forgets as well as silences, so the first reading of the new period is a
    // first observation rather than a continuation of the old one.
    let mut alerts = Alerts::in_memory();

    let expired = view_of(
        Window::stale(86.0)
            .with_window_minutes(10080)
            .with_resets_at("2026-09-07T11:00:00Z"),
        NOW,
    );
    assert!(alerts.evaluate(&expired, &rules()).is_empty());

    // Codex is opened again: a new week, and real usage in it.
    assert_eq!(
        thresholds(&alerts.evaluate(&view_at(90.0, RESET_B, NOW), &rules())),
        vec![85.0],
        "the new period gets its warning"
    );
    assert!(
        alerts
            .evaluate(&view_at(90.0, RESET_B, NOW), &rules())
            .is_empty(),
        "and gets it once"
    );
}
