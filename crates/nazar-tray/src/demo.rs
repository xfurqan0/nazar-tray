//! Synthetic numbers for `--demo`.
//!
//! Screenshots must not depend on what happens to be true on the machine that takes them,
//! and the states worth photographing are the ones that are hard to get right rather than
//! the ones that look good. So the demo document carries, deliberately:
//!
//! * a window comfortably inside its quota, and one **over the amber threshold**;
//! * one **over the red threshold**, which is also **model-scoped and detailed** — the
//!   window the maintainer's own account is constrained by, and the reason WP2b exists;
//! * one that **could not be read at all**, which must show the word and never a `0` bar;
//! * two providers with different ages, so the freshness line and the desaturated icon
//!   both appear in the same picture.
//!
//! Nothing here is read from disk and nothing here is written to it: `--demo` never takes
//! the writer's lock, so a screenshot session cannot overwrite the real `~/.nazar/limits.json`.
//!
//! # The usage history is synthetic too, and that is a privacy rule rather than a nicety
//!
//! Until 0.2.0 the quota numbers on a screenshot run were invented and the **usage** numbers
//! were the maintainer's own: `--demo` had no usage document, so [`crate::usage::get_usage`]
//! scanned the real transcripts and the tray tooltip read the real store. Every picture of the
//! usage view would have carried a month of somebody's real model use — which models, how
//! much, on which days — into a public repository.
//!
//! So [`usage`] is a store of its own: five weeks of hourly buckets, four model ids, both
//! providers, and a couple of days at the far end that only Claude Code's statistics cache
//! reaches. `crate::usage::store_dir` returns `None` for a demo run, which is where the
//! guarantee is enforced rather than merely intended — there is no directory for a demo run
//! to open, so nothing on that path can reach `%APPDATA%\nazar\usage` however much it wants
//! to.

use std::collections::BTreeMap;

use nazar_core::state::Snapshot;
use nazar_core::timefmt::{rfc3339_from_unix_seconds, unix_seconds_from_rfc3339};
use nazar_core::usage::{Bucket, Hours, PROVIDER, PROVIDER_CODEX, Raw};
use nazar_core::{Limits, Provider, Source, Window};

/// The demo document, stamped relative to `now` so the countdowns run like real ones.
#[must_use]
pub fn snapshot(now: &str) -> Snapshot {
    let seconds = unix_seconds_from_rfc3339(now).unwrap_or(0);
    let at = |offset: i64| rfc3339_from_unix_seconds(seconds + offset);

    let mut limits = Limits::new(now.to_owned());

    let mut claude = Provider {
        configured: true,
        plan: Some("max_20x".to_owned()),
        source: Some(Source::Endpoint),
        source_at: Some(at(-12)),
        ..Provider::default()
    };
    claude.windows.insert(
        "five_hour".to_owned(),
        Window::ok(12.0)
            .with_window_minutes(300)
            .with_resets_at(at(2 * 3600 + 10 * 60)),
    );
    claude.windows.insert(
        "seven_day".to_owned(),
        Window::ok(63.4)
            .with_window_minutes(10080)
            .with_resets_at(at(4 * 86400 + 3 * 3600)),
    );
    claude.windows.insert(
        "seven_day_fable".to_owned(),
        Window::ok(88.0)
            .with_window_minutes(10080)
            .with_resets_at(at(4 * 86400 + 3 * 3600))
            .with_model("Fable"),
    );

    let mut codex = Provider {
        configured: true,
        plan: Some("plus".to_owned()),
        source: Some(Source::Rollout),
        source_at: Some(at(-70 * 60)),
        ..Provider::default()
    };
    codex.windows.insert(
        "primary".to_owned(),
        Window::error("no quota line in the newest session log").with_window_minutes(300),
    );
    codex.windows.insert(
        "secondary".to_owned(),
        Window::stale(70.0)
            .with_window_minutes(10080)
            .with_resets_at(at(11 * 3600 + 26 * 60)),
    );

    limits.providers.claude = claude;
    limits.providers.codex = codex;
    Snapshot::new(limits)
}

// ------------------------------------------------------------------ usage history

/// How far back the demo store reaches, in whole days including today.
///
/// Five weeks, because the calendar the **All** tab draws is columns of seven and a history
/// shorter than three or four of them photographs as a single stub. It is also enough for the
/// **Weeks** list to be a list — five rows, one of them the part week being lived in — rather
/// than a single row with a full-length bar beside it.
pub const USAGE_DAYS: i64 = 35;

/// The days at the far end that no transcript survives for.
///
/// Claude Code prunes transcripts and keeps a statistics cache that reaches further back, so
/// the two oldest days of this history are days the *store* never measured: one total per
/// model, no breakdown, drawn as an outline rather than a shade. Turning the setting that
/// fills them off does not remove them from this fixture — the point of the picture is the
/// distinction, and a demo store has nothing else to be honest about.
pub const USAGE_REPORTED_DAYS: i64 = 2;

/// The hour of UTC the demo's working day starts at, and the hour after it ends.
///
/// A band rather than a full day, so the calendar has quiet cells as well as busy ones and a
/// day's detail is a plausible working day instead of a flat twenty-four hours.
const WORKING_HOURS: std::ops::Range<i64> = 6..21;

/// Every model id the usage fixture spends a token under.
///
/// Four — three for Claude Code and one for Codex — because the **Models** chart draws one
/// line each and a legend that names one line proves nothing, and because a demo with a single
/// provider would not show the heading a day's detail grows when both worked in it. They are
/// spelled the way a provider spells them: a model id is data, and the panel prints it exactly
/// as it arrives.
///
/// Public so that `crate::usage`'s tests can say *these four and nothing else*, which is how a
/// real bucket reaching a demo answer would be caught.
pub const MODELS: [&str; 4] = [
    "claude-fable-5-20260514",
    "claude-sonnet-5-20260901",
    "claude-haiku-4-5-20251001",
    "gpt-5.6-sol",
];

/// The Claude half of [`MODELS`], busiest first.
const CLAUDE_MODELS: [&str; 3] = [MODELS[0], MODELS[1], MODELS[2]];

/// The Codex half of [`MODELS`].
const CODEX_MODEL: &str = MODELS[3];

/// A synthetic usage store: hourly buckets, both providers, and the days behind them.
///
/// The same shape [`crate::usage::UsageResponse`] carries, because that is what it becomes:
/// `crate::usage::demo_answer` cuts it to the window the panel asked for and hands it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    /// Provider to UTC hour (`YYYY-MM-DDTHH`) to model to its counters.
    pub providers: BTreeMap<String, Hours>,
    /// Local day (`YYYY-MM-DD`) to model to the one total another program reported for it.
    pub reported: BTreeMap<String, BTreeMap<String, u64>>,
    /// The earliest instant the store holds a measured bucket for.
    pub since: String,
    /// When this store was last written, which for a demo run is two minutes ago.
    pub scanned_at: String,
}

/// The demo store, stamped relative to `now` so the history always ends today.
///
/// `now` is unix seconds. Every number below is a function of the day, the hour and the model,
/// so two screenshots taken a minute apart are the same picture and a reviewer can tell a
/// layout change from a data change.
#[must_use]
pub fn usage(now: i64) -> Usage {
    let today = now.div_euclid(86_400);
    let measured_days = USAGE_DAYS - USAGE_REPORTED_DAYS;

    let mut claude = Hours::new();
    let mut codex = Hours::new();
    for back in 0..measured_days {
        let day = today - back;
        for hour in WORKING_HOURS {
            let at = day * 86_400 + hour * 3_600;
            if at > now {
                continue;
            }
            let key = hour_key(at);
            for (index, model) in CLAUDE_MODELS.iter().enumerate() {
                if let Some(bucket) = claude_bucket(day, hour, index) {
                    claude
                        .entry(key.clone())
                        .or_default()
                        .insert((*model).to_owned(), bucket);
                }
            }
            if let Some(bucket) = codex_bucket(day, hour) {
                codex
                    .entry(key.clone())
                    .or_default()
                    .insert(CODEX_MODEL.to_owned(), bucket);
            }
        }
    }

    let mut reported = BTreeMap::new();
    for back in measured_days..USAGE_DAYS {
        let day = today - back;
        let mut models = BTreeMap::new();
        for (index, model) in CLAUDE_MODELS.iter().take(2).enumerate() {
            let total = 120_000_000 + spread(day, index as i64, 7) % 640_000_000;
            models.insert((*model).to_owned(), total);
        }
        reported.insert(day_key(day * 86_400), models);
    }

    Usage {
        providers: BTreeMap::from([
            (PROVIDER.to_owned(), claude),
            (PROVIDER_CODEX.to_owned(), codex),
        ]),
        reported,
        since: rfc3339_from_unix_seconds(
            (today - measured_days + 1) * 86_400 + WORKING_HOURS.start * 3_600,
        ),
        // Two minutes, so the panel's *scanned 2 m ago* is a sentence rather than *just now*.
        scanned_at: rfc3339_from_unix_seconds(now - 132),
    }
}

/// The UTC hour a bucket is filed under: `YYYY-MM-DDTHH`, thirteen characters.
fn hour_key(at: i64) -> String {
    rfc3339_from_unix_seconds(at)[..13].to_owned()
}

/// The UTC date an instant falls in: `YYYY-MM-DD`.
fn day_key(at: i64) -> String {
    rfc3339_from_unix_seconds(at)[..10].to_owned()
}

/// Monday is 0. 1970-01-01 was a Thursday, which is why the offset is three.
fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

/// A repeatable number from three small ones.
///
/// **Not randomness.** The same day, hour and model produce the same figure on every run, so
/// a screenshot is reproducible and a diff in `docs/screenshots` means the panel changed. The
/// mixing is one round of the constants `SplitMix64` uses; the only property asked of it is
/// that neighbouring hours do not come out as a sawtooth.
fn spread(day: i64, hour: i64, salt: i64) -> u64 {
    let mut value = (day as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (hour as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (salt as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    value ^= value >> 33;
    value = value.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    value ^= value >> 29;
    value
}

/// How hard one day was worked, as a percentage of an ordinary one.
///
/// **A fortnight of identical days is a picture of the layout, not of the view.** Without
/// this the law of large numbers flattens everything: twenty-five buckets a day average out,
/// the calendar comes out one shade, the week strip comes out one height, and the weeks list
/// comes out four rows with the same bar. Eight steps, so the busiest day is about seven
/// times the quietest and all four shades of the heat-map's scale are used.
fn intensity(day: i64) -> u64 {
    const STEPS: [u64; 8] = [24, 38, 52, 68, 85, 100, 135, 175];
    STEPS[(spread(day, 0, 3) % STEPS.len() as u64) as usize]
}

/// One model's hour of Claude Code, or `None` for an hour it did not work.
///
/// **Cache reads dominate**, which is the one thing about these numbers that is a measurement
/// rather than an invention: they were 98.5 % of the raw total over six days of the
/// maintainer's real work, and a demo whose four counters were the same size would make the
/// breakdown look like a decoration. `raw` is about 1.7× the deduplicated reading, the factor
/// measured on the same machine, so the *Count like Claude Code* switch moves the numbers by
/// as much here as it does on a real one.
fn claude_bucket(day: i64, hour: i64, model: usize) -> Option<Bucket> {
    // The busiest model works most hours, the smallest one a few; the weekend is a quarter of
    // a weekday, and never nothing, because a fortnight of blank Sundays is not a history.
    let chance = [72, 46, 18][model.min(2)] / if weekday(day) >= 5 { 4 } else { 1 };
    let roll = spread(day, hour, model as i64);
    if roll % 100 >= chance {
        return None;
    }

    // A percentage of the busiest model's spend, times how hard the day itself was worked.
    // Integer arithmetic throughout: these numbers are counters, and a float that rounded
    // differently on another machine would make the screenshots irreproducible.
    let weight = [100, 55, 18][model.min(2)] * intensity(day);
    let of =
        |base: u64, span: u64, salt: i64| (base + spread(day, hour, salt) % span) * weight / 10_000;
    let input = of(4_000, 26_000, 11);
    let output = of(900, 7_400, 12);
    let cache_create = of(9_000, 120_000, 13);
    let cache_read = of(1_800_000, 9_400_000, 14);
    Some(Bucket {
        input,
        output,
        cache_create,
        cache_read,
        requests: 1 + roll % 14,
        raw: Some(Raw {
            input: input * 17 / 10,
            output: output * 17 / 10,
            cache_create: cache_create * 17 / 10,
            cache_read: cache_read * 17 / 10,
        }),
        ..Bucket::default()
    })
}

/// One hour of Codex, or `None` for an hour it did not work.
///
/// **No `raw`**, and that is the documented truth rather than a shortcut: Codex writes each
/// `token_count` event once, so there are no copies to collapse and its per-line sum *is* its
/// five counters. A bucket with no `raw` says exactly that.
fn codex_bucket(day: i64, hour: i64) -> Option<Bucket> {
    let chance = 34 / if weekday(day) >= 5 { 4 } else { 1 };
    let roll = spread(day, hour, 21);
    if roll % 100 >= chance {
        return None;
    }
    let weight = intensity(day);
    let of =
        |base: u64, span: u64, salt: i64| (base + spread(day, hour, salt) % span) * weight / 100;
    Some(Bucket {
        input: of(3_000, 18_000, 22),
        output: of(700, 5_200, 23),
        cache_create: 0,
        cache_read: of(90_000, 1_500_000, 24),
        requests: 1 + roll % 7,
        raw: None,
        ..Bucket::default()
    })
}

/// How long each step of [`cross_sequence`] stays on screen.
///
/// Long enough to read a toast and watch the icon change colour, short enough that the whole
/// run is over in under twenty seconds.
pub const CROSS_STEP: std::time::Duration = std::time::Duration::from_secs(4);

/// The acceptance run for the notifications: `80 → 86 → 86 → reset → 86`.
///
/// One provider, one weekly window, four readings, and the toasts they produce are the test
/// `docs/PROJECT.md` WP5 asks for:
///
/// | Step | Reading | What should appear |
/// |---|---|---|
/// | 1 | 80 %, week A | one toast, **60 %** — arriving above a threshold is worth saying once |
/// | 2 | 86 %, week A | one toast, **85 %** — the crossing |
/// | 3 | 86 %, week A | **nothing**: still above is not crossing again |
/// | 4 | 86 %, week B | one toast, **85 %** — a new reset is a new week |
///
/// Codex rather than Claude because its weekly window is the one anybody can see on any
/// machine, and one provider rather than two because the point of the run is to count toasts.
#[must_use]
pub fn cross_sequence(now: &str) -> Vec<Snapshot> {
    let seconds = unix_seconds_from_rfc3339(now).unwrap_or(0);
    let at = |offset: i64| rfc3339_from_unix_seconds(seconds + offset);
    let week_a = at(11 * 3600 + 26 * 60);
    let week_b = at(7 * 86400 + 11 * 3600 + 26 * 60);

    [
        (80.0, &week_a),
        (86.0, &week_a),
        (86.0, &week_a),
        (86.0, &week_b),
    ]
    .into_iter()
    .map(|(percent, resets_at)| {
        let mut limits = Limits::new(now.to_owned());
        let mut codex = Provider {
            configured: true,
            plan: Some("plus".to_owned()),
            source: Some(Source::Rollout),
            source_at: Some(at(-12)),
            ..Provider::default()
        };
        codex.windows.insert(
            "primary".to_owned(),
            Window::ok(21.0)
                .with_window_minutes(300)
                .with_resets_at(at(2 * 3600)),
        );
        codex.windows.insert(
            "secondary".to_owned(),
            Window::ok(percent)
                .with_window_minutes(10080)
                .with_resets_at(resets_at.clone()),
        );
        limits.providers.codex = codex;
        Snapshot::new(limits)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nazar_core::alerts::{AlertRules, Alerts};
    use nazar_core::state::{Freshness, Rules, Severity};

    #[test]
    fn the_demo_shows_the_states_that_are_hard_to_get_right() {
        let now = "2026-09-07T12:00:00Z";
        let view = snapshot(now).view(now, &Rules::default());

        let severities: Vec<Severity> = view
            .providers
            .iter()
            .flat_map(|provider| provider.windows.iter().map(|window| window.severity))
            .collect();
        for wanted in [
            Severity::Ok,
            Severity::Warn,
            Severity::Critical,
            Severity::Unknown,
        ] {
            assert!(
                severities.contains(&wanted),
                "the demo must photograph {wanted:?}; it has {severities:?}"
            );
        }

        let claude = &view.providers[0];
        assert_eq!(claude.binding.as_deref(), Some("seven_day_fable"));
        assert_eq!(claude.freshness, Freshness::Fresh);
        let detailed = claude
            .windows
            .iter()
            .find(|window| window.detailed)
            .expect("a model-scoped window");
        assert_eq!(detailed.model.as_deref(), Some("Fable"));

        let codex = &view.providers[1];
        assert_eq!(
            codex.freshness,
            Freshness::Stale,
            "one provider must be old"
        );
        let unknown = codex
            .windows
            .iter()
            .find(|window| window.state == "error")
            .expect("a window nobody could read");
        assert_eq!(
            unknown.percent, None,
            "a window in error carries no percentage — that is the whole point of it"
        );
        assert!(unknown.error.is_some());
    }

    #[test]
    fn the_crossing_run_produces_exactly_the_toasts_the_acceptance_criterion_names() {
        // The same assertion the core's own test makes, made here against the sequence the
        // live run actually shows — so that a change to the demo numbers that quietly broke
        // the acceptance run would fail the build rather than the maintainer's evening.
        let now = "2026-09-07T12:00:00Z";
        let rules = AlertRules::default();
        let mut alerts = Alerts::in_memory();

        let fired: Vec<Vec<f64>> = cross_sequence(now)
            .iter()
            .map(|snapshot| {
                alerts
                    .evaluate(&snapshot.view(now, &Rules::default()), &rules)
                    .into_iter()
                    .map(|alert| alert.threshold)
                    .collect()
            })
            .collect();

        assert_eq!(
            fired,
            vec![vec![60.0], vec![85.0], vec![], vec![85.0]],
            "one 60 % toast on arrival, one 85 % toast for the crossing, silence, and one \
             more 85 % toast after the reset"
        );
        assert_eq!(CROSS_STEP.as_secs(), 4);
    }

    #[test]
    fn the_crossing_run_only_moves_the_window_it_is_about() {
        let now = "2026-09-07T12:00:00Z";
        let steps = cross_sequence(now);
        assert_eq!(steps.len(), 4);

        let five_hour: Vec<Option<f64>> = steps
            .iter()
            .map(|snapshot| snapshot.providers().codex.windows["primary"].percent)
            .collect();
        assert_eq!(
            five_hour,
            vec![Some(21.0); 4],
            "the five-hour window is scenery: a second window crossing at the same time \
             would make the toasts impossible to count"
        );

        let resets: Vec<_> = steps
            .iter()
            .map(|snapshot| {
                snapshot.providers().codex.windows["secondary"]
                    .resets_at
                    .clone()
            })
            .collect();
        assert_eq!(
            resets[0], resets[2],
            "the first three readings are one week"
        );
        assert_ne!(resets[2], resets[3], "and the fourth is the next one");
    }

    #[test]
    fn the_countdowns_run_forwards() {
        let now = "2026-09-07T12:00:00Z";
        let view = snapshot(now).view(now, &Rules::default());
        for provider in &view.providers {
            for window in &provider.windows {
                if let Some(remaining) = window.remaining_ms {
                    assert!(
                        remaining > 0,
                        "{}.{} resets in the past",
                        provider.name,
                        window.key
                    );
                }
            }
        }
    }

    /// The instant every usage test below reads the fixture at.
    const USAGE_NOW: &str = "2026-09-13T18:00:00Z";

    fn usage_at(now: &str) -> Usage {
        usage(unix_seconds_from_rfc3339(now).expect("a readable instant"))
    }

    #[test]
    fn the_usage_fixture_shows_the_states_the_view_was_built_for() {
        let store = usage_at(USAGE_NOW);

        let claude = &store.providers[PROVIDER];
        let codex = &store.providers[PROVIDER_CODEX];
        assert!(
            !claude.is_empty() && !codex.is_empty(),
            "both providers work"
        );

        let models: std::collections::BTreeSet<&str> = store
            .providers
            .values()
            .flat_map(|hours| hours.values())
            .flat_map(|models| models.keys().map(String::as_str))
            .collect();
        assert_eq!(
            models,
            MODELS.into_iter().collect(),
            "every model id in the fixture is a declared one, and every declared one is used"
        );

        let days: std::collections::BTreeSet<&str> = claude
            .keys()
            .chain(codex.keys())
            .map(|hour| &hour[..10])
            .collect();
        assert!(
            days.len() >= (USAGE_DAYS - USAGE_REPORTED_DAYS - 1) as usize,
            "five weeks of measured days, not a handful: {}",
            days.len()
        );
        assert_eq!(
            store.reported.len(),
            USAGE_REPORTED_DAYS as usize,
            "the days no transcript survives for are reported rather than measured"
        );
        for day in store.reported.keys() {
            assert!(
                !days.contains(day.as_str()),
                "{day} is both measured and reported; a reported day has to be older than \
                 every transcript, or the picture stops being about the distinction"
            );
        }
    }

    #[test]
    fn the_fixture_is_the_same_picture_every_time_it_is_taken() {
        assert_eq!(
            usage_at(USAGE_NOW),
            usage_at(USAGE_NOW),
            "a screenshot has to be reproducible: a diff in docs/screenshots must mean the \
             panel changed, never that the numbers rolled again"
        );
        assert_ne!(
            usage_at(USAGE_NOW).providers,
            usage_at("2026-09-12T18:00:00Z").providers,
            "and it still moves with the day, so the history always ends today"
        );
    }

    #[test]
    fn the_cache_dominates_and_the_per_line_count_is_the_larger_one() {
        let store = usage_at(USAGE_NOW);

        let mut four = 0u64;
        let mut cache_read = 0u64;
        let mut raw = 0u64;
        for hours in store.providers.values() {
            for models in hours.values() {
                for bucket in models.values() {
                    four += bucket.input + bucket.output + bucket.cache_create + bucket.cache_read;
                    cache_read += bucket.cache_read;
                    raw += bucket.raw_counters().total();
                }
            }
        }
        assert!(
            cache_read * 100 / four >= 90,
            "cache reads were 98.5 % of a real machine's raw total; a fixture whose four \
             counters were the same size would make the breakdown look like decoration"
        );
        assert!(
            raw > four,
            "every line counted has to come to more than every message counted"
        );

        // Codex writes each event once, so its per-line sum *is* its five counters, and the
        // way to say that is an absent `raw` rather than a copy of them.
        for models in store.providers[PROVIDER_CODEX].values() {
            for bucket in models.values() {
                assert!(bucket.raw.is_none(), "Codex has no copies to collapse");
            }
        }
    }
}
