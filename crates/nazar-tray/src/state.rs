//! What the panel is allowed to ask for, and the state it asks.
//!
//! The refresh loop owns the readers and runs on its own thread; the panel runs in a
//! webview and knows nothing about any of that. Between them there is one shared snapshot
//! behind a mutex, one event, and a short list of commands:
//!
//! | | |
//! |---|---|
//! | `get_snapshot` | the derived view, worked out **now** |
//! | `get_warnings` | the loop's counters, for a diagnostic line |
//! | `refresh_now` | ask the loop for a pass; the audit's finding B13 is why it exists |
//! | `get_ui_state` | theme, mode, language and whether the first-run hint is still due |
//! | `set_theme` | the panel's theme toggle, remembered in `config.json` |
//! | `dismiss_hint` | the overflow hint has been read; never show it again |
//! | `set_panel_height` | the panel measured its own content and wants a window that fits |
//! | `open_panel` | show the panel by the cursor |
//! | `quit` | stop the loop, **release the advisory lock**, and exit |
//! | `snapshot-changed` (event) | the document moved; ask again |
//!
//! `get_snapshot` derives rather than returns: which window binds and how long is left both
//! change with the clock and neither is stored, so the answer is computed for the instant
//! the panel asked. That is what makes the countdown right after a machine wakes up without
//! anything having refreshed.
//!
//! **`quit` is the one that fixes something.** WP3 shipped a tray that could only be closed
//! by killing the process, and a killed process leaves `~/.nazar/limits.lock` behind for its
//! five-minute grace period — so the next launch starts as a reader and looks broken. Every
//! way out of the application now goes through [`AppState::shutdown`] first.

use std::sync::{Arc, Mutex, PoisonError};

use nazar_core::config::{DEFAULT_THEME, DEFAULT_THEME_MODE};
use nazar_core::refresh::{LoopHandle, Warnings};
use nazar_core::state::{Rules, Snapshot, SnapshotView};
use nazar_core::{Config, now_rfc3339};
use serde::{Deserialize, Serialize};

/// The event the panel listens for. Emitted only when the document actually changed.
pub const SNAPSHOT_CHANGED: &str = "snapshot-changed";

/// Everything about the panel that is a choice rather than a measurement.
///
/// Sent to the webview once on load and again after every change, so the panel never has
/// to guess: the theme it paints itself in, whether the light/dark decision is the
/// operating system's, the language, and whether the first-run hint is still owed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    /// `nazar` or `graphite`. A name this build does not know falls back to `nazar`.
    pub theme: String,
    /// `system`, `light` or `dark`. `system` means `prefers-color-scheme` decides.
    pub mode: String,
    /// Language override, or `None` to let the panel detect one.
    pub locale: Option<String>,
    /// Whether the first-run overflow hint has already been dismissed.
    pub hint_dismissed: bool,
    /// Whether the numbers on screen are [`crate::demo`]'s rather than this machine's.
    ///
    /// The panel shows a badge when this is true. A screenshot that could be mistaken for
    /// real data is a screenshot that will be, eventually, by someone.
    pub demo: bool,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            theme: DEFAULT_THEME.to_owned(),
            mode: DEFAULT_THEME_MODE.to_owned(),
            locale: None,
            hint_dismissed: false,
            demo: false,
        }
    }
}

impl UiState {
    /// The panel settings a `config.json` describes.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        UiState {
            theme: config.theme.clone(),
            mode: config.theme_mode.clone(),
            locale: config.locale.clone(),
            hint_dismissed: config.first_run_hint_dismissed,
            demo: false,
        }
    }
}

/// Everything the commands need.
pub struct AppState {
    snapshot: Arc<Mutex<Snapshot>>,
    warnings: Arc<Mutex<Warnings>>,
    rules: Rules,
    refresher: Mutex<Option<LoopHandle>>,
    ui: Mutex<UiState>,
    /// Whether a change to [`AppState::ui`] is written to `config.json`.
    ///
    /// `false` under `--demo` and under `--theme`/`--mode`: a screenshot run must not leave
    /// the maintainer's settings different from how it found them.
    persist: bool,
}

impl AppState {
    /// Wrap the handles the engine handed out.
    pub fn new(
        snapshot: Arc<Mutex<Snapshot>>,
        warnings: Arc<Mutex<Warnings>>,
        rules: Rules,
        ui: UiState,
        persist: bool,
    ) -> Self {
        AppState {
            snapshot,
            warnings,
            rules,
            refresher: Mutex::new(None),
            ui: Mutex::new(ui),
            persist,
        }
    }

    /// Keep the loop's handle, so the panel can ask for a refresh and shutdown can stop it.
    pub fn attach(&self, handle: LoopHandle) {
        *self
            .refresher
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(handle);
    }

    /// The derived view, for right now.
    pub fn view(&self) -> SnapshotView {
        self.snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .view(&now_rfc3339(), &self.rules)
    }

    /// The loop's counters.
    pub fn warnings(&self) -> Warnings {
        self.warnings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Ask the loop for a pass. `false` when there is no loop, which is what a machine with
    /// no home directory looks like.
    pub fn refresh(&self) -> bool {
        self.refresher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(LoopHandle::refresh_now)
    }

    /// Stop the loop and let it release the advisory lock.
    ///
    /// Called when the application exits, by whichever route: the menu's Quit, the
    /// `quit` command, or the window manager. Without it the lock file would be left
    /// behind for its five-minute grace period, and the next launch would start as a
    /// reader — which looks exactly like the tray not working.
    ///
    /// Idempotent: the handle is taken out of its slot, so a second call does nothing.
    pub fn shutdown(&self) {
        if let Some(handle) = self
            .refresher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            // Dropping the engine drops the lock, whose `Drop` removes the file.
            handle.stop();
        }
    }

    /// The panel's own settings.
    pub fn ui(&self) -> UiState {
        self.ui
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Change the panel's settings and, unless this is a screenshot run, remember them.
    ///
    /// The settings file is read again rather than kept in memory: it is the user's file,
    /// another process may have edited it, and this writes back two keys of it. A failure
    /// to write is not reported to the panel — the theme has already changed on screen, and
    /// a settings file that cannot be written is not something the user can act on from a
    /// tray popup.
    pub fn update_ui(&self, change: impl FnOnce(&mut UiState)) -> UiState {
        let mut ui = self.ui.lock().unwrap_or_else(PoisonError::into_inner);
        change(&mut ui);
        if self.persist {
            let mut config = Config::load().unwrap_or_default();
            config.theme = ui.theme.clone();
            config.theme_mode = ui.mode.clone();
            config.first_run_hint_dismissed = ui.hint_dismissed;
            let _ = config.save();
        }
        ui.clone()
    }
}

/// The panel's view of the numbers, derived for the moment it asked.
#[tauri::command]
pub fn get_snapshot(state: tauri::State<'_, AppState>) -> SnapshotView {
    state.view()
}

/// The loop's counters, for a diagnostic line in the panel.
#[tauri::command]
pub fn get_warnings(state: tauri::State<'_, AppState>) -> Warnings {
    state.warnings()
}

/// Ask for a refresh now.
#[tauri::command]
pub fn refresh_now(state: tauri::State<'_, AppState>) -> bool {
    state.refresh()
}

/// The panel's settings: theme, mode, language, and whether the hint is still owed.
#[tauri::command]
pub fn get_ui_state(state: tauri::State<'_, AppState>) -> UiState {
    state.ui()
}

/// The theme toggle in the panel's footer.
///
/// Both values are taken as written and validated in the panel, which owns the theme
/// files: a name this build does not recognise is stored and falls back to `nazar` when it
/// is painted, rather than being rejected here and lost.
#[tauri::command]
pub fn set_theme(state: tauri::State<'_, AppState>, theme: String, mode: String) -> UiState {
    state.update_ui(|ui| {
        ui.theme = theme;
        ui.mode = mode;
    })
}

/// The first-run hint has been read. Never show it again.
#[tauri::command]
pub fn dismiss_hint(state: tauri::State<'_, AppState>) -> UiState {
    state.update_ui(|ui| ui.hint_dismissed = true)
}

/// The panel measured its content and would like a window that fits it.
///
/// The panel's height is not a constant: a provider can have two windows or three, the
/// first-run hint is there once and never again, and an error line wraps. Rather than
/// pick a height that is too big for most machines and too small for one, the panel
/// measures itself and asks. Clamped here, because a webview that miscalculates must not
/// be able to open a window taller than a screen.
#[tauri::command]
pub fn set_panel_height(app: tauri::AppHandle, height: f64) {
    crate::panel::resize(&app, height);
}

/// Show the panel by the cursor. The tray menu's first item, and a second launch's answer.
#[tauri::command]
pub fn open_panel(app: tauri::AppHandle) {
    crate::panel::show(&app);
}

/// Stop the loop, release the advisory lock, and exit.
#[tauri::command]
pub fn quit(app: tauri::AppHandle, state: tauri::State<'_, AppState>) {
    state.shutdown();
    app.exit(0);
}
