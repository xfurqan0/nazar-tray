//! The state model, checked by example and by property.
//!
//! The property tests use a hand-rolled generator rather than `proptest`. The rules here
//! are small enough that a shrinker would not earn its keep, and `proptest` would put ten
//! packages behind a crate whose whole dependency list is two lines. A failing case prints
//! its seed and a seed re-runs it exactly: the generator is integer arithmetic and nothing
//! else, so it produces the same sequence on every platform.

use std::collections::BTreeMap;

use super::*;
use crate::limits::{Provider, Source, Window};

/// xorshift64\*: four instructions, no dependency, the same sequence everywhere.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// A percentage with two decimals, in `0.00..=100.00`.
    fn percent(&mut self) -> f64 {
        self.below(10_001) as f64 / 100.0
    }
}

/// Window keys from both providers, so the generator exercises the real shapes.
const KEYS: [&str; 6] = [
    "five_hour",
    "seven_day",
    "seven_day_fable",
    "primary",
    "secondary",
    "seven_day_opus",
];

/// A random window set: one to six windows, some with no percentage at all.
fn window_set(rng: &mut Rng) -> BTreeMap<String, Window> {
    let count = 1 + rng.below(KEYS.len() as u64) as usize;
    let mut windows = BTreeMap::new();
    for _ in 0..count {
        let key = KEYS[rng.below(KEYS.len() as u64) as usize];
        // A quarter of windows are unknown. Far more than a real machine sees, on purpose:
        // the interesting cases are the ones nobody meets by accident.
        let window = if rng.below(4) == 0 {
            Window::error("nothing was read")
        } else {
            Window::ok(rng.percent())
        };
        let window = match rng.below(3) {
            0 => window.with_window_minutes(300),
            1 => window.with_window_minutes(10_080),
            _ => window,
        };
        windows.insert(key.to_owned(), window);
    }
    windows
}

fn rules() -> Rules {
    Rules::default()
}

// ---------------------------------------------------------------- binding selection

#[test]
fn the_binding_window_is_the_highest_percentage() {
    let mut windows = BTreeMap::new();
    windows.insert(
        "primary".to_owned(),
        Window::ok(54.0).with_window_minutes(300),
    );
    windows.insert(
        "secondary".to_owned(),
        Window::ok(70.0).with_window_minutes(10_080),
    );
    assert_eq!(binding(&windows).as_deref(), Some("secondary"));
}

#[test]
fn a_tie_goes_to_the_shorter_window() {
    let mut windows = BTreeMap::new();
    windows.insert(
        "primary".to_owned(),
        Window::ok(70.0).with_window_minutes(300),
    );
    windows.insert(
        "secondary".to_owned(),
        Window::ok(70.0).with_window_minutes(10_080),
    );
    assert_eq!(
        binding(&windows).as_deref(),
        Some("primary"),
        "equally full, the five-hour window is the one you hit first"
    );
}

#[test]
fn a_window_with_no_stated_length_loses_a_tie() {
    let mut windows = BTreeMap::new();
    windows.insert("a_unlabelled".to_owned(), Window::ok(70.0));
    windows.insert(
        "z_weekly".to_owned(),
        Window::ok(70.0).with_window_minutes(10_080),
    );
    assert_eq!(
        binding(&windows).as_deref(),
        Some("z_weekly"),
        "the window we know least about does not win a coin toss"
    );
}

#[test]
fn a_window_with_no_percentage_never_binds() {
    let mut windows = BTreeMap::new();
    windows.insert("five_hour".to_owned(), Window::error("no quota block"));
    windows.insert("seven_day".to_owned(), Window::ok(3.0));
    assert_eq!(binding(&windows).as_deref(), Some("seven_day"));
}

#[test]
fn a_provider_whose_windows_are_all_unknown_has_no_binding_window() {
    let mut windows = BTreeMap::new();
    windows.insert("five_hour".to_owned(), Window::error("no quota block"));
    windows.insert("seven_day".to_owned(), Window::error("no quota block"));
    assert_eq!(
        binding(&windows),
        None,
        "\"I do not know\" is not a candidate for \"the number that constrains you\""
    );
    assert!(binding(&BTreeMap::new()).is_none());
}

/// The property, over ten thousand random window sets.
#[test]
fn binding_picks_the_highest_then_the_shortest_then_the_first() {
    for seed in 1..10_001u64 {
        let windows = window_set(&mut Rng::new(seed));
        let chosen = binding(&windows);

        let known: Vec<(&String, &Window)> = windows
            .iter()
            .filter(|(_, window)| window.percent.is_some())
            .collect();

        let Some(key) = chosen else {
            assert!(
                known.is_empty(),
                "seed {seed}: no binding window was chosen but {} had a percentage",
                known.len()
            );
            continue;
        };

        let winner = &windows[&key];
        let percent = winner
            .percent
            .unwrap_or_else(|| panic!("seed {seed}: a window with no percentage was chosen"));
        let length = |window: &Window| window.window_minutes.unwrap_or(u32::MAX);

        for (other_key, other) in &known {
            let other_percent = other.percent.unwrap();
            assert!(
                percent >= other_percent,
                "seed {seed}: chose {key} at {percent} over {other_key} at {other_percent}"
            );
            if (other_percent - percent).abs() < f64::EPSILON && ***other_key != key {
                assert!(
                    length(winner) < length(other)
                        || (length(winner) == length(other) && key.as_str() < other_key.as_str()),
                    "seed {seed}: {key} and {other_key} are both at {percent}, and the tie \
                     was broken the wrong way"
                );
            }
        }
    }
}

#[test]
fn binding_gives_the_same_answer_every_time() {
    for seed in 1..2_001u64 {
        let windows = window_set(&mut Rng::new(seed));
        assert_eq!(binding(&windows), binding(&windows), "seed {seed}");
    }
}

// ---------------------------------------------------------------- severity

#[test]
fn the_severity_boundaries_are_where_the_plan_says() {
    let rules = Thresholds::default();
    assert_eq!(rules.severity(Some(0.0)), Severity::Ok);
    assert_eq!(rules.severity(Some(59.9)), Severity::Ok);
    assert_eq!(rules.severity(Some(60.0)), Severity::Warn);
    assert_eq!(rules.severity(Some(84.9)), Severity::Warn);
    assert_eq!(rules.severity(Some(85.0)), Severity::Critical);
    assert_eq!(rules.severity(Some(99.9)), Severity::Critical);
    assert_eq!(rules.severity(Some(100.0)), Severity::Exhausted);
    assert_eq!(rules.severity(Some(101.0)), Severity::Exhausted);
}

#[test]
fn a_window_with_no_percentage_is_unknown_and_not_ok() {
    let rules = Thresholds::default();
    assert_eq!(rules.severity(None), Severity::Unknown);
    assert_eq!(
        rules.severity(Some(f64::NAN)),
        Severity::Unknown,
        "a number that is not a number is not a reading either"
    );
    assert_ne!(
        rules.severity(None),
        rules.severity(Some(0.0)),
        "\"unknown\" and \"you have used nothing\" are opposite messages"
    );
    assert!(
        Severity::Unknown < Severity::Ok,
        "unknown must never win a max() against a real reading"
    );
}

#[test]
fn severity_never_falls_as_the_percentage_rises() {
    for seed in 1..2_001u64 {
        let mut rng = Rng::new(seed);
        let thresholds = Thresholds {
            warn: rng.percent(),
            critical: rng.percent(),
            exhausted: rng.percent(),
        };
        let mut previous = Severity::Ok;
        for step in 0..=1_000u64 {
            let percent = step as f64 / 10.0;
            let severity = thresholds.severity(Some(percent));
            assert!(
                severity >= previous,
                "seed {seed}: severity fell from {previous:?} to {severity:?} at {percent}"
            );
            previous = severity;
        }
    }
}

#[test]
fn custom_thresholds_are_honoured() {
    let strict = Thresholds {
        warn: 40.0,
        critical: 70.0,
        exhausted: 95.0,
    };
    assert_eq!(strict.severity(Some(39.9)), Severity::Ok);
    assert_eq!(strict.severity(Some(40.0)), Severity::Warn);
    assert_eq!(strict.severity(Some(95.0)), Severity::Exhausted);
}

// ---------------------------------------------------------------- freshness

#[test]
fn the_freshness_boundaries_are_five_and_forty_five_minutes() {
    let rules = FreshnessRules::default();
    assert_eq!(rules.classify(None), Freshness::Unknown);
    assert_eq!(rules.classify(Some(0)), Freshness::Fresh);
    assert_eq!(rules.classify(Some(5 * 60_000)), Freshness::Fresh);
    assert_eq!(rules.classify(Some(5 * 60_000 + 1)), Freshness::Aging);
    assert_eq!(rules.classify(Some(45 * 60_000)), Freshness::Aging);
    assert_eq!(rules.classify(Some(45 * 60_000 + 1)), Freshness::Stale);
    assert_eq!(rules.classify(Some(8 * 3_600_000)), Freshness::Stale);
}

#[test]
fn a_reading_from_the_future_is_fresh_rather_than_an_error() {
    assert_eq!(
        FreshnessRules::default().classify(Some(-30_000)),
        Freshness::Fresh
    );
}

#[test]
fn freshness_never_improves_as_a_reading_ages() {
    for seed in 1..1_001u64 {
        let mut rng = Rng::new(seed);
        let rules = FreshnessRules {
            fresh_minutes: rng.below(60) as u32,
            aging_minutes: rng.below(600) as u32,
        };
        let mut previous = Freshness::Fresh;
        for minutes in 0..720i64 {
            let freshness = rules.classify(Some(minutes * 60_000));
            assert!(
                freshness >= previous,
                "seed {seed}: freshness improved from {previous:?} to {freshness:?} at \
                 {minutes} minutes"
            );
            previous = freshness;
        }
    }
}

// ---------------------------------------------------------------- reset maths

/// A provider with one window resetting at `resets_at`, read at `source_at`.
fn one_window(resets_at: &str, source_at: &str) -> Provider {
    let mut windows = BTreeMap::new();
    windows.insert(
        "primary".to_owned(),
        Window::ok(54.0)
            .with_window_minutes(300)
            .with_resets_at(resets_at),
    );
    Provider {
        configured: true,
        source: Some(Source::Rollout),
        source_at: Some(source_at.to_owned()),
        binding: Some("primary".to_owned()),
        windows,
        ..Provider::default()
    }
}

fn remaining(resets_at: &str, now: &str) -> Option<i64> {
    let mut limits = Limits::new(now);
    limits.providers.codex = one_window(resets_at, now);
    let view = Snapshot::new(limits).view(now, &rules());
    view.providers
        .iter()
        .find(|provider| provider.name == "codex")?
        .windows
        .first()?
        .remaining_ms
}

#[test]
fn the_countdown_is_the_gap_between_two_instants() {
    assert_eq!(
        remaining("2026-09-07T12:24:52Z", "2026-09-07T11:24:52Z"),
        Some(3_600_000)
    );
    assert_eq!(
        remaining("2026-09-07T12:24:52Z", "2026-09-07T12:24:52Z"),
        Some(0)
    );
}

#[test]
fn a_reset_that_has_already_passed_counts_down_past_zero() {
    // Eight hours asleep, and the window reset while the machine was off. The retired
    // prototype printed "now" for ever here (audit scenario S7); the countdown says how far
    // past due it is and lets the panel decide what to draw.
    assert_eq!(
        remaining("2026-09-07T03:17:24Z", "2026-09-07T11:17:24Z"),
        Some(-28_800_000)
    );
}

/// The same instant, written six ways. Every one has to produce the same countdown.
#[test]
fn the_countdown_does_not_depend_on_how_the_instant_was_spelled() {
    // 2026-09-07T12:24:52Z is 15:24:52 in Istanbul (UTC+3, no daylight saving since 2016)
    // and 08:24:52 in New York on that date (EDT, UTC−4).
    let spellings = [
        "2026-09-07T12:24:52Z",
        "2026-09-07T12:24:52z",
        "2026-09-07T12:24:52.000Z",
        "2026-09-07T12:24:52+00:00",
        "2026-09-07T15:24:52+03:00",
        "2026-09-07T08:24:52-04:00",
    ];
    let expected = remaining(spellings[0], "2026-09-07T11:24:52Z").unwrap();
    assert_eq!(expected, 3_600_000);

    for spelling in spellings {
        for now in [
            "2026-09-07T11:24:52Z",
            "2026-09-07T14:24:52+03:00",
            "2026-09-07T07:24:52-04:00",
        ] {
            assert_eq!(
                remaining(spelling, now),
                Some(expected),
                "{spelling} counted down differently when now was written {now}"
            );
        }
    }
}

/// The two daylight-saving boundaries a European and an American user actually cross.
///
/// The point is that **nothing happens**: the countdown is the gap between two UTC instants,
/// so an hour the local calendar repeats or skips does not exist here. A consumer that
/// rendered local times without converting would get these wrong, which is why the contract
/// stores UTC and says so.
#[test]
fn daylight_saving_transitions_do_not_move_a_countdown() {
    // America/New_York, 2026-11-01: at 06:00 UTC the local clock goes back from 02:00 EDT
    // to 01:00 EST. A window resetting later is exactly as far away as the gap says.
    assert_eq!(
        remaining("2026-11-01T06:30:00Z", "2026-11-01T05:30:00Z"),
        Some(3_600_000)
    );
    assert_eq!(
        remaining("2026-11-01T06:30:00Z", "2026-11-01T06:00:00Z"),
        Some(1_800_000),
        "the repeated local hour is one instant here, and one gap"
    );
    // America/New_York, 2026-03-08: at 07:00 UTC the local clock jumps from 02:00 EST to
    // 03:00 EDT. The hour that does not exist locally is an ordinary hour of UTC.
    assert_eq!(
        remaining("2026-03-08T08:00:00Z", "2026-03-08T06:00:00Z"),
        Some(7_200_000)
    );
    // Europe/Istanbul is UTC+3 all year, which is exactly why the same arithmetic works for
    // it: the maintainer's own machine crosses no boundary, and this test would still catch
    // a change that made the answer depend on where it ran.
    assert_eq!(
        remaining("2026-03-29T04:00:00Z", "2026-03-29T00:00:00Z"),
        Some(14_400_000)
    );
    assert_eq!(
        remaining("2026-10-25T04:00:00Z", "2026-10-25T00:00:00Z"),
        Some(14_400_000)
    );
}

/// The property, over random instants and every offset a source could write.
#[test]
fn a_countdown_is_the_same_number_in_every_time_zone() {
    for seed in 1..3_001u64 {
        let mut rng = Rng::new(seed);
        // Instants across six years either side of the project's own dates.
        let now_seconds = 1_600_000_000 + rng.below(200_000_000) as i64;
        let gap = rng.below(1_209_601) as i64 - 604_800; // ±7 days
        let resets_seconds = now_seconds + gap;

        let now = crate::timefmt::rfc3339_from_unix_seconds(now_seconds);
        let utc = crate::timefmt::rfc3339_from_unix_seconds(resets_seconds);
        assert_eq!(remaining(&utc, &now), Some(gap * 1000), "seed {seed}");

        // The same instant with an offset. Whole hours only: that is what real zones use,
        // and the half-hour ones are covered by the fixed cases above.
        for offset_hours in [-11i64, -4, 0, 3, 5, 13] {
            let shifted =
                crate::timefmt::rfc3339_from_unix_seconds(resets_seconds + offset_hours * 3600);
            let sign = if offset_hours < 0 { '-' } else { '+' };
            let with_offset = format!(
                "{}{sign}{:02}:00",
                shifted.trim_end_matches('Z'),
                offset_hours.abs()
            );
            assert_eq!(
                remaining(&with_offset, &now),
                Some(gap * 1000),
                "seed {seed}: {with_offset} is the same instant as {utc}"
            );
        }
    }
}

#[test]
fn a_timestamp_that_is_not_one_leaves_the_countdown_unknown() {
    assert_eq!(remaining("whenever you like", "2026-09-07T11:24:52Z"), None);
    assert_eq!(remaining("2026-09-07T12:24:52Z", "not a time"), None);
}

// ---------------------------------------------------------------- the whole view

fn sample() -> Limits {
    Limits::from_json(include_str!("../../../../fixtures/limits.sample.json"))
        .expect("the shipped sample must parse")
}

#[test]
fn the_view_derives_the_sample_document() {
    let now = "2026-09-06T21:20:00Z";
    let view = Snapshot::new(sample()).view(now, &rules());

    assert_eq!(view.updated_at, "2026-09-06T21:12:34Z");
    assert_eq!(view.now, now);
    assert_eq!(view.providers.len(), 2);

    let claude = &view.providers[0];
    assert_eq!(claude.name, "claude");
    assert!(claude.configured);
    assert_eq!(claude.plan.as_deref(), Some("max_20x"));
    assert_eq!(claude.source.as_deref(), Some("endpoint"));
    assert_eq!(claude.binding.as_deref(), Some("seven_day_fable"));
    assert_eq!(claude.severity, Severity::Ok);
    // sourceAt 21:12:30, now 21:20:00 — seven and a half minutes.
    assert_eq!(claude.age_ms, Some(450_000));
    assert_eq!(claude.freshness, Freshness::Aging);

    // Shortest window first, whatever the map's own order.
    assert_eq!(
        claude
            .windows
            .iter()
            .map(|window| window.key.as_str())
            .collect::<Vec<_>>(),
        ["five_hour", "seven_day", "seven_day_fable"]
    );
    let binding = claude.windows.iter().find(|window| window.binding).unwrap();
    assert_eq!(binding.key, "seven_day_fable");
    assert_eq!(binding.percent, Some(23.0));
    assert_eq!(binding.model.as_deref(), Some("Fable"));
    assert!(binding.detailed);

    let codex = &view.providers[1];
    assert_eq!(codex.name, "codex");
    assert_eq!(codex.binding.as_deref(), Some("secondary"));
    assert_eq!(
        codex.severity,
        Severity::Warn,
        "70 % is over the warning line"
    );
    assert_eq!(view.severity(), Severity::Warn);
}

#[test]
fn an_unconfigured_provider_derives_to_unknown_and_nothing_else() {
    let view = Snapshot::empty("2026-09-07T10:00:00Z").view("2026-09-07T10:00:00Z", &rules());
    for provider in &view.providers {
        assert!(!provider.configured);
        assert!(provider.windows.is_empty());
        assert_eq!(provider.binding, None);
        assert_eq!(provider.age_ms, None);
        assert_eq!(provider.freshness, Freshness::Unknown);
        assert_eq!(provider.severity, Severity::Unknown);
    }
    assert_eq!(view.severity(), Severity::Unknown);
}

#[test]
fn the_view_recomputes_a_binding_window_the_file_got_wrong() {
    let mut limits = sample();
    limits.providers.codex.binding = Some("primary".to_owned());
    let view = Snapshot::new(limits).view("2026-09-06T21:20:00Z", &rules());
    assert_eq!(
        view.providers[1].binding.as_deref(),
        Some("secondary"),
        "a document written by hand does not get to decide which window binds"
    );
}

#[test]
fn a_window_state_a_newer_writer_invented_survives_the_view() {
    let mut limits = Limits::new("2026-09-07T10:00:00Z");
    let mut windows = BTreeMap::new();
    let mut window = Window::ok(12.0);
    window.state = WindowState::Other("degraded".to_owned());
    windows.insert("five_hour".to_owned(), window);
    limits.providers.claude = Provider {
        configured: true,
        windows,
        ..Provider::default()
    };

    let view = Snapshot::new(limits).view("2026-09-07T10:00:00Z", &rules());
    assert_eq!(view.providers[0].windows[0].state, "degraded");
}

#[test]
fn nothing_derived_is_stored_back_into_the_document() {
    let snapshot = Snapshot::new(sample());
    let before = snapshot.limits().to_json().unwrap();
    let _ = snapshot.view("2026-09-06T21:20:00Z", &rules());
    let _ = snapshot.view("2027-01-01T00:00:00Z", &rules());
    assert_eq!(
        snapshot.limits().to_json().unwrap(),
        before,
        "deriving a view must not change the document it was derived from"
    );
}

#[test]
fn the_view_is_json_the_panel_can_read() {
    let view = Snapshot::new(sample()).view("2026-09-06T21:20:00Z", &rules());
    let text = serde_json::to_string(&view).unwrap();
    for key in [
        "\"updatedAt\"",
        "\"providers\"",
        "\"remainingMs\"",
        "\"windowMinutes\"",
        "\"freshness\"",
        "\"severity\"",
        "\"binding\"",
    ] {
        assert!(text.contains(key), "the view is missing {key}: {text}");
    }

    let round_tripped: SnapshotView = serde_json::from_str(&text).unwrap();
    assert_eq!(round_tripped, view);
}
