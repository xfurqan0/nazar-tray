//! Showing, placing, sizing and hiding the popup panel.
//!
//! The panel is an ordinary Tauri window that is frameless, always on top, kept out of
//! the taskbar and hidden by default. Making it behave like a tray popup takes four
//! things: place it beside the cursor, size it to whatever it turns out to contain, hide
//! it when it loses focus, and get two focus races right.
//!
//! **The two races**, because they are the whole reason this file is not ten lines:
//!
//! 1. *The click that closes.* Clicking the tray while the panel is open blurs the panel
//!    first, which hides it, and the click then arrives at the tray and would reopen it.
//!    [`TOGGLE_GRACE`] makes those two events one gesture.
//! 2. *The overflow flyout.* Windows 11 keeps new tray icons behind the `^` button, and a
//!    click **inside that flyout** shows the panel and then closes the flyout — which moves
//!    the focus and blurs the panel about 200 ms later. The panel would vanish the instant
//!    it appeared, and only for users who have not pinned the icon, which is most of them
//!    on the first run. WP0's report measured it; [`OPEN_GRACE`] is the fix, and it takes
//!    the focus back rather than merely ignoring the blur, so the *next* click elsewhere
//!    still closes the panel.
//!
//! **The first of those four things is not available everywhere.** A Wayland client cannot
//! read the global pointer and cannot move its own toplevel, and neither call fails loudly:
//! `cursor_position()` answers `Ok((0, 0))` and `set_position` answers `Ok(())` and does
//! nothing. Placing the panel from those two answers puts it at the top-left corner of the
//! arithmetic and in the middle of the screen in fact — which is what a Fedora 44 GNOME
//! session showed on 2026-09-16, and what [`crate::desktop::window_placement`] now asks
//! about before this module does any of it. Where the answer is no, [`place`] is not called
//! at all: an ordinary centred window is an honest outcome, and one that lands in a
//! computed position that was never honoured is a bug with extra steps. What replaces the
//! popup on Linux is the tray menu, which the shell does place beside the icon — see
//! [`crate::tray`].

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, Window};

/// Window label; must match the `label` in `tauri.conf.json`.
pub const PANEL_LABEL: &str = "panel";

/// The panel's width in CSS pixels. The stylesheet is designed for exactly this.
pub const PANEL_WIDTH: f64 = 360.0;
/// Smallest height the panel is ever given, in CSS pixels.
const MIN_HEIGHT: f64 = 140.0;
/// Largest height the panel is ever given, in CSS pixels. A webview that miscalculates
/// must not be able to open a window taller than the screen.
const MAX_HEIGHT: f64 = 720.0;

/// Distance between the cursor and the nearest edge of the panel.
const CURSOR_GAP: i32 = 12;
/// Smallest gap left between the panel and the edge of the monitor.
const EDGE_MARGIN: i32 = 8;
/// How long after a blur-hide a tray click still counts as "close", not "open".
///
/// Clicking the tray while the panel is open fires blur first, which hides the panel;
/// without this grace period the click would immediately reopen it and the icon would
/// look like it does not toggle.
const TOGGLE_GRACE: Duration = Duration::from_millis(250);
/// How long after a tray-initiated open a blur is ignored rather than obeyed.
///
/// 300 ms against the ~200 ms the Windows 11 overflow flyout takes to close, measured in
/// WP0. Long enough to cover it, short enough that a user who clicks the tray and then
/// immediately clicks elsewhere still sees the panel close.
const OPEN_GRACE: Duration = Duration::from_millis(300);

/// What the window layer remembers between events.
#[derive(Default)]
pub struct PanelState {
    last_hidden: Mutex<Option<Instant>>,
    opened_at: Mutex<Option<Instant>>,
    /// Where the panel was last opened from, so a resize can keep that corner still.
    anchor: Mutex<Option<PhysicalPosition<f64>>>,
    /// `--scale`: the device scale factor forced on the webview, for screenshots.
    scale: Option<f64>,
    /// `--demo`: the panel stays open when it loses focus.
    ///
    /// Only ever true on a screenshot run. Taking a picture of a window means running a
    /// capture tool, and a capture tool takes the focus; a panel that hid itself first
    /// would be unphotographable.
    sticky: bool,
    /// Whether this desktop honours a position at all.
    ///
    /// [`crate::desktop::window_placement`]'s answer, taken once at start-up because it
    /// cannot change while the process runs: it is the windowing protocol GTK opened with.
    places: bool,
}

impl PanelState {
    /// The state, with the screenshot overrides if any were asked for.
    #[must_use]
    pub fn new(scale: Option<f64>, sticky: bool, places: bool) -> Self {
        PanelState {
            scale,
            sticky,
            places,
            ..PanelState::default()
        }
    }

    fn mark_hidden(&self) {
        // A poisoned lock must not take the tray down with it; the value is advisory.
        *self
            .last_hidden
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = Some(Instant::now());
    }

    fn hidden_just_now(&self) -> bool {
        self.last_hidden
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .is_some_and(|at| at.elapsed() < TOGGLE_GRACE)
    }

    fn mark_opened(&self, cursor: Option<PhysicalPosition<f64>>) {
        *self.opened_at.lock().unwrap_or_else(|err| err.into_inner()) = Some(Instant::now());
        if let Some(cursor) = cursor {
            *self.anchor.lock().unwrap_or_else(|err| err.into_inner()) = Some(cursor);
        }
    }

    fn opened_just_now(&self) -> bool {
        self.opened_at
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .is_some_and(|at| at.elapsed() < OPEN_GRACE)
    }

    fn anchor(&self) -> Option<PhysicalPosition<f64>> {
        *self.anchor.lock().unwrap_or_else(|err| err.into_inner())
    }
}

/// Open the panel beside `cursor`, or close it if it is already open.
pub fn toggle_near(app: &AppHandle, cursor: PhysicalPosition<f64>) {
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    let state = app.state::<PanelState>();

    if state.hidden_just_now() {
        // The same click blurred the panel a moment ago. That was the close.
        return;
    }
    if window.is_visible().unwrap_or(false) {
        hide(&window, &state);
        return;
    }

    // A failure to place the window is not a reason to refuse to show it; it opens
    // wherever it last was instead. On a desktop that does not honour a position the call
    // is not made at all — see the module note.
    if state.places {
        let _ = place(&window, cursor);
    }
    state.mark_opened(Some(cursor));
    let _ = window.show();
    let _ = window.set_focus();
}

/// Open the panel wherever the cursor is.
///
/// What a second launch of the application gets instead of a second tray icon: the refresh
/// loop notices the marker that launch left behind and calls this. Also the tray menu's
/// first item and the `open_panel` command. Safe from any thread — every call goes through
/// the `AppHandle`, which queues onto the event loop.
pub fn show(app: &AppHandle) {
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    // A cursor position we cannot read is not a reason to refuse to open: the panel then
    // appears wherever it last was, which is still an answer. On Wayland it is worse than
    // unreadable — it reads `Ok((0, 0))` — so the question is asked of the desktop rather
    // than of the return value.
    let state = app.state::<PanelState>();
    let cursor = state.places.then(|| app.cursor_position().ok()).flatten();
    if let Some(cursor) = cursor {
        let _ = place(&window, cursor);
    }
    state.mark_opened(cursor);
    let _ = window.show();
    let _ = window.set_focus();
}

/// Hide the panel and remember when, so the next tray click reads as a toggle.
pub fn hide(window: &WebviewWindow, state: &PanelState) {
    let _ = window.hide();
    state.mark_hidden();
}

/// Window-event handler: the panel closes as soon as it loses focus.
///
/// Except in the first [`OPEN_GRACE`] after a tray-initiated open, where the blur is the
/// overflow flyout closing behind us rather than the user looking elsewhere. Taking the
/// focus back is what keeps the *next* blur meaningful — ignoring the event alone would
/// leave a panel nobody can dismiss.
pub fn on_blur(window: &Window) {
    if window.label() != PANEL_LABEL {
        return;
    }
    let state = window.state::<PanelState>();
    if state.sticky {
        return;
    }
    if state.opened_just_now() {
        let _ = window.set_focus();
        return;
    }
    let _ = window.hide();
    state.mark_hidden();
}

/// Give the panel a window that fits the content it just measured.
///
/// Called by the `set_panel_height` command after every render. The width never changes —
/// the stylesheet is written for [`PANEL_WIDTH`] — and the height is clamped, so the worst
/// a confused webview can do is ask for a window that is merely wrong rather than one that
/// covers the screen.
pub fn resize(app: &AppHandle, css_height: f64) {
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    if !css_height.is_finite() {
        return;
    }
    let state = app.state::<PanelState>();
    let wanted = physical(
        &window,
        state.scale,
        css_height.clamp(MIN_HEIGHT, MAX_HEIGHT),
    );
    if window.outer_size().is_ok_and(|size| size == wanted) {
        return;
    }
    let _ = window.set_size(wanted);
    pin(&window, wanted);

    // The panel is anchored above the cursor, so growing it downwards would push it over
    // the taskbar. Re-placing keeps the bottom edge where the user pointed. Where nothing
    // was placed there is no anchor to keep still, and the window manager has already
    // decided where a window of the new size goes.
    if state.places && window.is_visible().unwrap_or(false) {
        if let Some(anchor) = state.anchor() {
            let _ = place(&window, anchor);
        }
    }
}

/// Hold the window at the size it was just given, because GTK will not otherwise.
///
/// **A measurement, not a precaution.** `tauri.conf.json` says `resizable: false`, and on
/// GTK that is not a hint: `tao 0.35.3` sends every `set_size` to `gtk_window_resize`, which
/// is documented to do nothing on a window whose resizable flag is off. The window is then
/// sized by its *size request* instead, and the size request of a window whose only child is
/// a `WebKitWebView` is the natural height of the document — so on 2026-09-17 the panel on a
/// Fedora 44 session was **1105 physical pixels tall** with 480 pixels of content in it,
/// measured through `xprop`/`import` on the real window, and it never shrank back when a
/// shorter view replaced a taller one. Every `set_size` this file makes had been silently
/// discarded since the first Linux build: `MIN_HEIGHT`, `MAX_HEIGHT` and the whole
/// measure-and-ask arrangement were Windows-only behaviour without anybody choosing that.
///
/// The door GTK does leave open on a fixed window is the geometry hint. `set_min_size` and
/// `set_max_size` reach `gtk_window_set_geometry_hints` with `MIN_SIZE | MAX_SIZE`, and a
/// minimum equal to the maximum *is* the size — the window manager resizes to it and the
/// natural size stops mattering. Pinning both also keeps the window unresizable by the
/// user's own gestures, which is what `resizable: false` was asked for in the first place.
///
/// Windows and macOS compile none of this: `set_size` works there, and adding constraints
/// would be a second authority over a size that already has one.
#[allow(unused_variables)]
fn pin(window: &WebviewWindow, wanted: PhysicalSize<u32>) {
    #[cfg(target_os = "linux")]
    {
        let _ = window.set_resizable(true);
        let _ = window.set_min_size(Some(wanted));
        let _ = window.set_max_size(Some(wanted));
    }
}

/// Apply `--scale` to the window before it is ever shown.
///
/// The webview is told to render at that device pixel ratio through
/// `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, which only changes what one CSS pixel is worth;
/// the window itself is still the size it was created at, so it has to be multiplied too or
/// the panel is drawn at 200 % into a 100 % window and cropped.
pub fn apply_scale(app: &AppHandle, scale: f64) {
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    let height = window
        .outer_size()
        .ok()
        .map_or(MIN_HEIGHT, |size| f64::from(size.height));
    let _ = window.set_size(PhysicalSize::new(
        (PANEL_WIDTH * scale).round() as u32,
        (height * scale).round() as u32,
    ));
}

/// CSS pixels to physical pixels, honouring the `--scale` override when there is one.
fn physical(window: &WebviewWindow, scale: Option<f64>, css_height: f64) -> PhysicalSize<u32> {
    let factor = scale.unwrap_or_else(|| window.scale_factor().unwrap_or(1.0));
    PhysicalSize::new(
        (PANEL_WIDTH * factor).round() as u32,
        (css_height * factor).round() as u32,
    )
}

/// Put the panel next to the cursor, kept inside the monitor the cursor is on.
fn place(window: &WebviewWindow, cursor: PhysicalPosition<f64>) -> tauri::Result<()> {
    let size = window.outer_size()?;
    let cursor_x = cursor.x.round() as i32;
    let cursor_y = cursor.y.round() as i32;

    // Centred on the cursor and above it: on Windows the tray sits at the bottom right,
    // so "above the cursor" is where a tray popup belongs.
    let mut x = cursor_x - size.width as i32 / 2;
    let mut y = cursor_y - size.height as i32 - CURSOR_GAP;

    if let Some((origin, bounds)) = monitor_bounds(window, cursor)? {
        let (x_clamped, y_clamped) = clamp_to_monitor((x, y), size, origin, bounds);
        x = x_clamped;
        y = y_clamped;
        // A taskbar at the top of the screen leaves no room above the cursor.
        if y < origin.y + EDGE_MARGIN {
            y = cursor_y + CURSOR_GAP;
        }
    }

    window.set_position(PhysicalPosition::new(x, y))
}

/// Keep a window of `size` inside a monitor at `origin` of `bounds`, leaving a margin.
fn clamp_to_monitor(
    (x, y): (i32, i32),
    size: PhysicalSize<u32>,
    origin: PhysicalPosition<i32>,
    bounds: PhysicalSize<u32>,
) -> (i32, i32) {
    let min_x = origin.x + EDGE_MARGIN;
    let min_y = origin.y + EDGE_MARGIN;
    // On a monitor smaller than the panel the maxima fall below the minima; clamping the
    // maximum up first keeps `clamp` from panicking and pins the panel to the top left.
    let max_x = (origin.x + bounds.width as i32 - size.width as i32 - EDGE_MARGIN).max(min_x);
    let max_y = (origin.y + bounds.height as i32 - size.height as i32 - EDGE_MARGIN).max(min_y);
    (x.clamp(min_x, max_x), y.clamp(min_y, max_y))
}

/// Position and size of the monitor the cursor is on, falling back to the primary one.
fn monitor_bounds(
    window: &WebviewWindow,
    cursor: PhysicalPosition<f64>,
) -> tauri::Result<Option<(PhysicalPosition<i32>, PhysicalSize<u32>)>> {
    let cursor_x = cursor.x.round() as i32;
    let cursor_y = cursor.y.round() as i32;

    for monitor in window.available_monitors()? {
        let origin = *monitor.position();
        let size = *monitor.size();
        let inside_x = cursor_x >= origin.x && cursor_x < origin.x + size.width as i32;
        let inside_y = cursor_y >= origin.y && cursor_y < origin.y + size.height as i32;
        if inside_x && inside_y {
            return Ok(Some((origin, size)));
        }
    }
    Ok(window
        .primary_monitor()?
        .map(|monitor| (*monitor.position(), *monitor.size())))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: PhysicalSize<u32> = PhysicalSize {
        width: 1920,
        height: 1080,
    };
    const ORIGIN: PhysicalPosition<i32> = PhysicalPosition { x: 0, y: 0 };
    const PANEL: PhysicalSize<u32> = PhysicalSize {
        width: 340,
        height: 300,
    };

    #[test]
    fn a_panel_near_the_tray_stays_on_screen() {
        // Cursor in the bottom-right corner, where the Windows tray lives.
        let wanted = (
            1900 - PANEL.width as i32 / 2,
            1060 - PANEL.height as i32 - 12,
        );
        let (x, y) = clamp_to_monitor(wanted, PANEL, ORIGIN, MONITOR);

        assert!(x >= EDGE_MARGIN);
        assert!(x + PANEL.width as i32 <= MONITOR.width as i32 - EDGE_MARGIN);
        assert!(y + PANEL.height as i32 <= MONITOR.height as i32 - EDGE_MARGIN);
    }

    #[test]
    fn a_second_monitor_left_of_the_first_keeps_its_own_origin() {
        let origin = PhysicalPosition { x: -1920, y: 0 };
        let (x, _) = clamp_to_monitor((-2000, 400), PANEL, origin, MONITOR);
        assert_eq!(x, origin.x + EDGE_MARGIN);
    }

    #[test]
    fn a_monitor_smaller_than_the_panel_does_not_panic() {
        let tiny = PhysicalSize {
            width: 200,
            height: 200,
        };
        let (x, y) = clamp_to_monitor((5000, 5000), PANEL, ORIGIN, tiny);
        assert_eq!((x, y), (EDGE_MARGIN, EDGE_MARGIN));
    }

    #[test]
    fn a_fresh_panel_has_not_just_hidden() {
        let state = PanelState::default();
        assert!(!state.hidden_just_now());
        state.mark_hidden();
        assert!(state.hidden_just_now());
    }

    #[test]
    fn a_panel_opened_from_the_tray_survives_the_flyout_closing_behind_it() {
        let state = PanelState::default();
        assert!(
            !state.opened_just_now(),
            "a panel nobody opened has no grace period"
        );

        state.mark_opened(Some(PhysicalPosition {
            x: 2400.0,
            y: 1400.0,
        }));
        assert!(
            state.opened_just_now(),
            "the blur that arrives while the overflow flyout closes must not hide the panel"
        );
        assert_eq!(
            state.anchor(),
            Some(PhysicalPosition {
                x: 2400.0,
                y: 1400.0
            }),
            "where it was opened from is what a resize re-places it against"
        );
    }

    #[test]
    fn the_grace_period_covers_the_race_wp0_measured() {
        // WP0's report: a click from the Windows 11 overflow flyout blurs the panel about
        // 200 ms later. A grace period shorter than that fixes nothing.
        assert!(
            OPEN_GRACE >= Duration::from_millis(300),
            "the measured race is ~200 ms; 300 ms is the margin the plan asked for"
        );
        assert!(
            OPEN_GRACE > TOGGLE_GRACE,
            "opening has to outlast the toggle window, or a tray click would reopen \
             what it just closed"
        );
    }

    #[test]
    fn opening_without_a_cursor_still_starts_the_grace_period() {
        // A machine that cannot report a cursor position still opens the panel — it just
        // opens it where it last was, and the flyout race applies exactly the same.
        let state = PanelState::new(Some(1.5), true, true);
        state.mark_opened(None);
        assert!(state.opened_just_now());
        assert_eq!(state.anchor(), None);
        assert_eq!(state.scale, Some(1.5));
        assert!(state.sticky, "a screenshot run keeps the panel on screen");
        assert!(
            !PanelState::default().sticky,
            "the shipped panel always hides on blur"
        );
    }

    /// A desktop that does not honour a position gets no arithmetic done on its behalf.
    ///
    /// The anchor is what `resize` re-places from, and on Wayland there is nothing to
    /// re-place: remembering a corner that was never honoured would make the panel jump
    /// every time the content changed height, which is worse than a window that simply
    /// stays where the compositor put it.
    #[test]
    fn a_desktop_that_places_nothing_remembers_no_anchor() {
        let state = PanelState::new(None, false, false);
        assert!(!state.places);
        state.mark_opened(None);
        assert_eq!(state.anchor(), None);
        assert!(
            state.opened_just_now(),
            "the grace period is about focus, not about position"
        );
    }
}
