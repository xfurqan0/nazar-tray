//! The tray icon: the bead, the tooltip, and the menu.
//!
//! WP0 shipped a static PNG and no menu. WP4 replaces the image with one [`crate::icon`]
//! draws for the current scale factor and the current numbers, gives it a tooltip that says
//! what those numbers are, and adds the three-item menu the plan asked for.
//!
//! **Why there is a menu now, when WP3's note said there deliberately was not.** That note
//! was written when the panel was the only surface and every action was going to live in
//! it. What changed is Quit. A tray application that can only be stopped by killing the
//! process leaves `~/.nazar/limits.lock` behind — the next launch then waits out a
//! five-minute grace period as a reader, which looks exactly like the tray not working.
//! Quit has to be reachable without the panel, and on Windows the place users look for it
//! is the right button.
//!
//! **What that costs, and the trade.** With a menu attached, the right button belongs to
//! the shell: it opens the menu rather than the panel. `docs/PROJECT.md` section 7 asks for
//! "panel opens on left/right click", and the honest reading of it here is one click for
//! the left button and one menu item — `Open`, the first entry — for the right. Doing both
//! at once is worse than either: the menu takes the focus, the panel blurs behind it, and
//! the blur-suppression that keeps it alive then leaves a panel nobody can dismiss. The
//! deviation is written up in `docs/PROJECT.md` section 9.

use nazar_core::state::{SnapshotView, WindowView};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::i18n::{self, Catalog};
use crate::icon::{self, IconState};
use crate::state::AppState;

/// Identifier of the one tray icon this app owns.
pub const TRAY_ID: &str = "nazar-tray";

/// Menu item: show the panel.
const MENU_OPEN: &str = "nazar-open";
/// Menu item: ask the loop for a pass right now.
const MENU_REFRESH: &str = "nazar-refresh";
/// Menu item: stop the loop, release the lock, exit.
const MENU_QUIT: &str = "nazar-quit";

/// Longest tooltip Windows will show. `NOTIFYICONDATA::szTip` holds 128 characters
/// including the terminator, and a tooltip that is silently dropped is worse than a short
/// one.
const TOOLTIP_LIMIT: usize = 127;

/// Create the tray icon, its menu, and the mouse bindings.
pub fn install(app: &AppHandle, catalog: &Catalog) -> tauri::Result<()> {
    let open = MenuItem::with_id(
        app,
        MENU_OPEN,
        catalog.text("tray.menu.open"),
        true,
        None::<&str>,
    )?;
    let refresh_item = MenuItem::with_id(
        app,
        MENU_REFRESH,
        catalog.text("tray.menu.refresh"),
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(
        app,
        MENU_QUIT,
        catalog.text("tray.menu.quit"),
        true,
        None::<&str>,
    )?;
    let menu = Menu::with_items(app, &[&open, &refresh_item, &separator, &quit])?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(image(&IconState::default(), 1.0))
        // The bead is a coloured mark, not a silhouette; letting macOS recolour it as a
        // template would throw away the signal.
        .icon_as_template(false)
        .tooltip(catalog.text("tray.tooltip.noData"))
        .menu(&menu)
        // The left button opens the panel and must not wait for a menu.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            MENU_OPEN => crate::panel::show(app),
            MENU_REFRESH => {
                if let Some(state) = app.try_state::<AppState>() {
                    state.refresh();
                }
            }
            MENU_QUIT => {
                // The lock first, the process second. This is the whole reason the menu
                // exists; see the module note.
                if let Some(state) = app.try_state::<AppState>() {
                    state.shutdown();
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Acts on release, so a click that started elsewhere does not count. The middle
            // button is deliberately left free: WP5 has a use for it.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = event
            {
                crate::panel::toggle_near(tray.app_handle(), position);
            }
        })
        .build(app)?;
    Ok(())
}

/// Redraw the icon and rewrite the tooltip from whatever the state holds now.
///
/// Called on `snapshot-changed` and when the display's scale factor changes. Cheap enough
/// to call on every event: a 32-pixel bead is a thousand pixels of arithmetic.
pub fn refresh(app: &AppHandle, catalog: &Catalog) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let view = state.view();
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    let _ = tray.set_icon(Some(image(&IconState::from_view(&view), scale_factor(app))));
    let _ = tray.set_tooltip(Some(tooltip(&view, catalog)));
}

/// The scale factor the tray icon should be drawn for.
///
/// The primary monitor's, because that is where Windows puts the taskbar unless the user
/// has moved it; on a mixed-DPI desktop the shell scales whatever it is given, so a bead
/// drawn for the other monitor is resampled rather than wrong.
fn scale_factor(app: &AppHandle) -> f64 {
    app.primary_monitor()
        .ok()
        .flatten()
        .map_or(1.0, |monitor| monitor.scale_factor())
}

/// Rasterise the bead for a scale factor and hand it to Tauri as raw pixels.
///
/// No PNG anywhere in this path: [`icon::render`] produces exactly the RGBA the shell
/// wants, so nothing is encoded and nothing is decoded.
fn image(state: &IconState, scale: f64) -> Image<'static> {
    let bitmap = icon::render(icon::size_for_scale(scale), state);
    Image::new_owned(bitmap.rgba, bitmap.width, bitmap.height)
}

/// The tooltip: every provider's binding window, and when the worst one resets.
///
/// `Claude Fable week 88 % · Codex week 70 % (resets in 2 h 10 m)`, or the "no data" line
/// when nothing could be read. A provider whose numbers are unknown is left out rather than
/// shown as `0 %` — the icon has already gone grey, and a tooltip repeating a number nobody
/// read would undo that.
#[must_use]
pub fn tooltip(view: &SnapshotView, catalog: &Catalog) -> String {
    let mut entries: Vec<String> = Vec::new();
    let mut worst: Option<(f64, Option<i64>)> = None;

    for provider in &view.providers {
        let Some(window) = provider.windows.iter().find(|window| window.binding) else {
            continue;
        };
        let Some(percent) = window.percent.filter(|value| value.is_finite()) else {
            continue;
        };
        entries.push(catalog.format(
            "tray.tooltip.entry",
            &[
                (
                    "provider",
                    &catalog.text(&format!("tray.provider.{}", provider.name)),
                ),
                ("window", &short_label(catalog, window)),
                // Floored, never rounded up: 99.6 % has not run out (finding B15).
                ("percent", &percent.floor().to_string()),
            ],
        ));
        if worst.is_none_or(|(highest, _)| percent > highest) {
            worst = Some((percent, window.remaining_ms));
        }
    }

    if entries.is_empty() {
        return catalog.text("tray.tooltip.noData");
    }

    let mut text = entries.join(" · ");
    if let Some((_, Some(remaining))) = worst
        && remaining > 0
    {
        text.push(' ');
        text.push_str(&catalog.format(
            "tray.tooltip.resets",
            &[("time", &i18n::duration(catalog, remaining))],
        ));
    }

    if text.chars().count() > TOOLTIP_LIMIT {
        text = text.chars().take(TOOLTIP_LIMIT - 1).collect::<String>() + "…";
    }
    text
}

/// A window's name in a tooltip: `5h`, `week`, `Fable week`.
///
/// Chosen by length rather than by key, so it works for both providers without either being
/// special-cased — `windowMinutes` is in the contract for exactly this reason. A length this
/// build does not know falls back to the raw key, which is data rather than a word and needs
/// no translation.
fn short_label(catalog: &Catalog, window: &WindowView) -> String {
    if let Some(model) = &window.model {
        return catalog.format("window.short.model", &[("model", model)]);
    }
    match window.window_minutes {
        Some(300) => catalog.text("window.short.fiveHour"),
        Some(10080) => catalog.text("window.short.weekly"),
        _ => window.key.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nazar_core::state::{Freshness, ProviderView, Severity};

    fn window(key: &str, percent: Option<f64>, minutes: u32, binding: bool) -> WindowView {
        WindowView {
            key: key.to_owned(),
            percent,
            window_minutes: Some(minutes),
            resets_at: None,
            remaining_ms: Some(7_800_000),
            state: "ok".to_owned(),
            error: None,
            model: None,
            detailed: false,
            binding,
            severity: if percent.is_some() {
                Severity::Warn
            } else {
                Severity::Unknown
            },
        }
    }

    fn provider(name: &str, windows: Vec<WindowView>) -> ProviderView {
        ProviderView {
            name: name.to_owned(),
            configured: true,
            plan: None,
            source: None,
            source_at: None,
            binding: windows
                .iter()
                .find(|window| window.binding)
                .map(|window| window.key.clone()),
            age_ms: None,
            freshness: Freshness::Fresh,
            severity: Severity::Warn,
            windows,
        }
    }

    fn view(providers: Vec<ProviderView>) -> SnapshotView {
        SnapshotView {
            updated_at: String::new(),
            now: String::new(),
            providers,
        }
    }

    #[test]
    fn the_tooltip_names_each_providers_binding_window_and_the_worst_reset() {
        let catalog = i18n::catalog("en");
        let text = tooltip(
            &view(vec![
                provider(
                    "claude",
                    vec![
                        window("five_hour", Some(12.0), 300, false),
                        window("seven_day", Some(88.4), 10080, true),
                    ],
                ),
                provider("codex", vec![window("secondary", Some(70.0), 10080, true)]),
            ]),
            &catalog,
        );

        assert_eq!(
            text, "Claude week 88 % · Codex week 70 % (resets in 2 h 10 m)",
            "the tooltip is the sentence docs/PROJECT.md asks for"
        );
    }

    #[test]
    fn a_model_scoped_window_says_which_model() {
        let catalog = i18n::catalog("en");
        let mut scoped = window("seven_day_fable", Some(88.0), 10080, true);
        scoped.model = Some("Fable".to_owned());
        scoped.detailed = true;

        let text = tooltip(&view(vec![provider("claude", vec![scoped])]), &catalog);
        assert!(text.starts_with("Claude Fable week 88 %"), "got {text}");
    }

    #[test]
    fn nothing_read_is_a_sentence_rather_than_a_zero() {
        let catalog = i18n::catalog("en");
        assert_eq!(
            tooltip(&view(vec![]), &catalog),
            "nazar-tray: no data",
            "an empty snapshot must not produce an empty tooltip"
        );
        assert_eq!(
            tooltip(
                &view(vec![provider(
                    "codex",
                    vec![window("primary", None, 300, false)]
                )]),
                &catalog
            ),
            "nazar-tray: no data",
            "a provider nobody could read contributes no number at all"
        );
    }

    #[test]
    fn the_tooltip_is_translated_rather_than_assembled_from_english() {
        let turkish = tooltip(
            &view(vec![provider(
                "codex",
                vec![window("secondary", Some(70.0), 10080, true)],
            )]),
            &i18n::catalog("tr"),
        );
        assert_eq!(turkish, "Codex hafta %70 (2 sa 10 dk sonra sıfırlanır)");
    }

    #[test]
    fn a_window_length_nobody_recognises_falls_back_to_its_key() {
        let catalog = i18n::catalog("en");
        let odd = window("monthly", Some(50.0), 43200, true);
        let text = tooltip(&view(vec![provider("claude", vec![odd])]), &catalog);
        assert!(text.starts_with("Claude monthly 50 %"), "got {text}");
    }

    #[test]
    fn a_tooltip_that_would_not_fit_is_cut_rather_than_dropped() {
        let catalog = i18n::catalog("en");
        let long = window(&"x".repeat(300), Some(50.0), 1, true);
        let text = tooltip(&view(vec![provider("claude", vec![long])]), &catalog);
        assert!(
            text.chars().count() <= TOOLTIP_LIMIT,
            "got {} characters",
            text.chars().count()
        );
        assert!(text.ends_with('…'));
    }

    #[test]
    fn a_reset_that_is_already_due_is_not_advertised_as_time_left() {
        let catalog = i18n::catalog("en");
        let mut due = window("seven_day", Some(70.0), 10080, true);
        due.remaining_ms = Some(-60_000);
        let text = tooltip(&view(vec![provider("codex", vec![due])]), &catalog);
        assert_eq!(text, "Codex week 70 %", "got {text}");
    }
}
