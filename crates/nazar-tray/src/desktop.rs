//! Whether this desktop has anywhere to put a tray icon — and what to do when it has not.
//!
//! On Windows and macOS the answer is always yes, and this module is four lines of `cfg`.
//! On Linux it is a question with a real answer, and getting it wrong is the worst first
//! impression this product can make.
//!
//! **The measurement.** A Linux tray icon is a `StatusNotifierItem` registered with a
//! *host* over D-Bus. `libappindicator` — which `tray-icon` loads at run time — registers
//! the item and returns success whether or not anybody is listening: on a GNOME session
//! with no AppIndicator extension the registration goes nowhere, the process stays up, no
//! error is printed, and nothing appears on screen. The Linux port audit watched exactly
//! that happen on 2026-09-15 (`~/.nazar/limits.json` was written, the icon PNG was
//! rasterised into `$XDG_RUNTIME_DIR/tray-icon/`, and the panel had no icon to be clicked
//! from), and the AppIndicator extension's **3.07 million** downloads say how many desktops
//! this is one installation step away from.
//!
//! So the question is asked before the icon is built, and it is asked of the bus rather
//! than of the desktop's name: `org.freedesktop.DBus.NameHasOwner("org.kde.StatusNotifier‑
//! Watcher")`. A name test rather than `$XDG_CURRENT_DESKTOP`, because the thing that
//! matters is whether a host is *running* — a GNOME session with the extension enabled
//! passes, a KDE session whose plasmashell has crashed fails, and an environment variable
//! knows neither.
//!
//! **What "no" means here is not "stop".** The tray is one of three things this process
//! does, and the other two do not need a panel to put an icon on:
//!
//! ```text
//!   watcher on the bus ─▶ tray icon + panel + refresh loop + notifications
//!   no watcher         ─▶            engine: refresh loop + notifications
//!                                    ~/.nazar/limits.json still written, and still the
//!                                    only thing Nazar's canvas, the GNOME panel and the
//!                                    Waybar module in `faces/` ever read
//! ```
//!
//! That is the whole of [`Mode::Engine`]: the refresh loop, the advisory lock, the atomic
//! writes and the threshold notifications are untouched, and the only thing skipped is the
//! icon that had nowhere to go. It is also what the contract was designed for — one writer,
//! many readers — so a desktop that cannot draw our icon can still draw somebody's.
//!
//! **And the user is told, once.** [`announce`] shows one desktop notification the first
//! time a machine runs in engine mode, because "I installed it and nothing happened" is the
//! failure mode a silent fallback produces. `org.freedesktop.Notifications` is implemented
//! by GNOME itself and needs no extension, which is what makes this the one channel that is
//! certain to reach the user whose tray does not work. It is claimed in `alerts.json` like
//! every other thing this product says once.
//!
//! **A watcher that arrives later is not noticed, on purpose.** A user who installs the
//! AppIndicator extension while the tray is running has to restart nazar-tray, and that is
//! written in the README rather than solved: the extension itself needs the GNOME Shell
//! session restarted on Wayland before it loads at all, so the session has already been
//! through something far heavier than a restart of this process by the time the name
//! appears. Watching `NameOwnerChanged` for a rare event whose cheaper path is a relog
//! would be a subscription held for the life of every Linux process, to save a step the
//! user has already taken.

use tauri::AppHandle;

/// The `alerts.json` key under which the "no tray host" notice is claimed.
///
/// Namespaced like a locale key and for the same reason: `alerts.json` also holds
/// `provider/window` records, and the two must never be able to collide.
const HIDDEN_NOTICE: &str = "notice.trayHidden";

/// The bus name a `StatusNotifierItem` host takes. KDE's spelling is the one everybody
/// implements, GNOME's AppIndicator extension included.
#[cfg(target_os = "linux")]
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

/// How this process runs: with an icon, or as the engine behind one somebody else draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A tray icon is built and the panel opens from it.
    Tray,
    /// No icon: the refresh loop, `limits.json` and the notifications, and nothing drawn.
    ///
    /// Carries **why**, because the two reasons deserve different sentences and only one of
    /// them is worth interrupting the user about.
    Engine(Reason),
}

/// Why a run has no tray icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// `--headless`: the user asked for it.
    Asked,
    /// Linux with no `org.kde.StatusNotifierWatcher` on the session bus.
    NoHost,
}

impl Mode {
    /// Whether this run builds a tray icon.
    #[must_use]
    pub fn draws_an_icon(self) -> bool {
        matches!(self, Mode::Tray)
    }

    /// One line for a terminal, in English like everything else on standard error.
    ///
    /// Printed at start-up and by `--print`, which is how somebody debugging a machine that
    /// "does nothing" finds out that it is doing all of its work and none of its drawing.
    #[must_use]
    pub fn status(self) -> &'static str {
        match self {
            Mode::Tray => "tray icon shown",
            Mode::Engine(Reason::Asked) => {
                "engine mode (--headless): no tray icon, limits.json still written"
            }
            Mode::Engine(Reason::NoHost) => {
                "engine mode: no StatusNotifierWatcher on the session bus; limits.json still written"
            }
        }
    }
}

/// Decide how this run draws itself.
///
/// `headless` is the command line's answer and wins outright: a user who asked for the
/// engine gets it on a desktop that would happily have shown an icon.
#[must_use]
pub fn mode(headless: bool) -> Mode {
    if headless {
        return Mode::Engine(Reason::Asked);
    }
    if tray_host_present() {
        Mode::Tray
    } else {
        Mode::Engine(Reason::NoHost)
    }
}

/// Say on standard error how this run is drawing itself, and — once per machine — tell the
/// user on their desktop when the answer is one they did not ask for.
///
/// Called from the setup, after the notifier is managed and instead of building the tray.
pub fn announce(app: &AppHandle, mode: Mode) {
    eprintln!("nazar-tray: {}", mode.status());
    if mode != Mode::Engine(Reason::NoHost) {
        return;
    }
    crate::alerts::notice(app, HIDDEN_NOTICE, "tray.hidden.title", "tray.hidden.body");
}

/// Whether a `StatusNotifierItem` host is on the session bus.
///
/// `zbus` rather than a hand-rolled D-Bus client, and not a new dependency in any sense
/// that costs anything: `tauri-plugin-notification` already pulls it in on Linux through
/// `notify-rust`, so it is in `Cargo.lock`, already compiled for this target, and in the
/// notices either way. What is new is one feature flag — `blocking-api`, which is a
/// question asked once at start-up on a thread that has nothing else to do, against an
/// async API that would need a runtime this process does not have.
///
/// **Every failure answers "no host".** A session with no bus at all (a machine with no
/// desktop, a container), a bus that will not answer, a reply that is not a boolean: each
/// of them means the icon would go nowhere, which is the same thing the honest `false`
/// means. The cost of a wrong `false` is a notification and a working engine; the cost of a
/// wrong `true` is the silent invisible tray this whole module exists to prevent.
#[cfg(target_os = "linux")]
fn tray_host_present() -> bool {
    let Ok(connection) = zbus::blocking::Connection::session() else {
        return false;
    };
    let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&connection) else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(WATCHER) else {
        return false;
    };
    dbus.name_has_owner(name).unwrap_or(false)
}

/// Windows and macOS both have a tray that is part of the shell, so there is nothing to
/// ask: an icon handed to either of them is drawn.
///
/// This is also the branch every non-Linux Unix takes. A BSD running KDE would be told it
/// has no host and would run as the engine, which is wrong but safe — and it is a platform
/// nothing in this repository builds for, so the alternative is an untested `cfg`.
#[cfg(not(target_os = "linux"))]
fn tray_host_present() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The command line wins over the desktop, on every platform.
    #[test]
    fn headless_is_engine_mode_wherever_it_is_asked_for() {
        assert_eq!(mode(true), Mode::Engine(Reason::Asked));
        assert!(!mode(true).draws_an_icon());
    }

    /// On Windows and macOS the answer is not a measurement, so it cannot come back wrong.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn a_shell_with_its_own_tray_is_never_asked() {
        assert_eq!(mode(false), Mode::Tray);
        assert!(mode(false).draws_an_icon());
    }

    /// On Linux it is whatever the bus said, and the test's job is only that asking is
    /// safe: a runner with no session bus must answer rather than panic or hang.
    #[cfg(target_os = "linux")]
    #[test]
    fn asking_the_bus_is_safe_even_where_there_is_no_bus() {
        let answer = mode(false);
        assert!(matches!(answer, Mode::Tray | Mode::Engine(Reason::NoHost)));
        assert_eq!(answer.draws_an_icon(), answer == Mode::Tray);
    }

    /// Three modes, three different sentences, and none of them empty — the line is what a
    /// user debugging a silent machine reads.
    #[test]
    fn every_mode_says_something_different_about_itself() {
        let lines = [
            Mode::Tray.status(),
            Mode::Engine(Reason::Asked).status(),
            Mode::Engine(Reason::NoHost).status(),
        ];
        assert!(lines.iter().all(|line| !line.is_empty()));
        assert_eq!(
            lines
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        assert!(
            lines[2].contains("StatusNotifierWatcher"),
            "the one sentence that has to name the thing to look for: {}",
            lines[2]
        );
    }
}
