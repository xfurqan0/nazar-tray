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
//! **WP5 makes it something you can leave running.** Three things, and the first is the
//! reason a quota tray exists at all:
//!
//! 1. **It warns you before you hit the wall.** Every refresh is measured against the
//!    thresholds; a window that crosses one produces a toast, once, and the key is written
//!    to `alerts.json` so that a restart does not repeat it. [`nazar_core::alerts`] is the
//!    rule and [`alerts`] is the sentence.
//! 2. **It can start with Windows**, hidden, through `tauri-plugin-autostart` — and the
//!    switch reads its state back from the registry rather than from our own settings file,
//!    so it agrees with what Task Manager's Startup tab shows.
//! 3. **It has settings**, in the panel: language, theme, thresholds, quiet hours, which
//!    providers are read, the opt-in detailed mode and its one-time offer, and where every
//!    file lives. Changing the language rebuilds the tray menu without a restart, which is
//!    the open risk WP4 wrote down and left.
//!
//! What this binary deliberately does **not** do yet, and the package that will add it:
//!
//! | Behaviour | Package |
//! |---|---|
//! | ZH, KO, RU and ES | WP6 |
//! | Installer, winget manifest, signed updates | WP7 |

// A tray app has no console. Kept for debug builds so `cargo tauri dev` still prints.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alerts;
mod cli;
mod demo;
mod i18n;
mod icon;
mod panel;
mod state;
mod statusline;
mod system;
mod tray;
mod usage;

use std::sync::{Arc, Mutex};

use nazar_core::clock::{Clock, SystemClock};
use nazar_core::lock::{Acquisition, LimitsLock};
use nazar_core::refresh::{self, Engine, Event, ReaderSet, Warnings};
use nazar_core::{Config, paths};
use tauri::{Emitter, Manager, RunEvent, WindowEvent};

use alerts::Notifier;
use i18n::Strings;
use panel::PanelState;
use state::{AppState, Overrides, SNAPSHOT_CHANGED};

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
    // One answer about the language, for the panel and the tray alike: the `--locale`
    // override, then the settings, then the operating system's UI language, then English.
    // WP4 had two answers and they could disagree; see `i18n::resolve`.
    let language = i18n::resolve(
        options.locale.as_deref().or(config.locale.as_deref()),
        system::ui_language().as_deref(),
    );
    let strings = Arc::new(Strings::new(&language));

    let overrides = Overrides {
        demo: options.demo,
        theme: options.theme.clone(),
        mode: options.mode.clone(),
        hint_dismissed: options.hint.map(|shown| !shown),
        suggest: options.offer,
        locale: options.locale.clone(),
        open_settings: options.view.as_deref() == Some("settings"),
        // The tab is only meaningful with the view, so the two travel as one value: `None`
        // means the panel opens where it always does.
        open_usage: (options.view.as_deref() == Some("usage")).then(|| {
            options
                .usage_tab
                .clone()
                .unwrap_or_else(|| cli::USAGE_TAB_DEFAULT.to_owned())
        }),
    };
    // A run that was told what to look like does not get to remember it: the screenshot
    // flags must leave the maintainer's settings exactly as they found them.
    let persist = options.may_persist();

    // `--demo` never claims the lock, so a screenshot session cannot become the writer and
    // cannot overwrite the real `~/.nazar/limits.json` with invented numbers. `--autostart`
    // does not either: it changes one registry value and exits, and a tray that is already
    // running must not be pushed aside by it.
    let lock = if options.demo || options.autostart.is_some() {
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

    // The same claim decides both files. The usage store is written by whoever holds
    // `~/.nazar/limits.lock` and by nobody else — one writer, many readers, no second lock —
    // so this is read off the acquisition before the lock is handed to the refresh loop.
    // `--demo` and `--autostart` never claim it, and so never scan.
    let writes_usage = lock.is_some();

    let limits_path = paths::limits_path().unwrap_or_default();
    let request_path = paths::request_path().unwrap_or_default();
    let readers = ReaderSet::discover(&config);
    let rules = config.rules();

    let setup_strings = Arc::clone(&strings);
    let event_strings = Arc::clone(&strings);
    let application = tauri::Builder::default()
        // Both plugins are driven from Rust rather than from the panel: the toast is shown
        // by the refresh loop, and the autostart switch goes through this application's own
        // `get_autostart` / `set_autostart` commands. That is why `capabilities/default.json`
        // grants the panel neither plugin's permissions — the webview never calls them.
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            // What the startup entry passes back to us. It means "open no window", and it is
            // the difference between a tray that starts quietly and one that greets you with
            // a popup every time you log in.
            Some(vec!["--hidden"]),
        ))
        .invoke_handler(tauri::generate_handler![
            state::get_snapshot,
            state::get_warnings,
            state::refresh_now,
            state::get_ui_state,
            state::set_theme,
            state::dismiss_hint,
            state::set_panel_height,
            state::open_panel,
            state::open_settings,
            state::get_config,
            state::set_config,
            state::reset_hint,
            state::dismiss_detailed_suggestion,
            state::get_autostart,
            state::set_autostart,
            statusline::statusline_status,
            statusline::statusline_preview,
            statusline::statusline_apply,
            usage::get_usage,
            state::quit
        ])
        .setup(move |app| {
            // First, before anything is created: `--autostart` reads or writes one registry
            // value through the plugin and exits, with no tray icon and no window. The
            // plugin's own setup has already run, which is why this cannot live in
            // `cli::run_if_requested` beside `--print`.
            if let Some(action) = options.autostart {
                cli::run_autostart(app.handle(), action);
            }

            app.manage(PanelState::new(options.scale, options.demo));
            app.manage(Arc::clone(&setup_strings));
            // The usage scan's own state: whether this instance may write, and when it last
            // did. Managed here rather than inside the refresh loop because the scan is
            // deliberately **not** on the refresh path — see `usage`.
            //
            // A demo run gets a state with no store behind it at all: it must not add invented
            // numbers to a real history, and it must not photograph one either.
            app.manage(if options.demo {
                usage::UsageState::demo()
            } else {
                usage::UsageState::new(writes_usage)
            });
            // A demo run remembers nothing: it must not write to `%APPDATA%\nazar`, and it
            // must not consume the keys of a real crossing nobody has been shown yet.
            app.manage(if options.demo {
                Notifier::in_memory()
            } else {
                Notifier::persistent()
            });

            // The state is managed before the tray is built, because the first thing the
            // tray does is ask it what to draw.
            let shared = if options.demo {
                AppState::new(
                    Arc::new(Mutex::new(demo::snapshot(&now))),
                    Arc::new(Mutex::new(Warnings::default())),
                    config,
                    overrides,
                    persist,
                )
            } else {
                let handle = app.handle().clone();
                let loop_strings = Arc::clone(&setup_strings);
                let engine = Engine::new(readers, limits_path, lock, rules, &now)
                    .with_request_file(request_path)
                    .on_event(move |event| on_loop_event(&handle, &loop_strings, event));

                let shared = AppState::new(
                    engine.snapshot(),
                    engine.diagnostics(),
                    config,
                    overrides,
                    persist,
                );
                shared.attach(refresh::spawn(engine, Arc::new(SystemClock))?);
                shared
            };
            app.manage(shared);

            tray::install(app.handle(), &setup_strings)?;
            tray::refresh(app.handle(), &setup_strings.catalog());

            if let Some(scale) = options.scale {
                panel::apply_scale(app.handle(), scale);
            }
            if options.demo_cross {
                demo_cross(app.handle(), &now);
            }
            // Nothing else would ever open the panel on a screenshot run: it has no user to
            // click. `--hidden` — what the startup entry passes — overrides even that.
            if options.demo && !options.hidden {
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
                tray::refresh(window.app_handle(), &event_strings.catalog());
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

/// Step the demo numbers across the thresholds, for `--demo-cross`.
///
/// A thread rather than a timer, because there is nothing to co-ordinate: it replaces the
/// snapshot, asks the notifications to look at it, redraws the icon and sleeps. What it
/// shows, and what should appear, is [`demo::cross_sequence`] — the WP5 acceptance criterion
/// (*"a toast appears exactly once when crossing 85 %"*) made visible on a real desktop.
fn demo_cross(app: &tauri::AppHandle, now: &str) {
    let app = app.clone();
    let steps = demo::cross_sequence(now);
    let strings = app.state::<Arc<Strings>>().inner().clone();
    std::thread::spawn(move || {
        for (index, snapshot) in steps.into_iter().enumerate() {
            std::thread::sleep(demo::CROSS_STEP);
            let Some(state) = app.try_state::<AppState>() else {
                return;
            };
            state.set_snapshot(snapshot);
            eprintln!("nazar-tray: demo-cross step {}", index + 1);
            alerts::on_refresh(&app);
            tray::refresh(&app, &strings.catalog());
            let _ = app.emit(SNAPSHOT_CHANGED, ());
        }
        eprintln!("nazar-tray: demo-cross finished");
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
fn on_loop_event(app: &tauri::AppHandle, strings: &Arc<Strings>, event: Event) {
    match event {
        // Every pass, changed or not. The notifications have to see a reading that has not
        // moved: a tray started when the weekly window is already at 91 % changes nothing,
        // and that is exactly the case where the user most needs to be told.
        //
        // The tooltip's second line comes from the usage store, which nothing in this pass
        // wrote and another process may have. Reading it back is one or two small
        // documents; **scanning** the transcripts is not, and does not happen here or
        // anywhere else on this thread — `usage` holds that rule and the reason for it.
        Event::Refreshed => {
            alerts::on_refresh(app);
            tray::refresh_tooltip(app);
        }
        // The panel re-reads the snapshot; the payload would be stale by the time it drew.
        // The icon and the tooltip are redrawn here rather than in the panel, because they
        // have to be right whether or not anybody has opened it.
        Event::SnapshotChanged => {
            let _ = app.emit(SNAPSHOT_CHANGED, ());
            tray::refresh(app, &strings.catalog());
        }
        Event::ShowRequested => panel::show(app),
    }
}
