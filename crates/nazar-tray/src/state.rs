//! What the panel is allowed to ask for, and the state it asks.
//!
//! The refresh loop owns the readers and runs on its own thread; the panel runs in a
//! webview and knows nothing about any of that. Between them there is one shared snapshot
//! behind a mutex, one settings document behind another, two events, and a short list of
//! commands:
//!
//! | | |
//! |---|---|
//! | `get_snapshot` | the derived view, worked out **now** |
//! | `get_warnings` | the loop's counters, for a diagnostic line |
//! | `refresh_now` | ask the loop for a pass; the audit's finding B13 is why it exists |
//! | `get_ui_state` | theme, mode, the **resolved** language, and whether the hint is due |
//! | `set_theme` | theme and mode, remembered in `config.json`; no caller since T-WP12 |
//! | `dismiss_hint` | the overflow hint has been read; never show it again |
//! | `set_panel_height` | the panel measured its own content and wants a window that fits |
//! | `open_panel` | show the panel by the cursor |
//! | `open_settings` | show the panel with the settings view open: the tray menu's entry |
//! | `get_config` | everything the settings form draws itself from |
//! | `set_config` | the settings form, **validated**, saved atomically, and applied at once |
//! | `reset_hint` | put the first-run overflow tip back |
//! | `dismiss_detailed_suggestion` | the Max-plan offer has been shown; do not ask again |
//! | `get_autostart` / `set_autostart` | "start with Windows", read back from the plugin |
//! | `quit` | stop the loop, **release the advisory lock**, and exit |
//! | `snapshot-changed` (event) | the document moved; ask again |
//! | `open-settings` (event) | the tray menu asked for the settings view |
//!
//! `get_snapshot` derives rather than returns: which window binds and how long is left both
//! change with the clock and neither is stored, so the answer is computed for the instant
//! the panel asked. That is what makes the countdown right after a machine wakes up without
//! anything having refreshed.
//!
//! **The settings document is the state.** WP4 kept a small [`UiState`] beside the loop and
//! wrote two keys of `config.json` when it changed. WP5 has settings the *readers* care
//! about — which providers exist at all — so the whole [`Config`] is held here, everything
//! derived comes off it, and [`AppState::apply_settings`] is the one place a change is
//! written and acted on. The panel's view of it is still [`UiState`]: that document with the
//! command-line overrides painted over the top.
//!
//! **Reading is forgiving, writing is strict.** [`set_config`] runs [`Config::validate`] and
//! writes **nothing** when it fails, returning the list of problems for the form to show. A
//! settings file already on disk is used as best it can be, because a user whose thresholds
//! are upside down still has to be able to open the form that fixes them.
//!
//! **`quit` is the one that fixes something.** WP3 shipped a tray that could only be closed
//! by killing the process, and a killed process leaves `~/.nazar/limits.lock` behind for its
//! five-minute grace period — so the next launch starts as a reader and looks broken. Every
//! way out of the application now goes through [`AppState::shutdown`] first.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nazar_core::config::{
    DEFAULT_THEME, DEFAULT_THEME_MODE, Invalid, ProviderSwitches, QuietHours,
};
use nazar_core::refresh::{LoopHandle, ReaderSet, Warnings};
use nazar_core::state::{Rules, Snapshot, SnapshotView, Thresholds};
use nazar_core::{Config, now_rfc3339, paths};
use serde::{Deserialize, Serialize};
use tauri::Emitter;

use crate::i18n::Strings;

/// The event the panel listens for. Emitted only when the document actually changed.
pub const SNAPSHOT_CHANGED: &str = "snapshot-changed";

/// The event that asks the panel to show its settings view.
///
/// Needed because the tray menu is on the Rust side and the settings view is not: there is
/// no way to open a view inside a webview except to ask it.
pub const OPEN_SETTINGS: &str = "open-settings";

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
    /// Language override as the settings file holds it, or `None` for "follow the system".
    pub locale: Option<String>,
    /// The language the panel should actually paint itself in.
    ///
    /// **The fix for WP4's open risk.** The panel used to guess from `navigator.languages`
    /// while the tray fell back to English, so a Turkish machine could get a Turkish panel
    /// under an English tooltip. Now one answer is worked out in Rust — the override, then
    /// the operating system, then English — and both surfaces are told what it is.
    pub resolved_locale: String,
    /// Whether the first-run overflow hint has already been dismissed.
    ///
    /// Always `true` where the hint does not apply, which is every platform but Windows:
    /// see [`crate::desktop::OVERFLOW_HINT`]. The panel needs no `cfg` of its own for the
    /// banner, only for the settings button that brings it back — which is
    /// [`UiState::hint_available`].
    pub hint_dismissed: bool,
    /// Whether this platform has a first-run overflow hint at all.
    ///
    /// The banner is governed by `hint_dismissed` alone; this is what hides the *settings*
    /// row that offers to show it again. A button that promises to bring back a tip about
    /// a control the desktop does not have is the same bug one screen further in.
    pub hint_available: bool,
    /// Whether the numbers on screen are [`crate::demo`]'s rather than this machine's.
    ///
    /// The panel shows a badge when this is true. A screenshot that could be mistaken for
    /// real data is a screenshot that will be, eventually, by someone.
    pub demo: bool,
    /// Whether the panel should open on the settings page. `--view settings`.
    ///
    /// Carried in the state rather than sent as an event, because an event emitted while
    /// the webview is still loading has nobody listening for it yet.
    pub open_settings: bool,
    /// Which tab the panel should open the usage view on, or `None` for the numbers.
    ///
    /// `--view usage`, with `--usage-tab` naming one of `week`, `weeks`, `all`, `models` or
    /// `day`. Carried here for the reason above, and a **string** rather than an enum because
    /// the tab names are the panel's own and a Rust spelling would be a second set of them.
    pub open_usage: Option<String>,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            theme: DEFAULT_THEME.to_owned(),
            mode: DEFAULT_THEME_MODE.to_owned(),
            locale: None,
            resolved_locale: "en".to_owned(),
            hint_dismissed: !crate::desktop::OVERFLOW_HINT,
            hint_available: crate::desktop::OVERFLOW_HINT,
            demo: false,
            open_settings: false,
            open_usage: None,
        }
    }
}

/// What the command line said the panel should look like, for one run.
///
/// Every one of these exists for `docs/screenshots`, and none of them is ever written back:
/// a run that was told what to look like must leave `config.json` exactly as it found it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overrides {
    /// `--demo`: the numbers are synthetic.
    pub demo: bool,
    /// `--theme`.
    pub theme: Option<String>,
    /// `--mode`.
    pub mode: Option<String>,
    /// `--hint on|off`, as "has it been dismissed".
    pub hint_dismissed: Option<bool>,
    /// `--offer on|off`: whether to make the one-time Max-plan offer this run.
    pub suggest: Option<bool>,
    /// `--locale`.
    pub locale: Option<String>,
    /// `--view settings`.
    pub open_settings: bool,
    /// `--view usage`, and the tab `--usage-tab` asked for.
    pub open_usage: Option<String>,
}

/// Everything the settings form draws itself from.
///
/// One round trip rather than six: the page is opened as a whole, and a panel that had to
/// ask five commands to draw one page would show it in pieces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    /// The settings as they are now.
    pub form: SettingsForm,
    /// The language the panel is painted in, once the override and the machine have voted.
    pub resolved_locale: String,
    /// The languages this build can actually paint itself in.
    ///
    /// Sent rather than hard-coded in the panel, so that the day WP6 fills a catalogue the
    /// language appears in the form without either side being edited.
    pub languages: Vec<String>,
    /// Whether the first-run overflow hint is still owed. The "Reset" button reads it.
    pub first_run_hint_dismissed: bool,
    /// Whether the app should offer the detailed-windows mode, once.
    ///
    /// WP2b decided this and left the asking to WP5: it is true when the passive path can
    /// see a Max plan, which is exactly the case where the binding window is invisible
    /// without the opt-in mode, and it goes false for good once the offer has been shown.
    pub suggest_detailed: bool,
    /// Where the files are, for the "where does this live" lines. `~` already collapsed.
    pub paths: SettingsPaths,
    /// The application version, for the footer.
    pub version: String,
    /// Whether the numbers on screen are synthetic.
    pub demo: bool,
    /// Whether this run may write to `config.json` at all.
    ///
    /// `false` under the screenshot flags. The form is shown read-only rather than hidden,
    /// because a picture of the settings page is a thing the documentation wants.
    pub writable: bool,
}

/// The paths the settings page shows, with `~` collapsed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPaths {
    /// `%APPDATA%\nazar\config.json`, or wherever `NAZAR_HOME` moved it.
    pub config: String,
    /// `~/.nazar/limits.json`: the file Nazar reads.
    pub limits: String,
    /// `~/.nazar/statusline`: where the wrapper leaves its captures.
    pub captures: String,
    /// `<settings dir>/alerts.json`: which thresholds have already been announced.
    pub alerts: String,
}

/// The settings form, exactly as the panel submits it.
///
/// A **whole** form rather than a patch: every field the page owns is sent on every save,
/// so there is no ambiguity between "unchanged" and "cleared", and `quietHours: null` means
/// what it looks like. Everything the form does *not* own — the schema version, the keys a
/// newer build wrote, whether the first-run hint has been dismissed — is untouched, because
/// [`AppState::apply_settings`] reads the file again and edits it rather than replacing it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsForm {
    /// `system`, or one of the languages this build ships.
    pub locale: String,
    /// `nazar` or `graphite`.
    pub theme: String,
    /// `system`, `light` or `dark`.
    pub theme_mode: String,
    /// Whether threshold notifications are shown at all.
    pub notifications: bool,
    /// The stretch of local time in which they are not. `null` when there is none.
    pub quiet_hours: Option<QuietHours>,
    /// The percentages a notification fires at.
    pub thresholds: Thresholds,
    /// Which providers are read.
    pub providers: ProviderSwitches,
    /// Whether the opt-in detailed-windows mode is on.
    pub detailed_windows: bool,
    /// Show the per-line numbers `/usage` shows, rather than the deduplicated spend.
    pub usage_count_like_claude_code: bool,
    /// Show the days before the transcripts, as Claude Code reported them.
    pub usage_fill_history_from_stats: bool,
}

impl SettingsForm {
    /// The form as a settings document describes it.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        SettingsForm {
            locale: config
                .locale
                .clone()
                .unwrap_or_else(|| nazar_core::config::LOCALE_SYSTEM.to_owned()),
            theme: config.theme.clone(),
            theme_mode: config.theme_mode.clone(),
            notifications: config.notifications,
            quiet_hours: config.quiet_hours.clone(),
            thresholds: config.thresholds,
            providers: config.providers,
            detailed_windows: config.detailed_windows,
            usage_count_like_claude_code: config.usage.count_like_claude_code,
            usage_fill_history_from_stats: config.usage.fill_history_from_stats,
        }
    }

    /// Write the form into a settings document, leaving every other key alone.
    ///
    /// `system` becomes the **absence** of `locale`: a setting nobody has chosen should not
    /// be written down as a choice, or a user who picked "follow the system" in one country
    /// and then moved would find their panel still in the old language.
    pub fn apply_to(&self, config: &mut Config) {
        config.locale =
            (self.locale != nazar_core::config::LOCALE_SYSTEM).then(|| self.locale.clone());
        config.theme = self.theme.clone();
        config.theme_mode = self.theme_mode.clone();
        config.notifications = self.notifications;
        config.quiet_hours = self.quiet_hours.clone();
        config.thresholds = self.thresholds;
        config.providers = self.providers;
        config.detailed_windows = self.detailed_windows;
        config.usage.count_like_claude_code = self.usage_count_like_claude_code;
        config.usage.fill_history_from_stats = self.usage_fill_history_from_stats;
    }
}

/// Everything the commands need.
pub struct AppState {
    snapshot: Arc<Mutex<Snapshot>>,
    warnings: Arc<Mutex<Warnings>>,
    config: Mutex<Config>,
    refresher: Mutex<Option<LoopHandle>>,
    overrides: Overrides,
    /// Whether a change to the settings is written to `config.json`.
    ///
    /// `false` under `--demo` and the other screenshot flags: a screenshot run must not
    /// leave the maintainer's settings different from how it found them.
    persist: bool,
}

impl AppState {
    /// Whether this run may change anything on the machine.
    ///
    /// Read by [`crate::statusline`] as well as by the settings form. A `--demo` run says on
    /// screen that it will not write the user's settings, and the status-line section is the
    /// one control on that page whose button would write somebody *else's* file — so it is
    /// refused here rather than merely hidden, and a screenshot run cannot edit
    /// `~/.claude/settings.json` even if something clicks it.
    pub fn writes_allowed(&self) -> bool {
        self.persist
    }
}

impl AppState {
    /// Wrap the handles the engine handed out.
    pub fn new(
        snapshot: Arc<Mutex<Snapshot>>,
        warnings: Arc<Mutex<Warnings>>,
        config: Config,
        overrides: Overrides,
        persist: bool,
    ) -> Self {
        AppState {
            snapshot,
            warnings,
            config: Mutex::new(config),
            refresher: Mutex::new(None),
            overrides,
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

    fn held(&self) -> MutexGuard<'_, Config> {
        self.config.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The settings as they stand.
    pub fn config(&self) -> Config {
        self.held().clone()
    }

    /// The rules every display derives with. One answer, from one place (finding B14).
    pub fn rules(&self) -> Rules {
        self.held().rules()
    }

    /// The derived view, for right now.
    pub fn view(&self) -> SnapshotView {
        let rules = self.rules();
        self.snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .view(&now_rfc3339(), &rules)
    }

    /// Replace the numbers on screen.
    ///
    /// Only [`crate::demo`] does this, and only under `--demo-cross`, where the whole point
    /// is to watch a threshold being crossed on purpose.
    pub fn set_snapshot(&self, snapshot: Snapshot) {
        *self.snapshot.lock().unwrap_or_else(PoisonError::into_inner) = snapshot;
    }

    /// The loop's counters.
    pub fn warnings(&self) -> Warnings {
        self.warnings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Ask the loop for a pass. `false` when there is no loop, which is what a machine with
    /// no home directory — or a `--demo` run — looks like.
    pub fn refresh(&self) -> bool {
        self.refresher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(LoopHandle::refresh_now)
    }

    /// Hand the loop a new set of readers, built from the settings as they now are.
    fn reconfigure(&self, config: &Config) -> bool {
        self.refresher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|handle| handle.reconfigure(ReaderSet::discover(config), config.rules()))
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

    /// The panel's own settings, with the command-line overrides painted over the top.
    pub fn ui(&self, strings: &Strings) -> UiState {
        let config = self.held();
        UiState {
            theme: self
                .overrides
                .theme
                .clone()
                .unwrap_or_else(|| config.theme.clone()),
            mode: self
                .overrides
                .mode
                .clone()
                .unwrap_or_else(|| config.theme_mode.clone()),
            locale: self
                .overrides
                .locale
                .clone()
                .or_else(|| config.locale.clone()),
            resolved_locale: strings.locale(),
            // The platform first, and the settings only where the platform has the
            // question: `--hint on` is a screenshot flag, and a screenshot of a Windows
            // banner is not a thing a Linux run should be able to take.
            hint_dismissed: !crate::desktop::OVERFLOW_HINT
                || self
                    .overrides
                    .hint_dismissed
                    .unwrap_or(config.first_run_hint_dismissed),
            hint_available: crate::desktop::OVERFLOW_HINT,
            demo: self.overrides.demo,
            open_settings: self.overrides.open_settings,
            open_usage: self.overrides.open_usage.clone(),
        }
    }

    /// Everything the settings form draws itself from.
    pub fn settings(&self, strings: &Strings) -> SettingsView {
        let view = self.view();
        let config = self.held();
        SettingsView {
            form: SettingsForm::from_config(&config),
            resolved_locale: strings.locale(),
            languages: crate::i18n::available()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            first_run_hint_dismissed: config.first_run_hint_dismissed,
            suggest_detailed: self
                .overrides
                .suggest
                .unwrap_or_else(|| suggest_detailed(&config, &view)),
            paths: settings_paths(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            demo: self.overrides.demo,
            writable: self.persist,
        }
    }

    /// Change one of the panel's own settings and, unless this is a screenshot run,
    /// remember it.
    ///
    /// The settings file is read again rather than kept in memory: it is the user's file,
    /// another process may have edited it, and this writes back a few keys of it. A failure
    /// to write is not reported to the panel — the theme has already changed on screen, and
    /// a settings file that cannot be written is not something the user can act on from a
    /// tray popup.
    pub fn update_ui(&self, strings: &Strings, change: impl FnOnce(&mut Config)) -> UiState {
        {
            let mut config = self.held();
            change(&mut config);
            if self.persist {
                let mut stored = Config::load().unwrap_or_else(|_| config.clone());
                stored.theme = config.theme.clone();
                stored.theme_mode = config.theme_mode.clone();
                stored.first_run_hint_dismissed = config.first_run_hint_dismissed;
                stored.detailed_suggested = config.detailed_suggested;
                let _ = stored.save();
            }
        }
        self.ui(strings)
    }

    /// Validate a form, write it, and act on it.
    ///
    /// The order matters and is the whole method: **validate, then write, then apply**. A
    /// form that does not validate changes nothing at all — not the file, not the readers,
    /// not the language — so a user who typed `85 / 60 / 100` gets an error and the
    /// settings they had, rather than an error and a tray that has already half-changed.
    ///
    /// Returns what changed, so the caller knows whether to rebuild the tray menu.
    pub fn apply_settings(
        &self,
        form: &SettingsForm,
        strings: &Strings,
    ) -> Result<Applied, Vec<Invalid>> {
        // The file, not the copy in memory: a key a newer build wrote survives because this
        // edits what is on disk rather than writing out what we happen to hold.
        let mut next = if self.persist {
            Config::load().unwrap_or_else(|_| self.config())
        } else {
            self.config()
        };
        form.apply_to(&mut next);

        let problems = next.validate();
        if !problems.is_empty() {
            return Err(problems);
        }

        let previous = self.config();
        if self.persist
            && let Err(error) = next.save()
        {
            // A settings file that cannot be written is worth saying out loud: the user is
            // looking at the form and would otherwise think it had saved.
            eprintln!("nazar-tray: could not save the settings: {error}");
        }

        let language = crate::i18n::resolve(
            self.overrides.locale.as_deref().or(next.locale.as_deref()),
            crate::system::ui_language().as_deref(),
        );
        let locale_changed = strings.set(&language);
        let readers_changed = previous.providers != next.providers
            || previous.detailed_windows != next.detailed_windows;
        let thresholds_changed = previous.thresholds != next.thresholds;

        *self.held() = next.clone();

        // A settings change always refreshes; when the *readers* changed it has to be the
        // heavier one, because the readers themselves are being replaced.
        if readers_changed {
            self.reconfigure(&next);
        } else {
            self.refresh();
        }

        Ok(Applied {
            locale_changed,
            readers_changed,
            thresholds_changed,
            view: self.settings(strings),
        })
    }

    /// Put the first-run hint back, so the overflow tip is shown once more.
    pub fn reset_hint(&self, strings: &Strings) -> SettingsView {
        self.update_ui(strings, |config| config.first_run_hint_dismissed = false);
        self.settings(strings)
    }

    /// Remember that the detailed-windows offer has been shown.
    pub fn mark_detailed_suggested(&self, strings: &Strings) -> SettingsView {
        self.update_ui(strings, |config| config.detailed_suggested = true);
        self.settings(strings)
    }
}

/// What [`AppState::apply_settings`] had to change beyond the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// The language moved, so the tray menu has to be rebuilt in it.
    pub locale_changed: bool,
    /// A provider was switched on or off, so the readers were replaced.
    pub readers_changed: bool,
    /// The thresholds moved, so the icon may be a different colour.
    pub thresholds_changed: bool,
    /// The settings as they now are, for the form to redraw itself from.
    pub view: SettingsView,
}

/// Whether to offer the detailed-windows mode, once.
///
/// **The offer is a question, and it is asked precisely because the app cannot answer it.**
/// WP2 found that the status-line payload carries no plan name at all — the passive path
/// derives none, on purpose, because inventing one would be worse — so on a machine that has
/// never turned the opt-in mode on, `plan` is simply *absent*. WP2b's
/// `should_suggest_detailed` decides on a plan when there is one to decide on, which is the
/// case only once the endpoint has already answered; the passive path needs the other branch,
/// and that is the one which actually fires on a fresh machine.
///
/// Three conditions make it a suggestion rather than a nag, and they are all here:
///
/// * the mode has to be **off** — there is nothing to offer somebody who already has it;
/// * nobody has been **asked before**, which `detailedSuggested` remembers for good;
/// * Claude has to be **set up on this machine**. Offering a Claude-only feature to somebody
///   who only uses Codex is the sort of thing that teaches people to ignore banners.
fn suggest_detailed(config: &Config, view: &SnapshotView) -> bool {
    if config.detailed_windows || config.detailed_suggested {
        return false;
    }
    let Some(claude) = view
        .providers
        .iter()
        .find(|provider| provider.name == "claude")
        .filter(|provider| provider.configured)
    else {
        return false;
    };

    match claude.plan.as_deref() {
        // A plan we can see: WP2b's rule, which offers only to Max accounts, because for
        // anybody else the global weekly window really is the binding one.
        //
        // Called unconditionally rather than behind a `cfg`: the tray depends on
        // `nazar-core` with its default features, which include the opt-in mode.
        // `nazar-statusline` is the crate that builds without it, and it has no settings
        // page to offer anything on.
        Some(plan) => nazar_core::should_suggest_detailed(Some(plan), false),
        // No plan at all, which is what the passive path always looks like. The app cannot
        // tell a Max account from a Pro one from here, and the honest thing is to ask once
        // rather than leave a Max user's binding window invisible for ever.
        None => claude.source.as_deref() != Some("endpoint"),
    }
}

/// The four paths the settings page shows, with the home directory collapsed to `~`.
fn settings_paths() -> SettingsPaths {
    SettingsPaths {
        config: collapse(paths::config_path().ok()),
        limits: collapse(paths::limits_path().ok()),
        captures: collapse(paths::statusline_dir().ok()),
        alerts: collapse(paths::alerts_path().ok()),
    }
}

/// `C:\Users\someone\.nazar\limits.json` → `~\.nazar\limits.json`.
///
/// Not decoration: these paths go on screen, screenshots of this page end up in the
/// documentation, and a user name is the sort of thing that should not be in one by
/// accident.
fn collapse(path: Option<PathBuf>) -> String {
    match path {
        Some(path) => collapse_home(&path),
        None => String::new(),
    }
}

/// [`collapse`] for a path that is definitely there.
///
/// Shared with [`crate::statusline`], which puts the wrapper's own location on the same
/// page. Two paths on one screen, one of them with a user name still in it, would be a
/// strange thing to ship after taking the trouble to hide the other.
pub(crate) fn collapse_home(path: &std::path::Path) -> String {
    let Ok(home) = paths::home_dir() else {
        return path.display().to_string();
    };
    match path.strip_prefix(&home) {
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.display().to_string(),
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

/// The panel's settings: theme, mode, the resolved language, and the hint.
#[tauri::command]
pub fn get_ui_state(
    state: tauri::State<'_, AppState>,
    strings: tauri::State<'_, Arc<Strings>>,
) -> UiState {
    state.ui(&strings)
}

/// Theme and mode, remembered in `config.json`.
///
/// Both values are taken as written and validated in the panel, which owns the theme
/// files: a name this build does not recognise is stored and falls back to `nazar` when it
/// is painted, rather than being rejected here and lost.
///
/// T-WP12 deleted the footer toggle that was its only caller; the settings form writes the
/// same two fields, with the rest of the document, through `set_config`. The command stays
/// registered because the bridge test names it and because removing a command a released
/// build answers is a decision of its own, not a side effect of moving a button.
#[tauri::command]
pub fn set_theme(
    state: tauri::State<'_, AppState>,
    strings: tauri::State<'_, Arc<Strings>>,
    theme: String,
    mode: String,
) -> UiState {
    state.update_ui(&strings, |config| {
        config.theme = theme;
        config.theme_mode = mode;
    })
}

/// The first-run hint has been read. Never show it again.
#[tauri::command]
pub fn dismiss_hint(
    state: tauri::State<'_, AppState>,
    strings: tauri::State<'_, Arc<Strings>>,
) -> UiState {
    state.update_ui(&strings, |config| config.first_run_hint_dismissed = true)
}

/// The panel measured its content and would like a window that fits it.
///
/// The panel's height is not a constant: a provider can have two windows or three, the
/// first-run hint is there once and never again, the settings view is a different page
/// altogether, and an error line wraps. Rather than pick a height that is too big for most
/// machines and too small for one, the panel measures itself and asks. Clamped in
/// [`crate::panel::resize`], because a webview that miscalculates must not be able to open
/// a window taller than a screen.
#[tauri::command]
pub fn set_panel_height(app: tauri::AppHandle, height: f64) {
    crate::panel::resize(&app, height);
}

/// Show the panel by the cursor. The tray menu's first item, and a second launch's answer.
#[tauri::command]
pub fn open_panel(app: tauri::AppHandle) {
    crate::panel::show(&app);
}

/// Show the panel with the settings view open. The tray menu's `Settings` entry.
#[tauri::command]
pub fn open_settings(app: tauri::AppHandle) {
    let _ = app.emit(OPEN_SETTINGS, ());
    crate::panel::show(&app);
}

/// Everything the settings form draws itself from.
#[tauri::command]
pub fn get_config(
    state: tauri::State<'_, AppState>,
    strings: tauri::State<'_, Arc<Strings>>,
) -> SettingsView {
    state.settings(&strings)
}

/// Save the settings form.
///
/// Returns the settings as they now are, or the list of problems and an unchanged file.
#[tauri::command]
pub fn set_config(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    strings: tauri::State<'_, Arc<Strings>>,
    form: SettingsForm,
) -> Result<SettingsView, Vec<Invalid>> {
    let applied = state.apply_settings(&form, &strings)?;
    if applied.locale_changed {
        // A menu item's text is set when the item is built, so a language change means a
        // new menu. WP4 left that as an open risk; this is what closes it.
        crate::tray::rebuild_menu(&app, &strings);
    }
    if applied.locale_changed || applied.thresholds_changed {
        crate::tray::refresh(&app, &strings.catalog());
    }
    Ok(applied.view)
}

/// Put the first-run overflow hint back.
#[tauri::command]
pub fn reset_hint(
    state: tauri::State<'_, AppState>,
    strings: tauri::State<'_, Arc<Strings>>,
) -> SettingsView {
    state.reset_hint(&strings)
}

/// The detailed-windows offer has been shown. Do not make it again.
#[tauri::command]
pub fn dismiss_detailed_suggestion(
    state: tauri::State<'_, AppState>,
    strings: tauri::State<'_, Arc<Strings>>,
) -> SettingsView {
    state.mark_detailed_suggested(&strings)
}

/// Whether the application starts with Windows, as the plugin reports it.
///
/// Read from the plugin rather than remembered in `config.json`, so that a user who removed
/// the entry by hand — or through Task Manager's Startup tab, which is where people
/// actually look — sees the switch agree with their machine instead of with our file.
#[tauri::command]
pub fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch()
        .is_enabled()
        .map_err(|error| error.to_string())
}

/// Turn "start with Windows" on or off, and report what it is afterwards.
///
/// The answer is read back from the plugin rather than assumed: writing to the registry can
/// fail, and a switch that flips in the interface without anything happening on the machine
/// is worse than one that refuses.
#[tauri::command]
pub fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let outcome = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    outcome.map_err(|error| error.to_string())?;
    manager.is_enabled().map_err(|error| error.to_string())
}

/// Stop the loop, release the advisory lock, and exit.
#[tauri::command]
pub fn quit(app: tauri::AppHandle, state: tauri::State<'_, AppState>) {
    state.shutdown();
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state with nothing in it: the defaults, and a snapshot nobody read.
    fn fresh_state(overrides: Overrides) -> AppState {
        AppState::new(
            Arc::new(Mutex::new(Snapshot::empty("2026-09-17T00:00:00Z"))),
            Arc::new(Mutex::new(Warnings::default())),
            Config::default(),
            overrides,
            false,
        )
    }

    /// The bug qarpus found on a Fedora 44 GNOME session on 2026-09-16.
    ///
    /// A fresh machine opened the panel and was told to drag the bead onto the taskbar and
    /// pin it out of the `^` overflow — a flyout and a taskbar that only Windows 11 has.
    /// `first_run_hint_dismissed` is `false` on every fresh machine of every platform, and
    /// nothing above it asked whose desktop this was.
    ///
    /// Two fields rather than one, because they answer two questions: the banner is owed or
    /// it is not, and the settings row that offers to bring it back exists or it does not.
    #[test]
    fn a_fresh_machine_is_offered_the_overflow_hint_on_windows_and_nowhere_else() {
        let strings = Strings::new("en");
        let ui = fresh_state(Overrides::default()).ui(&strings);

        assert_eq!(
            ui.hint_available,
            cfg!(target_os = "windows"),
            "the hint is about the Windows 11 overflow flyout"
        );
        assert_eq!(
            ui.hint_dismissed,
            !cfg!(target_os = "windows"),
            "a fresh Windows machine is owed the hint; a fresh Linux or macOS one is not"
        );
    }

    /// `--hint on` is a screenshot flag, and it does not travel to a platform with no
    /// banner to photograph.
    #[test]
    fn the_screenshot_flag_cannot_conjure_a_banner_the_platform_has_not_got() {
        let strings = Strings::new("en");
        let ui = fresh_state(Overrides {
            hint_dismissed: Some(false),
            ..Overrides::default()
        })
        .ui(&strings);

        assert_eq!(ui.hint_dismissed, !cfg!(target_os = "windows"));
    }

    #[test]
    fn the_form_is_the_settings_document_and_goes_back_into_it() {
        let config = Config {
            locale: Some("tr".to_owned()),
            theme: "graphite".to_owned(),
            theme_mode: "dark".to_owned(),
            notifications: false,
            quiet_hours: Some(QuietHours {
                from: "22:00".to_owned(),
                to: "07:00".to_owned(),
            }),
            providers: ProviderSwitches {
                claude: true,
                codex: false,
            },
            detailed_windows: true,
            ..Config::default()
        };

        let form = SettingsForm::from_config(&config);
        assert_eq!(form.locale, "tr");
        assert!(!form.notifications);
        assert!(!form.providers.codex);

        let mut round_trip = Config::default();
        form.apply_to(&mut round_trip);
        assert_eq!(round_trip.locale, config.locale);
        assert_eq!(round_trip.theme, config.theme);
        assert_eq!(round_trip.quiet_hours, config.quiet_hours);
        assert_eq!(round_trip.providers, config.providers);
        assert!(round_trip.detailed_windows);
    }

    #[test]
    fn follow_the_system_is_the_absence_of_a_language_rather_than_a_word() {
        let mut config = Config {
            locale: Some("tr".to_owned()),
            ..Config::default()
        };
        let form = SettingsForm {
            locale: nazar_core::config::LOCALE_SYSTEM.to_owned(),
            ..SettingsForm::from_config(&config)
        };
        form.apply_to(&mut config);
        assert_eq!(
            config.locale, None,
            "`system` in the form is no key in the file: a choice nobody made must not be \
             written down as one"
        );
        assert_eq!(
            SettingsForm::from_config(&config).locale,
            nazar_core::config::LOCALE_SYSTEM,
            "and it comes back as the word the form uses"
        );
    }

    #[test]
    fn the_form_leaves_every_key_it_does_not_own_alone() {
        let mut config = Config::from_json(
            r#"{ "schemaVersion": 1, "firstRunHintDismissed": true,
                 "detailedSuggested": true, "somethingNewer": 42 }"#,
        )
        .unwrap();

        SettingsForm::from_config(&Config::default()).apply_to(&mut config);
        assert!(
            config.first_run_hint_dismissed,
            "saving the settings must not un-dismiss the first-run hint"
        );
        assert!(config.detailed_suggested);
        assert_eq!(config.extra["somethingNewer"], serde_json::json!(42));
    }

    #[test]
    fn a_settings_form_serialises_the_way_the_panel_writes_it() {
        let form = SettingsForm::from_config(&Config::default());
        let json = serde_json::to_string(&form).unwrap();
        for key in [
            "\"locale\"",
            "\"themeMode\"",
            "\"notifications\"",
            "\"quietHours\"",
            "\"thresholds\"",
            "\"providers\"",
            "\"detailedWindows\"",
            "\"usageCountLikeClaudeCode\"",
            "\"usageFillHistoryFromStats\"",
        ] {
            assert!(json.contains(key), "{key} is missing from {json}");
        }
        assert_eq!(
            serde_json::from_str::<SettingsForm>(&json).unwrap(),
            form,
            "the form the panel sends has to be the form Rust reads"
        );
    }

    #[test]
    fn the_two_usage_switches_are_off_on_a_fresh_machine_and_survive_a_round_trip() {
        // Off is the honest default on both: what the panel shows without them is what this
        // machine spent, counted from logs this product read itself.
        let fresh = SettingsForm::from_config(&Config::default());
        assert!(!fresh.usage_count_like_claude_code);
        assert!(!fresh.usage_fill_history_from_stats);

        let form = SettingsForm {
            usage_count_like_claude_code: true,
            usage_fill_history_from_stats: true,
            ..fresh
        };
        let mut config = Config::default();
        form.apply_to(&mut config);
        assert!(config.usage.count_like_claude_code);
        assert!(config.usage.fill_history_from_stats);
        assert_eq!(SettingsForm::from_config(&config), form);

        // And they are nested under one key rather than loose at the top level, so the file
        // says what they are about.
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            json.contains("\"usage\":{\"countLikeClaudeCode\":true"),
            "the settings document spells them under `usage`: {json}"
        );
    }

    #[test]
    fn a_settings_file_written_before_the_two_switches_reads_as_both_off() {
        let config = Config::from_json(r#"{ "schemaVersion": 1, "theme": "graphite" }"#).unwrap();
        assert!(!config.usage.count_like_claude_code);
        assert!(!config.usage.fill_history_from_stats);
    }

    #[test]
    fn a_path_under_the_home_directory_is_shown_with_a_tilde() {
        let Ok(home) = paths::home_dir() else { return };
        let collapsed = collapse(Some(home.join(".nazar").join("limits.json")));
        assert!(collapsed.starts_with('~'), "got {collapsed}");
        assert!(collapsed.ends_with("limits.json"), "got {collapsed}");
        assert!(
            !collapsed.contains(&home.display().to_string()),
            "a user name must not end up in a screenshot of the settings page"
        );

        let elsewhere = PathBuf::from(if cfg!(windows) { "D:\\data" } else { "/data" });
        assert_eq!(
            collapse(Some(elsewhere.clone())),
            elsewhere.display().to_string()
        );
        assert_eq!(collapse(None), "");
    }

    #[test]
    fn the_settings_page_names_all_four_files() {
        if paths::home_dir().is_err() {
            return;
        }
        let shown = settings_paths();
        assert!(shown.config.ends_with("config.json"), "got {shown:?}");
        assert!(shown.limits.ends_with("limits.json"), "got {shown:?}");
        assert!(shown.alerts.ends_with("alerts.json"), "got {shown:?}");
        assert!(shown.captures.ends_with("statusline"), "got {shown:?}");
    }

    use nazar_core::state::{Freshness, ProviderView, Severity};

    /// A view with one Claude provider, as the panel would receive it.
    fn claude_view(configured: bool, plan: Option<&str>, source: Option<&str>) -> SnapshotView {
        SnapshotView {
            updated_at: String::new(),
            now: String::new(),
            providers: vec![ProviderView {
                name: "claude".to_owned(),
                configured,
                plan: plan.map(str::to_owned),
                source: source.map(str::to_owned),
                source_at: None,
                binding: None,
                age_ms: None,
                freshness: Freshness::Fresh,
                severity: Severity::Unknown,
                windows: Vec::new(),
            }],
        }
    }

    #[test]
    fn the_offer_is_made_on_the_passive_path_where_the_plan_is_unknowable() {
        // The case that actually happens. WP2 derives no plan from the status-line payload
        // because the payload does not carry one, so on a fresh machine `plan` is absent —
        // and a rule that only fired for a *known* Max plan would never fire at all.
        let fresh = Config::default();
        assert!(
            suggest_detailed(&fresh, &claude_view(true, None, Some("statusline"))),
            "the app cannot tell a Max account from a Pro one from here, which is exactly \
             why the banner is a question"
        );
        assert!(
            suggest_detailed(&fresh, &claude_view(true, None, None)),
            "a provider that has not said where its numbers came from is still the passive \
             path as far as this question goes"
        );
        assert!(
            !suggest_detailed(&fresh, &claude_view(true, None, Some("endpoint"))),
            "numbers from the endpoint mean the mode is already answering the question"
        );
    }

    #[test]
    fn the_offer_is_made_once_and_never_to_somebody_it_cannot_help() {
        let fresh = Config::default();

        assert!(
            suggest_detailed(
                &fresh,
                &claude_view(true, Some("max_20x"), Some("endpoint"))
            ),
            "a Max user's binding window is invisible to the passive path; that is the \
             whole reason WP2b exists"
        );
        assert!(
            !suggest_detailed(&fresh, &claude_view(true, Some("pro"), Some("endpoint"))),
            "for anybody else the global weekly window really is the binding one"
        );
        assert!(
            !suggest_detailed(&fresh, &claude_view(false, None, None)),
            "Claude is not set up here, and a banner about it would teach the user to \
             ignore banners"
        );
        assert!(
            !suggest_detailed(
                &fresh,
                &SnapshotView {
                    updated_at: String::new(),
                    now: String::new(),
                    providers: Vec::new(),
                }
            ),
            "and a view with no Claude at all offers nothing"
        );

        let asked = Config {
            detailed_suggested: true,
            ..Config::default()
        };
        assert!(
            !suggest_detailed(&asked, &claude_view(true, None, Some("statusline"))),
            "a user who said no is not asked again"
        );

        let already_on = Config {
            detailed_windows: true,
            ..Config::default()
        };
        assert!(!suggest_detailed(
            &already_on,
            &claude_view(true, None, Some("statusline"))
        ));
    }
}
