//! The settings page's half of the status-line wrapper.
//!
//! WP2 built `nazar-statusline` and its installer, and WP7 ships the two binaries side by
//! side in one bundle. What was still missing is the only part a person without a terminal
//! can reach: **the installer is offered from the settings page, and nowhere else does this
//! application touch `~/.claude/settings.json`.**
//!
//! Three rules shape what is below.
//!
//! 1. **Installing is never a side effect of installing nazar-tray.** The NSIS package
//!    copies `nazar-statusline.exe` next to the tray and stops there. Claude Code's settings
//!    file belongs to Claude Code and to its user; an installer that edited it would be
//!    editing a file the user never mentioned, on a machine where a broken `statusLine`
//!    means a broken prompt. So the wrapper arrives inert and the user asks for it.
//!
//! 2. **The diff is shown before anything is written.** Every button here runs
//!    `--dry-run` first, prints what the wrapper says it would change, and only writes
//!    after a second, explicit click. The backup the installer takes is the second net; the
//!    preview is the first, and it is the one that happens before the file is touched.
//!
//! 3. **This module does not reimplement the installer.** It runs the binary beside it.
//!    There is one implementation of the edit to `settings.json`, it lives in
//!    `crates/nazar-statusline`, it is the same code the command line runs, and it is the
//!    code the tests cover. Anything else would be a second implementation of the one
//!    operation in this product that can damage another program's configuration.
//!
//! The wrapper's own output is passed through verbatim and shown in a monospaced block. It
//! is English, like the rest of this application's command-line surface: it is a unified
//! diff of a JSON file and a path to a backup, and translating a diff would make it a
//! picture of a diff. The words *around* it come from the locale files like every other
//! string on the page.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

/// The name of the wrapper binary, without a platform suffix.
const PROGRAM: &str = "nazar-statusline";

/// Points this module at another binary. Set by the test suite; unset on a real machine.
const OVERRIDE: &str = "NAZAR_STATUSLINE_BIN";

/// What one run of the wrapper produced.
///
/// `outcome` is a key, not a sentence: the panel looks it up in the locale catalogue, so the
/// words a user reads are translated like every other word on the page while the diff below
/// them stays the wrapper's own bytes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// `ok`, `missing`, `failed` or `readOnly`.
    pub outcome: &'static str,
    /// `true`, `false`, or absent when the settings file could not be read at all.
    pub installed: Option<bool>,
    /// Everything the wrapper printed, standard output and standard error, in order.
    pub output: String,
    /// Where the wrapper is, with the home directory collapsed — empty when it is missing.
    pub program: String,
}

impl Outcome {
    /// The answer when `nazar-statusline` is not beside us.
    fn missing() -> Self {
        Self {
            outcome: "missing",
            installed: None,
            output: String::new(),
            program: String::new(),
        }
    }
}

/// Where `nazar-statusline` should be: beside this executable.
///
/// The bundle puts them in the same directory, and so does `cargo build`, which is why a
/// development build can exercise this page without an installer. The environment variable
/// is for the test suite, which points it at a stub rather than at the real thing.
fn program_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(OVERRIDE) {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    let directory = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let path = directory.join(format!("{PROGRAM}{}", std::env::consts::EXE_SUFFIX));
    path.is_file().then_some(path)
}

/// Run the wrapper with these arguments and collect everything it said.
///
/// Standard error is appended to standard output rather than kept apart, because the
/// interesting failures print a sentence on one and nothing on the other, and a panel that
/// showed only one of the two would show an empty box for exactly those cases.
fn run<I, S>(arguments: I) -> Outcome
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_at(program_path(), arguments)
}

/// [`run`], with the search for the binary already done, so a test can say there is none.
fn run_at<I, S>(program: Option<PathBuf>, arguments: I) -> Outcome
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let Some(path) = program else {
        return Outcome::missing();
    };

    let mut command = Command::new(&path);
    command.args(arguments);

    // Without this the user sees a console window blink every time they open the settings
    // page: this runs on a desktop application's UI thread, not from a terminal.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let program = crate::state::collapse_home(&path);
    match command.output() {
        Ok(finished) => {
            let mut output = String::from_utf8_lossy(&finished.stdout).into_owned();
            let errors = String::from_utf8_lossy(&finished.stderr);
            if !errors.trim().is_empty() {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&errors);
            }
            Outcome {
                outcome: if finished.status.success() {
                    "ok"
                } else {
                    "failed"
                },
                installed: installed_from(&output),
                output,
                program,
            }
        }
        // The file was there a moment ago and would not start. That is a real failure and
        // it is reported as one, with nothing of ours added to it.
        Err(error) => Outcome {
            outcome: "failed",
            installed: None,
            output: error.to_string(),
            program,
        },
    }
}

/// Read `installed: yes` / `installed: no` out of what `status` printed.
///
/// The wrapper's `status` subcommand prints a line per fact, and this is the one fact the
/// page needs as a value rather than as text: it decides whether the button offers to
/// install or to remove. `installed: unknown — …`, which is what a settings file that will
/// not parse produces, maps to `None`, and the page then shows the wrapper's own words.
fn installed_from(output: &str) -> Option<bool> {
    for line in output.lines() {
        let Some(value) = line.strip_prefix("installed:") else {
            continue;
        };
        return match value.trim() {
            "yes" => Some(true),
            "no" => Some(false),
            _ => None,
        };
    }
    None
}

/// Whether the wrapper is Claude Code's status line at this moment.
///
/// Asked of the machine every time the settings page opens, never remembered: a user who
/// ran `nazar-statusline uninstall` in a terminal — or edited `settings.json` by hand —
/// should see a page that agrees with their machine rather than with our memory. This is
/// the same rule [`crate::state::get_autostart`] follows for the startup entry.
#[tauri::command]
pub fn statusline_status() -> Outcome {
    run(["status"])
}

/// Show what installing would change, and change nothing.
#[tauri::command]
pub fn statusline_preview(remove: bool) -> Outcome {
    run([if remove { "uninstall" } else { "install" }, "--dry-run"])
}

/// Install or remove the wrapper for real, after the preview was shown and accepted.
///
/// The panel cannot reach this without having called [`statusline_preview`] first and having
/// had a second click on the button the preview put on screen. That is a rule of the page
/// rather than of this function — a second click is a user interface's way of asking twice,
/// and the file's real protection is the backup the installer takes before it writes.
///
/// The one thing enforced *here* rather than on the page is the read-only run. A `--demo`
/// session says on screen that it will not write the user's settings; this is the only
/// control on that page that would write another program's file, so it is refused rather than
/// hidden, and a screenshot run cannot touch `~/.claude/settings.json` even by accident.
#[tauri::command]
pub fn statusline_apply(state: tauri::State<'_, crate::state::AppState>, remove: bool) -> Outcome {
    if !state.writes_allowed() {
        return Outcome {
            outcome: "readOnly",
            ..statusline_status()
        };
    }
    let outcome = run([if remove { "uninstall" } else { "install" }]);
    if outcome.outcome != "ok" {
        return outcome;
    }
    // Report the state the machine is in afterwards rather than the state we intended, and
    // keep what the write printed — the diff and the backup path are the receipt.
    let after = run(["status"]);
    Outcome {
        installed: after.installed,
        ..outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installed_line_is_read_and_nothing_else_is() {
        let report = "settings:  C:\\x\\settings.json\ninstalled: yes\ncommand:   x\n";
        assert_eq!(installed_from(report), Some(true));
        assert_eq!(installed_from("installed: no\n"), Some(false));
        assert_eq!(installed_from("installed: unknown — broken JSON\n"), None);
        assert_eq!(installed_from("chains to: installed: yes\n"), None);
        assert_eq!(installed_from(""), None);
    }

    #[test]
    fn a_missing_binary_is_a_state_rather_than_an_error() {
        // A bundle whose second binary was deleted, or a `cargo run` from a target
        // directory that only holds the tray. The page says so and offers nothing; it does
        // not fail, and it certainly does not try to edit settings.json without the
        // program that knows how.
        let outcome = run_at(None, ["status"]);

        assert_eq!(outcome.outcome, "missing");
        assert_eq!(outcome.installed, None);
        assert!(outcome.output.is_empty());
        assert!(outcome.program.is_empty());
    }

    #[test]
    fn the_binary_is_looked_for_beside_this_one() {
        // Whatever the answer is, it is a sibling of the running executable and carries the
        // platform's own suffix — the property the bundle has to preserve.
        if let Some(found) = program_path() {
            let directory = std::env::current_exe().unwrap();
            assert_eq!(found.parent(), directory.parent());
            assert_eq!(
                found.file_name().and_then(|name| name.to_str()),
                Some(format!("{PROGRAM}{}", std::env::consts::EXE_SUFFIX).as_str())
            );
        }
    }
}
