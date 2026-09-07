//! nazar-tray — the system-tray face of Nazar.
//!
//! WP0 wired the shell: a tray icon, a frameless popup panel that opens near the cursor on
//! either mouse button, and a panel that closes itself on Esc or when it loses focus. WP1
//! added the first reader — Codex's quota, taken from its own session logs — and a `--print`
//! flag that dumps the `limits.json` document it produces. WP2 added the second: Claude's
//! quota, taken from the captures the `nazar-statusline` wrapper writes. WP2b added the
//! opt-in detailed-windows mode behind `config.detailedWindows`, off by default.
//!
//! **WP3 is what turns those readers into an application.** One background thread owns
//! them, refreshes on a schedule that notices file changes and sleeping laptops, writes
//! `~/.nazar/limits.json` when — and only when — the numbers have moved, and holds an
//! advisory lock so that exactly one process ever does. A second launch does not start a
//! second tray: it asks the first one to show its panel and leaves.
//!
//! ```text
//!   main ──▶ claim ~/.nazar/limits.lock
//!              │
//!              ├── ours    ─▶ tray + panel + refresh loop (the writer)
//!              └── taken   ─▶ ask the running instance to show itself, and exit
//! ```
//!
//! What this binary deliberately does **not** do yet, and the package that will add it:
//!
//! | Behaviour | Package |
//! |---|---|
//! | Bead drawn per scale factor, designed panel | WP4 |
//! | Notifications, autostart, the settings panel, the Max-plan offer | WP5 |

// A tray app has no console. Kept for debug builds so `cargo tauri dev` still prints.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod panel;
mod state;
mod tray;

use std::sync::Arc;

use nazar_core::clock::{Clock, SystemClock};
use nazar_core::lock::{Acquisition, LimitsLock};
use nazar_core::refresh::{self, Engine, Event, ReaderSet};
use nazar_core::{Config, paths};
use tauri::{Emitter, Manager, RunEvent, WindowEvent};

use panel::PanelState;
use state::{AppState, SNAPSHOT_CHANGED};

fn main() {
    // Checked before anything is created: `--print` must not open a window, touch the tray,
    // or leave a process behind.
    if cli::run_if_requested() {
        return;
    }

    let clock = SystemClock;
    let now = clock.now_rfc3339();

    // Settings that cannot be read are settings that were never written: the defaults. A
    // damaged `config.json` is not a reason to refuse to start, and certainly not a reason
    // to turn the opt-in mode on.
    let config = Config::load().unwrap_or_default();

    let lock = match claim_the_writer_role(&now) {
        Instance::Only(lock) => lock,
        Instance::Second => {
            // A tray is already running and has been asked to show itself. Saying so is
            // for a terminal; from Explorer nobody sees it, and the panel opening is the
            // answer.
            eprintln!("nazar-tray is already running; asked it to show its panel");
            return;
        }
    };

    let limits_path = paths::limits_path().unwrap_or_default();
    let request_path = paths::request_path().unwrap_or_default();
    let readers = ReaderSet::discover(config.detailed_windows);
    let rules = config.rules();

    let application = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            state::get_snapshot,
            state::get_warnings,
            state::refresh_now
        ])
        .setup(move |app| {
            app.manage(PanelState::default());
            tray::install(app.handle())?;

            let handle = app.handle().clone();
            let engine = Engine::new(readers, limits_path, lock, rules, &now)
                .with_request_file(request_path)
                .on_event(move |event| on_loop_event(&handle, event));

            let shared = AppState::new(engine.snapshot(), engine.diagnostics(), rules);
            shared.attach(refresh::spawn(engine, Arc::new(SystemClock))?);
            app.manage(shared);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Clicking anywhere else closes the panel. This is what makes it feel like a
            // tray popup rather than a window the user has to dismiss.
            if matches!(event, WindowEvent::Focused(false)) {
                panel::on_blur(window);
            }
        })
        .build(tauri::generate_context!())
        .expect("nazar-tray could not start");

    application.run(|app, event| {
        if matches!(event, RunEvent::Exit) {
            // Stop the loop before the process goes, so it releases the advisory lock. A
            // lock left behind would make the next launch wait out its grace period as a
            // reader, which looks exactly like the tray not working.
            if let Some(state) = app.try_state::<AppState>() {
                state.shutdown();
            }
        }
    });
}

/// What claiming the writer's role told us.
enum Instance {
    /// This process is the tray. `Some` when it may also write `limits.json`.
    Only(Option<LimitsLock>),
    /// A tray is already running, and has been asked to show its panel.
    Second,
}

/// Take the advisory lock, or defer to whoever has it.
///
/// The lock is the single-instance guard as well as the single-writer guard, and it is one
/// mechanism rather than two on purpose:
///
/// * A **named mutex** would be Windows-only, and would still leave the second process with
///   no way to reach the first one's window.
/// * **`tauri-plugin-single-instance`** does both, and would be the obvious answer if this
///   were only about instances — but it is a new dependency for a guarantee `limits.json`
///   needs anyway, and on Linux it brings a D-Bus stack with it.
/// * The lock file is already required by the contract ("one writer, many readers"), works
///   the same on all three target platforms, and costs nothing: `create_new` is atomic, so
///   of two processes racing for it exactly one wins. Finding B01 of the audit was a lock
///   whose check and whose write were separate operations, and two refreshers passing that
///   check together is what produced the observed HTTP 429.
///
/// Reaching the running instance is a marker file it is already watching: see
/// [`nazar_core::paths::request_path`].
fn claim_the_writer_role(now: &str) -> Instance {
    let Ok(path) = paths::lock_path() else {
        // No home directory. An unusual environment, and the honest behaviour is to run,
        // show whatever can be read, and write nothing.
        return Instance::Only(None);
    };

    match LimitsLock::acquire(&path, now) {
        Ok(Acquisition::Held(lock)) => Instance::Only(Some(lock)),
        Ok(Acquisition::Taken(_)) => {
            if let Ok(request) = paths::request_path() {
                let _ = refresh::place_request(&request, now);
            }
            Instance::Second
        }
        // The lock could not be taken for a reason that is not "somebody has it": a
        // read-only home directory, say. Starting without writing beats not starting.
        Err(_) => Instance::Only(None),
    }
}

/// Hand a loop event to the window layer. Called on the loop's own thread.
fn on_loop_event(app: &tauri::AppHandle, event: Event) {
    match event {
        // The panel re-reads the snapshot; the payload would be stale by the time it drew.
        Event::SnapshotChanged => {
            let _ = app.emit(SNAPSHOT_CHANGED, ());
        }
        Event::ShowRequested => panel::show(app),
    }
}
