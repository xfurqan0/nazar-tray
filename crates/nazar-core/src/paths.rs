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

/// The user's home directory, from `USERPROFILE` on Windows and `HOME` elsewhere.
pub fn home_dir() -> Result<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    match std::env::var_os(key) {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Err(Error::NoHomeDirectory),
    }
}

/// `~/.nazar` — the directory nazar-tray writes for other programs to read.
pub fn data_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".nazar"))
}

/// `~/.nazar/limits.json` — the one file Nazar reads from nazar-tray.
pub fn limits_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("limits.json"))
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
pub fn config_path() -> Result<PathBuf> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_paths_hang_off_the_home_directory() {
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
    fn the_frozen_v1_layout_is_a_single_file() {
        let Ok(path) = limits_path() else { return };
        assert_eq!(path.file_name().unwrap(), "limits.json");
        assert_eq!(path.parent().unwrap().file_name().unwrap(), ".nazar");
    }
}
