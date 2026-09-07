//! Finding the rollout log Codex is writing to right now.
//!
//! Codex lays its session logs out as
//! `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ISO>-<uuid>[_<uuid>].jsonl`, with
//! `CODEX_HOME` defaulting to `~/.codex`. The segments are zero padded, so sorting the
//! directory names as text sorts them as dates, and walking them backwards reaches the
//! newest session without listing a year of history.
//!
//! Two things about the real layout shape the walk, both observed on the maintainer's
//! machine:
//!
//! * **Date directories can be empty.** Four of the ten date directories there hold no
//!   rollout file at all (the sessions were archived). An empty directory must not end
//!   the walk, or the reader concludes there is no data on a machine full of it.
//! * **The newest file is not always the last quota line.** Three of the eighteen logs
//!   there carry no `rate_limits` line whatsoever — they are three- and nine-line stubs
//!   from sessions that ended before the first response. So the caller gets a short list
//!   of candidates rather than one path, and stops at the first that actually answers.
//!
//! The walk is bounded on both sides: at most [`MAX_DATE_DIRECTORIES`] date directories
//! are listed, and the caller opens at most [`MAX_FILES_OPENED`] of the files found.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How many rollout files the reader may open while looking for a quota line.
pub const MAX_FILES_OPENED: usize = 5;

/// How many date directories the walk lists before giving up.
///
/// A day's directory is one `read_dir`. Twenty of them is a cheap bound that still
/// reaches back three weeks on a machine used every day, and much further on one that is
/// not.
pub const MAX_DATE_DIRECTORIES: usize = 20;

/// Stop listing date directories once this many non-empty ones have been seen and enough
/// candidates are in hand.
///
/// One is not enough: a session opened yesterday and still being appended to today has a
/// newer modification time than a session opened today and already finished.
const MIN_NON_EMPTY_DIRECTORIES: usize = 3;

/// Prefix and suffix of a rollout file name.
const ROLLOUT_PREFIX: &str = "rollout-";
const ROLLOUT_SUFFIX: &str = ".jsonl";

/// One rollout log found by the walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Full path to the file.
    pub path: PathBuf,
    /// Last modification time, or the Unix epoch when the filesystem did not report one.
    pub modified: SystemTime,
}

/// `<home>/sessions` — where Codex keeps its session logs.
#[must_use]
pub fn sessions_dir(home: &Path) -> PathBuf {
    home.join("sessions")
}

/// The newest rollout logs under `home`, most recently modified first.
///
/// Returns at most `cap` entries and never fails: a missing directory, a permission
/// error or a name that is not a date all mean "nothing here", which is a normal state
/// on a machine where Codex has never run.
#[must_use]
pub fn newest_rollouts(home: &Path, cap: usize) -> Vec<Candidate> {
    let sessions = sessions_dir(home);
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut listed = 0usize;
    let mut non_empty = 0usize;

    'walk: for year in numeric_children(&sessions) {
        for month in numeric_children(&year) {
            for day in numeric_children(&month) {
                if listed >= MAX_DATE_DIRECTORIES {
                    break 'walk;
                }
                listed += 1;

                let before = candidates.len();
                collect_rollouts(&day, &mut candidates);
                if candidates.len() > before {
                    non_empty += 1;
                }

                if non_empty >= MIN_NON_EMPTY_DIRECTORIES && candidates.len() >= cap {
                    break 'walk;
                }
            }
        }
    }

    // Newest first. `path` breaks ties so the order is stable across runs, which matters
    // because two logs written in the same second is the normal case for a busy day.
    candidates.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| b.path.cmp(&a.path))
    });
    candidates.truncate(cap);
    candidates
}

/// Directory entries of `parent` whose names are all digits, newest name first.
///
/// Anything that is not a zero-padded number is not part of the date tree and is skipped
/// rather than guessed at.
fn numeric_children(parent: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut names: Vec<PathBuf> = entries
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();
    // Zero-padded numbers sort as text the way they sort as dates.
    names.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    names
}

/// Append every `rollout-*.jsonl` in `day` to `candidates`.
fn collect_rollouts(day: &Path, candidates: &mut Vec<Candidate>) {
    let Ok(entries) = std::fs::read_dir(day) else {
        return;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !(name.starts_with(ROLLOUT_PREFIX) && name.ends_with(ROLLOUT_SUFFIX)) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        candidates.push(Candidate {
            path: entry.path(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::time::Duration;

    /// Build `<root>/sessions/<date>/rollout-<label>.jsonl` and stamp its modification
    /// time `age` behind now, so "newest" is a fact and not a race.
    fn plant(root: &Path, date: &str, label: &str, age: Duration) -> PathBuf {
        let dir = sessions_dir(root).join(date.replace('/', std::path::MAIN_SEPARATOR_STR));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-{label}.jsonl"));
        std::fs::write(&path, b"{}\n").unwrap();
        set_modified(&path, age);
        path
    }

    fn set_modified(path: &Path, age: Duration) {
        let when = SystemTime::now() - age;
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(when).unwrap();
    }

    fn empty_day(root: &Path, date: &str) {
        std::fs::create_dir_all(
            sessions_dir(root).join(date.replace('/', std::path::MAIN_SEPARATOR_STR)),
        )
        .unwrap();
    }

    fn names(found: &[Candidate]) -> Vec<String> {
        found
            .iter()
            .map(|candidate| {
                candidate
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn a_missing_codex_home_is_empty_not_an_error() {
        let dir = TempDir::new("locate-missing");
        assert!(newest_rollouts(&dir.join("nope"), MAX_FILES_OPENED).is_empty());
    }

    #[test]
    fn a_codex_home_without_sessions_is_empty() {
        let dir = TempDir::new("locate-no-sessions");
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        assert!(newest_rollouts(&dir.path, MAX_FILES_OPENED).is_empty());
    }

    #[test]
    fn the_newest_modification_time_wins_regardless_of_the_date_directory() {
        let dir = TempDir::new("locate-newest");
        // The older *directory* holds the newer *file*: a session opened yesterday and
        // still being appended to. Sorting by name alone would get this wrong.
        plant(
            &dir.path,
            "2026/09/07",
            "today-old",
            Duration::from_secs(9_000),
        );
        plant(
            &dir.path,
            "2026/09/06",
            "yesterday-live",
            Duration::from_secs(10),
        );
        plant(
            &dir.path,
            "2026/09/05",
            "older",
            Duration::from_secs(90_000),
        );

        let found = newest_rollouts(&dir.path, MAX_FILES_OPENED);
        assert_eq!(
            names(&found),
            [
                "rollout-yesterday-live.jsonl",
                "rollout-today-old.jsonl",
                "rollout-older.jsonl"
            ]
        );
    }

    #[test]
    fn empty_date_directories_do_not_end_the_walk() {
        let dir = TempDir::new("locate-empty-days");
        // Exactly the shape of the maintainer's machine: several empty days sit between
        // the newest day and the older sessions.
        empty_day(&dir.path, "2026/09/06");
        empty_day(&dir.path, "2026/09/04");
        empty_day(&dir.path, "2026/09/03");
        plant(&dir.path, "2026/09/07", "newest", Duration::from_secs(10));
        plant(&dir.path, "2026/09/02", "buried", Duration::from_secs(100));

        let found = newest_rollouts(&dir.path, MAX_FILES_OPENED);
        assert_eq!(
            names(&found),
            ["rollout-newest.jsonl", "rollout-buried.jsonl"]
        );
    }

    #[test]
    fn the_cap_is_respected() {
        let dir = TempDir::new("locate-cap");
        for index in 0..12u64 {
            plant(
                &dir.path,
                "2026/09/07",
                &format!("s{index:02}"),
                Duration::from_secs(index + 1),
            );
        }

        let found = newest_rollouts(&dir.path, MAX_FILES_OPENED);
        assert_eq!(found.len(), MAX_FILES_OPENED);
        assert_eq!(names(&found)[0], "rollout-s00.jsonl");
        assert_eq!(names(&found)[4], "rollout-s04.jsonl");
    }

    #[test]
    fn only_rollout_jsonl_files_count() {
        let dir = TempDir::new("locate-filter");
        let day = sessions_dir(&dir.path).join("2026").join("09").join("07");
        std::fs::create_dir_all(&day).unwrap();
        for name in [
            "session_index.jsonl",
            "rollout-good.jsonl.tmp",
            "rollout-good.json",
            "notes.txt",
        ] {
            std::fs::write(day.join(name), b"{}\n").unwrap();
        }
        std::fs::create_dir_all(day.join("rollout-a-directory.jsonl")).unwrap();
        plant(&dir.path, "2026/09/07", "real", Duration::from_secs(1));

        let found = newest_rollouts(&dir.path, MAX_FILES_OPENED);
        assert_eq!(names(&found), ["rollout-real.jsonl"]);
    }

    #[test]
    fn directories_that_are_not_dates_are_skipped() {
        let dir = TempDir::new("locate-non-date");
        std::fs::create_dir_all(
            sessions_dir(&dir.path)
                .join("archive")
                .join("09")
                .join("07"),
        )
        .unwrap();
        std::fs::write(
            sessions_dir(&dir.path)
                .join("archive")
                .join("09")
                .join("07")
                .join("rollout-hidden.jsonl"),
            b"{}\n",
        )
        .unwrap();
        plant(&dir.path, "2026/09/07", "real", Duration::from_secs(1));

        let found = newest_rollouts(&dir.path, MAX_FILES_OPENED);
        assert_eq!(names(&found), ["rollout-real.jsonl"]);
    }

    #[test]
    fn the_walk_stops_after_a_bounded_number_of_date_directories() {
        let dir = TempDir::new("locate-bound");
        // Far more empty days than the bound, with the only file at the very bottom.
        for day in 1..=28 {
            empty_day(&dir.path, &format!("2026/09/{day:02}"));
        }
        plant(
            &dir.path,
            "2026/01/01",
            "unreachable",
            Duration::from_secs(1),
        );

        assert!(
            newest_rollouts(&dir.path, MAX_FILES_OPENED).is_empty(),
            "the walk must be bounded even when that means missing a very old log"
        );
    }
}
