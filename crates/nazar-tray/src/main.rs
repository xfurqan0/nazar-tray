//! nazar-tray — the system-tray face of Nazar.
//!
//! WP0 wires the shell and nothing else: a tray icon, a frameless popup panel that opens
//! near the cursor on either mouse button, and a panel that closes itself on Esc or when
//! it loses focus. There is no reader, no state model and no writer yet; the panel says
//! so in as many words.
//!
//! What this binary deliberately does **not** do, and the package that will add it:
//!
//! | Behaviour | Package |
//! |---|---|
//! | Read Codex `rollout-*.jsonl` | WP1 |
//! | Read Claude's status-line capture | WP2 |
//! | Detailed windows from the usage endpoint (opt-in) | WP2b |
//! | Binding window, staleness, `limits.json` writing | WP3 |
//! | Bead drawn per scale factor, real panel contents | WP4 |
//! | Notifications, autostart, settings | WP5 |

// A tray app has no console. Kept for debug builds so `cargo tauri dev` still prints.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod panel;
mod tray;

use tauri::{Manager, WindowEvent};

use panel::PanelState;

fn main() {
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
