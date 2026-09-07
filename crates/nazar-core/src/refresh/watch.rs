//! Noticing that a watched file or directory moved, by looking rather than by subscribing.
//!
//! ## Why a poll and not a watcher
//!
//! `notify`, the obvious crate for this, is well maintained and permissively licensed, and
//! it is still not what this loop wants:
//!
//! * The readers **already list these directories** on every refresh — the Claude reader to
//!   find the newest capture, the Codex reader to find the newest rollout. Fingerprinting
//!   them costs one more `read_dir` of a handful of entries every five seconds, against a
//!   second background thread, a platform backend per operating system, and the event
//!   coalescing a watcher makes the caller do anyway.
//! * A watcher's story across **suspend and resume** is the part that is least the same on
//!   every platform, and this loop already has to handle waking up (`Cause::Woke`) because
//!   the audit found three multi-hour sleeps in one week of logs. A poll has no state to
//!   lose over a sleep.
//! * On Windows it would add two packages to this workspace's lock file (`notify` and
//!   `notify-types`; `filetime`, `walkdir`, `same-file`, `crossbeam-channel` and `log` are
//!   already there through Tauri), and more on Linux and macOS, for a five-second
//!   improvement in latency on a display whose slowest input refreshes every thirty seconds.
//!
//! Five seconds is the number, not because it is a compromise but because it is below the
//! interval at which either source produces anything: Claude Code redraws its status line
//! every thirty seconds by default, and a Codex quota line arrives once per model response.
//! If that ever stops being true, `notify` is a contained change: this file is the only
//! thing that would have to know.
//!
//! ## What is watched
//!
//! The capture directory (`~/.nazar/statusline`), the rollout log the Codex reader is
//! currently following, and that log's own directory — the last one so that a **new**
//! session's first quota line is noticed rather than waited for. The readers name their own
//! targets; nothing here knows what a rollout log is.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What a path looked like the last time it was scanned.
///
/// Enough to notice an append, a new file, a deletion or a replacement, and nothing that
/// requires opening anything: a directory listing yields names and metadata, and neither is
/// content.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fingerprint {
    exists: bool,
    entries: usize,
    newest: Option<SystemTime>,
    len: u64,
}

impl Fingerprint {
    /// The fingerprint of a path that is not there.
    const fn missing() -> Self {
        Fingerprint {
            exists: false,
            entries: 0,
            newest: None,
            len: 0,
        }
    }
}

/// The set of paths whose changes wake the refresh loop.
#[derive(Debug, Default)]
pub struct WatchSet {
    targets: Vec<PathBuf>,
    seen: BTreeMap<PathBuf, Fingerprint>,
}

impl WatchSet {
    /// An empty watch set.
    #[must_use]
    pub fn new() -> Self {
        WatchSet::default()
    }

    /// Replace the watched paths.
    ///
    /// Called after every refresh, because the readers learn what to watch by reading: the
    /// Codex reader does not know which rollout log it is following until it has followed
    /// one, and tomorrow it will be a different file in a different directory.
    pub fn set_targets(&mut self, targets: Vec<PathBuf>) {
        self.targets = targets;
        self.targets.sort();
        self.targets.dedup();
    }

    /// The paths currently watched.
    #[must_use]
    pub fn targets(&self) -> &[PathBuf] {
        &self.targets
    }

    /// Look at every target. `true` when one of them moved since the previous scan.
    ///
    /// A target that is **newly** watched is recorded without being reported: it has not
    /// changed, it has only just started being looked at, and reporting it would make every
    /// new Codex session cost a spurious refresh.
    pub fn scan(&mut self) -> bool {
        let mut changed = false;
        let mut fresh = BTreeMap::new();
        for target in &self.targets {
            let print = fingerprint(target);
            if self
                .seen
                .get(target)
                .is_some_and(|previous| *previous != print)
            {
                changed = true;
            }
            fresh.insert(target.clone(), print);
        }
        self.seen = fresh;
        changed
    }
}

/// Fingerprint one path, without opening it.
fn fingerprint(path: &Path) -> Fingerprint {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Fingerprint::missing();
    };

    if metadata.is_dir() {
        let mut entries = 0usize;
        let mut newest: Option<SystemTime> = None;
        if let Ok(listing) = std::fs::read_dir(path) {
            for entry in listing.filter_map(std::result::Result::ok) {
                entries += 1;
                if let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) {
                    newest = Some(newest.map_or(modified, |seen| seen.max(modified)));
                }
            }
        }
        return Fingerprint {
            exists: true,
            entries,
            newest,
            len: 0,
        };
    }

    Fingerprint {
        exists: true,
        entries: 0,
        newest: metadata.modified().ok(),
        len: metadata.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    /// Filesystems on Windows keep modification times to about ten milliseconds, and some
    /// keep them to two seconds. A test that only changed a timestamp would be flaky; every
    /// test here changes the length or the entry count as well, which is what a real append
    /// or a real new capture does.
    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_new_target_is_recorded_rather_than_reported() {
        let dir = TempDir::new("watch-new");
        let mut watch = WatchSet::new();
        watch.set_targets(vec![dir.path.clone()]);

        assert!(
            !watch.scan(),
            "the first look at a path is not a change, it is the baseline"
        );
        assert!(!watch.scan());
    }

    #[test]
    fn a_file_that_grew_is_a_change() {
        let dir = TempDir::new("watch-append");
        let path = dir.join("rollout.jsonl");
        write(&path, "one line\n");

        let mut watch = WatchSet::new();
        watch.set_targets(vec![path.clone()]);
        watch.scan();

        write(&path, "one line\ntwo lines\n");
        assert!(watch.scan());
        assert!(!watch.scan(), "and it is only reported once");
    }

    #[test]
    fn a_new_file_in_a_watched_directory_is_a_change() {
        let dir = TempDir::new("watch-dir");
        let mut watch = WatchSet::new();
        watch.set_targets(vec![dir.path.clone()]);
        watch.scan();

        write(&dir.join("session.json"), "{}");
        assert!(watch.scan());
        assert!(!watch.scan());
    }

    #[test]
    fn a_deleted_file_in_a_watched_directory_is_a_change() {
        let dir = TempDir::new("watch-delete");
        let path = dir.join("session.json");
        write(&path, "{}");

        let mut watch = WatchSet::new();
        watch.set_targets(vec![dir.path.clone()]);
        watch.scan();

        std::fs::remove_file(&path).unwrap();
        assert!(watch.scan());
    }

    #[test]
    fn a_target_that_appears_or_vanishes_is_a_change() {
        let dir = TempDir::new("watch-appear");
        let path = dir.join("later.json");

        let mut watch = WatchSet::new();
        watch.set_targets(vec![path.clone()]);
        watch.scan();

        write(&path, "{}");
        assert!(watch.scan(), "a watched path that appeared is a change");

        std::fs::remove_file(&path).unwrap();
        assert!(watch.scan(), "and so is one that went away");
    }

    #[test]
    fn a_path_that_never_exists_never_reports_a_change() {
        let dir = TempDir::new("watch-absent");
        let mut watch = WatchSet::new();
        watch.set_targets(vec![dir.join("no-such-directory")]);
        for _ in 0..5 {
            assert!(!watch.scan());
        }
    }

    #[test]
    fn targets_are_deduplicated_and_ordered() {
        let dir = TempDir::new("watch-dedup");
        let mut watch = WatchSet::new();
        let one = dir.join("a");
        let two = dir.join("b");
        watch.set_targets(vec![two.clone(), one.clone(), two.clone()]);
        assert_eq!(watch.targets(), [one, two]);
    }
}
