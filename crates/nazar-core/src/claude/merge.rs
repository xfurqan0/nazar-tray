//! Which number wins when both paths have one.
//!
//! With the detailed-windows mode on there are two sources for the same provider, and they
//! do not overlap neatly:
//!
//! | | `five_hour` | `seven_day` | `seven_day_<model>` |
//! |---|---|---|---|
//! | status line (passive) | yes | yes | **no** |
//! | usage endpoint (opt-in) | yes | yes | yes |
//!
//! The endpoint is the reason the mode exists, so it lays down the block. But the status
//! line is **live**: it is rewritten every few seconds while a session is open, while the
//! endpoint is asked on a timer and then backed off from. So a passive reading that is
//! newer than the endpoint's replaces the two windows they share, and the model-scoped
//! ones — which the passive path cannot produce — stay as they were.
//!
//! ```text
//!  endpoint reading                        passive capture
//!  five_hour        23 %  ◀── replaced by ── five_hour   25 %   (newer capture)
//!  seven_day        18 %  ◀── replaced by ── seven_day   18 %
//!  seven_day_fable  31 %      kept: nothing else has it
//!                     │
//!                     ▼
//!            binding = seven_day_fable, the highest of the three
//! ```
//!
//! Written down as a table in `docs/limits-contract.md` as well, because a consumer that
//! sees `source: "endpoint"` next to a window without `detailed: true` should be able to
//! look up why.

use crate::claude::binding;
use crate::claude::detailed::{FRESH_FOR, Reading};
use crate::claude::{WINDOW_FIVE_HOUR, WINDOW_SEVEN_DAY};
use crate::limits::{Provider, Source, Window, WindowState};
use crate::timefmt::seconds_between;

/// The two windows both sources can produce.
const SHARED_WINDOWS: [&str; 2] = [WINDOW_FIVE_HOUR, WINDOW_SEVEN_DAY];

/// Combine the passive provider block with an endpoint reading.
///
/// `passive` is what `ClaudeReader::refresh` produced. `detailed` is `None` when the mode
/// is off, or on but never yet answered — in which case the passive block is returned
/// exactly as it came, which is the fallback the whole design rests on.
///
/// `now` is RFC 3339 in UTC and decides one thing only: whether the endpoint reading is
/// old enough to be called stale.
#[must_use]
pub fn merge(passive: Provider, detailed: Option<&Reading>, now: &str) -> Provider {
    let Some(reading) = detailed else {
        return passive;
    };
    if reading.windows.is_empty() {
        return passive;
    }

    let (state, reason) = freshness(reading, now);
    let mut windows = reading.windows.clone();
    for window in windows.values_mut() {
        if state != WindowState::Ok {
            window.state = state.clone();
            window.error.clone_from(&reason);
        }
    }

    // The status line wins on the two windows it also reports, when its capture is newer
    // than the fetch. A capture with no percentage in it never replaces one that has one:
    // "I could not read it" is not fresher information than a number.
    let passive_is_newer = passive.source_at.as_deref().is_some_and(|captured_at| {
        seconds_between(&reading.fetched_at, captured_at).is_some_and(|gap| gap > 0)
    });
    let mut used_passive = false;
    if passive_is_newer {
        for key in SHARED_WINDOWS {
            let Some(live) = passive.windows.get(key) else {
                continue;
            };
            if live.percent.is_none() {
                continue;
            }
            windows.insert(key.to_owned(), live.clone());
            used_passive = true;
        }
    }

    Provider {
        // The endpoint answering is proof enough that this machine has Claude Code on it,
        // even on one where the status-line wrapper was never installed.
        configured: true,
        plan: reading.plan.clone().or_else(|| passive.plan.clone()),
        source: Some(Source::Endpoint),
        source_at: Some(newest_source(&reading.fetched_at, &passive, used_passive)),
        binding: binding(&windows),
        windows,
        extra: passive.extra,
    }
}

/// Whether the endpoint's numbers still count as current, and why not.
///
/// Two ways to be old: the reading was remembered rather than fetched (the endpoint said
/// no, and this is what it said last time), or it was fetched but a while ago.
fn freshness(reading: &Reading, now: &str) -> (WindowState, Option<String>) {
    if reading.remembered {
        return (WindowState::Stale, reading.reason.clone());
    }
    match seconds_between(&reading.fetched_at, now) {
        Some(age) if age > FRESH_FOR.as_secs() as i64 => (
            WindowState::Stale,
            Some(format!(
                "the usage endpoint was last read {} minutes ago",
                age / 60
            )),
        ),
        _ => (WindowState::Ok, None),
    }
}

/// The newest of the timestamps that actually contributed a window.
///
/// `used_passive` is only ever true when the capture was already found to be the newer of
/// the two, so this needs no second comparison.
fn newest_source(fetched_at: &str, passive: &Provider, used_passive: bool) -> String {
    match passive.source_at.as_deref() {
        Some(captured_at) if used_passive => captured_at.to_owned(),
        _ => fetched_at.to_owned(),
    }
}

/// Whether a window in a merged block came from the endpoint.
///
/// Sugar for consumers and tests: the flag is `detailed`, and the merge takes it off a
/// window it replaced with a live passive reading.
#[must_use]
pub fn came_from_the_endpoint(window: &Window) -> bool {
    window.detailed == Some(true)
}
