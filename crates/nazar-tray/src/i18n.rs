//! The few strings the Rust side shows, taken from the same files the panel reads.
//!
//! The tray tooltip and the context menu are the only text this product draws outside the
//! webview, and the rule in `docs/PROJECT.md` — *every UI string comes from locale files;
//! no hard-coded text in code* — has no exception for them. So the same
//! `ui/locales/<lang>.json` catalogues are compiled in with `include_str!` and looked up
//! with the same `{placeholder}` substitution `ui/src/i18n.ts` does, in about forty lines.
//!
//! **WP5 made this the only opinion about the language.** WP4 left two: the tray fell back
//! to English while the panel guessed from `navigator.languages`, so a Turkish machine could
//! get a Turkish panel under an English tooltip. Now [`resolve`] answers once — the settings
//! override, then the operating system's UI language ([`crate::system::ui_language`]), then
//! English — and the answer is handed to the panel as well as used here. [`Strings`] holds
//! it behind a lock so that changing the language in the settings rebuilds the tray menu and
//! rewrites the tooltip without a restart.
//!
//! All six catalogues are compiled in, including the four WP6 has not written yet. An empty
//! catalogue is **not offered** — [`available`] leaves it out of the settings list — and if
//! one were selected anyway it would fall through to English rather than show message keys.
//! That way the day a translation lands is the day it appears, with no code change here.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// English, the fallback for every missing string.
const EN: &str = include_str!("../../../ui/locales/en.json");
/// Turkish, written by hand alongside English.
const TR: &str = include_str!("../../../ui/locales/tr.json");
/// The four WP6 owns. Empty today; compiled in so that filling them is all it takes.
const ZH: &str = include_str!("../../../ui/locales/zh.json");
const KO: &str = include_str!("../../../ui/locales/ko.json");
const RU: &str = include_str!("../../../ui/locales/ru.json");
const ES: &str = include_str!("../../../ui/locales/es.json");

/// A catalogue bound to one language, with English behind it.
#[derive(Debug)]
pub struct Catalog {
    messages: BTreeMap<String, String>,
    fallback: BTreeMap<String, String>,
}

/// Parse one catalogue. A file that does not parse is an empty catalogue, never a panic:
/// a damaged translation must not stop the tray from starting.
fn parse(text: &str) -> BTreeMap<String, String> {
    serde_json::from_str(text).unwrap_or_default()
}

/// The primary subtag of a language tag: `tr-TR` and `TR_tr` both become `tr`.
#[must_use]
pub fn primary_subtag(locale: &str) -> String {
    locale
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The source text of a catalogue this build carries, or `None`.
fn source(primary: &str) -> Option<&'static str> {
    match primary {
        "en" => Some(EN),
        "tr" => Some(TR),
        "zh" => Some(ZH),
        "ko" => Some(KO),
        "ru" => Some(RU),
        "es" => Some(ES),
        _ => None,
    }
}

/// The catalogue for a language tag such as `tr`, `tr-TR` or `en-GB`.
#[must_use]
pub fn catalog(locale: &str) -> Catalog {
    let primary = primary_subtag(locale);
    let messages = match primary.as_str() {
        // English is the fallback; loading it twice would only double the memory.
        "en" => BTreeMap::new(),
        other => source(other).map(parse).unwrap_or_default(),
    };
    Catalog {
        messages,
        fallback: parse(EN),
    }
}

/// The languages this build can actually paint itself in, in the order the form lists them.
///
/// English first because it is the fallback, then whatever else has been translated. A
/// language whose catalogue is still empty is left out: offering it and then showing English
/// would be a menu entry that lies.
#[must_use]
pub fn available() -> Vec<&'static str> {
    let mut languages = vec!["en"];
    for locale in nazar_core::config::LOCALES {
        if locale != "en" && source(locale).is_some_and(|text| !parse(text).is_empty()) {
            languages.push(locale);
        }
    }
    languages
}

/// The language the application is in: the override, the machine, then English.
///
/// `chosen` is `config.locale`; `system` is what the operating system says
/// ([`crate::system::ui_language`]). A tag neither this build nor WP6 has a catalogue for
/// falls through rather than being selected, so a machine set to German gets an English
/// panel and an English tooltip instead of a panel full of message keys.
#[must_use]
pub fn resolve(chosen: Option<&str>, system: Option<&str>) -> String {
    let offered = available();
    for tag in chosen.into_iter().chain(system) {
        let primary = primary_subtag(tag);
        if offered.contains(&primary.as_str()) {
            return primary;
        }
    }
    "en".to_owned()
}

/// The language the whole application is in, changeable while it runs.
///
/// One value, read by the tray menu, the tooltip and the notifications, and replaced by the
/// settings form. Behind an `RwLock` because it is read on the refresh loop's thread and on
/// the event loop's, and written on neither of them regularly: the contended case is a user
/// changing their language, which happens about once.
#[derive(Debug)]
pub struct Strings {
    inner: RwLock<(String, Arc<Catalog>)>,
}

impl Strings {
    /// Bind to a language tag, resolved by [`resolve`] beforehand.
    #[must_use]
    pub fn new(locale: &str) -> Self {
        Strings {
            inner: RwLock::new((locale.to_owned(), Arc::new(catalog(locale)))),
        }
    }

    /// The catalogue as it is right now.
    #[must_use]
    pub fn catalog(&self) -> Arc<Catalog> {
        Arc::clone(
            &self
                .inner
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .1,
        )
    }

    /// The language tag in force.
    #[must_use]
    pub fn locale(&self) -> String {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0
            .clone()
    }

    /// Switch language. `true` when it actually changed, which is what tells the caller
    /// the tray menu has to be rebuilt.
    pub fn set(&self, locale: &str) -> bool {
        let mut held = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if held.0 == locale {
            return false;
        }
        *held = (locale.to_owned(), Arc::new(catalog(locale)));
        true
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
    fn the_language_is_the_override_then_the_machine_then_english() {
        assert_eq!(
            resolve(Some("tr"), Some("en-GB")),
            "tr",
            "the user's choice wins"
        );
        assert_eq!(resolve(None, Some("tr-TR")), "tr", "then the machine's");
        assert_eq!(resolve(None, None), "en", "and English is the floor");
        assert_eq!(
            resolve(None, Some("de-DE")),
            "en",
            "a language nobody has translated must not be selected: an English panel beats              a panel full of message keys"
        );
        assert_eq!(
            resolve(Some("ko"), Some("tr-TR")),
            "tr",
            "a chosen language whose catalogue is still empty falls through to the machine's"
        );
        assert_eq!(resolve(Some("TR_tr"), None), "tr", "tags are normalised");
    }

    #[test]
    fn only_the_languages_that_are_actually_written_are_offered() {
        let offered = available();
        assert_eq!(
            offered[0], "en",
            "English is the fallback, so it is listed first"
        );
        assert!(offered.contains(&"tr"));
        for locale in &offered {
            assert!(
                nazar_core::config::LOCALES.contains(locale),
                "{locale} is not a language the settings know about"
            );
            assert!(
                !catalog(locale).fallback.is_empty(),
                "{locale} was offered with nothing behind it"
            );
        }
        // WP6's four are compiled in and still empty, so they are not offered yet. The day
        // one of them is written, this assertion is what says so.
        assert_eq!(
            offered.len(),
            2,
            "EN and TR are written by hand; ZH, KO, RU and ES are WP6's, got {offered:?}"
        );
    }

    #[test]
    fn the_language_can_be_changed_while_the_application_runs() {
        let strings = Strings::new("en");
        assert_eq!(strings.locale(), "en");
        assert_eq!(strings.catalog().text("tray.menu.quit"), "Quit");

        assert!(strings.set("tr"), "a real change has to be announced");
        assert_eq!(strings.catalog().text("tray.menu.quit"), "Çık");
        assert!(
            !strings.set("tr"),
            "setting the same language again must not rebuild the tray menu"
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
