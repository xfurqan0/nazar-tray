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

use nazar_core::state::Snapshot;
use nazar_core::timefmt::{rfc3339_from_unix_seconds, unix_seconds_from_rfc3339};
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
