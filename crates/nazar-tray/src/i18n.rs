//! The few strings the Rust side shows, taken from the same files the panel reads.
//!
//! The tray tooltip and the context menu are the only text this product draws outside the
//! webview, and the rule in `docs/PROJECT.md` — *every UI string comes from locale files;
//! no hard-coded text in code* — has no exception for them. So the same
//! `ui/locales/<lang>.json` catalogues are compiled in with `include_str!` and looked up
//! with the same `{placeholder}` substitution `ui/src/i18n.ts` does, in about forty lines.
//!
//! Two things this deliberately does not do, and where they land:
//!
//! * **Detecting the language.** WP5 reads the Windows UI language and adds the override in
//!   settings; until then the locale comes from `config.json`'s `locale` key, and the
//!   default is English. A guess here would be a second, different guess from the panel's.
//! * **Carrying ZH, KO, RU and ES.** Those catalogues are still empty (WP6). An empty
//!   catalogue falls through to English rather than showing a key.

use std::collections::BTreeMap;

/// English, the fallback for every missing string.
const EN: &str = include_str!("../../../ui/locales/en.json");
/// Turkish, written by hand alongside English.
const TR: &str = include_str!("../../../ui/locales/tr.json");

/// A catalogue bound to one language, with English behind it.
pub struct Catalog {
    messages: BTreeMap<String, String>,
    fallback: BTreeMap<String, String>,
}

/// Parse one catalogue. A file that does not parse is an empty catalogue, never a panic:
/// a damaged translation must not stop the tray from starting.
fn parse(text: &str) -> BTreeMap<String, String> {
    serde_json::from_str(text).unwrap_or_default()
}

/// The catalogue for a language tag such as `tr`, `tr-TR` or `en-GB`.
#[must_use]
pub fn catalog(locale: &str) -> Catalog {
    let primary = locale
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let messages = match primary.as_str() {
        "tr" => parse(TR),
        _ => BTreeMap::new(),
    };
    Catalog {
        messages,
        fallback: parse(EN),
    }
}

impl Catalog {
    /// The message for a key, with the placeholders filled in.
    ///
    /// Lookup order is the chosen language, then English, then the key itself — an
    /// untranslated string shows up as `tray.menu.quit` in the menu instead of as a gap,
    /// which is how it gets noticed.
    #[must_use]
    pub fn format(&self, key: &str, params: &[(&str, &str)]) -> String {
        let template = self
            .messages
            .get(key)
            .or_else(|| self.fallback.get(key))
            .map_or(key, String::as_str);
        interpolate(template, params)
    }

    /// The message for a key that has no placeholders.
    #[must_use]
    pub fn text(&self, key: &str) -> String {
        self.format(key, &[])
    }

    /// Every key English defines. Used by the test that proves TR keeps up with EN.
    #[cfg(test)]
    #[must_use]
    pub fn keys(&self) -> Vec<&str> {
        self.fallback.keys().map(String::as_str).collect()
    }

    /// Whether the chosen language defines a key of its own.
    ///
    /// Only the parity test asks: the running tray does not care which language a string
    /// came from, only that there is one.
    #[cfg(test)]
    #[must_use]
    pub fn translates(&self, key: &str) -> bool {
        self.messages.contains_key(key)
    }
}

/// Replace `{name}` with the value given for `name`.
///
/// A placeholder with no value is left as written rather than blanked, so a missing value
/// reads as `{percent}` instead of disappearing — the same rule as `ui/src/i18n.ts`.
#[must_use]
fn interpolate(template: &str, params: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}').map(|at| start + at) else {
            break;
        };
        let name = &rest[start + 1..end];
        out.push_str(&rest[..start]);
        match params.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => out.push_str(value),
            None => out.push_str(&rest[start..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// A duration as words: `4 d 2 h`, `2 h 10 m`, `10 m`, `45 s`.
///
/// Shorter than the panel's clock on purpose — a tooltip is read at a glance and a
/// countdown to the second is noise in it. The units are message keys, because "h" is not
/// "sa" and neither is a number; `ui/src/format.ts` formats the panel's durations from the
/// same four keys, so the tooltip and the panel never disagree about how long is left.
#[must_use]
pub fn duration(catalog: &Catalog, milliseconds: i64) -> String {
    let total = milliseconds.max(0) / 1000;
    let days = total / 86_400;
    let hours = (total % 86_400) / 3600;
    let minutes = (total % 3600) / 60;
    if days > 0 {
        catalog.format(
            "time.daysHours",
            &[("days", &days.to_string()), ("hours", &hours.to_string())],
        )
    } else if total >= 3600 {
        catalog.format(
            "time.hoursMinutes",
            &[
                ("hours", &hours.to_string()),
                ("minutes", &minutes.to_string()),
            ],
        )
    } else if minutes > 0 {
        catalog.format("time.minutes", &[("minutes", &minutes.to_string())])
    } else {
        catalog.format("time.seconds", &[("seconds", &(total % 60).to_string())])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_placeholder_is_filled_and_an_unknown_one_is_left_alone() {
        assert_eq!(
            interpolate("{a} and {b}", &[("a", "1"), ("b", "2")]),
            "1 and 2"
        );
        assert_eq!(
            interpolate("{a} and {b}", &[("a", "1")]),
            "1 and {b}",
            "a missing value must be visible, not silently blank"
        );
        assert_eq!(interpolate("nothing to fill", &[]), "nothing to fill");
        assert_eq!(interpolate("{unclosed", &[]), "{unclosed");
        assert_eq!(interpolate("{}", &[]), "{}");
    }

    #[test]
    fn turkish_is_used_when_it_has_the_string_and_english_when_it_does_not() {
        let turkish = catalog("tr-TR");
        assert_eq!(turkish.text("panel.window.unknown"), "bilinmiyor");
        assert!(turkish.translates("tray.menu.quit"));

        let english = catalog("en-GB");
        assert_eq!(english.text("panel.window.unknown"), "unknown");
        assert!(!english.translates("panel.window.unknown"));

        // A language WP6 has not filled in yet falls through to English rather than
        // showing a half-translated panel.
        assert_eq!(catalog("ko").text("panel.window.unknown"), "unknown");
        assert_eq!(catalog("").text("tray.menu.open"), "Open");
    }

    #[test]
    fn a_key_nobody_wrote_shows_as_the_key() {
        assert_eq!(catalog("en").text("nothing.like.this"), "nothing.like.this");
    }

    #[test]
    fn every_english_key_is_translated_into_turkish() {
        let turkish = catalog("tr");
        let missing: Vec<&str> = turkish
            .keys()
            .into_iter()
            .filter(|key| !turkish.translates(key))
            .collect();
        assert!(
            missing.is_empty(),
            "EN and TR are both written by hand and must stay level: {missing:?}"
        );
    }

    #[test]
    fn a_duration_is_words_in_the_chosen_language() {
        let english = catalog("en");
        assert_eq!(duration(&english, 7_800_000), "2 h 10 m");
        assert_eq!(
            duration(&english, 4 * 86_400_000 + 2 * 3_600_000),
            "4 d 2 h",
            "a weekly window is days away, and 98:00:00 is not a number anybody reads"
        );
        assert_eq!(duration(&english, 600_000), "10 m");
        assert_eq!(duration(&english, 45_000), "45 s");
        assert_eq!(
            duration(&english, -5),
            "0 s",
            "a due reset is not negative time"
        );

        let turkish = catalog("tr");
        assert_eq!(turkish.text("tray.menu.quit"), "Çık");
        assert!(duration(&turkish, 7_800_000).contains("sa"));
    }
}
