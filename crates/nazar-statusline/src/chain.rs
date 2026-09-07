//! The status line the user already had, and how it is given back to them.
//!
//! Installing this wrapper replaces `statusLine.command`. The command it replaced is not
//! thrown away: it is copied verbatim into `~/.nazar/statusline/chain.json` and then run
//! on every refresh with the same bytes on standard input, its output forwarded and its
//! exit code adopted. From the user's side nothing changed except that a file now gets
//! written first.
//!
//! `chain.json` is also the uninstall path of last resort. If this binary is deleted
//! before it is uninstalled, the original `statusLine` object is sitting there in plain
//! JSON and can be pasted back by hand — which is written down in
//! `docs/statusline-wrapper.md` rather than left as folklore.
//!
//! ## Running someone else's command line
//!
//! `statusLine.command` is a *command line*, not a program and a list of arguments:
//! `node "C:\…\statusline.js"` is one string that only a shell can take apart. So the
//! chained command runs through `cmd.exe /C` on Windows and `sh -c` elsewhere, which is
//! what it was already running under before this program existed.
//!
//! On Windows the command line is handed over **raw**. Rust's own argument escaping
//! follows the C runtime's rules, and `cmd.exe` does not; going through it would break
//! every command with a quoted path in it — which is to say the maintainer's own. The
//! whole command is wrapped in one more pair of quotes because `cmd /C` strips the first
//! and the last quote of what follows, so the wrap is what makes a command that *starts*
//! with a quoted program name survive.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fail::{Failure, Result};

/// Schema version of `chain.json`.
pub const CHAIN_SCHEMA_VERSION: u32 = 1;

/// What the installer recorded, so that uninstall can put it back exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chain {
    /// Version of this file's own shape.
    pub schema_version: u32,
    /// When the install happened, RFC 3339 in UTC.
    pub installed_at: String,
    /// The settings file that was edited.
    pub settings_path: String,
    /// The untouched copy taken before the edit. `None` when there was no file to copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    /// The command this program was installed as.
    pub installed_command: String,
    /// **The exact `statusLine` object the file held before.** `None` means the file had
    /// no `statusLine` key, and uninstall removes the key rather than inventing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Value>,
}

impl Chain {
    /// Render it the way it is written: pretty, trailing newline.
    pub fn to_json(&self) -> Result<String> {
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|error| Failure::new(format!("could not render chain.json: {error}")))?;
        text.push('\n');
        Ok(text)
    }

    /// Parse one.
    pub fn from_json(text: &str) -> Result<Self> {
        serde_json::from_str(text)
            .map_err(|error| Failure::new(format!("chain.json is not readable: {error}")))
    }

    /// The command line to chain to, if the recorded status line names one.
    ///
    /// A `statusLine` whose `type` is not `command` has no command line to run — a future
    /// Claude Code might grow another type — so it chains to nothing and the wrapper
    /// prints its own line instead of guessing.
    #[must_use]
    pub fn command(&self) -> Option<&str> {
        let line = self.previous.as_ref()?.as_object()?;
        if line.get("type").and_then(Value::as_str) != Some("command") {
            return None;
        }
        line.get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|command| !command.is_empty())
    }
}

/// `~/.nazar/statusline/chain.json` (`NAZAR_HOME` moves it).
pub fn chain_path() -> Result<PathBuf> {
    Ok(nazar_core::paths::statusline_dir()?.join(nazar_core::claude::CHAIN_FILE_NAME))
}

/// Read `chain.json`, or nothing if it is not there.
pub fn load() -> Result<Option<Chain>> {
    let path = chain_path()?;
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(Chain::from_json(&text)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Failure::new(format!(
            "could not read {}: {error}",
            path.display()
        ))),
    }
}

/// Write `chain.json` atomically.
pub fn store(chain: &Chain) -> Result<PathBuf> {
    let path = chain_path()?;
    nazar_core::atomic::write_bytes(&path, chain.to_json()?.as_bytes())?;
    Ok(path)
}

/// Run a chained command line with `payload` on its standard input.
///
/// Returns the command's exit code. Standard output and standard error are inherited, so
/// the chained command writes straight to the same places this process would have — no
/// buffering, no re-encoding, nothing between it and Claude Code.
///
/// A command that cannot be started is reported on standard error and produces `Some(127)`,
/// the shell's own "command not found" code, so the user sees something is wrong instead of
/// a silently empty status line.
#[must_use]
pub fn run(command: &str, payload: &[u8]) -> i32 {
    let mut child = match spawn(command) {
        Ok(child) => child,
        Err(error) => {
            eprintln!("nazar-statusline: could not run the chained status line: {error}");
            return 127;
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        // Written from another thread so a command that never reads its input cannot
        // wedge this process against a full pipe. The handle closes when the thread
        // ends, which is what gives the child its end-of-input.
        let payload = payload.to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&payload);
            let _ = stdin.flush();
        });
    }

    match child.wait() {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("nazar-statusline: the chained status line did not finish: {error}");
            1
        }
    }
}

/// Start the chained command under the platform's shell.
fn spawn(command: &str) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    let mut process = {
        use std::os::windows::process::CommandExt;

        let mut process = Command::new("cmd.exe");
        process.raw_arg("/C");
        // The extra pair of quotes is deliberate; see this module's header.
        process.raw_arg(format!("\"{command}\""));
        process
    };

    #[cfg(not(windows))]
    let mut process = {
        let mut process = Command::new("sh");
        process.arg("-c").arg(command);
        process
    };

    process
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain_with(previous: Option<Value>) -> Chain {
        Chain {
            schema_version: CHAIN_SCHEMA_VERSION,
            installed_at: "2026-09-07T06:40:00Z".to_owned(),
            settings_path: "C:\\somewhere\\settings.json".to_owned(),
            backup_path: None,
            installed_command: "C:\\bin\\nazar-statusline.exe".to_owned(),
            previous,
        }
    }

    #[test]
    fn a_chain_round_trips() {
        let chain = chain_with(Some(serde_json::json!({
            "type": "command",
            "command": "node \"C:\\\\projects\\\\example\\\\statusline.js\"",
            "padding": 0,
            "refreshInterval": 30
        })));
        let text = chain.to_json().unwrap();
        assert_eq!(Chain::from_json(&text).unwrap(), chain);
        assert!(text.ends_with("}\n"));
    }

    #[test]
    fn no_previous_status_line_means_nothing_to_chain_to() {
        let chain = chain_with(None);
        assert_eq!(chain.command(), None);
        let text = chain.to_json().unwrap();
        assert!(
            !text.contains("previous"),
            "an absent previous status line is absent, not null: {text}"
        );
        assert_eq!(Chain::from_json(&text).unwrap().previous, None);
    }

    #[test]
    fn the_command_comes_out_exactly_as_it_went_in() {
        let original = "node \"C:\\projects\\example\\statusline.js\"";
        let chain = chain_with(Some(serde_json::json!({
            "type": "command",
            "command": original
        })));
        assert_eq!(chain.command(), Some(original));
    }

    #[test]
    fn a_status_line_of_another_type_is_not_run() {
        for previous in [
            serde_json::json!({"type": "something-new", "command": "rm -rf /"}),
            serde_json::json!({"type": "command"}),
            serde_json::json!({"type": "command", "command": "   "}),
            serde_json::json!("a string"),
            serde_json::json!(null),
        ] {
            assert_eq!(
                chain_with(Some(previous.clone())).command(),
                None,
                "{previous}"
            );
        }
    }
}
