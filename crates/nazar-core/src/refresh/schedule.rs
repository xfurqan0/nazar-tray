//! When the refresh loop should do something, and how long it may sleep.
//!
//! Pure timing and no input or output at all, so the whole of it can be driven by a test
//! with a clock it moves by hand rather than by a test that waits.
//!
//! Five things make a refresh happen:
//!
//! | Trigger | Why |
//! |---|---|
//! | [`Cause::Startup`] | the first tick, so the tray has numbers before the first minute |
//! | [`Cause::Tick`] | every sixty seconds, the backstop that needs nothing to work |
//! | [`Cause::FileChange`] | a capture or a rollout log moved, seen by a five-second scan |
//! | [`Cause::Requested`] | somebody asked: the panel's refresh, or a second launch |
//! | [`Cause::Woke`] | the clock jumped by more than two ticks: the machine was asleep |
//!
//! Everything except the tick and the startup goes through a **250 ms debounce**, which is
//! what makes a burst one refresh instead of twenty. Codex writes a quota line dozens of
//! times per session and a status line rewrites its capture every few seconds; without
//! coalescing, a busy session would have the tray reading and writing continuously.
//!
//! The sleep-jump rule is finding S7 of the audit: the retired prototype's logs show gaps
//! of 480, 575 and 867 minutes, and everything that went wrong afterwards went wrong
//! because nothing had noticed the gap. Here the loop compares both of its clocks and
//! refreshes at once when either of them has moved by more than two ticks — in **either**
//! direction, because a clock corrected backwards is the same surprise.

use std::time::Duration;

/// How often a refresh happens when nothing else asks for one.
pub const TICK: Duration = Duration::from_secs(60);

/// How often the watched paths are looked at.
pub const WATCH_INTERVAL: Duration = Duration::from_secs(5);

/// How long a burst of triggers is collected before it becomes one refresh.
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// How much the clock has to move for the loop to decide the machine was asleep.
///
/// Two ticks. One missed tick is a busy machine; two is something else.
pub const SLEEP_JUMP: Duration = Duration::from_secs(2 * 60);

/// Shortest the loop will ever sleep, so that a pathological schedule cannot spin.
const MIN_SLEEP: Duration = Duration::from_millis(10);

/// Why a refresh is happening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// The loop has just started and has no numbers yet.
    Startup,
    /// The regular interval elapsed.
    Tick,
    /// Something changed in a watched directory or file.
    FileChange,
    /// The panel, or a second launch of the application, asked for one.
    Requested,
    /// The clock jumped: the machine was suspended, or its clock was corrected.
    Woke,
}

impl Cause {
    /// Whether this cause skips the debounce.
    ///
    /// Waking up does: the numbers on screen are hours old and every extra 250 ms of
    /// wrongness is worse than a coalesced refresh is expensive.
    #[must_use]
    pub fn is_immediate(self) -> bool {
        matches!(self, Cause::Startup | Cause::Woke)
    }

    /// The cause as a short word, for a diagnostic line.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Cause::Startup => "startup",
            Cause::Tick => "tick",
            Cause::FileChange => "file-change",
            Cause::Requested => "requested",
            Cause::Woke => "woke",
        }
    }
}

/// What the loop should do before it asks whether a refresh is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    /// Whether the watched paths should be looked at on this pass.
    pub scan: bool,
    /// Whether the clock jumped since the previous pass.
    pub woke: bool,
}

/// The timing state of one refresh loop.
#[derive(Debug, Clone)]
pub struct Schedule {
    tick: Duration,
    watch_every: Duration,
    debounce: Duration,
    jump: Duration,
    last_refresh: Option<u64>,
    last_watch: Option<u64>,
    pending: Option<(u64, Cause)>,
    seen: Option<(u64, Option<i64>)>,
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule::new(TICK, WATCH_INTERVAL, DEBOUNCE, SLEEP_JUMP)
    }
}

impl Schedule {
    /// A schedule with every interval handed in. The tests' way in.
    #[must_use]
    pub fn new(tick: Duration, watch_every: Duration, debounce: Duration, jump: Duration) -> Self {
        Schedule {
            tick,
            watch_every,
            debounce,
            jump,
            last_refresh: None,
            last_watch: None,
            pending: None,
            seen: None,
        }
    }

    /// How often a refresh happens with nothing else asking.
    #[must_use]
    pub fn tick_interval(&self) -> Duration {
        self.tick
    }

    /// Look at the clock: has it jumped, and is a watch scan due?
    ///
    /// `wall_seconds` is the wall clock; `None` when it could not be read, which simply
    /// leaves the jump detection to the monotonic clock rather than guessing.
    pub fn observe(&mut self, now_ms: u64, wall_seconds: Option<i64>) -> Observation {
        let woke = self.jumped(now_ms, wall_seconds);
        self.seen = Some((now_ms, wall_seconds));

        let scan = match self.last_watch {
            None => true,
            Some(last) => now_ms.saturating_sub(last) >= millis(self.watch_every),
        };
        if scan {
            self.last_watch = Some(now_ms);
        }
        Observation { scan, woke }
    }

    /// Whether either clock moved further than a sleeping machine's would not.
    fn jumped(&self, now_ms: u64, wall_seconds: Option<i64>) -> bool {
        let Some((previous_ms, previous_wall)) = self.seen else {
            return false;
        };
        let threshold = millis(self.jump);
        if now_ms.saturating_sub(previous_ms) > threshold {
            return true;
        }
        match (previous_wall, wall_seconds) {
            (Some(before), Some(after)) => {
                let moved = (after - before).saturating_mul(1000).unsigned_abs();
                moved > threshold
            }
            _ => false,
        }
    }

    /// Record that something wants a refresh.
    ///
    /// A second trigger inside an open burst is absorbed rather than pushing the refresh
    /// further away: coalescing must not be able to starve a continuously written file.
    /// The first cause wins, except that an immediate one takes over a debounced burst.
    pub fn signal(&mut self, now_ms: u64, cause: Cause) {
        if cause.is_immediate() {
            self.pending = Some((now_ms.saturating_sub(millis(self.debounce)), cause));
            return;
        }
        if self.pending.is_none() {
            self.pending = Some((now_ms, cause));
        }
    }

    /// Whether a refresh is due now, and why. Consumes the trigger.
    pub fn due(&mut self, now_ms: u64) -> Option<Cause> {
        if self.last_refresh.is_none() {
            self.refreshed(now_ms);
            return Some(Cause::Startup);
        }
        if let Some((since, cause)) = self.pending {
            if now_ms.saturating_sub(since) >= millis(self.debounce) {
                self.refreshed(now_ms);
                return Some(cause);
            }
        }
        if self
            .last_refresh
            .is_some_and(|last| now_ms.saturating_sub(last) >= millis(self.tick))
        {
            self.refreshed(now_ms);
            return Some(Cause::Tick);
        }
        None
    }

    fn refreshed(&mut self, now_ms: u64) {
        self.last_refresh = Some(now_ms);
        self.pending = None;
    }

    /// How long the loop may sleep before it has to look again.
    ///
    /// The minimum of the three deadlines it is waiting for, clamped so that it never spins
    /// and never sleeps through a watch scan.
    #[must_use]
    pub fn sleep_for(&self, now_ms: u64) -> Duration {
        let mut wait = millis(self.watch_every);
        let mut deadline = |at: u64| {
            wait = wait.min(at.saturating_sub(now_ms));
        };

        deadline(
            self.last_watch
                .map_or(now_ms, |last| last.saturating_add(millis(self.watch_every))),
        );
        deadline(
            self.last_refresh
                .map_or(now_ms, |last| last.saturating_add(millis(self.tick))),
        );
        if let Some((since, _)) = self.pending {
            deadline(since.saturating_add(millis(self.debounce)));
        }

        Duration::from_millis(wait).max(MIN_SLEEP)
    }
}

/// A duration in whole milliseconds, saturating rather than wrapping.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule() -> Schedule {
        Schedule::default()
    }

    #[test]
    fn the_first_pass_refreshes_without_being_asked() {
        let mut schedule = schedule();
        assert_eq!(schedule.due(0), Some(Cause::Startup));
        assert_eq!(schedule.due(1), None, "and not again a millisecond later");
    }

    #[test]
    fn the_tick_fires_at_sixty_seconds_and_not_before() {
        let mut schedule = schedule();
        schedule.due(0);
        assert_eq!(schedule.due(59_999), None);
        assert_eq!(schedule.due(60_000), Some(Cause::Tick));
        assert_eq!(schedule.due(60_001), None);
        assert_eq!(schedule.due(120_000), Some(Cause::Tick));
    }

    #[test]
    fn a_burst_of_file_changes_becomes_one_refresh() {
        let mut schedule = schedule();
        schedule.due(0);

        for at in [1_000, 1_010, 1_100, 1_200, 1_249] {
            schedule.signal(at, Cause::FileChange);
            assert_eq!(schedule.due(at), None, "still inside the debounce at {at}");
        }
        assert_eq!(schedule.due(1_250), Some(Cause::FileChange));
        assert_eq!(schedule.due(1_251), None, "the burst produced one refresh");
    }

    #[test]
    fn a_debounce_is_measured_from_the_first_trigger_not_the_last() {
        // A file that is being written continuously must not starve the refresh.
        let mut schedule = schedule();
        schedule.due(0);
        schedule.signal(1_000, Cause::FileChange);
        for at in 1_001..1_250 {
            schedule.signal(at, Cause::FileChange);
        }
        assert_eq!(schedule.due(1_250), Some(Cause::FileChange));
    }

    #[test]
    fn a_request_is_debounced_too_but_arrives_within_a_quarter_second() {
        let mut schedule = schedule();
        schedule.due(0);
        schedule.signal(5_000, Cause::Requested);
        assert_eq!(schedule.due(5_100), None);
        assert_eq!(schedule.due(5_250), Some(Cause::Requested));
    }

    #[test]
    fn waking_up_refreshes_at_once() {
        let mut schedule = schedule();
        schedule.due(0);
        schedule.signal(9_000, Cause::Woke);
        assert_eq!(
            schedule.due(9_000),
            Some(Cause::Woke),
            "a machine that has just woken does not wait out a debounce"
        );
    }

    #[test]
    fn waking_up_takes_over_a_burst_that_was_already_waiting() {
        let mut schedule = schedule();
        schedule.due(0);
        schedule.signal(9_000, Cause::FileChange);
        schedule.signal(9_010, Cause::Woke);
        assert_eq!(schedule.due(9_010), Some(Cause::Woke));
    }

    #[test]
    fn the_watch_scan_runs_every_five_seconds() {
        let mut schedule = schedule();
        assert!(schedule.observe(0, Some(0)).scan, "the first pass scans");
        assert!(!schedule.observe(1_000, Some(1)).scan);
        assert!(!schedule.observe(4_999, Some(4)).scan);
        assert!(schedule.observe(5_000, Some(5)).scan);
    }

    #[test]
    fn a_wall_clock_that_jumped_forward_is_a_sleep() {
        let mut schedule = schedule();
        // Two ordinary passes, five seconds apart.
        schedule.observe(0, Some(1_788_800_000));
        assert!(!schedule.observe(5_000, Some(1_788_800_005)).woke);

        // The machine slept for eight hours. The monotonic clock may or may not have
        // noticed; the wall clock always does.
        assert!(schedule.observe(10_000, Some(1_788_828_805)).woke);
    }

    #[test]
    fn a_monotonic_clock_that_jumped_is_a_sleep_too() {
        let mut schedule = schedule();
        schedule.observe(0, Some(1_788_800_000));
        // Eight hours of monotonic time, a wall clock that was never readable.
        assert!(schedule.observe(28_800_000, None).woke);
    }

    #[test]
    fn a_clock_corrected_backwards_counts_as_a_jump() {
        let mut schedule = schedule();
        schedule.observe(0, Some(1_788_800_000));
        assert!(
            schedule.observe(5_000, Some(1_788_700_000)).woke,
            "a clock that went backwards is the same surprise as one that went forwards"
        );
    }

    #[test]
    fn an_ordinary_pass_is_not_a_jump() {
        let mut schedule = schedule();
        schedule.observe(0, Some(1_788_800_000));
        for step in 1..20u64 {
            let observation = schedule.observe(step * 5_000, Some(1_788_800_000 + step as i64 * 5));
            assert!(!observation.woke, "step {step} looked like a sleep");
        }
    }

    #[test]
    fn the_loop_never_sleeps_longer_than_the_next_deadline() {
        let mut schedule = schedule();
        schedule.observe(0, Some(0));
        schedule.due(0);

        assert_eq!(schedule.sleep_for(0), WATCH_INTERVAL);
        assert_eq!(schedule.sleep_for(4_000), Duration::from_millis(1_000));

        schedule.signal(4_000, Cause::FileChange);
        assert_eq!(
            schedule.sleep_for(4_000),
            DEBOUNCE,
            "a pending burst is the nearest deadline"
        );
    }

    #[test]
    fn the_loop_never_sleeps_for_nothing() {
        let mut schedule = schedule();
        schedule.observe(0, Some(0));
        schedule.due(0);
        assert!(schedule.sleep_for(1_000_000) >= MIN_SLEEP);
    }
}
