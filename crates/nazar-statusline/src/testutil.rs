//! Throwaway directories, shared by the unit tests and the integration tests.
//!
//! Public rather than `#[cfg(test)]` because the integration tests in `tests/` link
//! against this crate as an ordinary library and cannot see a test-only module. It is
//! small enough that shipping it costs nothing, and every test in this crate needs it:
//! **no test may touch the real `~/.claude` or the real `~/.nazar`**, which is the point
//! of having it at all.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// A directory under the system temporary directory, removed when the value is dropped.
#[derive(Debug)]
pub struct TempDir {
    /// The directory itself.
    pub path: PathBuf,
}

impl TempDir {
    /// Create one, labelled so a leftover directory says which test made it.
    #[must_use]
    pub fn new(label: &str) -> Self {
        let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nazar-statusline-{label}-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("the temporary directory must be creatable");
        TempDir { path }
    }

    /// A path inside the directory.
    #[must_use]
    pub fn join(&self, relative: &str) -> PathBuf {
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
