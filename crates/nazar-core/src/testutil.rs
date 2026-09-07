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
/// The wall reading starts at the **real** current instant rather than at a fixed one. A
/// test that writes a file and then asks how old it is compares a filesystem timestamp with
/// this clock, and a fixed origin would make that comparison meaningless.
pub(crate) struct ManualClock {
    monotonic_ms: std::sync::Mutex<u64>,
    wall_seconds: std::sync::Mutex<i64>,
}

impl ManualClock {
    /// A clock at monotonic zero and the current wall instant.
    pub(crate) fn new() -> Self {
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
