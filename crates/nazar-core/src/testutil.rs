//! A throwaway directory for tests.
//!
//! Small enough to write, and it keeps the dependency list of a crate that ships in a
//! zero-network product down to serde and serde_json.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// A directory under the system temporary directory, removed when the value is dropped.
pub(crate) struct TempDir {
    pub(crate) path: PathBuf,
}

impl TempDir {
    pub(crate) fn new(label: &str) -> Self {
        let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nazar-core-{label}-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("the temporary directory must be creatable");
        TempDir { path }
    }

    pub(crate) fn join(&self, relative: &str) -> PathBuf {
        self.path.join(relative)
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A clock the test moves by hand.
///
/// Both readings advance together — the monotonic one in milliseconds, the wall one in
/// whole seconds from the same origin — so that a test that wants them to *disagree*, which
/// is how a sleeping machine looks, has to say so with [`ManualClock::sleep_through`].
///
/// # Which constructor
///
/// There is deliberately no `new`. A clock has to say where its wall reading starts,
/// because the two answers are not interchangeable and the wrong one rots:
///
/// * [`ManualClock::at`] pins the wall reading to an instant the test names. **This is the
///   one to reach for.** Every test that also uses a fixed timestamp — a `resetsAt`, a
///   captured document, one of this module's `2026-…` constants — needs its "now" to keep
///   the same relationship to those fixtures on every run, in CI, next year.
/// * [`ManualClock::at_real_now`] starts at the real current instant. Only for a test that
///   compares this clock with a **filesystem** timestamp, which the real clock writes
///   whatever this one says.
///
/// Seeding from the real clock used to be the default, and it planted a time bomb:
/// `alerts::tests::a_crossing_that_happened_during_a_sleep_fires_once_when_the_machine_wakes`
/// sleeps nine hours forward from "now" and then evaluates a view whose `resetsAt` is the
/// fixed `RESET_A`. It passed for as long as real now plus nine hours landed *before* that
/// instant and went red the day it did not, because a window past its reset is stale and a
/// stale view fires nothing. The test was right and the clock was wrong: nothing about the
/// behaviour under test depends on the calendar, so nothing about the harness should.
pub(crate) struct ManualClock {
    monotonic_ms: std::sync::Mutex<u64>,
    wall_seconds: std::sync::Mutex<i64>,
}

impl ManualClock {
    /// A clock at monotonic zero and the wall instant the caller names.
    ///
    /// Pass the same instant the test's other fixtures are written around; the result is a
    /// run that cannot change with the date.
    pub(crate) fn at(instant: &str) -> Self {
        let wall = crate::timefmt::unix_seconds_from_rfc3339(instant)
            .expect("a pinned test clock needs an instant this crate can parse");
        ManualClock {
            monotonic_ms: std::sync::Mutex::new(0),
            wall_seconds: std::sync::Mutex::new(wall),
        }
    }

    /// A clock at monotonic zero and the **real** current wall instant.
    ///
    /// The narrow case: a test that writes a file and then asks the code under test how old
    /// it is — [`crate::refresh`]'s request marker measures `now − mtime`, and the mtime
    /// comes from the operating system. Pinning such a clock would make the age meaningless.
    /// Anything else wants [`ManualClock::at`].
    pub(crate) fn at_real_now() -> Self {
        let wall = crate::timefmt::unix_seconds_from_rfc3339(&crate::timefmt::now_rfc3339())
            .unwrap_or(1_788_775_200);
        ManualClock {
            monotonic_ms: std::sync::Mutex::new(0),
            wall_seconds: std::sync::Mutex::new(wall),
        }
    }

    /// Move both clocks forward by the same amount.
    pub(crate) fn advance(&self, by: std::time::Duration) {
        *self.monotonic_ms.lock().unwrap() += by.as_millis() as u64;
        *self.wall_seconds.lock().unwrap() += by.as_secs() as i64;
    }

    /// Move the wall clock forward without the monotonic one: a suspended machine.
    pub(crate) fn sleep_through(&self, by: std::time::Duration) {
        *self.monotonic_ms.lock().unwrap() += 100;
        *self.wall_seconds.lock().unwrap() += by.as_secs() as i64;
    }

    /// The current wall instant, as the document would spell it.
    pub(crate) fn now(&self) -> String {
        crate::timefmt::rfc3339_from_unix_seconds(*self.wall_seconds.lock().unwrap())
    }
}

impl crate::clock::Clock for ManualClock {
    fn monotonic_millis(&self) -> u64 {
        *self.monotonic_ms.lock().unwrap()
    }

    fn now_rfc3339(&self) -> String {
        self.now()
    }
}
