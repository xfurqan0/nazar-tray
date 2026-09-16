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
//! **A watcher that arrives later is noticed** — and that sentence used to say the opposite.
//! T-WP-L2 decided against the subscription because a host appearing mid-run meant the
//! AppIndicator extension being installed, which needs the GNOME session restarted on
//! Wayland anyway: a subscription held for the life of every Linux process, to save a step
//! the user had already taken. **The premise was wrong, and the machine that proved it is
//! the one this was written on.** GNOME disables extensions while the session is locked, and
//! the watcher's bus name goes with them: `org.kde.StatusNotifierWatcher` was on the bus at
//! 00:03 on 2026-09-17 and gone at 00:25, with `gnome-shell` still on the same pid. So a
//! tray started while the screen is locked — which is what autostart does to anybody who
//! locks the machine and walks away — falls into engine mode and stays there until somebody
//! restarts it by hand. That is not a rare event with a cheaper path; it is every morning.
//!
//! [`watch`] therefore holds one `NameOwnerChanged` subscription, narrowed by a match rule
//! to that one bus name so the bus sends nothing else, and [`Presence`] is the whole of what
//! this process does about it:
//!
//! ```text
//!   arrives, no icon yet ─▶ build the icon — this is the run that started behind a lock
//!   arrives, icon built  ─▶ nothing: libappindicator re-registers the item by itself
//!   leaves               ─▶ note it. The icon object stays, the engine is untouched
//!   --headless           ─▶ nothing, ever. The user asked for the engine
//! ```
//!
//! **And none of it is worth a notification.** The one-off "no tray host" notice belongs to
//! the start-up question and stays there ([`Mode::worth_a_notice`]): a watcher leaving is
//! not a thing to interrupt somebody about, and a watcher arriving *is* an icon appearing,
//! which says it better than a toast could. The alternative is a machine that says something
//! about its tray every morning, to a user who just unlocked a working one.

use tauri::AppHandle;
// `try_state` and `run_on_main_thread`, which only the watcher thread needs.
#[cfg(target_os = "linux")]
use tauri::Manager;

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

    /// Whether this run has something to say to the user about the tray host.
    ///
    /// One expression, read by the two places that have to agree about it: [`announce`],
    /// which shows the notice, and [`Presence::start`], which is the state machine's account
    /// of the same run. **Only the mode nobody asked for qualifies** — `--headless` was
    /// requested and a desktop that drew the icon has already said everything by drawing it.
    #[must_use]
    pub fn worth_a_notice(self) -> bool {
        self == Mode::Engine(Reason::NoHost)
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
    if mode.worth_a_notice() {
        crate::alerts::notice(app, HIDDEN_NOTICE, "tray.hidden.title", "tray.hidden.body");
        return;
    }
    announce_gnome_indicator(app, mode);
}

/// What this run has on screen, and what it may still grow — the whole watcher state
/// machine, in three bits and no dependencies.
///
/// No bus, no Tauri, no clock, so "what happens when the watcher comes back for the third
/// time" is answered by a test rather than by a lock screen at two in the morning.
/// [`watch`] is the only thing that turns its answers into calls, and what it may answer is
/// [`Step`].
///
/// **`built` never goes back to false, and that is the load-bearing part.** The icon is
/// built once: libappindicator holds its own watch on the same bus name and re-registers
/// the item when a host returns, so a second [`crate::tray::install`] would be a second
/// `TrayIcon` under one id, a second menu, and a `QuotaRows` that no refresh writes to —
/// two beads and one of them frozen. What a watcher leaving changes is `shown`, which costs
/// nothing to be wrong about and keeps the log honest.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presence {
    /// Whether a `TrayIcon` exists in this process at all.
    built: bool,
    /// Whether a host is on the bus to draw it. `false` is the hidden state: nothing is
    /// drawn, and the refresh loop, the lock and the notifications do not notice.
    shown: bool,
    /// `--headless`. A watcher arriving is not an argument against what the user asked for.
    asked: bool,
}

/// What the bus said about `org.kde.StatusNotifierWatcher`.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Watcher {
    /// The name gained an owner: a host is listening.
    Arrived,
    /// The name lost its owner: a locked GNOME session, a crashed plasmashell, an extension
    /// turned off.
    Left,
}

/// What a change of presence is worth doing.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Build the tray icon. This run has not got one and now has somewhere to put it.
    Draw,
    /// Record that nothing is drawing the icon. One line on standard error and no GTK call:
    /// see [`Presence`] for why the item is left alone.
    Hide,
    /// Show the one-off "no tray host" notice. Returned by [`Presence::start`] and by
    /// nothing else, ever — the test below is what says so.
    Tell,
    /// Nothing to do.
    Rest,
}

#[cfg(any(target_os = "linux", test))]
impl Presence {
    /// Where a run begins, and the one thing it says on the way in.
    ///
    /// The [`Step`] is [`announce`]'s, and the two agree because both ask
    /// [`Mode::worth_a_notice`] rather than each deciding for itself.
    #[must_use]
    pub fn start(mode: Mode) -> (Self, Step) {
        let drawn = mode.draws_an_icon();
        let presence = Presence {
            built: drawn,
            shown: drawn,
            asked: mode == Mode::Engine(Reason::Asked),
        };
        let step = if mode.worth_a_notice() {
            Step::Tell
        } else {
            Step::Rest
        };
        (presence, step)
    }

    /// Whether there is any point subscribing to the bus for this run.
    #[must_use]
    pub fn listens(self) -> bool {
        !self.asked
    }

    /// Take one change of presence and say what it is worth doing.
    ///
    /// Idempotent in both directions: a second `Arrived` while an icon is up and a second
    /// `Left` while nothing is drawn are both [`Step::Rest`], because a bus can repeat
    /// itself — a shell restarting emits the name twice — and a user should not get two
    /// icons or two lines of log for one event.
    pub fn step(&mut self, watcher: Watcher) -> Step {
        if self.asked {
            return Step::Rest;
        }
        match watcher {
            Watcher::Arrived => {
                self.shown = true;
                if self.built {
                    Step::Rest
                } else {
                    self.built = true;
                    Step::Draw
                }
            }
            Watcher::Left if self.shown => {
                self.shown = false;
                Step::Hide
            }
            Watcher::Left => Step::Rest,
        }
    }
}

/// Follow `org.kde.StatusNotifierWatcher` for the life of the process.
///
/// One thread, one match rule, and a blocking iterator that spends its life parked in
/// `poll`: the bus filters on our behalf, so nothing is delivered to this process unless
/// that one name changes hands. Called from the setup, after the tray has or has not been
/// built — [`Presence::start`] is handed the same [`Mode`] the icon was built from, so the
/// thread starts out knowing what is on screen.
///
/// **Every failure is silence rather than a dead process.** No session bus, a bus that
/// refuses the match rule, a signal whose body will not deserialise: each of them leaves the
/// run exactly as T-WP-L2 left it, which is a working engine with whatever icon it started
/// with. A tray must not fall over because a subscription could not be held.
#[cfg(target_os = "linux")]
pub fn watch(app: &AppHandle, mode: Mode) {
    let (presence, _told_by_announce) = Presence::start(mode);
    if !presence.listens() {
        return;
    }
    let app = app.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("nazar-watcher".to_owned())
        .spawn(move || follow(&app, presence))
    {
        eprintln!("nazar-tray: could not watch for a tray host: {error}");
    }
}

/// Windows and macOS have their tray inside the shell: it is there before this process
/// starts and it is there after, so there is no name to follow and nothing to change.
#[cfg(not(target_os = "linux"))]
pub fn watch(_app: &AppHandle, _mode: Mode) {}

/// The subscription itself, on its own thread.
#[cfg(target_os = "linux")]
fn follow(app: &AppHandle, mut presence: Presence) {
    let Ok(connection) = zbus::blocking::Connection::session() else {
        eprintln!("nazar-tray: no session bus; a tray host arriving later will not be seen");
        return;
    };
    let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&connection) else {
        return;
    };
    // Arg 0 of `NameOwnerChanged` is the name that changed hands, so this match rule is the
    // difference between one wake-up a day and one per service the session starts.
    let signals = match dbus.receive_name_owner_changed_with_args(&[(0, WATCHER)]) {
        Ok(signals) => signals,
        Err(error) => {
            eprintln!("nazar-tray: could not watch for a tray host: {error}");
            return;
        }
    };

    // The rule is on the bus now, so anything that happens from here is ahead of us rather
    // than lost. What is behind us is the gap between `mode`'s question and this line — a
    // few milliseconds in which a session can finish unlocking — so the question is asked
    // once more, against the same connection, and the answer goes through the same machine.
    if has_owner(&dbus) {
        act(app, &mut presence, Watcher::Arrived);
    }

    for signal in signals {
        let Ok(args) = signal.args() else {
            continue;
        };
        // An empty new owner is how the bus spells "this name is gone".
        let watcher = if args.new_owner().is_some() {
            Watcher::Arrived
        } else {
            Watcher::Left
        };
        act(app, &mut presence, watcher);
    }
}

/// Step the machine and do what it says.
#[cfg(target_os = "linux")]
fn act(app: &AppHandle, presence: &mut Presence, watcher: Watcher) {
    match presence.step(watcher) {
        Step::Draw => draw(app),
        Step::Hide => eprintln!(
            "nazar-tray: the StatusNotifierWatcher left the session bus; nothing is drawing \
             the icon, and limits.json is still written"
        ),
        // `Tell` is the start-up notice and cannot come from an event; `Rest` is the common
        // case and says nothing, because a log line per unlock is its own kind of noise.
        Step::Tell | Step::Rest => {}
    }
}

/// Build the icon, on the thread GTK belongs to.
///
/// The bus thread decides and the main thread draws: everything under
/// [`crate::tray::install`] ends in a GTK call, and `tray-icon`'s GTK backend is not a thing
/// to touch from a thread that is not the one GTK was initialised on.
#[cfg(target_os = "linux")]
fn draw(app: &AppHandle) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        // Torn down between the signal and the main thread's next turn. `install` asks for
        // the state rather than trying for it, so this is checked rather than risked.
        let (Some(strings), Some(_)) = (
            handle.try_state::<std::sync::Arc<crate::i18n::Strings>>(),
            handle.try_state::<crate::state::AppState>(),
        ) else {
            return;
        };
        let strings = strings.inner().clone();
        if let Err(error) = crate::tray::install(&handle, &strings) {
            eprintln!("nazar-tray: a tray host arrived and the icon could not be built: {error}");
            return;
        }
        crate::tray::refresh(&handle, &strings.catalog());
        eprintln!("nazar-tray: a StatusNotifierWatcher arrived; the tray icon is up");
    }) {
        eprintln!("nazar-tray: could not reach the main thread to build the icon: {error}");
    }
}

/// Whether the watcher's name has an owner, asked of a proxy somebody else opened.
#[cfg(target_os = "linux")]
fn has_owner(dbus: &zbus::blocking::fdo::DBusProxy<'_>) -> bool {
    let Ok(name) = zbus::names::BusName::try_from(WATCHER) else {
        return false;
    };
    dbus.name_has_owner(name).unwrap_or(false)
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
    has_owner(&dbus)
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

    /// Every event a bus can deliver, in the order a locked GNOME session delivers them.
    ///
    /// Six modes of failure in one sequence, and the machine is small enough that all of
    /// them fit: the autostarted run that started behind a lock screen, the unlock, the
    /// second lock, the shell restart that emits the name twice, and the `--headless` run
    /// that must ignore the lot.
    const A_MORNING: [Watcher; 6] = [
        Watcher::Arrived,
        Watcher::Left,
        Watcher::Left,
        Watcher::Arrived,
        Watcher::Arrived,
        Watcher::Left,
    ];

    /// The package: a run that started with no host gets an icon when one turns up.
    ///
    /// This is the autostart-into-a-lock-screen case measured on 2026-09-17. Before
    /// T-WP-L10 the first line was the whole of the run's life.
    #[test]
    fn a_run_that_started_behind_a_lock_screen_draws_its_icon_when_the_screen_comes_back() {
        let (mut presence, start) = Presence::start(Mode::Engine(Reason::NoHost));
        assert_eq!(start, Step::Tell, "the machine had nowhere to put an icon");
        assert!(
            presence.listens(),
            "and nothing else can change that but the bus"
        );

        assert_eq!(presence.step(Watcher::Arrived), Step::Draw);
        assert_eq!(
            presence.step(Watcher::Left),
            Step::Hide,
            "locked again: the icon has nowhere to be drawn and the engine keeps going"
        );
        assert_eq!(
            presence.step(Watcher::Arrived),
            Step::Rest,
            "the icon already exists; libappindicator re-registers it by itself"
        );
    }

    /// An icon is built once per run, whatever the bus does afterwards.
    ///
    /// Two `TrayIcon`s under one id would be two beads, one of them frozen: `QuotaRows` is
    /// managed state and the second `install` cannot replace it.
    #[test]
    fn an_icon_is_built_once_and_a_repeated_signal_is_not_a_second_one() {
        let (mut presence, start) = Presence::start(Mode::Tray);
        assert_eq!(start, Step::Rest, "an icon that was drawn has said it all");

        let steps: Vec<Step> = A_MORNING
            .iter()
            .map(|watcher| presence.step(*watcher))
            .collect();
        assert_eq!(
            steps,
            [
                Step::Rest, // arrived, and one was already drawn
                Step::Hide, // left
                Step::Rest, // left again: the bus repeating itself is not a second event
                Step::Rest, // back, and the icon we have is the icon it draws
                Step::Rest, // back again
                Step::Hide, // and gone
            ]
        );
        assert_eq!(
            steps.iter().filter(|step| **step == Step::Draw).count(),
            0,
            "the icon was built before the first signal arrived"
        );
    }

    /// `--headless` is a decision, and the bus does not get a vote on it.
    #[test]
    fn a_headless_run_never_grows_an_icon() {
        let (mut presence, start) = Presence::start(Mode::Engine(Reason::Asked));
        assert_eq!(start, Step::Rest, "the user asked for this and knows");
        assert!(
            !presence.listens(),
            "and there is nothing to subscribe to the bus for"
        );
        for watcher in A_MORNING {
            assert_eq!(presence.step(watcher), Step::Rest);
        }
    }

    /// The notification counter: once per run at most, and only for the run that never had
    /// a host to begin with.
    ///
    /// **The half that matters is the zero.** A desktop notification every time a GNOME
    /// session is unlocked would be this application interrupting somebody to tell them
    /// about a tray that works — which is the thing that makes the whole subscription worth
    /// less than nothing.
    #[test]
    fn the_user_hears_about_a_missing_host_once_and_never_because_one_came_back() {
        for mode in [
            Mode::Tray,
            Mode::Engine(Reason::NoHost),
            Mode::Engine(Reason::Asked),
        ] {
            let (mut presence, start) = Presence::start(mode);
            let mut told = usize::from(start == Step::Tell);
            for watcher in A_MORNING {
                told += usize::from(presence.step(watcher) == Step::Tell);
            }
            assert_eq!(
                told,
                usize::from(mode.worth_a_notice()),
                "{mode:?} was told {told} times about its tray host"
            );
        }
    }

    /// Who may say it: one expression, read by the notice and by the machine alike.
    #[test]
    fn only_a_run_nobody_asked_for_is_worth_interrupting() {
        assert!(Mode::Engine(Reason::NoHost).worth_a_notice());
        assert!(!Mode::Engine(Reason::Asked).worth_a_notice());
        assert!(!Mode::Tray.worth_a_notice());
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
