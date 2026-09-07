//! The user's settings: `%APPDATA%\nazar\config.json`.
//!
//! Small on purpose. Two booleans today — whether the opt-in detailed-windows mode is on,
//! and whether the app has already offered it once — and the same three properties the
//! `limits.json` contract has, for the same reasons:
//!
//! * **Unknown fields survive.** A key a newer build wrote is read back and written out
//!   unchanged, so running an older tray once does not throw the user's settings away.
//! * **Atomic writes.** Temporary file beside the target, then a rename. A reader sees
//!   the old file or the new one, never half of either.
//! * **A missing file is not an error.** It means "the defaults", and the defaults are
//!   what the product does before anyone has chosen anything.
//!
//! The one thing this module will not do is write the file on its own. Reading settings
//! never creates them: a machine where nothing has been configured has no `config.json`,
//! and that is a state worth being able to see.
//!
//! Where the file lives is [`crate::paths::config_path`]: `%APPDATA%\nazar\config.json` on
//! Windows, `$XDG_CONFIG_HOME/nazar/config.json` or `~/.config/nazar/config.json`
//! elsewhere, and `$NAZAR_HOME/config.json` when that override is set.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;

use crate::atomic;
use crate::error::{Error, Result};
use crate::paths::config_path;
use crate::state::{FreshnessRules, Rules, Thresholds};

/// Settings version written by this build.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// The whole `config.json` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Settings version. Separate from `limits.json`'s: this file is the user's, that one
    /// is a contract with another program.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Whether the opt-in detailed-windows mode is on. **`false` is the shipped default**
    /// and the only value a fresh machine has: with it off, nothing in this product opens
    /// a credential file or a socket.
    #[serde(default)]
    pub detailed_windows: bool,
    /// Whether the app has already offered the detailed-windows mode once. Set when the
    /// suggestion is shown, so that a user who said no is not asked again.
    #[serde(default)]
    pub detailed_suggested: bool,
    /// Percentages at which a window turns amber, red and spent. `60 / 85 / 100`.
    ///
    /// Here rather than in three displays because the audit found three displays with three
    /// different answers to the same question (finding B14). Whatever a user changes it to,
    /// the tray icon, the panel and anything reading `limits.json` read the same numbers.
    #[serde(default)]
    pub thresholds: Thresholds,
    /// When a reading stops being fresh and starts being stale. `5` and `45` minutes.
    #[serde(default)]
    pub freshness: FreshnessRules,
    /// Keys a future version added. Preserved verbatim.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

fn default_schema_version() -> u32 {
    CONFIG_SCHEMA_VERSION
}

impl Default for Config {
    /// Everything off. This is what a machine with no `config.json` behaves like.
    fn default() -> Self {
        Config {
            schema_version: CONFIG_SCHEMA_VERSION,
            detailed_windows: false,
            detailed_suggested: false,
            thresholds: Thresholds::default(),
            freshness: FreshnessRules::default(),
            extra: Map::new(),
        }
    }
}

impl Config {
    /// The settings the derived view needs, in the shape [`crate::state`] wants them.
    #[must_use]
    pub fn rules(&self) -> Rules {
        Rules {
            thresholds: self.thresholds,
            freshness: self.freshness,
        }
    }

    /// Parse settings from JSON text.
    pub fn from_json(text: &str) -> Result<Self> {
        Ok(serde_json::from_str(text)?)
    }

    /// Render the settings the way [`Config::write`] writes them: pretty, trailing newline.
    pub fn to_json(&self) -> Result<String> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }

    /// Read settings from an explicit path.
    ///
    /// A file that is not there is not a failure — it is the defaults. A file that is
    /// there and unparseable **is** a failure, and is reported rather than replaced: the
    /// user wrote it, and quietly overwriting it with defaults would lose their settings
    /// to a stray comma.
    pub fn read(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Config::from_json(&text).map_err(|error| match error {
                Error::Json { source, .. } => Error::json(path, source),
                other => other,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(source) => Err(Error::io(path, source)),
        }
    }

    /// Write settings to an explicit path, atomically, creating the directory if needed.
    pub fn write(&self, path: &Path) -> Result<()> {
        let text = self.to_json()?;
        atomic::write_bytes(path, text.as_bytes())
    }

    /// Read the settings from [`config_path`].
    pub fn load() -> Result<Self> {
        Config::read(&config_path()?)
    }

    /// Write the settings to [`config_path`].
    pub fn save(&self) -> Result<()> {
        self.write(&config_path()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn the_default_is_everything_off() {
        let config = Config::default();
        assert!(
            !config.detailed_windows,
            "the detailed-windows mode ships off; a default that turns it on would make \
             this product read a token without being asked"
        );
        assert!(!config.detailed_suggested);
        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
    }

    #[test]
    fn a_missing_file_reads_as_the_defaults() {
        let dir = TempDir::new("config-missing");
        let path = dir.join("config.json");
        assert_eq!(Config::read(&path).unwrap(), Config::default());
        assert!(
            !path.exists(),
            "reading settings must not create them; an absent file is a state worth seeing"
        );
    }

    #[test]
    fn it_round_trips_through_a_file() {
        let dir = TempDir::new("config-round-trip");
        let path = dir.join("nested").join("config.json");

        let config = Config {
            detailed_windows: true,
            detailed_suggested: true,
            ..Config::default()
        };
        config.write(&path).unwrap();

        assert_eq!(Config::read(&path).unwrap(), config);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with('\n'), "got {text:?}");
        assert!(text.contains("\"detailedWindows\": true"), "got {text}");
    }

    #[test]
    fn unknown_keys_survive_a_rewrite() {
        let newer = r#"{
  "schemaVersion": 1,
  "detailedWindows": true,
  "detailedSuggested": false,
  "theme": "graphite",
  "quietHours": [22, 7]
}"#;
        let config = Config::from_json(newer).unwrap();
        assert!(config.detailed_windows);
        assert_eq!(config.extra["theme"], Value::from("graphite"));

        let written = Config::from_json(&config.to_json().unwrap()).unwrap();
        assert_eq!(
            written, config,
            "an older build must not drop the settings a newer one wrote"
        );
    }

    #[test]
    fn the_thresholds_have_the_documented_defaults() {
        let config = Config::default();
        assert_eq!(config.thresholds.warn, 60.0);
        assert_eq!(config.thresholds.critical, 85.0);
        assert_eq!(config.thresholds.exhausted, 100.0);
        assert_eq!(config.freshness.fresh_minutes, 5);
        assert_eq!(
            config.freshness.aging_minutes, 45,
            "45 minutes is the number docs/PROJECT.md has always named"
        );
        assert_eq!(config.rules().thresholds, config.thresholds);
    }

    #[test]
    fn a_file_that_names_no_thresholds_gets_the_defaults() {
        let minimal = r#"{ "detailedWindows": false }"#;
        let config = Config::from_json(minimal).unwrap();
        assert_eq!(config.thresholds, Thresholds::default());
        assert_eq!(config.freshness, FreshnessRules::default());
    }

    #[test]
    fn thresholds_round_trip_through_the_file() {
        let dir = TempDir::new("config-thresholds");
        let path = dir.join("config.json");

        let config = Config {
            thresholds: Thresholds {
                warn: 50.0,
                critical: 80.0,
                exhausted: 100.0,
            },
            freshness: FreshnessRules {
                fresh_minutes: 2,
                aging_minutes: 30,
            },
            ..Config::default()
        };
        config.write(&path).unwrap();

        assert_eq!(Config::read(&path).unwrap(), config);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"warn\": 50"), "got {text}");
        assert!(text.contains("\"freshMinutes\": 2"), "got {text}");
    }

    #[test]
    fn an_empty_object_is_the_defaults() {
        assert_eq!(Config::from_json("{}").unwrap(), Config::default());
    }

    #[test]
    fn a_damaged_file_is_reported_not_replaced() {
        let dir = TempDir::new("config-damaged");
        let path = dir.join("config.json");
        std::fs::write(&path, "{ \"detailedWindows\": tru").unwrap();

        let error = Config::read(&path).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("config.json"), "got {message}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ \"detailedWindows\": tru",
            "a damaged settings file is left exactly as the user left it"
        );
    }

    #[test]
    fn a_write_leaves_no_temporary_behind() {
        let dir = TempDir::new("config-atomic");
        let path = dir.join("config.json");
        Config::default().write(&path).unwrap();

        let entries: Vec<_> = std::fs::read_dir(&*dir)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["config.json".to_owned()], "got {entries:?}");
    }
}
