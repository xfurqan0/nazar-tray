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
//! **Why this is hand-written FFI rather than a crate.** Two calls into `kernel32`, which is
//! already linked into every Windows binary, against a documented ABI that has not changed
//! since Windows 2000. The alternatives were a time-zone crate (`chrono` and `time` both
//! bring a tree, and `time::UtcOffset::current_local_offset` refuses to answer in a
//! multi-threaded process, which this is) or a locale crate for one string. The work package
//! is allowed two new dependencies and both are Tauri plugins; spending one of them on
//! `GetLocalTime` would have been a poor trade.
//!
//! **Everywhere that is not Windows answers `None`**, and every caller is written for that:
//! an unknown local time suppresses no notification, and an unknown UI language falls back to
//! English. v1 ships Windows only (`docs/PROJECT.md` section 3); when the macOS build lands,
//! this is the file it adds a branch to, and nothing else changes.

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

#[cfg(not(windows))]
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
