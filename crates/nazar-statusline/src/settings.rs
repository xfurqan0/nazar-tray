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
pub fn config_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = explicit {
        return Ok(dir.to_path_buf());
    }
    if let Some(value) = std::env::var_os(CLAUDE_CONFIG_DIR_VAR) {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    Ok(nazar_core::paths::home_dir()?.join(".claude"))
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
#[must_use]
pub fn quote_program(path: &str) -> String {
    if path.contains(' ') && !path.starts_with('"') {
        format!("\"{path}\"")
    } else {
        path.to_owned()
    }
}

/// Whether a command string is this program.
///
/// Used for the idempotency check and for `uninstall`'s "is it even installed" question.
/// A substring test on the file name rather than a path comparison, because the path in
/// the file can be the installed copy, a build directory, or a copy the user moved.
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
    fn our_own_command_is_recognised_however_it_is_spelled() {
        assert!(is_our_command("C:\\bin\\nazar-statusline.exe"));
        assert!(is_our_command(
            "\"C:\\Program Files\\nazar\\NAZAR-STATUSLINE.EXE\""
        ));
        assert!(is_our_command("/usr/local/bin/nazar-statusline"));
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
