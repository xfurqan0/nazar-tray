//! The tray icon.
//!
//! WP0 ships a static bead. WP4 replaces the image with one drawn in Rust per scale
//! factor, filled from the bottom by the binding window, so the theme has a single
//! source of truth and 100 / 150 / 200 % all render correctly (decision K3).
//!
//! There is deliberately no context menu: both mouse buttons open the panel, and the
//! panel is where every action will live. A menu on right-click would put the same
//! commands in two places and make the right button feel different from the left.

use tauri::AppHandle;
use tauri::image::Image;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

/// Identifier of the one tray icon this app owns.
pub const TRAY_ID: &str = "nazar-tray";

/// Placeholder bead, generated from `ui/assets/bead.svg` by `scripts/render-bead-png.mjs`.
const ICON_PNG: &[u8] = include_bytes!("../icons/128x128.png");

/// Create the tray icon and hook both mouse buttons up to the panel.
pub fn install(app: &AppHandle) -> tauri::Result<()> {
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(ICON_PNG)?)
        // The bead is a coloured mark, not a silhouette; letting macOS recolour it as a
        // template would throw away the signal. (macOS also needs form, not just colour,
        // which is why WP4 adds the number. See the plan, section 1.3.)
        .icon_as_template(false)
        .tooltip("nazar-tray")
        // No menu exists, and the left button must not wait for one.
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            // Both buttons do the same thing, and act on release so a click that started
            // elsewhere does not count. The middle button is deliberately left free: WP5
            // has a use for it, and binding it now would spend it on a duplicate.
            if let TrayIconEvent::Click {
                button: MouseButton::Left | MouseButton::Right,
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
