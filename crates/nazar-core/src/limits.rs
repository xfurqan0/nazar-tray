//! The `limits.json` contract, frozen at schema version 1.
//!
//! The full contract, with a worked sample, lives in `docs/limits-contract.md`. This
//! module is its executable form. Three properties are load-bearing and every one of
//! them has a test in this file:
//!
//! * **Round-trip.** Reading a document and writing it back produces the same bytes.
//! * **Forward compatibility.** Fields this version does not know are kept, not dropped,
//!   so a newer writer and an older reader can share a machine without losing data.
//! * **No invented numbers.** [`Window::percent`] is optional. A window that could not be
//!   read carries no percentage at all.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::atomic;
use crate::error::{Error, Result};

/// Schema version written by this build. Bumping it is a breaking change for Nazar.
pub const SCHEMA_VERSION: u32 = 1;

/// The whole `limits.json` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// Contract version. See [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// When the tray last wrote this file, RFC 3339 in UTC (`…Z`). See [`crate::timefmt`].
    pub updated_at: String,
    /// One entry per supported provider.
    pub providers: Providers,
    /// Fields a future version added and this one does not understand. Preserved verbatim.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

impl Limits {
    /// A document with both providers unconfigured, stamped with `updated_at`.
    pub fn new(updated_at: impl Into<String>) -> Self {
        Limits {
            schema_version: SCHEMA_VERSION,
            updated_at: updated_at.into(),
            providers: Providers::default(),
            extra: Map::new(),
        }
    }

    /// Parse a document from JSON text.
    pub fn from_json(text: &str) -> Result<Self> {
        Ok(serde_json::from_str(text)?)
    }

    /// Render the document the way [`write_limits`] writes it: pretty, trailing newline.
    pub fn to_json(&self) -> Result<String> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }
}

/// The providers block. Unknown providers added by a future version land in `extra`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Providers {
    /// Claude Code, fed by the status-line wrapper and, opt-in, by the usage endpoint.
    #[serde(default)]
    pub claude: Provider,
    /// Codex, fed by the newest `rollout-*.jsonl` session log.
    #[serde(default)]
    pub codex: Provider,
    /// Providers a future version added. Preserved verbatim.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

/// One provider's state.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    /// `false` when the provider's files are absent from this machine.
    #[serde(default)]
    pub configured: bool,
    /// Plan name as the source reported it, e.g. `max_20x` or `plus`. Never normalised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// Where the numbers came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// When the source produced them, RFC 3339. Older than `updated_at` by design.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_at: Option<String>,
    /// Key of the window that constrains the user right now: the highest percentage of
    /// all this provider's windows. Never taken from a flag the source does not provide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<String>,
    /// Windows by key. Claude: `five_hour`, `seven_day`, and model-scoped weeklies such
    /// as `seven_day_fable`. Codex: `primary`, `secondary`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub windows: BTreeMap<String, Window>,
    /// Fields a future version added. Preserved verbatim.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

/// One usage window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    /// Percentage used, `0`–`100`, unrounded. **Absent when nothing could be read.**
    /// Consumers render "unknown" for an absent value; they never substitute `0`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_percent"
    )]
    pub percent: Option<f64>,
    /// When this window resets, RFC 3339 as the source reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    /// Window length in minutes: `300` for five-hour windows, `10080` for weekly ones.
    /// Written for both providers so consumers need no provider-specific logic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<u32>,
    /// Freshness of this window. Required; there is no safe default.
    pub state: WindowState,
    /// Short, human-readable reason the window is stale or in error. Never file contents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Model this window is scoped to, for model-scoped weeklies. Absent for global ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `true` when the window came from the opt-in detailed-windows mode (WP2b).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detailed: Option<bool>,
    /// Fields a future version added. Preserved verbatim.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

impl Window {
    fn bare(percent: Option<f64>, state: WindowState) -> Self {
        Window {
            percent,
            resets_at: None,
            window_minutes: None,
            state,
            error: None,
            model: None,
            detailed: None,
            extra: Map::new(),
        }
    }

    /// A window read successfully.
    pub fn ok(percent: f64) -> Self {
        Window::bare(Some(percent), WindowState::Ok)
    }

    /// A window whose last known value is older than the refresh interval.
    pub fn stale(percent: f64) -> Self {
        Window::bare(Some(percent), WindowState::Stale)
    }

    /// A window that could not be read. Carries no percentage on purpose.
    pub fn error(reason: impl Into<String>) -> Self {
        let mut window = Window::bare(None, WindowState::Error);
        window.error = Some(reason.into());
        window
    }

    /// Set `resetsAt`.
    #[must_use]
    pub fn with_resets_at(mut self, resets_at: impl Into<String>) -> Self {
        self.resets_at = Some(resets_at.into());
        self
    }

    /// Set `windowMinutes`.
    #[must_use]
    pub fn with_window_minutes(mut self, minutes: u32) -> Self {
        self.window_minutes = Some(minutes);
        self
    }

    /// Set `model` and mark the window as coming from detailed mode.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self.detailed = Some(true);
        self
    }
}

/// Freshness of a single window.
///
/// Tolerant on the way in: a value this build does not know becomes [`WindowState::Other`]
/// and is written back unchanged, so an older tray never corrupts a newer file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum WindowState {
    /// Read from a live source within the refresh interval.
    Ok,
    /// Last known value, older than the refresh interval.
    Stale,
    /// Could not be read. `percent` is absent.
    Error,
    /// A state a newer version introduced.
    Other(String),
}

impl From<String> for WindowState {
    fn from(value: String) -> Self {
        match value.as_str() {
            "ok" => WindowState::Ok,
            "stale" => WindowState::Stale,
            "error" => WindowState::Error,
            _ => WindowState::Other(value),
        }
    }
}

impl From<WindowState> for String {
    fn from(value: WindowState) -> Self {
        match value {
            WindowState::Ok => "ok".to_owned(),
            WindowState::Stale => "stale".to_owned(),
            WindowState::Error => "error".to_owned(),
            WindowState::Other(other) => other,
        }
    }
}

/// Where a provider's numbers came from. Tolerant in the same way as [`WindowState`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Source {
    /// Claude Code's status-line payload, captured by the `nazar-statusline` wrapper.
    Statusline,
    /// Claude Code's official usage endpoint, read only in opt-in detailed mode.
    Endpoint,
    /// A Codex `rollout-*.jsonl` session log.
    Rollout,
    /// A source a newer version introduced.
    Other(String),
}

impl From<String> for Source {
    fn from(value: String) -> Self {
        match value.as_str() {
            "statusline" => Source::Statusline,
            "endpoint" => Source::Endpoint,
            "rollout" => Source::Rollout,
            _ => Source::Other(value),
        }
    }
}

impl From<Source> for String {
    fn from(value: Source) -> Self {
        match value {
            Source::Statusline => "statusline".to_owned(),
            Source::Endpoint => "endpoint".to_owned(),
            Source::Rollout => "rollout".to_owned(),
            Source::Other(other) => other,
        }
    }
}

/// Write whole percentages as JSON integers so the file stays readable by hand.
///
/// `12.0_f64` would otherwise serialise as `12.0`, which parses to the same number but
/// makes the documented sample and the produced file differ byte for byte.
fn serialize_percent<S>(value: &Option<f64>, serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        None => serializer.serialize_none(),
        Some(percent) if percent.is_finite() && percent.fract() == 0.0 && percent.abs() < 1e9 => {
            serializer.serialize_i64(*percent as i64)
        }
        Some(percent) => serializer.serialize_f64(*percent),
    }
}

/// Read and parse `limits.json`.
pub fn read_limits(path: &Path) -> Result<Limits> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::io(path, source))?;
    serde_json::from_str(&text).map_err(|source| Error::json(path, source))
}

/// Write `limits.json` atomically: temporary file in the same directory, then rename.
///
/// A reader either sees the previous document or the new one. It never sees a prefix of
/// either, and an interrupted write leaves no temporary file behind.
pub fn write_limits(path: &Path, limits: &Limits) -> Result<()> {
    let text = limits.to_json()?;
    atomic::write_bytes(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    /// The sample from `docs/PROJECT.md` section 6, with the 2026-09-07 04:40 additions
    /// (per-window `state`, provider `source`, `windowMinutes` on Claude too).
    const SAMPLE: &str = include_str!("../../../fixtures/limits.sample.json");

    fn sample() -> Limits {
        Limits::from_json(SAMPLE).expect("the shipped sample must parse")
    }

    #[test]
    fn sample_round_trips_byte_for_byte() {
        assert_eq!(sample().to_json().unwrap(), SAMPLE);
    }

    #[test]
    fn sample_matches_the_frozen_contract() {
        let limits = sample();
        assert_eq!(limits.schema_version, SCHEMA_VERSION);

        let claude = &limits.providers.claude;
        assert!(claude.configured);
        assert_eq!(claude.plan.as_deref(), Some("max_20x"));
        assert_eq!(claude.source, Some(Source::Statusline));
        assert_eq!(claude.binding.as_deref(), Some("seven_day_fable"));
        assert_eq!(claude.windows.len(), 3);

        let detailed = &claude.windows["seven_day_fable"];
        assert_eq!(detailed.percent, Some(23.0));
        assert_eq!(detailed.window_minutes, Some(10080));
        assert_eq!(detailed.model.as_deref(), Some("Fable"));
        assert_eq!(detailed.detailed, Some(true));
        assert_eq!(detailed.state, WindowState::Ok);

        let codex = &limits.providers.codex;
        assert_eq!(codex.source, Some(Source::Rollout));
        assert_eq!(codex.binding.as_deref(), Some("secondary"));
        assert_eq!(codex.windows["primary"].window_minutes, Some(300));
        assert_eq!(codex.windows["secondary"].window_minutes, Some(10080));

        // The binding window is the highest percentage of the provider's windows.
        for provider in [claude, codex] {
            let binding = provider.binding.as_deref().unwrap();
            let highest = provider
                .windows
                .iter()
                .filter_map(|(key, window)| window.percent.map(|percent| (percent, key)))
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .unwrap()
                .1;
            assert_eq!(binding, highest);
        }
    }

    #[test]
    fn round_trips_through_a_file() {
        let dir = TempDir::new("round-trip");
        let path = dir.join("limits.json");
        let limits = sample();

        write_limits(&path, &limits).unwrap();
        assert_eq!(read_limits(&path).unwrap(), limits);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SAMPLE);
    }

    #[test]
    fn an_unreadable_window_has_no_percent() {
        let window = Window::error("rollout log unreadable");
        assert_eq!(window.percent, None);
        assert_eq!(window.state, WindowState::Error);

        let json = serde_json::to_string(&window).unwrap();
        assert!(
            !json.contains("percent"),
            "an unknown window must not carry a percentage at all, got {json}"
        );
    }

    #[test]
    fn whole_percentages_serialise_as_integers() {
        let whole = serde_json::to_string(&Window::ok(12.0)).unwrap();
        assert!(whole.contains("\"percent\":12,"), "got {whole}");

        let fractional = serde_json::to_string(&Window::ok(12.5)).unwrap();
        assert!(fractional.contains("\"percent\":12.5,"), "got {fractional}");
    }

    #[test]
    fn unknown_fields_are_preserved() {
        let newer = r#"{
  "schemaVersion": 1,
  "updatedAt": "2026-09-07T00:12:34+03:00",
  "providers": {
    "claude": { "configured": false, "quotaPool": "shared" },
    "codex": { "configured": false },
    "gemini": { "configured": true }
  },
  "writerBuild": "0.9.0"
}"#;

        let limits = Limits::from_json(newer).unwrap();
        assert_eq!(limits.extra["writerBuild"], Value::from("0.9.0"));
        assert_eq!(
            limits.providers.claude.extra["quotaPool"],
            Value::from("shared")
        );
        assert!(limits.providers.extra.contains_key("gemini"));

        let written = Limits::from_json(&limits.to_json().unwrap()).unwrap();
        assert_eq!(
            written, limits,
            "a rewrite must not drop what it did not understand"
        );
    }

    #[test]
    fn missing_optional_fields_are_fine() {
        let minimal = r#"{
  "schemaVersion": 1,
  "updatedAt": "2026-09-07T00:12:34+03:00",
  "providers": {
    "claude": { "configured": false },
    "codex": {
      "configured": true,
      "windows": { "primary": { "percent": 54, "state": "ok" } }
    }
  }
}"#;

        let limits = Limits::from_json(minimal).unwrap();
        assert!(!limits.providers.claude.configured);
        assert!(limits.providers.claude.windows.is_empty());
        assert_eq!(limits.providers.codex.plan, None);
        assert_eq!(limits.providers.codex.windows["primary"].resets_at, None);
    }

    #[test]
    fn an_absent_providers_entry_defaults_to_unconfigured() {
        let one_sided = r#"{
  "schemaVersion": 1,
  "updatedAt": "2026-09-07T00:12:34+03:00",
  "providers": { "codex": { "configured": true } }
}"#;

        let limits = Limits::from_json(one_sided).unwrap();
        assert!(!limits.providers.claude.configured);
        assert!(limits.providers.codex.configured);
    }

    #[test]
    fn unknown_enum_values_survive_a_rewrite() {
        let future = r#"{
  "schemaVersion": 1,
  "updatedAt": "2026-09-07T00:12:34+03:00",
  "providers": {
    "claude": {
      "configured": true,
      "source": "telemetry",
      "windows": { "five_hour": { "percent": 3, "state": "degraded" } }
    },
    "codex": { "configured": false }
  }
}"#;

        let limits = Limits::from_json(future).unwrap();
        assert_eq!(
            limits.providers.claude.source,
            Some(Source::Other("telemetry".to_owned()))
        );
        assert_eq!(
            limits.providers.claude.windows["five_hour"].state,
            WindowState::Other("degraded".to_owned())
        );
        assert!(limits.to_json().unwrap().contains("\"degraded\""));
    }

    #[test]
    fn a_window_without_state_is_rejected_rather_than_guessed() {
        let stateless = r#"{
  "schemaVersion": 1,
  "updatedAt": "2026-09-07T00:12:34+03:00",
  "providers": {
    "claude": { "configured": true, "windows": { "five_hour": { "percent": 12 } } },
    "codex": { "configured": false }
  }
}"#;

        let error = Limits::from_json(stateless).unwrap_err();
        assert!(
            error.to_string().contains("state"),
            "the error should name the missing field, got {error}"
        );
    }

    #[test]
    fn errors_do_not_leak_file_contents() {
        let dir = TempDir::new("no-leak");
        let path = dir.join("limits.json");
        std::fs::write(&path, "{ \"secret-looking-token\": ").unwrap();

        let message = read_limits(&path).unwrap_err().to_string();
        assert!(message.contains("limits.json"), "got {message}");
        assert!(!message.contains("secret-looking-token"), "got {message}");
    }
}
