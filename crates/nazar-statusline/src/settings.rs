//! Reading and rewriting Claude Code's `settings.json` without damaging it.
//!
//! This is the one file in the whole product that nazar-tray **writes** on somebody
//! else's behalf, and the risk register calls breaking it the largest single risk in the
//! project (R10). Everything here exists to shrink that risk:
//!
//! * The document is parsed as a plain [`Value`], so **every key the file already had
//!   survives**, including ones this build has never heard of.
//! * `serde_json`'s `preserve_order` feature is on for the whole workspace, so keys keep
//!   the order the user's editor put them in.
//! * The only key ever touched is `statusLine`. A test asserts that: it round-trips all
//!   three shipped fixtures and compares them byte for byte.
//! * A file that is not valid JSON is **reported and left alone**. It is never "repaired",
//!   and it is never replaced by `{}` — which is exactly what one of the tools surveyed in
//!   the research phase does.
//!
//! Nothing in this module writes anything. It parses, renders and compares; the caller
//! decides, and [`crate::install`] is the only caller that writes.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::fail::{Failure, Result};

/// Environment variable that moves Claude Code's configuration directory.
pub const CLAUDE_CONFIG_DIR_VAR: &str = "CLAUDE_CONFIG_DIR";

/// Name of the settings file inside that directory.
pub const SETTINGS_FILE: &str = "settings.json";

/// The one key this program is allowed to add, change or remove.
pub const STATUS_LINE_KEY: &str = "statusLine";

/// Files whose presence means another process is editing the settings right now.
///
/// None of them is a documented convention; they are the shapes an editor, an installer
/// or a half-finished write leaves behind. Refusing while one exists costs a retry and
/// avoids two writers meeting in the middle of the file the user cares most about.
pub const LOCK_FILES: [&str; 3] = ["settings.json.lock", ".settings.json.lock", "settings.lock"];

/// Where Claude Code keeps its settings.
///
/// `explicit` wins (the `--config-dir` flag), then `CLAUDE_CONFIG_DIR`, then `~/.claude`.
/// An empty environment variable is not a directory and is ignored.
///
/// The environment half is [`nazar_core::paths::claude_config_dir`], so the two crates
/// cannot drift on where Claude Code keeps its files; only the `--config-dir` override,
/// which is this program's own, lives here.
pub fn config_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = explicit {
        return Ok(dir.to_path_buf());
    }
    Ok(nazar_core::paths::claude_config_dir()?)
}

/// `<config dir>/settings.json`.
#[must_use]
pub fn settings_path(dir: &Path) -> PathBuf {
    dir.join(SETTINGS_FILE)
}

/// The lock-like file that is in the way, if there is one.
#[must_use]
pub fn lock_in_the_way(dir: &Path) -> Option<PathBuf> {
    LOCK_FILES
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.exists())
}

/// One settings file, parsed and ready to be rendered again.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Where it came from.
    pub path: PathBuf,
    /// The bytes as read. `None` when the file does not exist yet.
    pub original: Option<String>,
    /// The parsed document. An empty object when the file does not exist yet.
    pub document: Map<String, Value>,
    /// Whether the file on disk ended with a newline, so a rewrite can keep it that way.
    pub trailing_newline: bool,
}

impl Settings {
    /// Read and parse a settings file.
    ///
    /// A file that is not there is not an error: installing on a machine that has never
    /// written one is a normal first run, and the caller is told through
    /// [`Settings::exists`].
    pub fn load(path: &Path) -> Result<Self> {
        let original = match std::fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(Failure::new(format!(
                    "could not read {}: {error}",
                    path.display()
                )));
            }
        };

        let (document, trailing_newline) = match &original {
            None => (Map::new(), true),
            Some(text) => {
                let value: Value = serde_json::from_str(text).map_err(|error| {
                    // The message names the file and the position, never the contents:
                    // this file sits next to things that are none of our business.
                    Failure::new(format!(
                        "{} is not valid JSON ({error}). Nothing was changed — fix the \
                         file, or move it aside, and run this again.",
                        path.display()
                    ))
                })?;
                let Value::Object(map) = value else {
                    return Err(Failure::new(format!(
                        "{} does not hold a JSON object at the top level. Nothing was \
                         changed.",
                        path.display()
                    )));
                };
                (map, text.ends_with('\n'))
            }
        };

        Ok(Settings {
            path: path.to_path_buf(),
            original,
            document,
            trailing_newline,
        })
    }

    /// Whether the file was on disk when it was loaded.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.original.is_some()
    }

    /// The `statusLine` object as the file holds it, if it holds one.
    #[must_use]
    pub fn status_line(&self) -> Option<&Value> {
        self.document.get(STATUS_LINE_KEY)
    }

    /// The `statusLine.command` string, if there is one.
    #[must_use]
    pub fn status_line_command(&self) -> Option<&str> {
        self.status_line()
            .and_then(Value::as_object)
            .and_then(|line| line.get("command"))
            .and_then(Value::as_str)
    }

    /// A copy with `statusLine` set to `line`, or removed when `line` is `None`.
    ///
    /// Position matters: replacing an existing key keeps its place in the file, and a new
    /// key is appended at the end, which is where a person adding one by hand would put
    /// it. Removal uses a shifting remove — the swapping one would silently reorder the
    /// document, and this program's whole promise is that it does not.
    #[must_use]
    pub fn with_status_line(&self, line: Option<Value>) -> Settings {
        let mut copy = self.clone();
        match line {
            Some(value) => {
                copy.document.insert(STATUS_LINE_KEY.to_owned(), value);
            }
            None => {
                copy.document.shift_remove(STATUS_LINE_KEY);
            }
        }
        copy
    }

    /// The document as it would be written: two-space indentation, keys in order.
    ///
    /// A trailing newline is kept when the file had one and added when the file is new,
    /// because every editor writes one and a missing one shows up as a diff in the user's
    /// next commit.
    #[must_use]
    pub fn render(&self) -> String {
        let value = Value::Object(self.document.clone());
        let mut text = serde_json::to_string_pretty(&value)
            .expect("a document parsed from JSON always serialises back");
        if self.trailing_newline {
            text.push('\n');
        }
        text
    }

    /// The text a diff should compare against: the bytes on disk, or nothing.
    #[must_use]
    pub fn before_text(&self) -> String {
        self.original.clone().unwrap_or_default()
    }
}

/// Build the `statusLine` object that points at this program.
///
/// `padding`, `refreshInterval` and any other key the user's own `statusLine` carried are
/// preserved: they configure the status line, not the command, and dropping a key the
/// user set would be a silent change. Only `type` and `command` are ours.
#[must_use]
pub fn status_line_for(command: &str, previous: Option<&Value>) -> Value {
    let mut line = Map::new();
    line.insert("type".to_owned(), Value::from("command"));
    line.insert("command".to_owned(), Value::from(command));
    if let Some(Value::Object(old)) = previous {
        for (key, value) in old {
            if key == "type" || key == "command" {
                continue;
            }
            line.insert(key.clone(), value.clone());
        }
    }
    Value::Object(line)
}

/// Quote a program path for a shell command line if it needs quoting.
///
/// Claude Code hands `statusLine.command` to a shell, so a path with a space in it —
/// `C:\Program Files\…` — has to arrive quoted or the shell splits it. Windows path
/// separators need no escaping inside double quotes; JSON encoding of the backslashes is
/// `serde_json`'s job and happens after this.
///
/// Quoting alone is not enough on Windows, and [`command_for`] is what this is composed
/// into. Nothing outside a test calls it on its own.
#[must_use]
pub fn quote_program(path: &str) -> String {
    if path.contains(' ') && !path.starts_with('"') {
        format!("\"{path}\"")
    } else {
        path.to_owned()
    }
}

/// A program path written as a command line the shell Claude Code uses will really run.
///
/// **This is the fix for a status line that never ran at all.** On Windows, Claude Code
/// runs `statusLine.command` through **Git Bash** where it can find one, and through
/// PowerShell where it cannot; in `sh` a backslash outside quotes is the escape character,
/// so the perfectly ordinary-looking `C:\nazar\nazar-statusline.exe` reaches the shell as
/// `C:nazarnazar-statusline.exe`. No program is spawned, no capture is written, and the
/// only symptom is a status line that shows nothing — which is exactly how it presented on
/// the maintainer's machine, with `status` cheerfully reporting `installed: yes`.
///
/// So the separators go in as forward slashes. Windows has accepted them in a path for as
/// long as it has had paths, `sh` and PowerShell both pass them through untouched, and a
/// path with a space in it is quoted on top of that, which the two of them also agree on.
///
/// Only a Windows-shaped path is rewritten. A POSIX file name is allowed to contain a
/// backslash, and turning one into a separator there would name a different file.
#[must_use]
pub fn command_for(path: &str) -> String {
    if looks_like_a_windows_path(path) {
        quote_program(&path.replace('\\', "/"))
    } else {
        quote_program(path)
    }
}

/// Whether a path is spelled the way Windows spells one: a drive letter, or a UNC root.
///
/// Deliberately not `cfg!(windows)`. The question is about the string, the answer has to be
/// the same wherever the test runs, and a settings file written on Windows is still a
/// settings file written on Windows when something else reads it.
fn looks_like_a_windows_path(path: &str) -> bool {
    if path.starts_with(r"\\") {
        return true;
    }
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// Whether `sh` would eat a backslash in this command line.
///
/// The rule it follows: outside quotes a backslash escapes the next character and both
/// disappear; inside double quotes it escapes only `"`, `\`, `$` and a backtick and is
/// literal otherwise; inside single quotes it is always literal. So the only backslash
/// worth a word is an **unquoted** one — which is every backslash in a Windows path that
/// was written into `settings.json` without quotes around it.
///
/// This says nothing about whether a command works: a command line may carry a deliberate
/// escape, and on a machine whose shell is not `sh` it carries none of this meaning at all.
/// What the caller does with the answer is warn on Windows, where a backslash is a
/// separator far more often than it is an escape.
#[must_use]
pub fn has_unquoted_backslash(command: &str) -> bool {
    let mut characters = command.chars().peekable();
    let mut single = false;
    let mut double = false;
    while let Some(character) = characters.next() {
        match character {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '\\' if single => {}
            // Consuming the escaped character is what keeps the quote state right for a
            // command line that really does contain `"a\"b"`.
            '\\' if double => {
                if matches!(characters.peek(), Some('"' | '\\' | '$' | '`')) {
                    characters.next();
                }
            }
            '\\' => return true,
            _ => {}
        }
    }
    false
}

/// Whether two command strings name the same program, however each is spelled.
///
/// One executable can sit in `settings.json` as `C:\bin\nazar-statusline.exe`, as
/// `"C:\bin\nazar-statusline.exe"`, or — after this build installs it — as
/// `C:/bin/nazar-statusline.exe`. All three are one installation. An installer that could
/// not see that would either rewrite a file needing no rewrite, or refuse to fix one that
/// did; the same normalisation is what lets `status` compare the settings file with
/// `chain.json`'s `installedCommand` without reporting a difference that is only spelling.
///
/// Case is folded, which is Windows's own rule for a path and the rule [`is_our_command`]
/// already uses.
#[must_use]
pub fn same_command(left: &str, right: &str) -> bool {
    normalised_command(left) == normalised_command(right)
}

/// One command string in the form [`same_command`] compares.
fn normalised_command(command: &str) -> String {
    let trimmed = command.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(trimmed);
    unquoted.replace('\\', "/").to_lowercase()
}

/// Whether a command string is this program.
///
/// Used for the idempotency check and for `uninstall`'s "is it even installed" question.
/// A substring test on the file name rather than a path comparison, because the path in
/// the file can be the installed copy, a build directory, or a copy the user moved — and
/// because it has to recognise every spelling this program has ever written, backslashes
/// and forward slashes, quoted and bare, so that an installation made by an older build is
/// still an installation to `status` and to `uninstall`.
#[must_use]
pub fn is_our_command(command: &str) -> bool {
    command.to_lowercase().contains("nazar-statusline")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    const NO_LINE: &str = include_str!("../../../fixtures/claude/settings-no-statusline.json");
    const CCSTATUSLINE: &str = include_str!("../../../fixtures/claude/settings-ccstatusline.json");
    const CUSTOM: &str = include_str!("../../../fixtures/claude/settings-custom-node.json");

    fn load(text: &str) -> (TempDir, Settings) {
        let dir = TempDir::new("settings");
        let path = dir.join(SETTINGS_FILE);
        std::fs::write(&path, text).unwrap();
        let settings = Settings::load(&path).unwrap();
        (dir, settings)
    }

    #[test]
    fn all_three_fixtures_round_trip_byte_for_byte() {
        for (name, text) in [
            ("no statusLine", NO_LINE),
            ("ccstatusline", CCSTATUSLINE),
            ("a custom script", CUSTOM),
        ] {
            let (_dir, settings) = load(text);
            assert_eq!(
                settings.render(),
                text,
                "{name} did not survive a parse and a render"
            );
        }
    }

    #[test]
    fn a_missing_file_is_an_empty_document_not_an_error() {
        let dir = TempDir::new("settings-missing");
        let settings = Settings::load(&dir.join(SETTINGS_FILE)).unwrap();
        assert!(!settings.exists());
        assert!(settings.document.is_empty());
        assert_eq!(settings.status_line(), None);
        assert_eq!(settings.render(), "{}\n");
    }

    #[test]
    fn invalid_json_is_reported_and_the_contents_are_not_quoted_back() {
        let dir = TempDir::new("settings-broken");
        let path = dir.join(SETTINGS_FILE);
        std::fs::write(&path, "{ \"model\": \"x\", oops \"secret-looking-value\" }").unwrap();

        let failure = Settings::load(&path).unwrap_err().to_string();
        assert!(failure.contains(SETTINGS_FILE), "{failure}");
        assert!(failure.contains("not valid JSON"), "{failure}");
        assert!(!failure.contains("secret-looking-value"), "{failure}");
    }

    #[test]
    fn a_top_level_array_is_refused_rather_than_wrapped() {
        let dir = TempDir::new("settings-array");
        let path = dir.join(SETTINGS_FILE);
        std::fs::write(&path, "[1, 2, 3]").unwrap();
        assert!(Settings::load(&path).is_err());
    }

    #[test]
    fn replacing_the_status_line_keeps_the_key_in_place() {
        let (_dir, settings) = load(CUSTOM);
        let patched = settings.with_status_line(Some(status_line_for(
            "C:\\bin\\nazar-statusline.exe",
            settings.status_line(),
        )));

        let keys_before: Vec<&String> = settings.document.keys().collect();
        let keys_after: Vec<&String> = patched.document.keys().collect();
        assert_eq!(keys_before, keys_after, "the key order moved");

        let line = patched.status_line().unwrap().as_object().unwrap();
        assert_eq!(
            line["command"],
            Value::from("C:\\bin\\nazar-statusline.exe")
        );
        assert_eq!(line["type"], Value::from("command"));
        assert_eq!(line["padding"], Value::from(0));
        assert_eq!(line["refreshInterval"], Value::from(30));
    }

    #[test]
    fn adding_a_status_line_appends_it_and_removing_it_leaves_the_rest_alone() {
        let (_dir, settings) = load(NO_LINE);
        let added = settings.with_status_line(Some(status_line_for("nazar-statusline", None)));
        assert_eq!(
            added.document.keys().next_back().map(String::as_str),
            Some(STATUS_LINE_KEY),
            "a new key belongs at the end"
        );

        let removed = added.with_status_line(None);
        assert_eq!(removed.render(), settings.render());
    }

    #[test]
    fn an_unknown_key_inside_the_status_line_is_preserved() {
        let previous = serde_json::json!({
            "type": "command",
            "command": "npx ccstatusline@latest",
            "padding": 0,
            "somethingNewerClaudeAdded": {"nested": true}
        });
        let line = status_line_for("nazar-statusline", Some(&previous));
        let object = line.as_object().unwrap();
        assert_eq!(object["command"], Value::from("nazar-statusline"));
        assert_eq!(object["padding"], Value::from(0));
        assert_eq!(
            object["somethingNewerClaudeAdded"]["nested"],
            Value::from(true)
        );
    }

    #[test]
    fn a_status_line_that_is_not_an_object_does_not_stop_us() {
        let previous = Value::from("some string a future version writes");
        let line = status_line_for("nazar-statusline", Some(&previous));
        assert_eq!(line.as_object().unwrap().len(), 2);
    }

    #[test]
    fn a_program_path_with_a_space_is_quoted_and_one_without_is_not() {
        assert_eq!(
            quote_program("C:\\Program Files\\nazar\\nazar-statusline.exe"),
            "\"C:\\Program Files\\nazar\\nazar-statusline.exe\""
        );
        assert_eq!(
            quote_program("C:\\bin\\nazar-statusline.exe"),
            "C:\\bin\\nazar-statusline.exe"
        );
        assert_eq!(
            quote_program("/usr/local/bin/nazar-statusline"),
            "/usr/local/bin/nazar-statusline"
        );
        assert_eq!(
            quote_program("\"already quoted path\""),
            "\"already quoted path\""
        );
    }

    #[test]
    fn a_windows_path_is_installed_with_forward_slashes_so_git_bash_can_run_it() {
        assert_eq!(
            command_for(r"C:\bin\nazar\nazar-statusline.exe"),
            "C:/bin/nazar/nazar-statusline.exe"
        );
        assert_eq!(
            command_for(r"C:\Program Files\nazar\nazar-statusline.exe"),
            "\"C:/Program Files/nazar/nazar-statusline.exe\"",
            "a space still needs quotes; the slashes are not a substitute for them"
        );
        assert_eq!(
            command_for(r"\\server\share\nazar-statusline.exe"),
            "//server/share/nazar-statusline.exe"
        );
        assert_eq!(
            command_for("D:/already/forward/nazar-statusline.exe"),
            "D:/already/forward/nazar-statusline.exe"
        );
    }

    #[test]
    fn a_posix_path_is_left_exactly_as_it_is() {
        assert_eq!(
            command_for("/usr/local/bin/nazar-statusline"),
            "/usr/local/bin/nazar-statusline"
        );
        // A backslash in a POSIX file name is part of the name, not a separator.
        assert_eq!(
            command_for("/opt/odd\\name/nazar-statusline"),
            "/opt/odd\\name/nazar-statusline"
        );
        assert!(!looks_like_a_windows_path("/usr/local/bin"));
        assert!(looks_like_a_windows_path(r"C:\bin"));
        assert!(looks_like_a_windows_path("c:/bin"));
        assert!(!looks_like_a_windows_path("C:"));
    }

    #[test]
    fn an_unquoted_backslash_is_the_one_the_shell_eats() {
        assert!(has_unquoted_backslash(r"C:\bin\nazar-statusline.exe"));
        assert!(has_unquoted_backslash(r"node C:\bin\statusline.js"));
        assert!(!has_unquoted_backslash("\"C:\\bin\\nazar-statusline.exe\""));
        assert!(!has_unquoted_backslash("node \"C:\\bin\\statusline.js\""));
        assert!(!has_unquoted_backslash("'C:\\bin\\nazar-statusline.exe'"));
        assert!(!has_unquoted_backslash("C:/bin/nazar-statusline.exe"));
        assert!(!has_unquoted_backslash("npx ccstatusline@latest"));
        // An escaped quote inside a quoted run must not flip the quote state and turn the
        // backslashes after it into unquoted ones.
        assert!(!has_unquoted_backslash("\"a\\\"b\\c\""));
    }

    #[test]
    fn one_program_spelled_three_ways_is_one_installation() {
        let installed = "C:/bin/nazar/nazar-statusline.exe";
        assert!(same_command(
            installed,
            r"C:\bin\nazar\nazar-statusline.exe"
        ));
        assert!(same_command(
            installed,
            "\"C:\\bin\\nazar\\nazar-statusline.exe\""
        ));
        assert!(same_command(
            installed,
            "  C:/BIN/nazar/nazar-statusline.exe  "
        ));
        assert!(!same_command(
            installed,
            "D:/bin/nazar/nazar-statusline.exe"
        ));
        assert!(!same_command(installed, "npx ccstatusline@latest"));
    }

    #[test]
    fn what_is_written_into_the_document_is_the_runnable_spelling() {
        let (_dir, settings) = load(CUSTOM);
        let line = settings
            .with_status_line(Some(status_line_for(
                &command_for(r"C:\bin\nazar\nazar-statusline.exe"),
                settings.status_line(),
            )))
            .status_line()
            .unwrap()
            .clone();
        let command = line["command"].as_str().unwrap().to_owned();
        assert_eq!(command, "C:/bin/nazar/nazar-statusline.exe");
        assert!(!has_unquoted_backslash(&command));
        assert_eq!(line["padding"], Value::from(0));
    }

    #[test]
    fn our_own_command_is_recognised_however_it_is_spelled() {
        assert!(is_our_command("C:\\bin\\nazar-statusline.exe"));
        assert!(is_our_command(
            "\"C:\\Program Files\\nazar\\NAZAR-STATUSLINE.EXE\""
        ));
        assert!(is_our_command("/usr/local/bin/nazar-statusline"));
        // The spelling this build writes, and the one older builds wrote, are both ours:
        // `uninstall` and `status` have to recognise an installation either way.
        assert!(is_our_command("C:/bin/nazar/nazar-statusline.exe"));
        assert!(is_our_command(
            "\"C:/Program Files/nazar/nazar-statusline.exe\""
        ));
        assert!(!is_our_command("npx ccstatusline@latest"));
        assert!(!is_our_command(
            "node \"C:\\projects\\example\\statusline.js\""
        ));
    }

    #[test]
    fn the_config_directory_prefers_the_flag() {
        let explicit = PathBuf::from("D:\\somewhere\\claude");
        assert_eq!(config_dir(Some(&explicit)).unwrap(), explicit);
        assert_eq!(
            settings_path(Path::new("D:\\x")),
            PathBuf::from("D:\\x").join(SETTINGS_FILE)
        );
        assert_eq!(CLAUDE_CONFIG_DIR_VAR, "CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn a_lock_file_is_found_by_any_of_its_names() {
        let dir = TempDir::new("settings-lock");
        assert_eq!(lock_in_the_way(&dir.path), None);
        std::fs::write(dir.join(LOCK_FILES[0]), "").unwrap();
        assert_eq!(lock_in_the_way(&dir.path), Some(dir.join(LOCK_FILES[0])));
    }
}
