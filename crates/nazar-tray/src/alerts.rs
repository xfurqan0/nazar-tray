//! Turning a crossing into a toast.
//!
//! [`nazar_core::alerts`] decides **whether** to warn; this file decides **what it says** and
//! hands it to Windows. The split is the same one the rest of the product uses: the rule is
//! testable arithmetic in the core, the sentence is a locale lookup, and the only thing that
//! needs a running application is the last line.
//!
//! ```text
//!   Event::Refreshed ─▶ derive the view ─▶ Alerts::evaluate ─▶ [Alert]
//!                                              │                  │
//!                                     alerts.json (atomic)        ├─ suppressed? stop here
//!                                                                 └─ toast(catalog, alert)
//!                                                                        │
//!                                                          "Claude Code · weekly window 85 %"
//!                                                          "Resets in 2 h 10 m"
//! ```
//!
//! **Why it listens for `Refreshed` rather than for `SnapshotChanged`.** A tray that starts
//! with the weekly window already at 91 % changes nothing — the document on disk said 91 %
//! before the process existed — and that is exactly the case where the user most needs to be
//! told. The loop announces every pass; the change is a second, narrower event for the panel.
//!
//! **The title names the threshold, not the reading.** `weekly window 85 %` with the window
//! at 86.4 %: the toast is about a line being crossed, and the line is the number the user
//! set. The reading itself is in the panel, a click away, where it is a live number rather
//! than a frozen one.
//!
//! **Clicking a toast cannot open the panel, and that is the plugin's limit rather than a
//! decision.** `tauri-plugin-notification` 2.4 builds the notification, spawns `show()` onto
//! the async runtime and drops the handle, so the `on_activated` callback that `notify-rust`
//! does offer on Windows never reaches an application. There is no `on_action` on desktop —
//! `action_type_id` is mobile-only. Written up in `docs/PROJECT.md` section 9 and in the
//! README so nobody has to rediscover it; the tray icon a click away is the workaround, and
//! the toast says which window it is about so the click is informed.

use std::sync::{Arc, Mutex, PoisonError};

use nazar_core::alerts::{Alert, Alerts};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::i18n::{self, Catalog, Strings};
use crate::state::AppState;

/// The notification state machine, wrapped for Tauri to manage.
///
/// One instance per process, behind a mutex, because two evaluations of the same view would
/// both decide a threshold had been crossed.
pub struct Notifier {
    alerts: Mutex<Alerts>,
}

impl Notifier {
    /// Backed by `alerts.json`, so a restart does not repeat a warning.
    #[must_use]
    pub fn persistent() -> Self {
        Notifier {
            alerts: Mutex::new(Alerts::discover()),
        }
    }

    /// Remembering nothing between runs.
    ///
    /// What `--demo` gets. A screenshot session — or a `--demo-cross` run showing the
    /// notifications off — must not write to the maintainer's `%APPDATA%\nazar`, and must
    /// certainly not consume the keys of a real crossing that has not been shown yet.
    #[must_use]
    pub fn in_memory() -> Self {
        Notifier {
            alerts: Mutex::new(Alerts::in_memory()),
        }
    }
}

/// Evaluate the thresholds against the current view and show whatever crossed.
///
/// Called on every `Refreshed` event, on the refresh loop's own thread.
pub fn on_refresh(app: &AppHandle) {
    let (Some(notifier), Some(state), Some(strings)) = (
        app.try_state::<Notifier>(),
        app.try_state::<AppState>(),
        app.try_state::<Arc<Strings>>(),
    ) else {
        // The application is being torn down. Nothing to warn anybody about.
        return;
    };

    let config = state.config();
    let view = state.view();
    let rules = config.alert_rules(crate::system::local_minutes());

    let fired = {
        let mut alerts = notifier
            .alerts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let fired = alerts.evaluate(&view, &rules);
        // Written before anything is shown. A toast that appears and is then forgotten
        // because the process died half a second later would come back on the next launch,
        // and the whole promise of `alerts.json` is that it does not.
        if let Err(error) = alerts.save() {
            eprintln!("nazar-tray: could not record the notification: {error}");
        }
        fired
    };

    if fired.is_empty() {
        return;
    }
    let catalog = strings.catalog();
    for alert in fired.iter().filter(|alert| !alert.suppressed) {
        show(app, &catalog, alert);
    }
}

/// Hand one alert to the operating system.
fn show(app: &AppHandle, catalog: &Catalog, alert: &Alert) {
    let (title, body) = toast(catalog, alert);
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        // A desktop that cannot show a notification is not a reason to stop watching the
        // quota. The icon has already changed colour, which is the part that always works.
        eprintln!("nazar-tray: could not show the notification: {error}");
    }
}

/// Show a one-off notice about the application itself, at most once per machine.
///
/// Not a threshold and not about a window: a notice is something nazar-tray has to say
/// about *itself*, and [`nazar_core::alerts::Alerts::claim_notice`] is what makes "once"
/// mean once across restarts rather than once per process. The first caller is
/// [`crate::desktop::announce`], on a Linux desktop with no tray host.
///
/// It goes through the same [`Notifier`] as the threshold toasts, which means it obeys the
/// same two facts about that state: a `--demo` run claims it in memory and so never
/// consumes a notice the user has not seen, and the claim is written before anything is
/// shown, so a process that dies half a second later does not repeat itself on the next
/// launch. It does **not** go through the quiet hours or the notifications switch: both are
/// about the quota, and a user who turned off "warn me at 85 %" has not asked to be kept in
/// the dark about the tray icon they cannot see.
pub fn notice(app: &AppHandle, key: &str, title: &str, body: &str) {
    let (Some(notifier), Some(strings)) =
        (app.try_state::<Notifier>(), app.try_state::<Arc<Strings>>())
    else {
        return;
    };

    let claimed = {
        let mut alerts = notifier
            .alerts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let claimed = alerts.claim_notice(key);
        if claimed && let Err(error) = alerts.save() {
            eprintln!("nazar-tray: could not record the notice: {error}");
        }
        claimed
    };
    if !claimed {
        return;
    }

    let catalog = strings.catalog();
    if let Err(error) = app
        .notification()
        .builder()
        .title(catalog.text(title))
        .body(catalog.text(body))
        .show()
    {
        eprintln!("nazar-tray: could not show the notification: {error}");
    }
}

/// The two lines of a toast, in the user's language.
///
/// `Claude Code · weekly window 85 %` over `Resets in 2 h 10 m`, and the Turkish of both.
/// Separated from [`show`] so it can be tested without an application: the sentence is the
/// part that can be wrong, and the handing-over is one call.
#[must_use]
pub fn toast(catalog: &Catalog, alert: &Alert) -> (String, String) {
    let title = catalog.format(
        "alert.title",
        &[
            (
                "provider",
                &catalog.text(&format!("panel.provider.{}", alert.provider)),
            ),
            ("window", &window_name(catalog, alert)),
            ("percent", &percent(alert.threshold)),
        ],
    );

    let body = match alert.remaining_ms {
        Some(remaining) if remaining > 0 => catalog.format(
            "alert.body.resets",
            &[("time", &i18n::duration(catalog, remaining))],
        ),
        Some(_) => catalog.text("alert.body.resetDue"),
        None => catalog.text("alert.body.noReset"),
    };
    (title, body)
}

/// What a window is called inside a sentence.
///
/// Named by its **length** rather than by its provider, which is why `windowMinutes` is in
/// the contract at all; a model-scoped weekly says which model, because "weekly window"
/// twice in one account would be a lie. A length this build does not know falls back to the
/// raw key — data rather than a word, and it needs no translation.
fn window_name(catalog: &Catalog, alert: &Alert) -> String {
    if let Some(model) = &alert.model {
        return catalog.format("alert.window.modelWeekly", &[("model", model)]);
    }
    match alert.window_minutes {
        Some(300) => catalog.text("alert.window.fiveHour"),
        Some(10080) => catalog.text("alert.window.weekly"),
        _ => alert.window.clone(),
    }
}

/// A threshold as a number a person would write: `85`, `62.5`.
///
/// Never `85.0`, and never rounded up — the same rule the rest of the product follows about
/// percentages, for the same reason (finding B15).
fn percent(value: f64) -> String {
    if !value.is_finite() {
        return "?".to_owned();
    }
    if (value.fract()).abs() < 1e-9 {
        return format!("{}", value.trunc() as i64);
    }
    let text = format!("{value:.1}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(window: &str, minutes: Option<u32>, threshold: f64, remaining: Option<i64>) -> Alert {
        Alert {
            provider: "claude".to_owned(),
            window: window.to_owned(),
            window_minutes: minutes,
            model: None,
            threshold,
            percent: threshold + 1.4,
            remaining_ms: remaining,
            suppressed: false,
        }
    }

    #[test]
    fn the_toast_is_the_sentence_the_plan_asks_for() {
        let catalog = i18n::catalog("en");
        let (title, body) = toast(
            &catalog,
            &alert("seven_day", Some(10080), 85.0, Some(7_800_000)),
        );
        assert_eq!(title, "Claude Code · weekly window 85 %");
        assert_eq!(body, "Resets in 2 h 10 m");
    }

    #[test]
    fn and_the_same_sentence_in_turkish() {
        let catalog = i18n::catalog("tr");
        let (title, body) = toast(
            &catalog,
            &alert("seven_day", Some(10080), 85.0, Some(7_800_000)),
        );
        assert_eq!(title, "Claude Code · haftalık pencere %85");
        assert_eq!(
            body, "2 sa 10 dk sonra sıfırlanır",
            "the units come from the locale file too: 'h' is not 'sa'"
        );
    }

    #[test]
    fn and_in_the_four_languages_wp6_translated() {
        // The toast is two sentences assembled from four keys — the title, the window's
        // name, the percentage's place, and the duration's units — so a language that had
        // three of them would produce a sentence half in English. Both lines are written
        // out per language for the same reason the tooltip's are: a notification cannot be
        // screenshotted after the fact.
        for (locale, title, body) in [
            ("zh", "Claude Code · 每周窗口 85%", "2 小时 10 分后重置"),
            ("ko", "Claude Code · 주간 구간 85%", "2시간 10분 후 초기화"),
            (
                "ru",
                "Claude Code · недельное окно 85 %",
                "Сброс через 2 ч 10 мин",
            ),
            (
                "es",
                "Claude Code · ventana semanal 85 %",
                "Se restablece en 2 h 10 min",
            ),
        ] {
            let said = toast(
                &i18n::catalog(locale),
                &alert("seven_day", Some(10080), 85.0, Some(7_800_000)),
            );
            assert_eq!(
                said,
                (title.to_owned(), body.to_owned()),
                "the {locale} toast"
            );
        }
    }

    #[test]
    fn a_five_hour_window_and_a_model_scoped_one_are_named_differently() {
        let catalog = i18n::catalog("en");
        let (title, _) = toast(&catalog, &alert("five_hour", Some(300), 60.0, None));
        assert_eq!(title, "Claude Code · 5-hour window 60 %");

        let mut scoped = alert("seven_day_fable", Some(10080), 85.0, None);
        scoped.model = Some("Fable".to_owned());
        let (title, _) = toast(&catalog, &scoped);
        assert_eq!(
            title, "Claude Code · Fable weekly window 85 %",
            "'weekly window' twice in one account would be a lie"
        );
    }

    #[test]
    fn a_window_length_nobody_recognises_falls_back_to_its_key() {
        let catalog = i18n::catalog("en");
        let (title, _) = toast(&catalog, &alert("monthly", Some(43200), 85.0, None));
        assert!(title.contains("monthly"), "got {title}");
    }

    #[test]
    fn codex_is_named_as_codex() {
        let catalog = i18n::catalog("en");
        let mut codex = alert("secondary", Some(10080), 100.0, Some(-1));
        codex.provider = "codex".to_owned();
        let (title, body) = toast(&catalog, &codex);
        assert_eq!(title, "Codex · weekly window 100 %");
        assert_eq!(body, "The reset is due");
    }

    #[test]
    fn a_window_with_no_reset_time_says_so_rather_than_inventing_one() {
        let catalog = i18n::catalog("en");
        let (_, body) = toast(&catalog, &alert("seven_day", Some(10080), 85.0, None));
        assert_eq!(body, "No reset time reported");
        assert_ne!(body, "Resets in 0 s", "a countdown nobody read is not zero");
    }

    #[test]
    fn a_threshold_is_written_the_way_a_person_would_write_it() {
        assert_eq!(percent(85.0), "85");
        assert_eq!(percent(100.0), "100");
        assert_eq!(percent(62.5), "62.5");
        assert_eq!(percent(f64::NAN), "?");
    }

    #[test]
    fn a_suppressed_alert_is_the_callers_business_and_not_the_sentences() {
        // `toast` says what a crossing means; whether anybody is shown it is decided in
        // `on_refresh`, so that quiet hours cannot change the words by accident.
        let catalog = i18n::catalog("en");
        let mut quiet = alert("seven_day", Some(10080), 85.0, Some(60_000));
        quiet.suppressed = true;
        assert_eq!(
            toast(&catalog, &quiet),
            toast(
                &catalog,
                &alert("seven_day", Some(10080), 85.0, Some(60_000))
            )
        );
    }
}
