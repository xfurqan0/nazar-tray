//! The one clock abstraction, and the only thing in this crate that asks the operating
//! system what time it is.
//!
//! Two readings, because the two questions are different:
//!
//! * `monotonic_millis` answers *how long since*. It only moves forward, so a scheduler
//!   built on it cannot be tricked into firing a hundred times by a clock correction.
//! * `now_rfc3339` answers *what time is it*. That is what goes in a file and what a
//!   stored expiry is compared against.
//!
//! The refresh loop reads both, on purpose: the difference between them is how a machine
//! notices it has been asleep. See [`crate::refresh`].
//!
//! It lives at the crate root rather than beside the backoff that first needed it because
//! the refresh loop needs it too, and the refresh loop exists whether or not the opt-in
//! detailed-windows feature is compiled in.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// A clock the tests can move.
pub trait Clock: Send + Sync {
    /// Milliseconds since an arbitrary origin that only moves forward.
    fn monotonic_millis(&self) -> u64;

    /// The current instant as RFC 3339 in UTC.
    fn now_rfc3339(&self) -> String;
}

/// The real clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn monotonic_millis(&self) -> u64 {
        // `Instant` has no epoch, so one is made at first use. `Instant::elapsed` is
        // monotonic on every platform this runs on, which is the property that matters.
        use std::sync::OnceLock;
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN.get_or_init(Instant::now).elapsed().as_millis() as u64
    }

    fn now_rfc3339(&self) -> String {
        crate::timefmt::now_rfc3339()
    }
}

/// The wall clock as Unix seconds, for code that needs to subtract two instants.
///
/// Derived from [`Clock::now_rfc3339`] rather than added to the trait: a fake clock in a
/// test then has one thing to implement instead of two that can disagree with each other.
/// Returns `None` on a clock whose reading is not a timestamp, which is a state a machine
/// can genuinely be in and not a reason to panic.
#[must_use]
pub fn wall_seconds(clock: &dyn Clock) -> Option<i64> {
    crate::timefmt::unix_seconds_from_rfc3339(&clock.now_rfc3339())
}

/// The process's own start, as close as the standard library can name it.
///
/// There is no portable way to ask the operating system when a process started, so this is
/// the first time anything in the process asked — which for the tray is a few milliseconds
/// into `main`. It is written into the advisory lock file as a diagnostic, never as a
/// liveness proof; [`crate::lock`] explains what actually proves a holder is alive.
#[must_use]
pub fn process_start_rfc3339() -> String {
    use std::sync::OnceLock;
    static START: OnceLock<String> = OnceLock::new();
    START.get_or_init(crate::timefmt::now_rfc3339).clone()
}

/// Seconds since the epoch, for the one caller that has no [`Clock`] to hand.
#[must_use]
pub(crate) fn system_time_seconds(at: SystemTime) -> i64 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clock_moves_forward_and_names_an_instant() {
        let clock = SystemClock;
        let first = clock.monotonic_millis();
        let second = clock.monotonic_millis();
        assert!(second >= first, "{second} came before {first}");

        let now = clock.now_rfc3339();
        assert!(now.ends_with('Z'), "got {now}");
        assert!(wall_seconds(&clock).is_some());
    }

    #[test]
    fn the_process_start_is_stable_across_calls() {
        assert_eq!(process_start_rfc3339(), process_start_rfc3339());
    }
}
