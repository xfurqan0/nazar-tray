//! Where nazar-tray keeps its files.
//!
//! Every path is derived from the environment at run time. The audit's rule was blunt:
//! **no absolute paths in code**, because they break the moment a user is not called
//! what the developer is called.
//!
//! Frozen for v1 (decision K22): the data file is `~/.nazar/limits.json`, a single file.
//! Multi-account support in v2 will move to `~/.nazar/limits/<profile>.json`; consumers
//! written against v1 may assume the single file, which is why this is written down here
//! and in `docs/limits-contract.md` rather than discovered later.

use std::path::PathBuf;

use crate::error::{Error, Result};

/// Environment variable that moves `~/.nazar` somewhere else.
///
/// It exists so that a test can point the whole data layer at a throwaway directory
/// without touching the machine it runs on. The status-line wrapper honours it too, which
/// is what makes the wrapper's tests safe to run on a developer's own computer.
pub const NAZAR_HOME_VAR: &str = "NAZAR_HOME";

/// The user's home directory, from `USERPROFILE` on Windows and `HOME` elsewhere.
pub fn home_dir() -> Result<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    match std::env::var_os(key) {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Err(Error::NoHomeDirectory),
    }
}

/// `~/.nazar` — the directory nazar-tray writes for other programs to read.
///
/// `NAZAR_HOME` overrides it wholesale. An empty variable is not a directory and is
/// ignored, the same rule `CODEX_HOME` follows.
pub fn data_dir() -> Result<PathBuf> {
    let variable = std::env::var_os(NAZAR_HOME_VAR);
    match resolve_data_dir(variable.as_deref()) {
        Some(dir) => Ok(dir),
        None => Ok(home_dir()?.join(".nazar")),
    }
}

/// The override half of [`data_dir`], separated so it can be tested.
///
/// Mutating the process environment from a test is unsound once anything else reads it,
/// and a test suite runs in threads, so the rule is checked directly instead.
fn resolve_data_dir(variable: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    variable
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// `~/.nazar/limits.json` — the one file Nazar reads from nazar-tray.
pub fn limits_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("limits.json"))
}

/// `~/.nazar/limits.lock` — the advisory file that says who is allowed to write.
///
/// Beside the file it guards, on the same volume, for the same reason the atomic writer's
/// temporary file is: a lock on one filesystem guarding a file on another guards nothing.
/// What is in it, and what makes a holder count as alive, is [`crate::lock`].
pub fn lock_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("limits.lock"))
}

/// `~/.nazar/tray.request` — "somebody launched the tray again; show the panel".
///
/// A second launch cannot open the first one's window directly without a channel between
/// two processes, and every portable way to build one is heavier than this: a file that
/// exists for a moment, is noticed within five seconds by the loop that is already looking
/// at this directory's neighbours, and is deleted as it is acted on. It carries a timestamp
/// and nothing else.
pub fn request_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("tray.request"))
}

/// `~/.nazar/statusline` — per-session capture files from the `nazar-statusline` wrapper.
///
/// Keyed by session id, never a single fixed path: three concurrent Claude Code sessions
/// would otherwise overwrite each other's capture.
pub fn statusline_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("statusline"))
}

/// Where the user's own settings live: `%APPDATA%\nazar\config.json` on Windows,
/// `~/.config/nazar/config.json` elsewhere.
///
/// Separate from [`data_dir`] on purpose: settings are the user's, `~/.nazar` is ours.
///
/// `NAZAR_HOME` overrides both, and it is checked **first**. The variable exists so that
/// a whole installation can be pointed at a throwaway directory; settings that stayed
/// behind in the real `%APPDATA%` would make that promise only half true, and a test that
/// turned the detailed-windows mode on would turn it on for the machine it ran on.
pub fn config_path() -> Result<PathBuf> {
    if let Some(dir) = resolve_data_dir(std::env::var_os(NAZAR_HOME_VAR).as_deref()) {
        return Ok(dir.join("config.json"));
    }
    let dir = if cfg!(windows) {
        match std::env::var_os("APPDATA") {
            Some(value) if !value.is_empty() => PathBuf::from(value).join("nazar"),
            _ => data_dir()?,
        }
    } else {
        match std::env::var_os("XDG_CONFIG_HOME") {
            Some(value) if !value.is_empty() => PathBuf::from(value).join("nazar"),
            _ => home_dir()?.join(".config").join("nazar"),
        }
    };
    Ok(dir.join("config.json"))
}

/// `<CLAUDE_CONFIG_DIR or ~/.claude>` — Claude Code's own directory.
///
/// Nothing on the default path opens anything in here except the wrapper's installer,
/// which edits `settings.json`. The opt-in detailed-windows mode reads one more file from
/// it, and only while that mode is on; see `docs/detailed-windows.md`.
pub fn claude_config_dir() -> Result<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Ok(home_dir()?.join(".claude")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_paths_hang_off_the_home_directory() {
        if std::env::var_os(NAZAR_HOME_VAR).is_some_and(|value| !value.is_empty()) {
            // The override is in force on this machine; the next test covers it.
            return;
        }
        let Ok(home) = home_dir() else {
            // A build environment without HOME or USERPROFILE is a valid environment;
            // the functions report it rather than guessing a path.
            assert!(matches!(data_dir(), Err(Error::NoHomeDirectory)));
            return;
        };

        assert_eq!(data_dir().unwrap(), home.join(".nazar"));
        assert_eq!(
            limits_path().unwrap(),
            home.join(".nazar").join("limits.json")
        );
        assert_eq!(
            statusline_dir().unwrap(),
            home.join(".nazar").join("statusline")
        );
        assert!(config_path().unwrap().ends_with("nazar/config.json") || cfg!(windows));
    }

    #[test]
    fn nazar_home_moves_the_whole_data_directory() {
        use std::ffi::OsStr;

        assert_eq!(
            resolve_data_dir(Some(OsStr::new("D:\\throwaway\\nazar"))),
            Some(PathBuf::from("D:\\throwaway\\nazar"))
        );
        assert_eq!(
            resolve_data_dir(Some(OsStr::new("/tmp/nazar-test"))),
            Some(PathBuf::from("/tmp/nazar-test"))
        );
        assert_eq!(resolve_data_dir(None), None, "unset falls back to ~/.nazar");
        assert_eq!(
            resolve_data_dir(Some(OsStr::new(""))),
            None,
            "an empty variable is not a directory"
        );
        assert_eq!(NAZAR_HOME_VAR, "NAZAR_HOME");
    }

    #[test]
    fn the_frozen_v1_layout_is_a_single_file() {
        let Ok(path) = limits_path() else { return };
        assert_eq!(path.file_name().unwrap(), "limits.json");
        assert_eq!(path.parent().unwrap(), data_dir().unwrap());
        assert_eq!(
            statusline_dir().unwrap().parent().unwrap(),
            data_dir().unwrap()
        );
    }

    #[test]
    fn the_lock_sits_beside_the_file_it_guards() {
        let Ok(limits) = limits_path() else { return };
        let lock = lock_path().unwrap();
        assert_eq!(lock.file_name().unwrap(), "limits.lock");
        assert_eq!(lock.parent(), limits.parent());
        assert_eq!(request_path().unwrap().parent(), limits.parent());
    }
}
