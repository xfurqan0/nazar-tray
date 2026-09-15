//! The two questions only the operating system can answer.
//!
//! `nazar-core` has no dependencies beyond serde and deliberately never asks what time it is
//! *locally* or what language the user reads: every timestamp it writes is UTC, and every
//! word it shows comes from a locale file. Both of those decisions are right, and both of
//! them leave exactly one gap that WP5 has to close:
//!
//! | Question | Why it cannot be answered anywhere else |
//! |---|---|
//! | What is the time on the user's own clock? | Quiet hours are wall-clock hours. "Do not interrupt me between eleven and seven" means eleven where the user is, and the standard library has no time-zone database. |
//! | What language does this machine speak? | WP4 shipped a tray that fell back to English and a panel that guessed from `navigator.languages`, so the tooltip and the panel could disagree. One answer, read once, given to both. |
//!
//! **Why this is FFI rather than a crate.** Two calls into `kernel32` and two into libc,
//! both already linked into every binary on their platform, against ABIs that have not moved
//! in decades. The alternatives were a time-zone crate (`chrono` and `time` both bring a
//! tree, and `time::UtcOffset::current_local_offset` refuses to answer in a multi-threaded
//! process, which this is) or a locale crate for one string. WP5 was allowed two new
//! dependencies and both are Tauri plugins; spending one of them on `GetLocalTime` would
//! have been a poor trade.
//!
//! The Windows half is hand-written and the POSIX half is not, and the line between them is
//! whether there is a layout to get wrong. `SYSTEMTIME` is sixteen bytes of `WORD` and
//! `tzset` takes nothing, so both are written out below. `struct tm` ends in two fields that
//! are a glibc and BSD extension, and a declaration missing them is `localtime_r` writing
//! past the end of a stack local rather than a compile error — so that one comes from `libc`,
//! which is in `Cargo.lock` and in the notices either way and adds nothing to the build.
//!
//! **What each platform can answer**, and every caller is written for the gaps: an unknown
//! local time suppresses no notification, and an unknown UI language falls back to English.
//!
//! | | Local clock | UI language |
//! |---|---|---|
//! | Windows | `GetLocalTime` | `GetUserDefaultLocaleName` |
//! | Linux | `localtime_r` | `LC_ALL` / `LC_MESSAGES` / `LANG` |
//! | macOS | `localtime_r` — the POSIX branch is `cfg(unix)` | the same variables, which a GUI session there usually does not set, so `None` and English until a `CFLocale` branch is written |
//! | anywhere else | `None` | `None` |
//!
//! T-WP-L1 added the POSIX branch, and the gap it closed was not cosmetic: before it, quiet
//! hours did not exist on Linux at all — `None` means "not quiet", so a 60 % crossing
//! interrupted at three in the morning — and a Turkish desktop got an English tooltip beside
//! a Turkish panel, which is the exact disagreement WP5 was written to end.

/// Minutes since local midnight, `0..=1439`, or `None` where nobody can say.
///
/// Only ever compared against [`nazar_core::config::QuietHours`], which is why minutes are
/// enough and a date is not: quiet hours repeat every day.
#[must_use]
pub fn local_minutes() -> Option<u16> {
    platform::local_minutes()
}

/// The operating system's UI language as a tag such as `tr-TR`, or `None`.
///
/// The **display** language, not the formatting locale: a machine set to Turkish with
/// British date formats reads Turkish, and the panel is words rather than dates.
#[must_use]
pub fn ui_language() -> Option<String> {
    platform::ui_language()
}

#[cfg(windows)]
mod platform {
    /// `SYSTEMTIME`, as `minwinbase.h` declares it. Sixteen bytes of `WORD`.
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }

    /// Longest locale name Windows will produce, including the terminator
    /// (`LOCALE_NAME_MAX_LENGTH`).
    const LOCALE_NAME_MAX_LENGTH: usize = 85;

    unsafe extern "system" {
        /// Fills `SYSTEMTIME` with the current **local** date and time. Cannot fail.
        fn GetLocalTime(system_time: *mut SystemTime);
        /// Writes the user's UI language into a UTF-16 buffer; returns the length written,
        /// including the terminator, or `0` on failure.
        fn GetUserDefaultLocaleName(name: *mut u16, length: i32) -> i32;
    }

    pub(super) fn local_minutes() -> Option<u16> {
        let mut now = SystemTime::default();
        // SAFETY: `GetLocalTime` writes exactly one `SYSTEMTIME` through the pointer and
        // returns nothing. The value is a live local, correctly aligned and the right size.
        unsafe { GetLocalTime(&raw mut now) };
        if now.hour > 23 || now.minute > 59 {
            return None;
        }
        Some(now.hour * 60 + now.minute)
    }

    pub(super) fn ui_language() -> Option<String> {
        let mut buffer = [0u16; LOCALE_NAME_MAX_LENGTH];
        // SAFETY: the pointer is to a buffer of exactly the length being passed, and the
        // function writes at most that many UTF-16 code units into it.
        let written =
            unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), LOCALE_NAME_MAX_LENGTH as i32) };
        if written <= 1 {
            return None;
        }
        // The count includes the terminating null, which is not part of the tag.
        let tag = String::from_utf16_lossy(&buffer[..(written as usize) - 1]);
        (!tag.is_empty()).then_some(tag)
    }
}

#[cfg(unix)]
mod platform {
    use std::sync::Once;
    use std::time::{SystemTime, UNIX_EPOCH};

    unsafe extern "C" {
        /// POSIX `tzset`: read `TZ`, or the system zone when it is unset, into the process
        /// globals `localtime_r` reads.
        ///
        /// Declared here rather than taken from `libc`, which exposes it on Windows only.
        /// It takes nothing and returns nothing, so there is no layout to get wrong -- the
        /// one thing hand-written FFI is unambiguously safe for, and the reason `struct tm`
        /// next door is not hand-written.
        fn tzset();
    }

    pub(super) fn local_minutes() -> Option<u16> {
        // `localtime_r`, unlike `localtime`, is not required to read the TZ environment
        // variable, and glibc's does not: without this the answer is UTC on a machine whose
        // clock says something else. Once is enough — `tzset` writes process-wide globals,
        // and every later call would only write the same ones again from another thread.
        static TIME_ZONE: Once = Once::new();
        // SAFETY: `tzset` takes nothing, returns nothing, and is called exactly once here.
        TIME_ZONE.call_once(|| unsafe { tzset() });

        let epoch_seconds = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        // Fails only where `time_t` is 32 bits and the year is past 2038, which is an
        // honest `None` rather than a wrong hour.
        let clock = libc::time_t::try_from(epoch_seconds).ok()?;

        // SAFETY: `tm` is a live local of exactly the type the function fills, and `clock` a
        // live local of exactly the type it reads. `localtime_r` writes the whole structure
        // or returns null, and it is the reentrant form: no static buffer is shared with
        // another thread.
        let mut broken_down: libc::tm = unsafe { std::mem::zeroed() };
        let filled = unsafe { libc::localtime_r(&raw const clock, &raw mut broken_down) };
        if filled.is_null() {
            return None;
        }

        // `tm_hour` is 0..=23 and `tm_min` 0..=59 per POSIX. Checked rather than trusted,
        // because the alternative is quiet hours comparing against a nonsense number.
        if !(0..=23).contains(&broken_down.tm_hour) || !(0..=59).contains(&broken_down.tm_min) {
            return None;
        }
        u16::try_from(broken_down.tm_hour * 60 + broken_down.tm_min).ok()
    }

    pub(super) fn ui_language() -> Option<String> {
        language_from(|name| std::env::var(name).ok())
    }

    /// The POSIX answer to "what language is this session in", from a lookup of the
    /// environment.
    ///
    /// Taking the lookup as an argument is what makes the precedence testable: setting real
    /// environment variables is a process-wide write, unsafe in the 2024 edition, and a race
    /// against every other test running beside it.
    ///
    /// **The first variable that is set and non-empty wins, and nothing after it is read.**
    /// That is the POSIX rule rather than a search for the first variable that happens to
    /// parse: `LC_ALL=C` means this session has no language, and falling through to a `LANG`
    /// left over from somewhere else would be a translated tray on a machine that asked for
    /// an untranslated one. An *empty* value is what POSIX calls unset, so it is skipped.
    fn language_from(lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .find_map(|name| lookup(name).filter(|value| !value.is_empty()))
            .and_then(|locale| language_tag(&locale))
    }

    /// A POSIX locale name as a BCP 47 language tag: `tr_TR.UTF-8` becomes `tr-TR`.
    ///
    /// The encoding after the dot and the modifier after the at-sign say how to *render*
    /// text rather than which language it is in, and neither belongs in a tag the catalogue
    /// lookup reads. `C` and `POSIX` are the absence of a language, which is `None` here and
    /// English one call later.
    fn language_tag(locale: &str) -> Option<String> {
        let name = locale
            .split(['.', '@'])
            .next()
            .unwrap_or_default()
            .replace('_', "-");
        if name.is_empty() || name.eq_ignore_ascii_case("C") || name.eq_ignore_ascii_case("POSIX") {
            return None;
        }
        // A tag is letters, digits and hyphens. Anything else is a locale name this does not
        // understand, and a tag nobody has a catalogue for would only be taken apart and
        // thrown away by `i18n::primary_subtag`.
        name.chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
            .then_some(name)
    }

    #[cfg(test)]
    mod tests {
        use super::{language_from, language_tag};
        use std::collections::BTreeMap;

        /// A lookup over a fixed set of variables, standing in for the real environment.
        fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
            let map: BTreeMap<String, String> = pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect();
            move |name| map.get(name).cloned()
        }

        #[test]
        fn a_posix_locale_becomes_a_language_tag() {
            assert_eq!(language_tag("tr_TR.UTF-8").as_deref(), Some("tr-TR"));
            assert_eq!(language_tag("en_GB.iso88591").as_deref(), Some("en-GB"));
            assert_eq!(language_tag("sr_RS@latin").as_deref(), Some("sr-RS"));
            assert_eq!(language_tag("zh_CN.UTF-8@pinyin").as_deref(), Some("zh-CN"));
            assert_eq!(language_tag("tr").as_deref(), Some("tr"));
        }

        /// The C locale is not a language, and neither is a name this cannot read.
        #[test]
        fn the_absence_of_a_language_is_not_a_tag() {
            for locale in ["C", "c", "POSIX", "C.UTF-8", "", ".UTF-8", "@euro"] {
                assert_eq!(language_tag(locale), None, "{locale:?}");
            }
            assert_eq!(language_tag("tr_TR:en_US"), None);
        }

        #[test]
        fn lc_all_wins_then_lc_messages_then_lang() {
            let all = environment(&[
                ("LC_ALL", "ru_RU.UTF-8"),
                ("LC_MESSAGES", "ko_KR.UTF-8"),
                ("LANG", "tr_TR.UTF-8"),
            ]);
            assert_eq!(language_from(all).as_deref(), Some("ru-RU"));

            let messages = environment(&[("LC_MESSAGES", "ko_KR.UTF-8"), ("LANG", "tr_TR.UTF-8")]);
            assert_eq!(language_from(messages).as_deref(), Some("ko-KR"));

            let lang = environment(&[("LANG", "tr_TR.UTF-8")]);
            assert_eq!(language_from(lang).as_deref(), Some("tr-TR"));

            assert_eq!(language_from(environment(&[])), None);
        }

        /// An empty value is what POSIX calls unset, and the next variable is read.
        #[test]
        fn an_empty_variable_is_an_unset_one() {
            let empty =
                environment(&[("LC_ALL", ""), ("LC_MESSAGES", ""), ("LANG", "es_ES.UTF-8")]);
            assert_eq!(language_from(empty).as_deref(), Some("es-ES"));
        }

        /// The one that is not a search for the first variable that parses: a session that
        /// asked for no language does not get one from a leftover `LANG`.
        #[test]
        fn a_c_locale_stops_the_search_rather_than_falling_through() {
            let c_locale = environment(&[("LC_ALL", "C"), ("LANG", "tr_TR.UTF-8")]);
            assert_eq!(language_from(c_locale), None);
        }
    }
}

#[cfg(not(any(windows, unix)))]
mod platform {
    pub(super) fn local_minutes() -> Option<u16> {
        None
    }

    pub(super) fn ui_language() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `None` is a valid answer everywhere but Windows, and every caller is written for it:
    /// an unknown local time suppresses no notification, and an unknown language is English.
    /// What those answers *mean* is tested in `nazar_core::config`.
    #[test]
    fn the_local_clock_is_a_time_of_day_or_nothing_at_all() {
        if let Some(minutes) = local_minutes() {
            assert!(minutes < 24 * 60, "got {minutes}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_posix_machine_always_knows_the_time_too() {
        assert!(
            local_minutes().is_some(),
            "quiet hours are wall-clock hours, and this is the only thing that can read one"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_always_knows_the_time_and_the_language() {
        assert!(
            local_minutes().is_some(),
            "quiet hours are wall-clock hours, and this is the only thing that can read one"
        );
        assert!(ui_language().is_some());
    }

    #[test]
    fn the_ui_language_is_a_tag_or_nothing_at_all() {
        let Some(tag) = ui_language() else {
            return;
        };
        assert!(!tag.is_empty());
        assert!(
            !tag.contains('\0'),
            "the terminator is not part of the tag: {tag:?}"
        );
        assert!(
            tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "a BCP 47 tag is letters, digits and hyphens; got {tag:?}"
        );
        // Whatever this machine is set to, the catalogue lookup has to survive it.
        let _ = crate::i18n::catalog(&tag);
    }

    #[test]
    fn two_readings_of_the_local_clock_agree_within_a_minute() {
        let (first, second) = (local_minutes(), local_minutes());
        if let (Some(first), Some(second)) = (first, second) {
            let apart = (i32::from(second) - i32::from(first)).rem_euclid(24 * 60);
            assert!(apart <= 1, "{first} and {second} are {apart} minutes apart");
        }
    }
}
