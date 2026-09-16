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
//!
//! **T-WP-L8 makes the menu the surface on Linux, because nothing else can be.** Three
//! measurements, all of them in `tray-icon 0.24.2` and in a run on Fedora 44 / GNOME 50:
//!
//! * `platform_impl/gtk` **sends no `TrayIconEvent` at all** — `TrayIconEvent::send` is
//!   called from the Windows and macOS backends and from nowhere else. The left-click
//!   handler below is dead code on Linux, and the click is handled by libappindicator: it
//!   opens the menu.
//! * `TrayIcon::rect()` on GTK is `fn rect(&self) -> Option<Rect> { None }`. There is no
//!   icon geometry to open a panel next to even if a click arrived.
//! * `set_tooltip` on GTK is `Ok(())` and nothing else. Everything the tooltip says on
//!   Windows — the binding window, the percentage, the countdown, the week — reaches a
//!   Linux user nowhere.
//!
//! Put together with the Wayland placement problem in [`crate::panel`], the answer is not to
//! fix the click but to move the content: the **menu** is positioned beside the icon, by the
//! shell, on every desktop that draws a `StatusNotifierItem`. So on Linux the menu opens
//! with a live quota row per provider above the actions, rewritten on every refresh through
//! `muda`'s `set_text` — which reaches a `dbusmenu` property update, so an open menu changes
//! under the pointer rather than going stale. Windows and macOS keep the four-item menu they
//! had, byte for byte: [`QUOTA_IN_MENU`] is a `cfg!`.
//!
//! **WP5 adds `Settings` and makes the menu rebuildable.** A menu item's text is set when the
//! item is *built*, so a menu built in English stays in English however the settings change —
//! which was WP4's open risk, written down at the time and closed here. [`rebuild_menu`]
//! throws the whole menu away and makes a new one in the current language; it is called from
//! `set_config`, and only when the language actually moved, because replacing a menu the user
//! may have open is not a thing to do for nothing.

use std::sync::{Arc, Mutex, PoisonError};

use nazar_core::state::{ProviderView, SnapshotView, WindowView};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::i18n::{self, Catalog, Strings};
use crate::icon::{self, IconState};
use crate::state::AppState;
use crate::usage::{UsageState, WeekUsage};

/// Identifier of the one tray icon this app owns.
pub const TRAY_ID: &str = "nazar-tray";

/// Menu item: show the panel.
const MENU_OPEN: &str = "nazar-open";
/// Menu item: ask the loop for a pass right now.
const MENU_REFRESH: &str = "nazar-refresh";
/// Menu item: show the panel with the settings view open.
const MENU_SETTINGS: &str = "nazar-settings";
/// Menu item: stop the loop, release the lock, exit.
const MENU_QUIT: &str = "nazar-quit";
/// Id prefix of a live quota row: the provider name is appended.
const MENU_QUOTA: &str = "nazar-quota-";

/// Whether the menu carries the numbers as well as the actions.
///
/// **Linux only, and it is the tray's whole face there.** See the module note for the three
/// measurements: no click event, no icon rectangle, no tooltip. Where the shell will not
/// tell us where the icon is and will not show a tooltip, the one surface it does put beside
/// the icon is the menu it opens itself.
///
/// On Windows the tooltip already says all of this on hover and the panel opens on a left
/// click beside the icon, so rows in the menu would be a third copy of the same two numbers
/// — and a menu that changes height while the user is reading it. macOS gets the tooltip
/// and the popover for the same reason.
const QUOTA_IN_MENU: bool = cfg!(target_os = "linux");

/// Longest tooltip Windows will show. `NOTIFYICONDATA::szTip` holds 128 characters
/// including the terminator, and a tooltip that is silently dropped is worse than a short
/// one.
const TOOLTIP_LIMIT: usize = 127;

/// What separates the tooltip's two lines.
///
/// `\r\n` rather than `\n`: the shell draws this string itself, out of `szTip`, and the
/// carriage return is the break Win32 tooltips have always taken. It costs two of the 127
/// characters and it is one constant to change if a Windows build disagrees — which is not
/// something this repository's tests can find out, because nothing here can screenshot a
/// tooltip.
const TOOLTIP_BREAK: &str = "\r\n";

/// The live quota rows, kept so that a refresh can rewrite them without rebuilding the menu.
///
/// Empty on Windows and macOS, where [`QUOTA_IN_MENU`] is `false` and the tooltip carries
/// the numbers instead. Managed state rather than a field on anything, because the two
/// things that touch it — [`install`] and [`rebuild_menu`] on one side, [`refresh`] on the
/// other — are reached through an `AppHandle` and nothing else.
///
/// A `Mutex` because [`refresh`] runs on the refresh loop's thread. `MenuItem` is `Send` and
/// `Sync` in Tauri and every method on it hops to the main thread, so the lock is guarding
/// the `Vec` rather than GTK.
#[derive(Default)]
pub struct QuotaRows {
    items: Mutex<Vec<MenuItem<tauri::Wry>>>,
}

impl QuotaRows {
    fn replace(&self, items: Vec<MenuItem<tauri::Wry>>) {
        *self.items.lock().unwrap_or_else(PoisonError::into_inner) = items;
    }

    /// Rewrite every row from a view. Silent about failures: a menu row that could not be
    /// updated is a row showing the previous reading, which is a far smaller problem than a
    /// tray that stopped refreshing because a menu item went away.
    fn write(&self, view: &SnapshotView, catalog: &Catalog) {
        let items = self.items.lock().unwrap_or_else(PoisonError::into_inner);
        for (item, provider) in items.iter().zip(&view.providers) {
            let _ = item.set_text(quota_row(provider, catalog));
        }
    }
}

/// One provider as a line of menu: `Claude 5h 62 % (resets in 2 h 10 m)`.
///
/// The same three keys the tooltip uses, so the two surfaces cannot drift into two ways of
/// saying one number — and the same rule about what is *not* said: a provider whose binding
/// window carries no percentage gets a sentence about why, never a reassuring `0 %`.
fn quota_row(provider: &ProviderView, catalog: &Catalog) -> String {
    if let Some((entry, _, remaining)) = binding_entry(provider, catalog) {
        return match resets_clause(catalog, remaining) {
            Some(resets) => format!("{entry} {resets}"),
            None => entry,
        };
    }
    let reason = if provider.configured {
        catalog.text("tray.menu.noReading")
    } else {
        not_configured(catalog, &provider.name)
    };
    catalog.format(
        "tray.menu.provider",
        &[
            (
                "provider",
                &catalog.text(&format!("tray.provider.{}", provider.name)),
            ),
            ("reason", &reason),
        ],
    )
}

/// Why a provider has no numbers, in as much detail as this build has for that provider.
///
/// `panel.provider.notConfigured.<name>` when there is one and the generic sentence when
/// there is not, which is the same lookup `ui/src/main.ts` does for the panel's own card.
/// "Not set up" is a true sentence and a useless one: the two providers are not set up in
/// two different ways, and a user who is told which one theirs is can act on it.
fn not_configured(catalog: &Catalog, provider: &str) -> String {
    let specific = format!("panel.provider.notConfigured.{provider}");
    let text = catalog.text(&specific);
    if text == specific {
        return catalog.text("panel.provider.notConfigured");
    }
    text
}

/// Build the context menu in one language.
///
/// Four entries and a separator, in the order a Windows user looks for them: the thing they
/// came for, the thing they might want next, the settings, and the way out. On Linux a row
/// per provider goes above them, and a second separator: see [`QUOTA_IN_MENU`].
fn build_menu(
    app: &AppHandle,
    catalog: &Catalog,
    view: &SnapshotView,
) -> tauri::Result<(Menu<tauri::Wry>, Vec<MenuItem<tauri::Wry>>)> {
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
    let settings = MenuItem::with_id(
        app,
        MENU_SETTINGS,
        catalog.text("tray.menu.settings"),
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

    if !QUOTA_IN_MENU {
        let menu = Menu::with_items(app, &[&open, &refresh_item, &settings, &separator, &quit])?;
        return Ok((menu, Vec::new()));
    }

    // Enabled rather than greyed out, and clicking one opens the panel. A disabled item is
    // the conventional way to draw a label in a menu, and it is drawn in the disabled
    // colour — which is the wrong colour for the one number this application exists to
    // show. The gesture a user makes on a percentage they want more of is a click.
    let mut rows = Vec::with_capacity(view.providers.len());
    for provider in &view.providers {
        rows.push(MenuItem::with_id(
            app,
            format!("{MENU_QUOTA}{}", provider.name),
            quota_row(provider, catalog),
            true,
            None::<&str>,
        )?);
    }
    let under_the_numbers = PredefinedMenuItem::separator(app)?;

    let mut items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = Vec::new();
    for row in &rows {
        items.push(row);
    }
    items.push(&under_the_numbers);
    items.push(&open);
    items.push(&refresh_item);
    items.push(&settings);
    items.push(&separator);
    items.push(&quit);
    let menu = Menu::with_items(app, &items)?;
    Ok((menu, rows))
}

/// Replace the menu and the tooltip with ones written in the current language.
///
/// Called when — and only when — the language actually changed. A menu item's text cannot be
/// changed after the item is built, so this makes new items; the tray icon keeps its own
/// identity, so nothing flickers and nothing moves in the Windows 11 overflow.
pub fn rebuild_menu(app: &AppHandle, strings: &Strings) {
    let catalog = strings.catalog();
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let view = app
        .try_state::<AppState>()
        .map(|state| state.view())
        .unwrap_or_else(|| SnapshotView {
            updated_at: String::new(),
            now: String::new(),
            providers: Vec::new(),
        });
    match build_menu(app, &catalog, &view) {
        Ok((menu, rows)) => {
            let _ = tray.set_menu(Some(menu));
            // The old rows belong to a menu that has just been thrown away; keeping them
            // would leave every refresh writing into a menu nobody can open.
            if let Some(state) = app.try_state::<QuotaRows>() {
                state.replace(rows);
            }
        }
        // A menu that could not be rebuilt is a menu in the old language, which is a much
        // smaller problem than no menu at all — and `Quit` is in the old one too.
        Err(error) => eprintln!("nazar-tray: could not rebuild the tray menu: {error}"),
    }
    if let Some(state) = app.try_state::<AppState>() {
        let _ = tray.set_tooltip(Some(tooltip(&state.view(), &catalog, week(app).as_ref())));
    }
}

/// This week, if this process has a usage store to read.
///
/// `None` before [`UsageState`] is managed, which is the only moment in a real run when it
/// is missing, and `None` for every way [`crate::usage::week_usage`] can come to nothing.
///
/// A `--demo` run reads [`crate::demo::usage`] instead of the real store, which is the same
/// answer [`crate::usage::get_usage`] gives the panel: one fixture, two surfaces. Until 0.2.0
/// it read the real store here and a screenshot carried the maintainer's own month of model
/// use; `crate::usage::store_dir` is where that is now impossible rather than merely avoided.
fn week(app: &AppHandle) -> Option<WeekUsage> {
    let state = app.try_state::<UsageState>()?;
    // The same setting the panel's headline follows, read from the same place: a tooltip and
    // a view that counted differently would be this application answering one question with
    // two numbers, which is what T-WP20b spent a package on not doing.
    let per_line = app
        .try_state::<AppState>()
        .is_some_and(|state| state.config().usage.count_like_claude_code);
    crate::usage::week_usage(state.inner(), per_line)
}

/// Create the tray icon, its menu, and the mouse bindings.
pub fn install(app: &AppHandle, strings: &Arc<Strings>) -> tauri::Result<()> {
    let catalog = strings.catalog();
    let view = app.state::<AppState>().view();
    let (menu, rows) = build_menu(app, &catalog, &view)?;
    let quota = QuotaRows::default();
    quota.replace(rows);
    app.manage(quota);

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(image(IconState::default(), 1.0))
        // The bead is a coloured mark, not a silhouette; letting macOS recolour it as a
        // template would throw away the signal.
        .icon_as_template(false)
        .tooltip(catalog.text("tray.tooltip.noData"))
        .menu(&menu)
        // The left button opens the panel and must not wait for a menu.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            MENU_OPEN => crate::panel::show(app),
            // A quota row is a reading, and the gesture on a reading is "show me the rest".
            id if id.starts_with(MENU_QUOTA) => crate::panel::show(app),
            MENU_REFRESH => {
                if let Some(state) = app.try_state::<AppState>() {
                    state.refresh();
                }
            }
            // The settings live in the panel, so the menu asks the panel to show them.
            MENU_SETTINGS => crate::state::open_settings(app.clone()),
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
            // button is deliberately left free.
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

    let _ = tray.set_icon(Some(image(IconState::from_view(&view), scale_factor(app))));
    let _ = tray.set_tooltip(Some(tooltip(&view, catalog, week(app).as_ref())));
    write_quota_rows(app, &view, catalog);
}

/// Rewrite the menu's quota rows, where there are any.
///
/// Called from both refresh paths, because on Linux these rows are what the tooltip is on
/// Windows — and `set_tooltip` there is a GTK no-op, so a Linux run that only rewrote the
/// tooltip would have rewritten nothing at all.
fn write_quota_rows(app: &AppHandle, view: &SnapshotView, catalog: &Catalog) {
    if !QUOTA_IN_MENU {
        return;
    }
    if let Some(rows) = app.try_state::<QuotaRows>() {
        rows.write(view, catalog);
    }
}

/// Rewrite the tooltip and leave the icon alone.
///
/// The icon says nothing about usage — decision K25 — so the pass that keeps the week line
/// current has no reason to rasterise a bead. Called from two places, for two different
/// reasons: the refresh tick, because a scan in another process may have moved the store,
/// and [`crate::usage::get_usage`], because a scan in *this* one just did.
///
/// Reads the language from the application rather than taking a catalogue, because both
/// callers are holding an [`AppHandle`] and neither is holding a catalogue. What it costs is
/// the one or two month documents the current week touches; what it must never do is scan,
/// and `crate::usage` is where that rule is written.
pub fn refresh_tooltip(app: &AppHandle) {
    let Some(strings) = app.try_state::<Arc<Strings>>() else {
        return;
    };
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let catalog = strings.catalog();
    let view = state.view();
    let _ = tray.set_tooltip(Some(tooltip(&view, &catalog, week(app).as_ref())));
    write_quota_rows(app, &view, &catalog);
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
fn image(state: IconState, scale: f64) -> Image<'static> {
    let bitmap = icon::render(icon::size_for_scale(scale), state);
    Image::new_owned(bitmap.rgba, bitmap.width, bitmap.height)
}

/// The tooltip: every provider's binding window, when the worst one resets, and — since
/// T-WP17 — what this week has cost.
///
/// ```text
/// Claude Fable week 88 % · Codex week 70 % (resets in 2 h 10 m)
/// This week 1.5B · claude-sonnet-5
/// ```
///
/// or the "no data" line when nothing could be read. A provider whose numbers are unknown is
/// left out rather than shown as `0 %` — the icon has already gone grey, and a tooltip
/// repeating a number nobody read would undo that.
///
/// The week number is [`crate::usage::headline`]'s four-way total, the one Claude Code's
/// `/usage` calls *total tokens* — T-WP20b, and the reason the reading above is a `B` where
/// T-WP17 wrote an `M`. The line does not get longer for it: `compact` writes five characters
/// or fewer for anything under a trillion and nine for `u64::MAX`, and the worst-case test
/// below is still measured at `u64::MAX`.
///
/// # Two lines in 127 characters
///
/// Windows shows 127 and drops the rest, and the first line is already 69 of them in Turkish.
/// So the second line is fitted rather than assumed, in three steps, each of which gives up
/// the least valuable thing left:
///
/// 1. **Both lines as written.** This is what fits on every reading a real machine produces:
///    the widest of the six languages is Spanish, and with the fullest quota line, a dated
///    model id and a `u64::MAX` of tokens it comes to 119 of the 127. The test below is that
///    measurement, and steps 2 and 3 exist for the shapes it does not cover — a window key
///    this build does not recognise is printed raw, and a raw key has no length.
/// 2. **The reset clause goes.** It is the least load-bearing part of the first line: the
///    percentage is the alarm, the countdown is detail, and the panel is one click away and
///    shows both. This is the mitigation risk R2 was written for.
/// 3. **The week line goes.** If neither fits, quota wins, because quota is why this
///    application exists. The week is not cut in half — half a number is worse than no
///    number — and what is left is exactly the tooltip this build shipped before.
///
/// `week` is `None` on a machine whose store has nothing for this week, and then none of
/// this happens: step 3 is also the ordinary case on a fresh install.
///
/// # What the week line does *not* do
///
/// It does not touch the icon. Decision K25 says the bead is the mark at every reading and
/// grey only for unknown, and a usage figure is not a state the icon has an opinion about.
#[must_use]
pub fn tooltip(view: &SnapshotView, catalog: &Catalog, week: Option<&WeekUsage>) -> String {
    let Some((entries, resets)) = quota_line(view, catalog) else {
        // Nothing could be read. The sentence says so, and pairing it with a week the store
        // does happen to hold would be an answer to a question nobody asked.
        return catalog.text("tray.tooltip.noData");
    };
    let full = match &resets {
        Some(resets) => format!("{entries} {resets}"),
        None => entries.clone(),
    };

    if let Some(week) = week {
        let second = catalog.format(
            "tray.tooltip.week",
            &[
                ("tokens", &crate::usage::compact(week.headline)),
                ("model", &week.model),
            ],
        );
        let both = format!("{full}{TOOLTIP_BREAK}{second}");
        if fits(&both) {
            return both;
        }
        if resets.is_some() {
            let without = format!("{entries}{TOOLTIP_BREAK}{second}");
            if fits(&without) {
                return without;
            }
        }
    }

    clamp(full)
}

/// Whether a tooltip is one Windows will show whole.
fn fits(text: &str) -> bool {
    text.chars().count() <= TOOLTIP_LIMIT
}

/// A tooltip cut to what the shell will show, with an ellipsis where it was cut.
fn clamp(text: String) -> String {
    if fits(&text) {
        return text;
    }
    text.chars().take(TOOLTIP_LIMIT - 1).collect::<String>() + "…"
}

/// The quota half of the tooltip: the joined entries, and the reset clause if there is one.
///
/// Separated from [`tooltip`] so the reset clause can be dropped on its own when the week
/// line will not otherwise fit. `None` when no provider reported a percentage at all.
fn quota_line(view: &SnapshotView, catalog: &Catalog) -> Option<(String, Option<String>)> {
    let mut entries: Vec<String> = Vec::new();
    let mut worst: Option<(f64, Option<i64>)> = None;

    for provider in &view.providers {
        let Some((entry, percent, remaining)) = binding_entry(provider, catalog) else {
            continue;
        };
        entries.push(entry);
        if worst.is_none_or(|(highest, _)| percent > highest) {
            worst = Some((percent, remaining));
        }
    }

    if entries.is_empty() {
        return None;
    }

    let resets = worst.and_then(|(_, remaining)| resets_clause(catalog, remaining));
    Some((entries.join(" · "), resets))
}

/// One provider's binding window as a phrase, its percentage, and what it has left.
///
/// `None` when the provider has no binding window or its binding window carries no
/// percentage — a provider nobody could read is left out of the tooltip rather than shown at
/// zero, which is finding B03, and the menu row says why instead.
///
/// Shared by the tooltip and by [`quota_row`] so that the two surfaces are one sentence with
/// two frames around it. The percentage comes back as well as the text because the tooltip
/// needs it to pick the worst window, and re-deriving it there would be the second opinion
/// this function exists to prevent.
fn binding_entry(provider: &ProviderView, catalog: &Catalog) -> Option<(String, f64, Option<i64>)> {
    let window = provider.windows.iter().find(|window| window.binding)?;
    let percent = window.percent.filter(|value| value.is_finite())?;
    let entry = catalog.format(
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
    );
    Some((entry, percent, window.remaining_ms))
}

/// `(resets in 2 h 10 m)`, or nothing when the source named no reset or it is already due.
fn resets_clause(catalog: &Catalog, remaining: Option<i64>) -> Option<String> {
    let remaining = remaining.filter(|left| *left > 0)?;
    Some(catalog.format(
        "tray.tooltip.resets",
        &[("time", &i18n::duration(catalog, remaining))],
    ))
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

    /// The tooltip on a machine whose usage store has nothing for this week.
    ///
    /// Shadows [`super::tooltip`] so that every test written before T-WP17 still reads as
    /// the question it was asking, and so that "the week line changes nothing when there is
    /// no week" is checked by all of them at once rather than by one more assertion.
    fn tooltip(view: &SnapshotView, catalog: &Catalog) -> String {
        super::tooltip(view, catalog, None)
    }

    /// A week worth putting in a tooltip.
    fn week_of(headline: u64, model: &str) -> WeekUsage {
        WeekUsage {
            headline,
            model: model.to_owned(),
        }
    }

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

    /// A provider the tray could not read at all: the key stays, the windows do not.
    fn unread(name: &str, configured: bool) -> ProviderView {
        ProviderView {
            configured,
            binding: None,
            severity: Severity::Unknown,
            windows: Vec::new(),
            ..provider(name, Vec::new())
        }
    }

    /// Where the numbers go on this platform, and why.
    #[test]
    fn the_menu_carries_the_numbers_exactly_where_the_tooltip_cannot() {
        assert_eq!(
            QUOTA_IN_MENU,
            cfg!(target_os = "linux"),
            "tray-icon's GTK backend makes set_tooltip a no-op and sends no click event, \
             so the menu is the only surface the shell puts beside the icon"
        );
    }

    /// The row and the tooltip entry are one sentence with two frames around it.
    ///
    /// Not "they look similar": the row **is** the tooltip's entry for that provider, plus
    /// the reset clause the tooltip only has room for once. A second way of writing
    /// `Claude week 88 %` is a second opinion about a number, which is finding B14 wearing
    /// a different hat.
    #[test]
    fn a_readable_provider_reads_the_same_in_the_menu_as_in_the_tooltip() {
        let catalog = i18n::catalog("en");
        let claude = provider("claude", vec![window("seven_day", Some(88.4), 10080, true)]);

        let row = quota_row(&claude, &catalog);
        assert_eq!(row, "Claude week 88 % (resets in 2 h 10 m)");
        assert!(
            tooltip(&view(vec![claude]), &catalog).starts_with("Claude week 88 %"),
            "the tooltip and the menu row disagree about the same reading"
        );
    }

    /// A provider nobody could read says why. It never says `0 %`.
    ///
    /// The whole argument of finding B03, in the surface T-WP-L8 added: a row is a place a
    /// zero could appear, and a zero is the most reassuring number this application can
    /// print about a quota nobody measured.
    #[test]
    fn a_provider_with_no_reading_says_so_rather_than_showing_zero() {
        let catalog = i18n::catalog("en");

        let not_set_up = quota_row(&unread("claude", false), &catalog);
        assert!(not_set_up.starts_with("Claude "), "{not_set_up}");
        assert!(!not_set_up.contains('0'), "{not_set_up}");

        let silent = quota_row(&unread("codex", true), &catalog);
        assert_eq!(silent, "Codex — no reading yet");

        // And a provider that is configured and merely unreadable is a different sentence
        // from one that was never set up: the first is a wait, the second is a step.
        assert_ne!(not_set_up, silent.replace("Codex", "Claude"));
    }

    /// Every language can write a row, and none of them leaves a placeholder in it.
    #[test]
    fn the_menu_row_reads_as_a_sentence_in_all_six_languages() {
        for locale in nazar_core::config::LOCALES {
            let catalog = i18n::catalog(locale);
            for view in [
                provider("codex", vec![window("secondary", Some(70.0), 10080, true)]),
                unread("claude", false),
                unread("codex", true),
            ] {
                let row = quota_row(&view, &catalog);
                assert!(!row.is_empty(), "{locale}: an empty row");
                assert!(
                    !row.contains('{') && !row.contains('}'),
                    "{locale}: a placeholder survived: {row}"
                );
            }
        }
    }

    #[test]
    fn the_tooltip_reads_as_a_sentence_in_all_six_languages() {
        // One picture of the tooltip per language, written down rather than described.
        // The tooltip is the one piece of this product's text nobody can screenshot — the
        // shell draws it, and it appears on hover — so this test is its documentation as
        // well as its regression guard.
        //
        // Three things are being checked at once: the units come from the locale file
        // (`h` is not `ч`), the percent sign sits where the language puts it (Turkish
        // writes `%88`, Chinese writes `88%` with no space), and the whole thing still
        // fits in the 127 characters Windows will show.
        let mut scoped = window("seven_day_fable", Some(88.4), 10080, true);
        scoped.model = Some("Fable".to_owned());
        scoped.detailed = true;
        let view = view(vec![
            provider("claude", vec![scoped]),
            provider("codex", vec![window("secondary", Some(70.0), 10080, true)]),
        ]);

        for (locale, expected) in [
            (
                "en",
                "Claude Fable week 88 % · Codex week 70 % (resets in 2 h 10 m)",
            ),
            (
                "tr",
                "Claude Fable hafta %88 · Codex hafta %70 (2 sa 10 dk sonra sıfırlanır)",
            ),
            (
                "zh",
                "Claude Fable 周 88% · Codex 周 70% （2 小时 10 分后重置）",
            ),
            (
                "ko",
                "Claude Fable 주 88% · Codex 주 70% (2시간 10분 후 초기화)",
            ),
            (
                "ru",
                "Claude Fable нед. 88 % · Codex нед. 70 % (сброс через 2 ч 10 мин)",
            ),
            (
                "es",
                "Claude Fable sem. 88 % · Codex sem. 70 % (se restablece en 2 h 10 min)",
            ),
        ] {
            let text = tooltip(&view, &i18n::catalog(locale));
            assert_eq!(text, expected, "the {locale} tooltip");
            assert!(
                text.chars().count() <= TOOLTIP_LIMIT,
                "the {locale} tooltip is {} characters, and Windows shows {TOOLTIP_LIMIT}",
                text.chars().count()
            );
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
    fn the_tooltip_follows_the_language_the_settings_choose_at_run_time() {
        // WP4's open risk in one test. The tooltip is drawn from `Strings`, and `Strings`
        // is swapped by `set_config` rather than fixed at start-up — so a language change
        // rewrites the tooltip without the process restarting. (The menu cannot be tested
        // here because building a `MenuItem` needs a running application; what proves it
        // is `rebuild_menu` above, and the bridge test that `set_config` calls it.)
        let strings = crate::i18n::Strings::new("en");
        let view = view(vec![provider(
            "codex",
            vec![window("secondary", Some(70.0), 10080, true)],
        )]);

        assert_eq!(
            tooltip(&view, &strings.catalog()),
            "Codex week 70 % (resets in 2 h 10 m)"
        );

        assert!(strings.set("tr"), "the language actually moved");
        assert_eq!(
            tooltip(&view, &strings.catalog()),
            "Codex hafta %70 (2 sa 10 dk sonra sıfırlanır)",
            "the same numbers, in the language the user has just chosen"
        );
    }

    #[test]
    fn a_reset_that_is_already_due_is_not_advertised_as_time_left() {
        let catalog = i18n::catalog("en");
        let mut due = window("seven_day", Some(70.0), 10080, true);
        due.remaining_ms = Some(-60_000);
        let text = tooltip(&view(vec![provider("codex", vec![due])]), &catalog);
        assert_eq!(text, "Codex week 70 %", "got {text}");
    }

    // --------------------------------------------------------------- T-WP17: the second line

    /// The fullest tooltip a machine can produce, in both places it is fullest at once.
    fn crowded() -> SnapshotView {
        let mut scoped = window("seven_day_fable", Some(88.4), 10080, true);
        scoped.model = Some("Fable".to_owned());
        scoped.detailed = true;
        view(vec![
            provider("claude", vec![scoped]),
            provider("codex", vec![window("secondary", Some(70.0), 10080, true)]),
        ])
    }

    #[test]
    fn the_week_line_reads_as_a_sentence_in_all_six_languages() {
        // The companion to the picture test above, and the same argument for writing the
        // whole string down: nobody can screenshot a tooltip, so this is its documentation.
        // Two things are being checked beyond the words — the magnitude mark is the Latin
        // one this build writes itself rather than the one `Intl` would have chosen, and the
        // model id travels raw through all six catalogues.
        //
        // The number is the six-day measurement in `docs/usage-contract.md` added up all four
        // ways — 1 514 068 891 — because that is the reading this build now produces on the
        // machine that table was measured on. T-WP17 wrote `22.3M` here, which was the same
        // week with `cache_read` left out of it.
        let view = crowded();
        let week = week_of(1_514_068_891, "claude-sonnet-5");

        for (locale, expected) in [
            (
                "en",
                "Claude Fable week 88 % · Codex week 70 % (resets in 2 h 10 m)\r\n\
                 This week 1.5B · claude-sonnet-5",
            ),
            (
                "tr",
                "Claude Fable hafta %88 · Codex hafta %70 (2 sa 10 dk sonra sıfırlanır)\r\n\
                 Bu hafta 1.5B · claude-sonnet-5",
            ),
            (
                "zh",
                "Claude Fable 周 88% · Codex 周 70% （2 小时 10 分后重置）\r\n\
                 本周 1.5B · claude-sonnet-5",
            ),
            (
                "ko",
                "Claude Fable 주 88% · Codex 주 70% (2시간 10분 후 초기화)\r\n\
                 이번 주 1.5B · claude-sonnet-5",
            ),
            (
                "ru",
                "Claude Fable нед. 88 % · Codex нед. 70 % (сброс через 2 ч 10 мин)\r\n\
                 За неделю 1.5B · claude-sonnet-5",
            ),
            (
                "es",
                "Claude Fable sem. 88 % · Codex sem. 70 % (se restablece en 2 h 10 min)\r\n\
                 Esta sem. 1.5B · claude-sonnet-5",
            ),
        ] {
            let text = super::tooltip(&view, &i18n::catalog(locale), Some(&week));
            assert_eq!(text, expected, "the {locale} tooltip");
        }
    }

    #[test]
    fn the_worst_case_a_real_machine_can_produce_still_fits_in_all_six_languages() {
        // The fullest quota line this product draws; the largest number a `u64` can be,
        // which is nine characters and five more than the four-way total of a real week
        // measured over six days; and the longest model id measured on a real machine, a
        // dated Claude one at twenty-five. If this passes, no reading a user can arrive at
        // is cut.
        //
        // T-WP20b widened the headline and this bound did not move: `headline` saturates at
        // `u64::MAX` either way, so the worst case was already being measured at the top of
        // the type and adding a fourth counter cannot reach past it.
        let view = crowded();
        let week = week_of(u64::MAX, "claude-haiku-4-5-20251001");

        for locale in ["en", "tr", "zh", "ko", "ru", "es"] {
            let text = super::tooltip(&view, &i18n::catalog(locale), Some(&week));
            let length = text.chars().count();
            assert!(
                length <= TOOLTIP_LIMIT,
                "the {locale} tooltip is {length} characters and Windows shows {TOOLTIP_LIMIT}: \
                 {text}"
            );
            assert!(
                text.contains("18446744T") && text.contains("claude-haiku-4-5-20251001"),
                "the {locale} tooltip lost part of the week line: {text}"
            );
            assert!(
                text.contains(TOOLTIP_BREAK),
                "the {locale} tooltip has no second line: {text}"
            );
            assert!(!text.ends_with('…'), "the {locale} tooltip was cut: {text}");
        }
    }

    #[test]
    fn a_week_that_would_not_fit_costs_the_reset_clause_first() {
        // A quota line long enough that the two together overflow. The countdown is the
        // least load-bearing thing on screen — the percentage is the alarm, and the panel
        // shows both — so it is what goes.
        let catalog = i18n::catalog("en");
        let long = window(&"x".repeat(70), Some(50.0), 1, true);
        let view = view(vec![provider("claude", vec![long])]);
        let week = week_of(22_345_678, "claude-sonnet-5");

        let with_reset = tooltip(&view, &catalog);
        assert!(with_reset.contains("resets in"), "got {with_reset}");

        let both = super::tooltip(&view, &catalog, Some(&week));
        assert!(both.chars().count() <= TOOLTIP_LIMIT, "got {both}");
        assert!(
            !both.contains("resets in"),
            "the countdown should have made room for the week: {both}"
        );
        assert!(
            both.ends_with("This week 22.3M · claude-sonnet-5"),
            "got {both}"
        );
    }

    #[test]
    fn a_quota_line_that_fills_the_tooltip_by_itself_keeps_it() {
        // Nothing can be dropped that would make room, so the week goes rather than half of
        // it: quota is why this application exists, and half a number is worse than none.
        // What is left is byte for byte the tooltip this build shipped before T-WP17.
        let catalog = i18n::catalog("en");
        let long = window(&"x".repeat(300), Some(50.0), 1, true);
        let view = view(vec![provider("claude", vec![long])]);

        let text = super::tooltip(
            &view,
            &catalog,
            Some(&week_of(22_345_678, "claude-sonnet-5")),
        );
        assert_eq!(
            text,
            tooltip(&view, &catalog),
            "the week line must not be cut"
        );
        assert!(text.chars().count() <= TOOLTIP_LIMIT);
        assert!(text.ends_with('…'));
        assert!(!text.contains(TOOLTIP_BREAK), "got {text}");
    }

    #[test]
    fn a_tooltip_nobody_could_read_stays_one_sentence_whatever_the_store_holds() {
        // "no data" followed by data would be two contradictory sentences. The week is the
        // second half of an answer, and there is no first half here.
        let catalog = i18n::catalog("en");
        assert_eq!(
            super::tooltip(
                &view(vec![]),
                &catalog,
                Some(&week_of(22_345_678, "claude-sonnet-5"))
            ),
            "nazar-tray: no data"
        );
    }

    #[test]
    fn the_icon_says_nothing_about_usage() {
        // Decision K25 in one assertion: the bead is the mark at every reading and grey only
        // for unknown, and a week's tokens do not change that. `IconState` takes the
        // snapshot and nothing else, so this is checked by the type — what the test adds is
        // that nobody quietly gave it a second argument.
        let view = crowded();
        let before = IconState::from_view(&view);
        let _ = super::tooltip(&view, &i18n::catalog("en"), Some(&week_of(1, "x")));
        assert_eq!(before, IconState::from_view(&view));
    }
}
