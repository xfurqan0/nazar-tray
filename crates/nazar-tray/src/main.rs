//! nazar-tray — the system-tray face of Nazar.
//!
//! WP0 wired the shell: a tray icon, a frameless popup panel that opens near the cursor
//! on either mouse button, and a panel that closes itself on Esc or when it loses focus.
//! WP1 added the first reader — Codex's quota, taken from its own session logs — and a
//! `--print` flag that dumps the `limits.json` document it produces. WP2 adds the second:
//! Claude's quota, taken from the captures the `nazar-statusline` wrapper writes. WP2b
//! adds the opt-in detailed-windows mode behind `config.detailedWindows`, off by default,
//! and `--print --detailed` to force it on for one run. The tray itself still shows a
//! static bead; wiring the readers into the icon and the panel is WP3 and WP4.
//!
//! What this binary deliberately does **not** do yet, and the package that will add it:
//!
//! | Behaviour | Package |
//! |---|---|
//! | The settings panel that turns detailed windows on, and the Max-plan offer | WP5 |
//! | Staleness, local countdown, writing `~/.nazar/limits.json` | WP3 |
//! | Bead drawn per scale factor, real panel contents | WP4 |
//! | Notifications, autostart, settings | WP5 |

// A tray app has no console. Kept for debug builds so `cargo tauri dev` still prints.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod panel;
mod tray;

use tauri::{Manager, WindowEvent};

use panel::PanelState;

fn main() {
    // Checked before anything is created: `--print` must not open a window, touch the
    // tray, or leave a process behind.
    if cli::run_if_requested() {
        return;
    }

    tauri::Builder::default()
        .setup(|app| {
            app.manage(PanelState::default());
            tray::install(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Clicking anywhere else closes the panel. This is what makes it feel like a
            // tray popup rather than a window the user has to dismiss.
            if matches!(event, WindowEvent::Focused(false)) {
                panel::on_blur(window);
            }
        })
        .run(tauri::generate_context!())
        .expect("nazar-tray could not start");
}
