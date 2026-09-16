//! Whether this desktop has anywhere to put a tray icon — and what to do when it has not.
//!
//! It also answers the smaller question that turns out to have the same shape: **which of
//! the things this build says once per machine are true on this desktop at all.** A first
//! run is the one moment a user has no idea what they are looking at, and a sentence that
//! names a control their desktop does not have spends that moment telling them to go and
//! look for something that is not there. See [`first_run_notices`].
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
//! **The second question this desktop answers is where a window goes**, and on Wayland the
//! answer is "not where you asked". A Wayland client cannot read the global pointer and
//! cannot move its own toplevel; Tauri's `cursor_position()` returns `Ok((0, 0))` rather
//! than an error and `set_position` returns `Ok(())` and does nothing, so the panel's
//! careful arithmetic about which side of the cursor to open on runs on a lie and the
//! compositor centres the window anyway. That is [`window_placement`], and what it means
//! for the tray is in [`crate::tray`]: where the panel cannot be put beside the icon, the
//! numbers go **into the menu**, which the shell does position beside the icon.
//!
//! **And the user is told, once.** [`announce`] shows one desktop notification the first
//! time a machine runs in engine mode, because "I installed it and nothing happened" is the
//! failure mode a silent fallback produces. It shows a second one on GNOME, where the tray
//! icon works but the panel cannot be placed: `nazar-gnome` draws the numbers in the panel
//! the shell owns, which is the only surface on a GNOME session that can be beside the
//! icon at all. `org.freedesktop.Notifications` is implemented
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

/// Whether the panel's first-run overflow hint is about anything on this platform.
///
/// **Windows 11 hides every new tray icon behind the `^` button**, so the first run says so
/// and tells the user to drag the bead onto the taskbar. Neither half of that sentence
/// exists anywhere else: a Linux tray icon is a `StatusNotifierItem` published on D-Bus and
/// drawn by whatever is listening — there is no overflow to be behind and nothing to drag
/// — and the macOS menu bar shows every item it is handed. On 2026-09-16 a Fedora 44 GNOME
/// session was told to drag a bead into a taskbar it does not have, which is where this
/// constant comes from.
///
/// A compile-time `cfg!` rather than a runtime question, because the answer is the
/// operating system's shell and not the session's: it cannot change while the process runs
/// and it cannot be measured wrong.
pub const OVERFLOW_HINT: bool = cfg!(target_os = "windows");

/// One thing this build says to a user exactly once per machine.
///
/// Compiled only into the test build: nothing in the running tray reads the list, because
/// each notice is produced where it belongs — the banner by the panel, the notification by
/// [`announce`]. What the list is for is the question no single call site can answer, which
/// is what a fresh machine of *this* platform ends up being told.
#[cfg(test)]
pub struct FirstRun {
    /// Where "once" is remembered. `alerts.json`'s notice set for a desktop notification,
    /// and `None` for the panel banner — which is claimed by `config.json`'s
    /// `firstRunHintDismissed` instead, because the user dismisses it with a button.
    pub claim: Option<&'static str>,
    /// The locale key of the text the user actually reads. A notification's title, for the
    /// ones that have a body as well.
    pub message: &'static str,
}

/// Everything a fresh machine of this platform can be told once, and nothing it cannot.
///
/// The list is small enough to read and that is the point of it: every entry here is a
/// sentence written for one desktop, and the test below asks each one whether it is a
/// sentence this build can still produce in all six languages. Adding a notice without
/// adding it here costs nothing, so the list is not a registry the code reads — it is the
/// per-platform answer, written down where the platform question already lives.
#[cfg(test)]
#[must_use]
pub fn first_run_notices() -> Vec<FirstRun> {
    let mut notices = Vec::new();
    if OVERFLOW_HINT {
        notices.push(FirstRun {
            claim: None,
            message: "panel.hint.overflow",
        });
    }
    #[cfg(target_os = "linux")]
    {
        notices.push(FirstRun {
            claim: Some(HIDDEN_NOTICE),
            message: "tray.hidden.title",
        });
        notices.push(FirstRun {
            claim: Some(GNOME_NOTICE),
            message: "tray.gnome.title",
        });
    }
    notices
}

/// The `alerts.json` key under which the "there is a GNOME indicator for this" notice is
/// claimed. Namespaced like [`HIDDEN_NOTICE`], and for the same reason.
#[cfg(target_os = "linux")]
const GNOME_NOTICE: &str = "notice.gnomeIndicator";

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
    // Silence is the answer nobody needs: a run that drew its icon has said everything it
    // has to say by drawing it. `cli::run_if_requested` prints the line under the same rule.
    if !mode.draws_an_icon() {
        eprintln!("nazar-tray: {}", mode.status());
    }
    if mode == Mode::Engine(Reason::NoHost) {
        crate::alerts::notice(app, HIDDEN_NOTICE, "tray.hidden.title", "tray.hidden.body");
        return;
    }
    announce_gnome_indicator(app, mode);
}

/// On GNOME, once: the numbers can live in the panel, where the shell places them.
///
/// **This is a different failure from the one above, and it is the one qarpus hit.** The
/// AppIndicator extension was installed, the icon was drawn, and the click still did not
/// behave like a tray popup — because on GNOME Wayland nothing this process owns can be put
/// beside that icon. The menu is as close as the tray gets (the shell positions it, and
/// [`crate::tray`] fills it with the numbers for exactly that reason); a panel *indicator*
/// is closer still, and it is a GNOME Shell extension rather than anything a window can do.
///
/// Only on GNOME, only in tray mode, and only once. On KDE, XFCE, Cinnamon and Budgie the
/// tray is native and a second face would be two indicators saying the same thing; in engine
/// mode the notice above has already been shown and adding a second interruption to the same
/// start-up would be this application talking over itself.
#[cfg(target_os = "linux")]
fn announce_gnome_indicator(app: &AppHandle, mode: Mode) {
    if mode != Mode::Tray || !is_gnome() {
        return;
    }
    crate::alerts::notice(app, GNOME_NOTICE, "tray.gnome.title", "tray.gnome.body");
}

/// Nothing to say: neither other platform has a GNOME Shell to have an extension for.
#[cfg(not(target_os = "linux"))]
fn announce_gnome_indicator(_app: &AppHandle, _mode: Mode) {}

/// Whether this session is GNOME Shell.
///
/// `XDG_CURRENT_DESKTOP` rather than the bus, because unlike the tray host this is not a
/// service that can be up or down — it is which shell is drawing the screen, and the only
/// thing that answers it is the session's own environment. The value is colon-separated and
/// often prefixed (`ubuntu:GNOME`), so it is split rather than compared.
#[cfg(target_os = "linux")]
fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|value| {
        value
            .split(':')
            .any(|name| name.eq_ignore_ascii_case("GNOME"))
    })
}

/// Whether a window this process opens can be put where this process wants it.
///
/// Called once, from `main`, **before anything creates a GTK display** — it may set
/// `GDK_BACKEND`, and a backend chosen after GTK has initialised is a backend nobody uses.
///
/// | Session | `asked` | Answer |
/// |---|---|---|
/// | Windows, macOS | — | `true`; the shell has always let a window say where it goes |
/// | `GDK_BACKEND` already set | ignored | whatever the user typed |
/// | X11 (no `WAYLAND_DISPLAY`) | ignored | `true`; nothing to force |
/// | Wayland | `false` | `false`; the panel opens where the compositor puts it |
/// | Wayland, no `DISPLAY` | `true` | `false`; there is no XWayland to fall back to |
/// | Wayland | `true` | `true`, and `GDK_BACKEND=x11` is set |
///
/// `asked` is `config.window.x11Positioning`. An environment variable outranks it: somebody
/// who typed `GDK_BACKEND=wayland` in front of the command has made a decision, and a
/// settings file is not the place to overrule it.
#[cfg(target_os = "linux")]
#[must_use]
pub fn window_placement(asked: bool) -> bool {
    if let Ok(chosen) = std::env::var("GDK_BACKEND") {
        return chosen.split(',').any(|name| name == "x11");
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        // GTK takes X11 when there is no Wayland to prefer — and nothing at all when there
        // is no display either, in which case the placement question never comes up.
        return std::env::var_os("DISPLAY").is_some();
    }
    if !asked || std::env::var_os("DISPLAY").is_none() {
        return false;
    }
    // SAFETY: `main` calls this before the Tauri builder and before any thread of ours
    // exists, which is the same window `force_device_scale_factor` writes in.
    unsafe {
        std::env::set_var("GDK_BACKEND", "x11");
    }
    true
}

/// Windows and macOS place a window where they are told, and always have.
#[cfg(not(target_os = "linux"))]
#[must_use]
pub fn window_placement(_asked: bool) -> bool {
    true
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

    /// The first-run hint is Windows-only, and this is the whole of that claim.
    ///
    /// It was not, until 2026-09-16: a Fedora 44 GNOME session opened the panel and read
    /// "drag the bead onto the taskbar to pin it", about an overflow flyout that only
    /// Windows 11 has. The gate is a `cfg!`, so the test can only ever check the platform
    /// it is compiled for — which is exactly the three platforms CI compiles for.
    #[test]
    fn the_overflow_hint_belongs_to_windows_and_to_nowhere_else() {
        assert_eq!(
            OVERFLOW_HINT,
            cfg!(target_os = "windows"),
            "no other desktop has an overflow flyout to be behind or a taskbar to be \
             dragged onto"
        );
        // And the sentence it gates really is about that flyout, in English at least: a
        // hint reworded into something every desktop has would want a different gate.
        assert!(
            crate::i18n::catalog("en")
                .text("panel.hint.overflow")
                .contains("overflow")
        );
    }

    /// What a fresh machine of this platform is told once — the list, per platform.
    ///
    /// Two assertions, and the second is the one that keeps the list honest: every message
    /// it names has to still resolve in all six catalogues, so a key renamed in
    /// `ui/locales/*.json` fails here instead of shipping a notification that says
    /// `tray.hidden.title`.
    #[test]
    fn a_fresh_machine_is_told_what_its_own_desktop_can_act_on() {
        let notices = first_run_notices();
        let messages: Vec<&str> = notices.iter().map(|notice| notice.message).collect();

        #[cfg(target_os = "windows")]
        assert_eq!(
            messages,
            ["panel.hint.overflow"],
            "Windows has the overflow flyout and no tray host to be missing"
        );
        #[cfg(target_os = "linux")]
        assert_eq!(
            messages,
            ["tray.hidden.title", "tray.gnome.title"],
            "Linux has a tray host that may be absent, a shell that may have a better \
             face than ours, and no overflow flyout"
        );
        #[cfg(target_os = "macos")]
        assert!(
            messages.is_empty(),
            "the menu bar shows what it is handed, and it is always there"
        );

        for notice in &notices {
            for language in nazar_core::config::LOCALES {
                let catalog = crate::i18n::catalog(language);
                assert_ne!(
                    catalog.text(notice.message),
                    notice.message,
                    "{} has no text in {language}",
                    notice.message
                );
            }
            if let Some(claim) = notice.claim {
                assert!(
                    claim.starts_with("notice."),
                    "a notice key shares alerts.json with provider/window records: {claim}"
                );
            }
        }
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
