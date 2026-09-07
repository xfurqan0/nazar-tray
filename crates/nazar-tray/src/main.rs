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
//! **WP4 gives it a face.** The tray icon is a bead drawn in Rust for the current scale
//! factor and filled by the binding window; the panel is designed rather than dumped; the
//! menu carries Quit, so the advisory lock is released on the way out instead of being left
//! behind by a killed process.
//!
//! What this binary deliberately does **not** do yet, and the package that will add it:
//!
//! | Behaviour | Package |
//! |---|---|
//! | Notifications, autostart, the settings panel, the Max-plan offer | WP5 |
//! | The Windows UI language, and ZH/KO/RU/ES | WP5, WP6 |

// A tray app has no console. Kept for debug builds so `cargo tauri dev` still prints.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod demo;
mod i18n;
mod icon;
mod panel;
mod state;
mod tray;

use std::sync::{Arc, Mutex};

use nazar_core::clock::{Clock, SystemClock};
use nazar_core::lock::{Acquisition, LimitsLock};
use nazar_core::refresh::{self, Engine, Event, ReaderSet, Warnings};
use nazar_core::{Config, paths};
use tauri::{Emitter, Manager, RunEvent, WindowEvent};

use i18n::Catalog;
use panel::PanelState;
use state::{AppState, SNAPSHOT_CHANGED, UiState};

fn main() {
    // Checked before anything is created: `--print` and `--icons` must not open a window,
    // touch the tray, or leave a process behind.
    if cli::run_if_requested() {
        return;
    }
    let options = cli::options();
    force_device_scale_factor(options.scale);

    let clock = SystemClock;
    let now = clock.now_rfc3339();

    // Settings that cannot be read are settings that were never written: the defaults. A
    // damaged `config.json` is not a reason to refuse to start, and certainly not a reason
    // to turn the opt-in mode on.
    let config = Config::load().unwrap_or_default();
    // WP5 reads the Windows UI language; until then the settings file is the only opinion,
    // `--locale` overrides it for one run, and English is the fallback. The panel makes the
    // same choice from the same values, so the tooltip and the panel are never in two
    // different languages.
    let language = options
        .locale
        .clone()
        .or_else(|| config.locale.clone())
        .unwrap_or_else(|| "en".to_owned());
    let strings = Arc::new(i18n::catalog(&language));

    let mut ui = UiState::from_config(&config);
    ui.demo = options.demo;
    if let Some(theme) = options.theme.clone() {
        ui.theme = theme;
    }
    if let Some(mode) = options.mode.clone() {
        ui.mode = mode;
    }
    if let Some(hint) = options.hint {
        ui.hint_dismissed = !hint;
    }
    if options.locale.is_some() {
        ui.locale = options.locale.clone();
    }
    // A run that was told what to look like does not get to remember it: the screenshot
    // flags must leave the maintainer's settings exactly as they found them.
    let persist = options.may_persist();

    // `--demo` never claims the lock, so a screenshot session cannot become the writer and
    // cannot overwrite the real `~/.nazar/limits.json` with invented numbers.
    let lock = if options.demo {
        None
    } else {
        match claim_the_writer_role(&now) {
            Instance::Only(lock) => lock,
            Instance::Second => {
                // A tray is already running and has been asked to show itself. Saying so is
                // for a terminal; from Explorer nobody sees it, and the panel opening is
                // the answer.
                eprintln!("nazar-tray is already running; asked it to show its panel");
                return;
            }
        }
    };

    let limits_path = paths::limits_path().unwrap_or_default();
    let request_path = paths::request_path().unwrap_or_default();
    let readers = ReaderSet::discover(config.detailed_windows);
    let rules = config.rules();

    let setup_strings = Arc::clone(&strings);
    let event_strings = Arc::clone(&strings);
    let application = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            state::get_snapshot,
            state::get_warnings,
            state::refresh_now,
            state::get_ui_state,
            state::set_theme,
            state::dismiss_hint,
            state::set_panel_height,
            state::open_panel,
            state::quit
        ])
        .setup(move |app| {
            app.manage(PanelState::new(options.scale, options.demo));

            // The state is managed before the tray is built, because the first thing the
            // tray does is ask it what to draw.
            let shared = if options.demo {
                AppState::new(
                    Arc::new(Mutex::new(demo::snapshot(&now))),
                    Arc::new(Mutex::new(Warnings::default())),
                    rules,
                    ui,
                    persist,
                )
            } else {
                let handle = app.handle().clone();
                let loop_strings = Arc::clone(&setup_strings);
                let engine = Engine::new(readers, limits_path, lock, rules, &now)
                    .with_request_file(request_path)
                    .on_event(move |event| on_loop_event(&handle, &loop_strings, event));

                let shared =
                    AppState::new(engine.snapshot(), engine.diagnostics(), rules, ui, persist);
                shared.attach(refresh::spawn(engine, Arc::new(SystemClock))?);
                shared
            };
            app.manage(shared);

            tray::install(app.handle(), &setup_strings)?;
            tray::refresh(app.handle(), &setup_strings);

            if let Some(scale) = options.scale {
                panel::apply_scale(app.handle(), scale);
            }
            if options.demo {
                // Nothing else would ever open it: a screenshot run has no user to click.
                panel::show(app.handle());
            }
            Ok(())
        })
        .on_window_event(move |window, event| match event {
            // Clicking anywhere else closes the panel. This is what makes it feel like a
            // tray popup rather than a window the user has to dismiss.
            WindowEvent::Focused(false) => panel::on_blur(window),
            // The bead is drawn for one scale factor. Moving the window to a display with
            // another one, or changing the display's scaling, makes that drawing wrong.
            WindowEvent::ScaleFactorChanged { .. } => {
                tray::refresh(window.app_handle(), &event_strings);
            }
            _ => {}
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

/// Tell WebView2 to render at a scale factor of our choosing, for the screenshots.
///
/// `--force-device-scale-factor` is a Chromium switch, and WebView2 takes extra switches
/// through this environment variable. It has to be set before the webview environment is
/// created — which is why this is the second thing `main` does — and it is the only way to
/// photograph a 150 % panel on a 100 % display without changing the maintainer's display
/// settings for the duration.
///
/// `set_var` is unsafe in the 2024 edition because another thread could be reading the
/// environment. Nothing else has started yet: this is the beginning of `main`.
fn force_device_scale_factor(scale: Option<f64>) {
    let Some(scale) = scale else {
        return;
    };
    unsafe {
        std::env::set_var(
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
            format!("--force-device-scale-factor={scale}"),
        );
    }
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
fn on_loop_event(app: &tauri::AppHandle, strings: &Catalog, event: Event) {
    match event {
        // The panel re-reads the snapshot; the payload would be stale by the time it drew.
        // The icon and the tooltip are redrawn here rather than in the panel, because they
        // have to be right whether or not anybody has opened it.
        Event::SnapshotChanged => {
            let _ = app.emit(SNAPSHOT_CHANGED, ());
            tray::refresh(app, strings);
        }
        Event::ShowRequested => panel::show(app),
    }
}
