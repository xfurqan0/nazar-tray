//! The command-line modes: `nazar-tray --print`, and `--print --write`.
//!
//! `--print` builds the `limits.json` document from whichever readers exist and writes it
//! to standard output. Nothing else happens: no window, no tray icon, and nothing on disk.
//!
//! `--print --write` adds the one thing WP3 made possible — **writing the file** — for the
//! two cases the tray cannot cover:
//!
//! * **Scripts.** A cron job, a status line, a dashboard: anything that wants the file
//!   refreshed once and does not want a tray running to get it.
//! * **Linux**, where v1 ships no tray at all (`docs/PROJECT.md` section 3). The CLI plus
//!   the Nazar canvas *is* the Linux story, and this is the half that produces the file.
//!
//! It takes the same advisory lock the tray takes, and if the tray already holds it this
//! **reads instead of writing**: it prints the document the running tray maintains and
//! touches nothing. One writer, many readers, with no way to ask for an exception.
//!
//! One limitation, deliberate and documented rather than worked around here: the tray is a
//! GUI-subsystem binary on Windows, so a release build launched from an interactive console
//! has no console to print to. Redirected output — `nazar-tray --print > out.json` or a
//! pipe, which is how a consumer actually calls it — works in every build. Attaching to the
//! parent console belongs with the rest of the command-line surface in WP8.
//!
//! ## `--detailed`
//!
//! Turns the opt-in detailed-windows mode on **for one run**, without touching
//! `config.json`. It is here so the mode can be checked against a real account without
//! first switching it on for the machine, and so a script can ask for the model-scoped
//! numbers once. Everything the mode does with it on is what `docs/detailed-windows.md`
//! describes: one request, a token held in memory, nothing written.
//!
//! ## The screenshot flags: `--demo`, `--scale`, `--theme`, `--mode`, `--icons`
//!
//! WP4 has to produce pictures of states that are hard to arrange on purpose — a window
//! over the red threshold, one nobody could read, a reading an hour old — at three display
//! scales and in two themes. These five flags are how, and they are documented here rather
//! than hidden because a maintainer looking at `docs/screenshots` should be able to
//! reproduce every file in it:
//!
//! | Flag | What it does |
//! |---|---|
//! | `--demo` | synthetic numbers ([`crate::demo`]), panel open at start-up, no blur-hide, **no lock and no writing** |
//! | `--scale 1.5` | forces the webview's device scale factor and sizes the panel to match, so 150 % is photographable on a 100 % display |
//! | `--theme graphite` | overrides the theme for this run without touching settings |
//! | `--mode light` | overrides light/dark for this run without touching settings |
//! | `--hint on\|off` | shows or hides the first-run overflow hint, which is otherwise a once-per-machine state |
//! | `--offer on\|off` | the same, for the one-time Max-plan offer |
//! | `--locale tr` | the panel and the tooltip in one language, whatever the machine's is |
//! | `--icons <dir>` | writes the bead's two states — the mark, and unknown — at 16, 32 and 64 px, plus one strip of all six, then exits; nothing else happens |
//! | `--view settings` | opens the panel on the settings page, which is otherwise a click away |
//! | `--view usage` | the same for the usage view, and `--usage-tab` says which of its tabs |
//! | `--usage-tab <name>` | `week`, `weeks`, `all`, `models`, or `day` for the Week tab with its newest day opened |
//!
//! `--demo` is the one that matters for safety: a screenshot session must not be able to
//! overwrite the real `~/.nazar/limits.json` with invented numbers, so it never becomes the
//! writer at all. It also gets an **in-memory** notification log, so a demo run cannot
//! consume the keys of a real crossing that has not been shown yet.
//!
//! ## `--hidden` and `--demo-cross`
//!
//! `--hidden` is what the autostart entry passes. An ordinary launch already shows no window
//! — the panel is created invisible — so on its own the flag changes almost nothing, and that
//! is rather the point: it is a promise rather than a behaviour. Explicitly, it means **no
//! window opens at start-up for any reason**, including the one `--demo` would open.
//!
//! ## `--autostart on|off|status`
//!
//! The startup entry, from a terminal. It exists for two reasons and neither is a test:
//!
//! * **A user whose panel will not open** — a broken WebView2, a display that has gone away —
//!   still has to be able to stop an application from starting with Windows, and telling
//!   them to edit the registry is not an answer.
//! * **It is checkable.** `docs/PROJECT.md` WP5 asks for the round trip to be verified on a
//!   real machine and left off; the switch in the settings needs a person to click it, and a
//!   flag that prints what the registry now says can be run and read.
//!
//! It goes through the plugin, so it is the same code path the settings switch uses, and it
//! prints what the plugin reports **afterwards** rather than what was asked for. The process
//! then exits without a tray icon, without a panel, and without taking the advisory lock.
//!
//! One cosmetic artefact, so that nobody reads it as a failure: the panel window is created
//! by `tauri::Builder::build`, which runs before this does, so exiting here tears down a
//! WebView2 that never appeared and a debug build prints
//! `Failed to unregister class Chrome_WidgetWin_0` on standard error. The exit code is `0`
//! and standard output carries the answer.
//!
//! ## `--demo-cross`
//!
//! `--demo-cross` is the acceptance run for the notifications. It steps the demo document
//! through `80 → 86 → 86 → reset → 86` on the Codex weekly window, a few seconds apart, and
//! the toasts that appear are the whole test: one at 60 % on arrival, one at 85 % for the
//! crossing, **silence** for the repeat, and one more at 85 % after the reset. It implies
//! `--demo`, so nothing it does reaches `~/.nazar` or `%APPDATA%\nazar`.

use std::io::Write;

use nazar_core::claude::ClaudeReader;
use nazar_core::claude::detailed::{DetailedWindows, SystemClock};
use nazar_core::claude::merge::merge;
use nazar_core::codex::CodexReader;
use nazar_core::lock::{Acquisition, LimitsLock};
use nazar_core::writer::LimitsWriter;
use nazar_core::{Config, Limits, Provider, now_rfc3339, paths};

/// The flag that turns the tray into a one-shot printer.
const PRINT_FLAG: &str = "--print";

/// The flag that turns the opt-in mode on for this run only.
const DETAILED_FLAG: &str = "--detailed";

/// The flag that makes the one-shot run write `~/.nazar/limits.json` as well as print it.
const WRITE_FLAG: &str = "--write";

/// The flag that rasterises the tray bead into a directory and exits.
const ICONS_FLAG: &str = "--icons";

/// Run a command-line mode if one was asked for.
///
/// Returns `true` when the process has done its job and should exit without starting the
/// tray. Unknown arguments are ignored: this is a tray application that happens to have a
/// flag, not a command-line tool that happens to have a tray, so an argument it does not
/// understand must not stop it from starting.
pub fn run_if_requested() -> bool {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if let Some(directory) = value_of(&arguments, ICONS_FLAG) {
        return write_icons(std::path::Path::new(&directory));
    }
    if !arguments.iter().any(|argument| argument == PRINT_FLAG) {
        return false;
    }
    let asked = |flag: &str| arguments.iter().any(|argument| argument == flag);

    let document = if asked(WRITE_FLAG) {
        write_once(asked(DETAILED_FLAG))
    } else {
        snapshot(asked(DETAILED_FLAG))
    };

    let text = match document.to_json() {
        Ok(text) => text,
        Err(error) => {
            // The document is built from our own types, so this cannot happen; saying so
            // out loud is still better than an empty stdout and a zero exit code.
            eprintln!("nazar-tray: could not render limits.json: {error}");
            std::process::exit(1);
        }
    };

    let mut stdout = std::io::stdout().lock();
    if stdout.write_all(text.as_bytes()).is_err() {
        // A closed pipe (`nazar-tray --print | head`) is not an error worth a message.
        std::process::exit(0);
    }
    let _ = stdout.flush();
    true
}

/// The value that follows a `--flag`, or `None` when the flag is absent or last.
fn value_of(arguments: &[String], flag: &str) -> Option<String> {
    let at = arguments.iter().position(|argument| argument == flag)?;
    arguments.get(at + 1).cloned()
}

/// `--icons <dir>`: rasterise both bead states and the documentation strip.
///
/// Seven files: `bead-mark-*.png` and `bead-unknown-*.png` at 16, 32 and 64 pixels, and
/// `bead-states.png` with all six in one picture. Two states because the icon has two — it
/// stopped carrying the quota on 2026-09-09, and what is left is the mark and "nothing could
/// be read".
///
/// The tray itself never encodes a PNG — it hands the shell raw pixels — so this exists for
/// the repository rather than for the product: everything in `docs/design/` is produced by
/// the same code that draws the real icon, which is the only way a picture in the
/// documentation can be trusted to still be what the application looks like.
fn write_icons(directory: &std::path::Path) -> bool {
    match crate::icon::export(directory) {
        Ok(written) => {
            println!(
                "nazar-tray: wrote {} icon files to {}",
                written.len(),
                directory.display()
            );
        }
        Err(error) => {
            eprintln!("nazar-tray: could not write the icons: {error}");
            std::process::exit(1);
        }
    }
    true
}

/// The pages `--view` can open the panel on, other than the numbers it opens on anyway.
///
/// A list rather than an enum, because the value crosses into the webview as a string and a
/// Rust spelling in `UiState` would be a third name for the same page. A name this build does
/// not know is dropped: a typo leaves the panel where it opens rather than blanking it.
const VIEWS: [&str; 2] = ["settings", "usage"];

/// What `--usage-tab` may name: the usage view's four tabs, plus one state inside the first.
const USAGE_TABS: [&str; 5] = ["week", "weeks", "all", "models", "day"];

/// The tab the usage view opens on, and so the one `--view usage` lands on unasked.
pub const USAGE_TAB_DEFAULT: &str = USAGE_TABS[0];

/// Settings that change how the tray runs, taken from the command line.
///
/// All five exist for the screenshots in `docs/screenshots` and for looking at a state that
/// is hard to reach on purpose. None of them writes anything: `--demo` in particular never
/// claims the writer's lock, so a screenshot session cannot overwrite the real
/// `~/.nazar/limits.json`, and a run that was told what to look like never writes that back
/// into `config.json`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Options {
    /// `--demo`: synthetic numbers, the panel open at start-up, and no blur-hide.
    pub demo: bool,
    /// `--scale <factor>`: force the webview's device scale factor and size the panel to
    /// match, so 150 % and 200 % can be photographed on a 100 % display.
    pub scale: Option<f64>,
    /// `--theme <nazar|graphite>`: override the theme for this run only.
    pub theme: Option<String>,
    /// `--mode <light|dark>`: override the light/dark mode for this run only.
    pub mode: Option<String>,
    /// `--hint <on|off>`: show or hide the first-run overflow hint for this run only.
    ///
    /// The hint is shown once per machine and then never again, which makes it the one
    /// state a screenshot cannot reach twice. `None` means "whatever the settings say".
    pub hint: Option<bool>,
    /// `--offer <on|off>`: show or hide the one-time Max-plan offer for this run only.
    ///
    /// The same problem `--hint` solves, for the other banner that is shown once per machine
    /// and then never again. It is **off** in the documented screenshots: the offer is worth
    /// a picture of its own, and worth not being in every other one.
    pub offer: Option<bool>,
    /// `--locale <tag>`: one language for this run, panel and tooltip alike.
    ///
    /// The documentation is in English and the maintainer's machine is not, so without this
    /// every screenshot in the repository would be Turkish. It is also the quickest way to
    /// see whether a translation still fits the layout.
    pub locale: Option<String>,
    /// `--hidden`: open no window at start-up, whatever else was asked for.
    ///
    /// What the autostart entry passes. A normal launch is already hidden, so this is a
    /// promise rather than a behaviour — but it is a promise worth being able to make, and
    /// it is the flag that keeps `--demo`'s automatic panel out of a start-up run.
    pub hidden: bool,
    /// `--view settings|usage`: open the panel somewhere other than on the numbers.
    ///
    /// A screenshot flag like the others. Both pages are reached with a click, and a script
    /// cannot click; without this there would be no picture of either in the documentation,
    /// and no way to look at a whole form at three display scales.
    pub view: Option<String>,
    /// `--usage-tab <name>`: which tab `--view usage` lands on.
    ///
    /// `week`, `weeks`, `all` and `models` are the four tabs; `day` is the Week tab with its
    /// newest day already opened, which is the one state of that view a flag naming a tab
    /// cannot otherwise reach. `None` — including for a name this build does not know — leaves
    /// the view on `week`, the tab it opens on.
    pub usage_tab: Option<String>,
    /// `--demo-cross`: step the demo numbers across the thresholds, for the acceptance run.
    ///
    /// Implies `--demo`. See the module note for the sequence and for what it proves.
    pub demo_cross: bool,
    /// `--autostart on|off|status`: read or change the startup entry, then exit.
    pub autostart: Option<Autostart>,
}

/// What `--autostart` was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Autostart {
    /// Add the startup entry.
    On,
    /// Remove it.
    Off,
    /// Say what it is and change nothing.
    Status,
}

impl Autostart {
    /// Parse the word after the flag. Anything else is not an instruction.
    #[must_use]
    fn parse(value: &str) -> Option<Self> {
        match value {
            "on" | "enable" | "true" => Some(Autostart::On),
            "off" | "disable" | "false" => Some(Autostart::Off),
            "status" | "show" => Some(Autostart::Status),
            _ => None,
        }
    }
}

/// Do what `--autostart` asked, say what the machine now reports, and exit.
///
/// Called as the **first** thing in the application's setup, before the tray is built or any
/// state is managed, so nothing appears on screen. It goes through the plugin rather than
/// touching the registry directly, which is what makes it the same code path as the switch
/// in the settings; and it reports what [`tauri_plugin_autostart`] says *afterwards* rather
/// than what was asked for, because a write that failed must not be reported as a success.
pub fn run_autostart(app: &tauri::AppHandle, action: Autostart) -> ! {
    use tauri_plugin_autostart::ManagerExt;

    let manager = app.autolaunch();
    let outcome = match action {
        Autostart::On => manager.enable(),
        Autostart::Off => manager.disable(),
        Autostart::Status => Ok(()),
    };
    if let Err(error) = outcome {
        eprintln!("nazar-tray: could not change the startup entry: {error}");
        std::process::exit(1);
    }
    match manager.is_enabled() {
        Ok(enabled) => {
            println!(
                "nazar-tray: start with Windows is {}",
                if enabled { "on" } else { "off" }
            );
            std::process::exit(0);
        }
        Err(error) => {
            eprintln!("nazar-tray: could not read the startup entry: {error}");
            std::process::exit(1);
        }
    }
}

impl Options {
    /// Whether a change the panel makes may be written back to `config.json`.
    ///
    /// A screenshot must leave the maintainer's settings exactly as it found them.
    #[must_use]
    pub fn may_persist(&self) -> bool {
        !self.demo
            && self.theme.is_none()
            && self.mode.is_none()
            && self.hint.is_none()
            && self.offer.is_none()
            && self.locale.is_none()
    }
}

/// `on` or `off`, or nothing at all — which leaves the settings in charge.
fn on_or_off(value: &str) -> Option<bool> {
    match value {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

/// Read [`Options`] from this process's arguments.
#[must_use]
pub fn options() -> Options {
    parse_options(&std::env::args().skip(1).collect::<Vec<String>>())
}

/// The parser, kept apart from the environment so it can be tested.
#[must_use]
fn parse_options(arguments: &[String]) -> Options {
    let asked = |flag: &str| arguments.iter().any(|argument| argument == flag);
    let demo_cross = asked("--demo-cross");
    Options {
        // The crossing run is a demo run: it must not take the writer's lock either.
        demo: demo_cross || asked("--demo"),
        demo_cross,
        hidden: asked("--hidden"),
        // A scale that is not a number, or one outside what a display can be set to, is
        // ignored rather than clamped: it is a typo, and a 0.1× panel would look like a bug.
        scale: value_of(arguments, "--scale")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|scale| (0.5..=4.0).contains(scale)),
        theme: value_of(arguments, "--theme"),
        mode: value_of(arguments, "--mode"),
        hint: value_of(arguments, "--hint").and_then(|value| on_or_off(&value)),
        offer: value_of(arguments, "--offer").and_then(|value| on_or_off(&value)),
        locale: value_of(arguments, "--locale"),
        view: value_of(arguments, "--view").filter(|name| VIEWS.contains(&name.as_str())),
        usage_tab: value_of(arguments, "--usage-tab")
            .filter(|name| USAGE_TABS.contains(&name.as_str())),
        autostart: value_of(arguments, "--autostart")
            .as_deref()
            .and_then(Autostart::parse),
    }
}

/// One refresh, written to `~/.nazar/limits.json` if this process may write it.
///
/// The lock decides. With it, this is the writer for as long as the process lives, which is
/// a fraction of a second. Without it, the tray is running and already keeps that file
/// current, so the honest thing is to print what the tray wrote rather than to produce a
/// second opinion — and to say on standard error why, since a script that asked to write is
/// entitled to know that it did not.
fn write_once(force_detailed: bool) -> Limits {
    let now = now_rfc3339();
    let Ok(path) = paths::limits_path() else {
        eprintln!("nazar-tray: no home directory, so there is nowhere to write; printing only");
        return snapshot(force_detailed);
    };
    let lock = paths::lock_path()
        .ok()
        .and_then(|lock_path| LimitsLock::acquire(&lock_path, &now).ok());

    let Some(Acquisition::Held(lock)) = lock else {
        eprintln!(
            "nazar-tray: another instance is the writer; printing what it wrote and \
             changing nothing"
        );
        return nazar_core::read_limits(&path).unwrap_or_else(|_| snapshot(force_detailed));
    };

    let limits = snapshot(force_detailed);
    let mut writer = LimitsWriter::adopting(&path);
    match writer.write_if_changed(&limits, &now) {
        Ok(written) => {
            let mut stamped = limits;
            if let nazar_core::Written::Wrote(at) = written {
                stamped.updated_at = at;
            }
            drop(lock);
            stamped
        }
        Err(error) => {
            eprintln!("nazar-tray: could not write limits.json: {error}");
            drop(lock);
            limits
        }
    }
}

/// Build the `limits.json` document from the readers that exist.
///
/// Both providers are read from local files their own tools wrote: Codex from its newest
/// `rollout-*.jsonl`, Claude from the status-line captures `nazar-statusline` leaves in
/// `~/.nazar/statusline`. Both keys are always present — the contract says a provider's key
/// never disappears, so a consumer can tell "not set up on this machine" from "this build
/// does not know about that provider" without special cases.
///
/// `force_detailed` is the `--detailed` flag. Without it the opt-in mode does exactly what
/// `config.json` says, which on a machine nobody has configured is nothing at all.
#[must_use]
pub fn snapshot(force_detailed: bool) -> Limits {
    // Settings that cannot be read are settings that were never written: the defaults,
    // which have the mode off. A damaged `config.json` must not be a reason for `--print`
    // to fail, and it must certainly not be a reason to turn something on.
    let configured = Config::load().is_ok_and(|config| config.detailed_windows);
    build(&mut DetailedWindows::from_config(
        force_detailed || configured,
    ))
}

/// The snapshot with the opt-in mode handed in rather than discovered.
///
/// This is what the tests call, with a [`DetailedWindows`] that is off. A test suite that
/// went through [`snapshot`] would ask the real endpoint on the machine of anybody who had
/// turned the mode on for themselves, which is precisely the thing this repository's tests
/// are not allowed to do.
fn build(mode: &mut DetailedWindows) -> Limits {
    let now = now_rfc3339();
    let mut limits = Limits::new(now.clone());

    limits.providers.codex = match CodexReader::discover() {
        Ok(mut reader) => reader.refresh(),
        // No home directory at all: an unusual environment, and the honest answer is the
        // same as an uninstalled Codex.
        Err(_) => Provider::default(),
    };

    let passive = match ClaudeReader::discover() {
        Ok(mut reader) => reader.refresh(),
        Err(_) => Provider::default(),
    };
    let outcome = mode.refresh(&SystemClock);
    let reading = outcome
        .as_ref()
        .and_then(|outcome| outcome.reading.as_ref());

    limits.providers.claude = merge(passive, reading, &now);
    limits
}

#[cfg(test)]
mod tests {
    use super::*;
    use nazar_core::SCHEMA_VERSION;

    /// A snapshot with the opt-in mode off, whatever this machine's settings say.
    ///
    /// Every test here goes through this. Nothing in this repository's test suite is
    /// allowed to read a token or open a socket, and a test that read `config.json` would
    /// do both on the machine of anyone who had turned the mode on.
    fn offline_snapshot() -> Limits {
        build(&mut DetailedWindows::default())
    }

    #[test]
    fn the_snapshot_is_a_valid_contract_document() {
        let limits = offline_snapshot();
        assert_eq!(limits.schema_version, SCHEMA_VERSION);
        assert!(
            nazar_core::timefmt::sanitize_timestamp(&limits.updated_at).is_some(),
            "updatedAt must be a timestamp, got {}",
            limits.updated_at
        );

        // Round-trips through the contract's own parser, whatever this machine holds.
        let text = limits.to_json().unwrap();
        let parsed = Limits::from_json(&text).expect("the printed document must parse");
        assert_eq!(parsed, limits);
    }

    /// Both provider keys are always in the document, whatever this machine holds.
    ///
    /// The assertion cannot be "Claude is configured" or "Claude is not": the answer
    /// depends on whether the wrapper is installed on the machine running the tests, and a
    /// test that demands one of those is a test that fails on somebody else's computer.
    /// What the contract actually promises is that the key is there either way, and that
    /// an unconfigured provider carries no windows.
    #[test]
    fn both_providers_are_present_however_the_machine_is_set_up() {
        let limits = offline_snapshot();
        for provider in [&limits.providers.claude, &limits.providers.codex] {
            if !provider.configured {
                assert!(provider.windows.is_empty());
                assert_eq!(provider.binding, None);
            }
        }
    }

    #[test]
    fn the_printed_document_never_carries_a_percentage_it_did_not_read() {
        let limits = offline_snapshot();
        for (key, provider) in [
            ("claude", &limits.providers.claude),
            ("codex", &limits.providers.codex),
        ] {
            for (name, window) in &provider.windows {
                if window.state == nazar_core::WindowState::Error {
                    assert_eq!(
                        window.percent, None,
                        "{key}.{name} has a percentage it could not read"
                    );
                }
            }
        }
    }

    /// `--write` is opt-in, and `--print` on its own still writes nothing.
    ///
    /// Checked on the flag rather than by running the mode: a test that called `write_once`
    /// would write `~/.nazar/limits.json` on the machine running the tests, and no test in
    /// this repository is allowed to touch the real one.
    #[test]
    fn the_write_flag_is_a_separate_word() {
        assert_eq!(WRITE_FLAG, "--write");
        assert_ne!(WRITE_FLAG, PRINT_FLAG);
        assert_ne!(WRITE_FLAG, DETAILED_FLAG);
    }

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn the_screenshot_flags_parse_and_default_to_off() {
        assert_eq!(parse_options(&[]), Options::default());
        assert!(!parse_options(&words("--print")).demo);

        let options = parse_options(&words(
            "--demo --scale 1.5 --theme graphite --mode light --hint on",
        ));
        assert!(options.demo);
        assert_eq!(options.scale, Some(1.5));
        assert_eq!(options.theme.as_deref(), Some("graphite"));
        assert_eq!(options.mode.as_deref(), Some("light"));
        assert_eq!(options.hint, Some(true));
        assert_eq!(parse_options(&words("--hint off")).hint, Some(false));
        assert_eq!(parse_options(&words("--offer on")).offer, Some(true));
        assert_eq!(parse_options(&words("--offer off")).offer, Some(false));
        assert_eq!(parse_options(&words("--offer maybe")).offer, None);
        assert_eq!(
            parse_options(&words("--hint maybe")).hint,
            None,
            "a value nobody defined leaves the settings in charge"
        );
    }

    #[test]
    fn a_run_that_was_told_what_to_look_like_does_not_remember_it() {
        assert!(
            parse_options(&[]).may_persist(),
            "an ordinary launch remembers the theme the user picks"
        );
        for line in [
            "--demo",
            "--theme graphite",
            "--mode dark",
            "--hint on",
            "--offer on",
            "--locale tr",
        ] {
            assert!(
                !parse_options(&words(line)).may_persist(),
                "{line} must leave config.json exactly as it found it"
            );
        }
    }

    #[test]
    fn the_startup_flags_parse_and_default_to_off() {
        let plain = parse_options(&[]);
        assert!(!plain.hidden);
        assert!(!plain.demo_cross);

        let hidden = parse_options(&words("--hidden"));
        assert!(hidden.hidden);
        assert!(
            !hidden.demo,
            "a start-up launch is a real launch: it reads real files and writes limits.json"
        );
        assert!(
            hidden.may_persist(),
            "and it may still remember a theme the user picks"
        );

        assert_eq!(
            parse_options(&words("--autostart on")).autostart,
            Some(Autostart::On)
        );
        assert_eq!(
            parse_options(&words("--autostart off")).autostart,
            Some(Autostart::Off)
        );
        assert_eq!(
            parse_options(&words("--autostart status")).autostart,
            Some(Autostart::Status)
        );
        for line in ["--autostart", "--autostart maybe", "--autostart 1"] {
            assert_eq!(
                parse_options(&words(line)).autostart,
                None,
                "{line} is not an instruction, and a flag that guessed would be worse                  than one that did nothing"
            );
        }
        assert!(
            parse_options(&words("--autostart on")).may_persist(),
            "the startup entry is not part of config.json, so it changes nothing there"
        );

        assert_eq!(
            parse_options(&words("--view settings")).view.as_deref(),
            Some("settings")
        );
        assert_eq!(
            parse_options(&words("--view nonsense")).view,
            None,
            "a view nobody defined leaves the panel where it opens"
        );
        assert!(
            parse_options(&words("--view settings")).may_persist(),
            "opening a page is navigation, not a claim about what the settings are: a user              who is shown the form may still save from it"
        );

        assert_eq!(
            parse_options(&words("--view usage")).view.as_deref(),
            Some("usage"),
            "the usage view is reachable by a flag, or there is no picture of it"
        );
        for tab in ["week", "weeks", "all", "models", "day"] {
            assert_eq!(
                parse_options(&words(&format!("--view usage --usage-tab {tab}")))
                    .usage_tab
                    .as_deref(),
                Some(tab)
            );
        }
        assert_eq!(
            parse_options(&words("--view usage --usage-tab nonsense")).usage_tab,
            None,
            "a tab nobody defined leaves the view on the one it opens with"
        );
        assert_eq!(
            parse_options(&words("--usage-tab models")).view,
            None,
            "the tab is only meaningful with the view, and naming it alone opens nothing"
        );

        let crossing = parse_options(&words("--demo-cross"));
        assert!(crossing.demo_cross);
        assert!(
            crossing.demo,
            "the crossing run must not be able to take the writer's lock, so it is a demo run"
        );
        assert!(!crossing.may_persist());
    }

    #[test]
    fn a_scale_that_is_not_a_scale_is_ignored_rather_than_clamped() {
        // A typo must not produce a panel nobody can read; running at the display's own
        // scale is the honest fallback.
        for line in [
            "--scale",
            "--scale x",
            "--scale 0.1",
            "--scale 9",
            "--scale -2",
        ] {
            assert_eq!(parse_options(&words(line)).scale, None, "{line}");
        }
        assert_eq!(parse_options(&words("--scale 2")).scale, Some(2.0));
    }

    #[test]
    fn a_flag_with_no_value_takes_nothing_from_the_flag_after_it() {
        let arguments = words("--icons");
        assert_eq!(value_of(&arguments, ICONS_FLAG), None);
        assert_eq!(
            value_of(&words("--demo --icons out"), ICONS_FLAG),
            Some("out".to_owned())
        );
    }
}
