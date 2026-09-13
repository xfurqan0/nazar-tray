//! The user's settings: `%APPDATA%\nazar\config.json`.
//!
//! Everything in the settings panel lives here: the opt-in detailed-windows mode, the
//! severity thresholds, the theme, the language, the notification switch and its quiet
//! hours, and which providers are read at all. It has the same three properties the
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
//! **Reading is forgiving; writing is strict.** [`Config::read`] takes whatever is there and
//! makes the best of it, because a user whose settings file has a bad number still has to be
//! able to open the form that fixes it. [`Config::validate`] is the gate that form goes
//! through, and it refuses to save what would only be silently useless — thresholds that do
//! not ascend, a language this build cannot paint, a quiet-hours endpoint that is not a
//! time.
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
    /// Which theme the panel paints itself in: `nazar` (default) or `graphite`.
    ///
    /// A free string rather than an enum: the themes are data files, and a settings file
    /// naming one this build has never heard of is a thing to survive — by falling back to
    /// `nazar` — rather than a reason to refuse to start.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// `system` (follow `prefers-color-scheme`), `light`, or `dark`.
    #[serde(default = "default_theme_mode")]
    pub theme_mode: String,
    /// Whether the first-run "the icon is in the overflow" hint has been dismissed.
    ///
    /// Windows 11 hides every new tray icon behind the `^` button, and an application whose
    /// icon nobody can find looks broken rather than hidden (`docs/PROJECT.md` section 5).
    /// The hint is shown once, in the panel, and dismissed for good here.
    #[serde(default)]
    pub first_run_hint_dismissed: bool,
    /// Language override, e.g. `tr`. `None` means "follow the system".
    ///
    /// One of [`LOCALES`], or absent. The tray resolves it once — override, then the
    /// operating system's UI language, then English — and hands the answer to the panel as
    /// well as to its own tooltip and menu, so the two can never end up in different
    /// languages. (They could in WP4, and did: the panel guessed from `navigator.languages`
    /// while the tray fell back to English.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// Whether threshold notifications are shown at all.
    ///
    /// The master switch above the thresholds and the quiet hours. With it off the state
    /// machine still runs and still consumes its keys, so turning it back on does not
    /// produce a burst of warnings about crossings that happened while it was off.
    #[serde(default = "default_true")]
    pub notifications: bool,
    /// Hours in which a notification is not shown. Local time, `HH:MM`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_hours: Option<QuietHours>,
    /// Which providers are read at all.
    #[serde(default)]
    pub providers: ProviderSwitches,
    /// The two choices about how the usage history is counted and how far back it goes.
    #[serde(default)]
    pub usage: UsageSwitches,
    /// Keys a future version added. Preserved verbatim.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

/// The theme a machine that has never been configured paints itself in.
pub const DEFAULT_THEME: &str = "nazar";
/// The default light/dark behaviour: whatever the operating system says.
pub const DEFAULT_THEME_MODE: &str = "system";

/// Every language the panel and the tray can be shown in.
///
/// The same list as `ui/src/i18n.ts`'s `LOCALES`, and a test in `ui/test/bridge.test.mjs`
/// keeps the two files level: a language offered in the settings that the panel cannot
/// paint itself in would be a menu entry that does nothing.
pub const LOCALES: [&str; 6] = ["en", "tr", "zh", "ko", "ru", "es"];

/// The word the settings form uses for "follow the operating system".
///
/// In the form it is a value; in the file it is the **absence** of `locale`, because a
/// setting nobody has chosen should not be written down as a choice.
pub const LOCALE_SYSTEM: &str = "system";

/// The light/dark modes the panel understands.
pub const THEME_MODES: [&str; 3] = ["system", "light", "dark"];

/// The themes this build ships. A settings file naming another one falls back to `nazar`.
pub const THEMES: [&str; 2] = ["nazar", "graphite"];

fn default_true() -> bool {
    true
}

/// A stretch of local time in which no notification is shown.
///
/// `from` and `to` are `HH:MM` on a 24-hour clock, and the range **wraps midnight** when
/// `to` is earlier than `from` — which is the only shape anybody actually wants, since
/// quiet hours are for the night. Equal endpoints are an empty range rather than a whole
/// day: a user who wants no notifications at all turns [`Config::notifications`] off, and
/// reading `22:00–22:00` as "always" would silence the product by accident.
///
/// Deliberately **not** stored with an offset or a zone. It is wall-clock time on the
/// machine the tray runs on: "do not interrupt me between eleven and seven" means eleven on
/// the user's own clock, and it goes on meaning that after a flight. This is the one place
/// in the product where local time is a value rather than a rendering decision, and it is
/// why [`crate::alerts::AlertRules`] takes the answer rather than the question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuietHours {
    /// When quiet starts, `HH:MM` local.
    pub from: String,
    /// When quiet ends, `HH:MM` local.
    pub to: String,
}

impl QuietHours {
    /// Both endpoints as minutes since local midnight, or `None` if either is not a time.
    #[must_use]
    pub fn bounds(&self) -> Option<(u16, u16)> {
        Some((parse_hhmm(&self.from)?, parse_hhmm(&self.to)?))
    }

    /// Whether both endpoints are times this build understands.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.bounds().is_some()
    }

    /// Whether `minutes` past local midnight falls inside the quiet stretch.
    ///
    /// A range this build cannot parse suppresses nothing: the failure mode of a typo in
    /// the settings file has to be *more* notifications, never silence.
    #[must_use]
    pub fn contains(&self, minutes: u16) -> bool {
        let Some((from, to)) = self.bounds() else {
            return false;
        };
        if from == to {
            return false;
        }
        if from < to {
            minutes >= from && minutes < to
        } else {
            // 22:00 → 07:00 is two pieces of one range.
            minutes >= from || minutes < to
        }
    }
}

/// `HH:MM` to minutes since midnight. Strict: two digits, a colon, two digits.
///
/// Strict because the value comes from an `<input type="time">`, which produces exactly
/// that, and because a lenient parser here would turn `7:00` — which a user might mean as
/// seven in the morning — into something that silently never matches.
#[must_use]
pub fn parse_hhmm(text: &str) -> Option<u16> {
    let (hours, minutes) = text.split_once(':')?;
    if hours.len() != 2 || minutes.len() != 2 {
        return None;
    }
    let hours: u16 = hours.parse().ok()?;
    let minutes: u16 = minutes.parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(hours * 60 + minutes)
}

/// Which providers the tray reads.
///
/// A provider switched off is **not read at all** — no reader is built for it, so its files
/// are never opened — and its card is not drawn. That is a stronger promise than hiding it:
/// somebody who does not use Codex should not have a program listing their session logs
/// every five seconds to find out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSwitches {
    /// Whether Claude Code's status-line captures are read.
    #[serde(default = "default_true")]
    pub claude: bool,
    /// Whether Codex's rollout logs are read.
    #[serde(default = "default_true")]
    pub codex: bool,
}

impl Default for ProviderSwitches {
    fn default() -> Self {
        ProviderSwitches {
            claude: true,
            codex: true,
        }
    }
}

impl ProviderSwitches {
    /// Whether a provider key is switched on. A key this build has never heard of is on.
    #[must_use]
    pub fn enabled(&self, provider: &str) -> bool {
        match provider {
            "claude" => self.claude,
            "codex" => self.codex,
            _ => true,
        }
    }
}

/// How the usage history is counted, and how far back it claims to go.
///
/// **Both are off, and the defaults are the honest answers.** What the panel shows without
/// either of them is what this machine actually spent, counted from logs this product read
/// itself. Each switch trades one of those properties for agreement with another program,
/// which is a thing to choose rather than a thing to arrive at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSwitches {
    /// Show the per-line numbers that Claude Code's `/usage` shows.
    ///
    /// Claude Code writes one transcript line per content block and every one of them carries
    /// the whole `usage` object, so adding the lines up counts a message once per block —
    /// about 1.7× the real spend on the machine this was measured on, and Claude Code's own
    /// Stats screen does exactly that (`anthropics/claude-code#91775`). The store keeps both
    /// numbers; this decides which one is drawn, in the panel and in the tray tooltip
    /// together, so the two surfaces can never disagree.
    #[serde(default)]
    pub count_like_claude_code: bool,
    /// Show the days before the transcripts, as Claude Code reported them.
    ///
    /// `~/.claude/stats-cache.json` reaches further back than the transcripts do, because
    /// Claude Code prunes those and keeps this. What it holds is one total per model per day
    /// and nothing else — no split into the four counters, and per-line rather than
    /// deduplicated — so those days are drawn apart from the measured ones and labelled as
    /// somebody else's arithmetic. Off by default: a history that quietly mixes two kinds of
    /// number is worse than a shorter one.
    #[serde(default)]
    pub fill_history_from_stats: bool,
}

/// Something wrong with a settings document, in the vocabulary the panel translates.
///
/// Returned by [`Config::validate`], which is what the settings form is checked against
/// before anything is written. Reading is deliberately more forgiving than writing: a file
/// already on disk is used as best it can be, because refusing to start over a bad number
/// would leave the user with no way to reach the form that fixes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Invalid {
    /// The thresholds are not strictly ascending, or one is outside `0 < value <= 100`.
    Thresholds,
    /// The language is not one of [`LOCALES`].
    Locale,
    /// The light/dark mode is not one of [`THEME_MODES`].
    ThemeMode,
    /// A quiet-hours endpoint is not an `HH:MM` time.
    QuietHours,
    /// The freshness rules are not ascending.
    Freshness,
}

fn default_schema_version() -> u32 {
    CONFIG_SCHEMA_VERSION
}

fn default_theme() -> String {
    DEFAULT_THEME.to_owned()
}

fn default_theme_mode() -> String {
    DEFAULT_THEME_MODE.to_owned()
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
            theme: default_theme(),
            theme_mode: default_theme_mode(),
            first_run_hint_dismissed: false,
            locale: None,
            notifications: true,
            quiet_hours: None,
            providers: ProviderSwitches::default(),
            usage: UsageSwitches::default(),
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

    /// Everything wrong with these settings, or an empty list.
    ///
    /// The gate the settings form is put through before anything is written. Five rules,
    /// and each one is a value that would otherwise be *silently* useless rather than
    /// obviously wrong:
    ///
    /// * **Thresholds must strictly ascend inside `0 < value <= 100`.** `85 / 60 / 100`
    ///   would make the amber threshold unreachable, and the notification ladder would sort
    ///   itself behind the user's back rather than telling them.
    /// * **The language must be one this build can paint.** Offering `de` in the form and
    ///   then falling back to English is worse than not offering it.
    /// * **The mode must be `system`, `light` or `dark`.**
    /// * **A quiet-hours endpoint must be a time.** An unparseable one suppresses nothing,
    ///   so the user would think they were quiet and not be.
    /// * **Fresh must come before stale.** One staleness rule, in one place, is finding B14.
    ///
    /// The theme is deliberately *not* checked: it is a data file, a name this build has
    /// never heard of falls back to `nazar` when it is painted, and rejecting it here would
    /// throw away a theme a newer build wrote.
    #[must_use]
    pub fn validate(&self) -> Vec<Invalid> {
        let mut problems = Vec::new();

        let steps = [
            self.thresholds.warn,
            self.thresholds.critical,
            self.thresholds.exhausted,
        ];
        let in_range = steps
            .iter()
            .all(|value| value.is_finite() && *value > 0.0 && *value <= 100.0);
        let ascending = steps.windows(2).all(|pair| pair[0] < pair[1]);
        if !in_range || !ascending {
            problems.push(Invalid::Thresholds);
        }

        if let Some(locale) = self.locale.as_deref()
            && !LOCALES.contains(&locale)
        {
            problems.push(Invalid::Locale);
        }
        if !THEME_MODES.contains(&self.theme_mode.as_str()) {
            problems.push(Invalid::ThemeMode);
        }
        if self
            .quiet_hours
            .as_ref()
            .is_some_and(|hours| !hours.is_valid())
        {
            problems.push(Invalid::QuietHours);
        }
        if self.freshness.fresh_minutes >= self.freshness.aging_minutes {
            problems.push(Invalid::Freshness);
        }
        problems
    }

    /// Whether a notification crossing `threshold` may interrupt at `local_minutes`.
    ///
    /// `local_minutes` is minutes since **local** midnight, which is the one thing this
    /// crate cannot work out for itself; the tray asks the operating system and passes the
    /// answer in. `None` — a platform where nobody asked — is treated as "not quiet",
    /// because the failure mode of not knowing has to be a notification too many rather
    /// than a warning the user never got.
    #[must_use]
    pub fn is_quiet_at(&self, local_minutes: Option<u16>) -> bool {
        match (self.quiet_hours.as_ref(), local_minutes) {
            (Some(hours), Some(minutes)) => hours.contains(minutes),
            _ => false,
        }
    }

    /// The rules [`crate::alerts`] evaluates a crossing against, for this instant.
    #[must_use]
    pub fn alert_rules(&self, local_minutes: Option<u16>) -> crate::alerts::AlertRules {
        crate::alerts::AlertRules {
            thresholds: self.thresholds,
            // The master switch and the quiet hours arrive as one flag, because they mean
            // the same thing to the state machine: the crossing happens and is recorded,
            // the icon changes colour, and nobody is interrupted. That is what makes
            // switching the notifications back on quiet rather than a burst of catching up.
            quiet: !self.notifications || self.is_quiet_at(local_minutes),
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
  "snoozeUntil": "2026-09-08T07:00:00Z"
}"#;
        let config = Config::from_json(newer).unwrap();
        assert!(config.detailed_windows);
        // `theme` is a key this build knows about; `snoozeUntil` is not, and is the one
        // that has to survive being read and written by a build that cannot use it.
        // (`quietHours` used to play this part, until WP5 made it a real key — which is
        // the test noticing the day a placeholder became a feature.)
        assert_eq!(config.theme, "graphite");
        assert_eq!(
            config.extra["snoozeUntil"],
            Value::from("2026-09-08T07:00:00Z")
        );

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
    fn the_panel_settings_have_the_documented_defaults_and_round_trip() {
        let fresh = Config::default();
        assert_eq!(fresh.theme, DEFAULT_THEME);
        assert_eq!(fresh.theme_mode, DEFAULT_THEME_MODE);
        assert!(
            !fresh.first_run_hint_dismissed,
            "the overflow hint is shown once, so a fresh machine has not dismissed it"
        );
        assert_eq!(
            fresh.locale, None,
            "no language is chosen until one is chosen"
        );

        let dir = TempDir::new("config-panel");
        let path = dir.join("config.json");
        let chosen = Config {
            theme: "graphite".to_owned(),
            theme_mode: "dark".to_owned(),
            first_run_hint_dismissed: true,
            locale: Some("tr".to_owned()),
            ..Config::default()
        };
        chosen.write(&path).unwrap();
        assert_eq!(Config::read(&path).unwrap(), chosen);

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"theme\": \"graphite\""), "got {text}");
        assert!(
            text.contains("\"firstRunHintDismissed\": true"),
            "got {text}"
        );
        assert!(text.contains("\"locale\": \"tr\""), "got {text}");
    }

    #[test]
    fn a_settings_file_from_before_the_panel_settings_still_reads() {
        // What WP3 wrote. An older file must not lose the user their settings, and must
        // not arrive with an empty theme name that the panel would then have to guess at.
        let older = r#"{ "schemaVersion": 1, "detailedWindows": true }"#;
        let config = Config::from_json(older).unwrap();
        assert!(config.detailed_windows);
        assert_eq!(config.theme, DEFAULT_THEME);
        assert_eq!(config.theme_mode, DEFAULT_THEME_MODE);
        assert!(!config.first_run_hint_dismissed);
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
    fn the_defaults_notify_and_read_both_providers() {
        let fresh = Config::default();
        assert!(
            fresh.notifications,
            "being warned before you hit the wall is the reason a quota tray exists"
        );
        assert_eq!(fresh.quiet_hours, None, "nobody has asked for quiet yet");
        assert!(fresh.providers.claude && fresh.providers.codex);
        assert!(
            fresh.validate().is_empty(),
            "the shipped defaults are valid"
        );
    }

    #[test]
    fn a_settings_file_from_before_wp5_gets_the_notification_defaults() {
        // What WP4 wrote. Reading it must not silence a tray that was upgraded.
        let older = r#"{ "schemaVersion": 1, "theme": "graphite", "themeMode": "dark" }"#;
        let config = Config::from_json(older).unwrap();
        assert!(config.notifications);
        assert!(config.providers.enabled("claude"));
        assert!(config.providers.enabled("codex"));
        assert_eq!(config.quiet_hours, None);
    }

    #[test]
    fn the_wp5_settings_round_trip_through_the_file() {
        let dir = TempDir::new("config-wp5");
        let path = dir.join("config.json");

        let chosen = Config {
            notifications: true,
            quiet_hours: Some(QuietHours {
                from: "22:30".to_owned(),
                to: "07:00".to_owned(),
            }),
            providers: ProviderSwitches {
                claude: true,
                codex: false,
            },
            locale: Some("tr".to_owned()),
            ..Config::default()
        };
        chosen.write(&path).unwrap();
        assert_eq!(Config::read(&path).unwrap(), chosen);

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"quietHours\""), "got {text}");
        assert!(text.contains("\"from\": \"22:30\""), "got {text}");
        assert!(text.contains("\"codex\": false"), "got {text}");
    }

    #[test]
    fn a_half_written_provider_block_leaves_the_other_provider_on() {
        let partial = r#"{ "providers": { "codex": false } }"#;
        let config = Config::from_json(partial).unwrap();
        assert!(
            config.providers.claude,
            "switching one provider off must not switch the other one off by omission"
        );
        assert!(!config.providers.codex);
        assert!(
            config.providers.enabled("something-new"),
            "a provider a newer build added is on until somebody turns it off"
        );
    }

    #[test]
    fn quiet_hours_wrap_midnight_and_an_empty_range_is_empty() {
        let night = QuietHours {
            from: "22:00".to_owned(),
            to: "07:00".to_owned(),
        };
        assert!(night.contains(22 * 60), "22:00 is quiet");
        assert!(night.contains(23 * 60 + 59));
        assert!(
            night.contains(0),
            "midnight is on the other side of the wrap"
        );
        assert!(night.contains(6 * 60 + 59));
        assert!(
            !night.contains(7 * 60),
            "the end of the range is not inside it"
        );
        assert!(!night.contains(12 * 60));
        assert!(!night.contains(21 * 60 + 59));

        let afternoon = QuietHours {
            from: "13:00".to_owned(),
            to: "14:00".to_owned(),
        };
        assert!(afternoon.contains(13 * 60 + 30));
        assert!(!afternoon.contains(12 * 60 + 59));
        assert!(!afternoon.contains(14 * 60));

        let empty = QuietHours {
            from: "22:00".to_owned(),
            to: "22:00".to_owned(),
        };
        for minute in [0, 22 * 60, 23 * 60 + 59] {
            assert!(
                !empty.contains(minute),
                "equal endpoints must not silence the product for a whole day by accident"
            );
        }
    }

    #[test]
    fn a_quiet_range_nobody_can_parse_suppresses_nothing() {
        let broken = QuietHours {
            from: "10pm".to_owned(),
            to: "7".to_owned(),
        };
        assert!(!broken.is_valid());
        for minute in [0, 60, 22 * 60] {
            assert!(
                !broken.contains(minute),
                "the failure mode of a typo has to be one toast too many, never silence"
            );
        }
    }

    #[test]
    fn a_time_is_two_digits_a_colon_and_two_digits() {
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("07:05"), Some(7 * 60 + 5));
        assert_eq!(parse_hhmm("23:59"), Some(23 * 60 + 59));
        for text in ["7:00", "24:00", "22:60", "22-00", "2200", "", ":", "aa:bb"] {
            assert_eq!(parse_hhmm(text), None, "{text} is not a time");
        }
    }

    #[test]
    fn quiet_hours_only_apply_when_the_local_clock_could_be_read() {
        let config = Config {
            quiet_hours: Some(QuietHours {
                from: "22:00".to_owned(),
                to: "07:00".to_owned(),
            }),
            ..Config::default()
        };
        assert!(config.is_quiet_at(Some(23 * 60)));
        assert!(!config.is_quiet_at(Some(12 * 60)));
        assert!(
            !config.is_quiet_at(None),
            "a platform that cannot say what time it is locally does not get to silence us"
        );
        assert!(config.alert_rules(Some(23 * 60)).quiet);
        assert_eq!(
            config.alert_rules(None).thresholds,
            config.thresholds,
            "the ladder is the settings' ladder either way"
        );

        // The master switch reaches the state machine as the same flag, so turning the
        // notifications off consumes the keys quietly instead of storing up a burst.
        let silent = Config {
            notifications: false,
            ..Config::default()
        };
        assert!(silent.alert_rules(Some(12 * 60)).quiet);
        assert!(
            !Config::default().alert_rules(Some(12 * 60)).quiet,
            "and a machine nobody has configured is interruptible at noon"
        );
    }

    #[test]
    fn validation_catches_what_would_otherwise_be_silently_useless() {
        let with = |warn, critical, exhausted| Config {
            thresholds: Thresholds {
                warn,
                critical,
                exhausted,
            },
            ..Config::default()
        };

        assert_eq!(
            with(85.0, 60.0, 100.0).validate(),
            vec![Invalid::Thresholds]
        );
        assert_eq!(
            with(60.0, 60.0, 100.0).validate(),
            vec![Invalid::Thresholds],
            "two thresholds at the same number make one of them unreachable"
        );
        assert_eq!(with(0.0, 85.0, 100.0).validate(), vec![Invalid::Thresholds]);
        assert_eq!(
            with(60.0, 85.0, 120.0).validate(),
            vec![Invalid::Thresholds]
        );
        assert!(with(1.0, 2.0, 100.0).validate().is_empty());

        let config = Config {
            locale: Some("de".to_owned()),
            ..Config::default()
        };
        assert_eq!(config.validate(), vec![Invalid::Locale]);
        for locale in LOCALES {
            let config = Config {
                locale: Some(locale.to_owned()),
                ..Config::default()
            };
            assert!(
                config.validate().is_empty(),
                "{locale} is a language we ship"
            );
        }
        assert!(
            Config {
                locale: None,
                ..Config::default()
            }
            .validate()
            .is_empty(),
            "no language chosen is a valid state and the one a fresh machine is in"
        );
        assert!(
            !LOCALES.contains(&LOCALE_SYSTEM),
            "`system` is a word the form uses, never a value the file holds"
        );

        let config = Config {
            theme_mode: "sepia".to_owned(),
            ..Config::default()
        };
        assert_eq!(config.validate(), vec![Invalid::ThemeMode]);

        let config = Config {
            quiet_hours: Some(QuietHours {
                from: "22:00".to_owned(),
                to: "sunrise".to_owned(),
            }),
            ..Config::default()
        };
        assert_eq!(config.validate(), vec![Invalid::QuietHours]);

        let config = Config {
            freshness: FreshnessRules {
                fresh_minutes: 60,
                aging_minutes: 45,
            },
            ..Config::default()
        };
        assert_eq!(config.validate(), vec![Invalid::Freshness]);
    }

    #[test]
    fn a_theme_this_build_has_never_heard_of_is_not_a_validation_failure() {
        let config = Config {
            theme: "midnight".to_owned(),
            ..Config::default()
        };
        assert!(
            config.validate().is_empty(),
            "themes are data files; rejecting an unknown one would throw away a newer \
             build's theme on the first save"
        );
        assert!(THEMES.contains(&DEFAULT_THEME));
    }

    #[test]
    fn a_damaged_file_is_still_read_even_though_it_would_not_be_saved() {
        // Reading is forgiving and writing is strict, on purpose: a user whose settings
        // file has a bad number still has to be able to reach the form that fixes it.
        let bad = r#"{ "thresholds": { "warn": 90, "critical": 50, "exhausted": 100 } }"#;
        let config = Config::from_json(bad).unwrap();
        assert_eq!(config.thresholds.warn, 90.0);
        assert_eq!(config.validate(), vec![Invalid::Thresholds]);
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
