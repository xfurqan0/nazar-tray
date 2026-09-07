//! Showing, placing and hiding the popup panel.
//!
//! The panel is an ordinary Tauri window that is frameless, always on top, kept out of
//! the taskbar and hidden by default. Making it behave like a tray popup takes three
//! things: place it beside the cursor, hide it when it loses focus, and treat the click
//! that both blurred it and hit the tray as one gesture rather than two.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, Window};

/// Window label; must match the `label` in `tauri.conf.json`.
pub const PANEL_LABEL: &str = "panel";

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

/// The one piece of state WP0 needs: when the panel last hid itself.
#[derive(Default)]
pub struct PanelState {
    last_hidden: Mutex<Option<Instant>>,
}

impl PanelState {
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
    // wherever it last was instead.
    let _ = place(&window, cursor);
    let _ = window.show();
    let _ = window.set_focus();
}

/// Open the panel wherever the cursor is.
///
/// What a second launch of the application gets instead of a second tray icon: the refresh
/// loop notices the marker that launch left behind and calls this. Safe from the loop's own
/// thread — every call goes through the `AppHandle`, which queues onto the event loop.
pub fn show(app: &AppHandle) {
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    // A cursor position we cannot read is not a reason to refuse to open: the panel then
    // appears wherever it last was, which is still an answer.
    if let Ok(cursor) = app.cursor_position() {
        let _ = place(&window, cursor);
    }
    let _ = window.show();
    let _ = window.set_focus();
}

/// Hide the panel and remember when, so the next tray click reads as a toggle.
pub fn hide(window: &WebviewWindow, state: &PanelState) {
    let _ = window.hide();
    state.mark_hidden();
}

/// Window-event handler: the panel closes as soon as it loses focus.
pub fn on_blur(window: &Window) {
    if window.label() != PANEL_LABEL {
        return;
    }
    let _ = window.hide();
    window.state::<PanelState>().mark_hidden();
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
}
