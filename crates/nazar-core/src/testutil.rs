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
