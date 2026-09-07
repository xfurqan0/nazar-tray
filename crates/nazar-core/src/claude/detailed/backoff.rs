//! When to ask the usage endpoint again after it said no.
//!
//! The endpoint is undocumented and rate-limited, and the retired prototype's logs are the
//! argument for this file: a 401 that lasted three hours produced seven identical requests
//! (finding B06), and two schedulers racing each other produced the 429 that started all
//! of this (finding B01). So every failure pushes the next attempt further away —
//! `1 s, 2 s, 4 s, …` up to half an hour — and a success puts it back to nothing.
//!
//! Two things override the doubling:
//!
//! * **`Retry-After`.** When the server says how long, the server wins, up to the same
//!   half-hour cap. Reading that header is finding B07; the prototype threw the response
//!   headers away and guessed twenty minutes.
//! * **A new sign-in.** When Claude Code rewrites the credential file, the wait is
//!   cleared: the most likely reason it changed is that the token that was being refused
//!   has just been refreshed, and making the user wait out a backoff for a problem that is
//!   already fixed is how a quota tray earns a bug report.
//!
//! The state lives in memory and nowhere else. A wait that survived a restart would need a
//! file, the file would need to say which endpoint and which account it was about, and
//! none of that is worth the paperwork for a value that is at most thirty minutes old.

use std::time::Duration;

/// The clock this module measures its waits against.
///
/// It moved to [`crate::clock`] in WP3, when the refresh loop turned out to need the same
/// two readings and to exist whether or not this feature is compiled in. Re-exported here
/// so that `detailed::Clock` keeps meaning what it always meant.
pub use crate::clock::{Clock, SystemClock};

/// First wait after a failure.
pub const BASE_DELAY: Duration = Duration::from_secs(1);

/// Longest wait, however many failures there have been.
pub const MAX_DELAY: Duration = Duration::from_secs(30 * 60);

/// How long to wait, and how much of the wait is left.
#[derive(Debug, Clone, Default)]
pub struct Backoff {
    failures: u32,
    /// Monotonic milliseconds before which nothing should be attempted.
    blocked_until: Option<u64>,
}

impl Backoff {
    /// A backoff that is not waiting for anything.
    #[must_use]
    pub fn new() -> Self {
        Backoff::default()
    }

    /// How many failures in a row have been recorded.
    #[must_use]
    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// How much of the current wait is left at `now_millis`, if any.
    #[must_use]
    pub fn remaining(&self, now_millis: u64) -> Option<Duration> {
        let until = self.blocked_until?;
        until
            .checked_sub(now_millis)
            .filter(|left| *left > 0)
            .map(Duration::from_millis)
    }

    /// Whether a request may be sent at `now_millis`.
    #[must_use]
    pub fn allows(&self, now_millis: u64) -> bool {
        self.remaining(now_millis).is_none()
    }

    /// Record a success: the next attempt is free.
    pub fn succeeded(&mut self) {
        self.failures = 0;
        self.blocked_until = None;
    }

    /// Record a failure and start the wait.
    ///
    /// `retry_after` is the server's own answer when it gave one; it wins over the
    /// doubling in both directions, so a server asking for two seconds is not made to wait
    /// eight, and one asking for an hour is still capped at [`MAX_DELAY`].
    pub fn failed(&mut self, now_millis: u64, retry_after: Option<Duration>) {
        self.failures = self.failures.saturating_add(1);
        let delay = match retry_after {
            Some(asked) => asked.min(MAX_DELAY),
            None => self.doubling_delay(),
        };
        self.blocked_until = Some(now_millis.saturating_add(delay.as_millis() as u64));
    }

    /// Clear the wait without pretending anything succeeded.
    ///
    /// Used when the credential file changes: the reason for the last failure may have
    /// gone away, so the next refresh is allowed to find out.
    pub fn cleared(&mut self) {
        self.failures = 0;
        self.blocked_until = None;
    }

    /// `1 s, 2 s, 4 s, 8 s …` capped at [`MAX_DELAY`].
    fn doubling_delay(&self) -> Duration {
        // `failures` has already been incremented, so the first failure gets `1 << 0`.
        let shift = self.failures.saturating_sub(1).min(31);
        let seconds = BASE_DELAY.as_secs().saturating_mul(1u64 << shift);
        Duration::from_secs(seconds).min(MAX_DELAY)
    }
}
