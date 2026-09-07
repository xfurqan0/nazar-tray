//! `~/.nazar/limits.lock` — the advisory file that makes "one writer" true.
//!
//! `limits.json` has exactly one writer by design, and the audit of the retired prototype
//! is the argument for taking that seriously. Finding B01 was a lock whose check and whose
//! write were two separate operations, so two refreshers passed the check together; the
//! logs caught them starting four seconds and two seconds apart, and the double request is
//! what produced the observed HTTP 429.
//!
//! So the lock here is not a check followed by a write. It is **one** operation:
//!
//! ```text
//!   OpenOptions::new().create_new(true).open("~/.nazar/limits.lock")
//!            │
//!            ├── Ok      ─▶ we are the writer. Write the record, heartbeat every tick.
//!            └── Err(AlreadyExists)
//!                     │
//!                     ├── the record is alive   ─▶ someone else writes; we only read.
//!                     └── the record is stale   ─▶ delete it and go round again.
//! ```
//!
//! `create_new` is atomic on every filesystem this runs on, so of two processes racing for
//! it exactly one wins and the other is told why.
//!
//! ## What proves a holder is alive
//!
//! The **heartbeat**, not the process id. There is no portable way to ask the operating
//! system whether a given process is still running, and buying one would cost a platform
//! crate in the crate that is meant to be boring. A heartbeat costs nothing extra: the
//! holder is already awake every sixty seconds, so it rewrites this file while it is there.
//! A record whose heartbeat is older than [`STALE_AFTER`] is reclaimable.
//!
//! That also covers a case a process-id check cannot: a holder that is still running but
//! wedged stops heart-beating and gets replaced, where a liveness probe would wait for ever
//! on a process that will never write again.
//!
//! `pid` and `startedAt` are written for the human reading the file — "which process, since
//! when" is the first question anyone asks — and for a holder to recognise its own record.
//! Neither is trusted as a liveness proof.
//!
//! ## What a loser does
//!
//! Reads. A second tray instance, or `nazar-tray --print --write` started while the tray is
//! running, does not queue and does not force: it reports who holds the lock and reads the
//! document that holder maintains. One writer, many readers, the same rule the whole
//! contract rests on.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::atomic;
use crate::error::{Error, Result};
use crate::timefmt::{seconds_between, unix_seconds_from_rfc3339};

/// Version of the lock record this build writes.
pub const LOCK_SCHEMA_VERSION: u32 = 1;

/// How long a record may go without a heartbeat before it may be reclaimed.
///
/// Five refresh ticks. Long enough that a machine grinding through a big build does not
/// lose its lock to itself, short enough that a tray killed with the task manager does not
/// keep the next one out for a coffee break.
pub const STALE_AFTER: Duration = Duration::from_secs(5 * 60);

/// How many times an acquisition will reclaim a stale record before giving up.
///
/// Two processes that both find the same stale record can both delete it, and one of them
/// then loses the `create_new` race and comes round again. A handful of attempts covers
/// that; an unbounded loop would turn a filesystem that refuses deletion into a spin.
const ATTEMPTS: u32 = 8;

/// What is written in `~/.nazar/limits.lock`.
///
/// Three facts and nothing else. No path, no account, no session: the file sits beside
/// `limits.json` and is safe to paste into a bug report for the same reasons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockRecord {
    /// Record version. See [`LOCK_SCHEMA_VERSION`].
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Process id of the holder. A diagnostic, never a liveness proof.
    pub pid: u32,
    /// When the holder took the lock, RFC 3339 in UTC.
    pub started_at: String,
    /// When the holder last said it was still there, RFC 3339 in UTC.
    pub heartbeat_at: String,
    /// Fields a future version added. Preserved verbatim.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

fn default_schema_version() -> u32 {
    LOCK_SCHEMA_VERSION
}

impl LockRecord {
    /// A fresh record for this process.
    #[must_use]
    pub fn new(pid: u32, started_at: impl Into<String>, now: impl Into<String>) -> Self {
        LockRecord {
            schema_version: LOCK_SCHEMA_VERSION,
            pid,
            started_at: started_at.into(),
            heartbeat_at: now.into(),
            extra: Map::new(),
        }
    }

    /// How long ago the holder last said it was there, in seconds.
    ///
    /// `None` when either timestamp is unreadable, which is treated as "cannot tell" by
    /// every caller rather than as "dead".
    #[must_use]
    pub fn silence_seconds(&self, now: &str) -> Option<i64> {
        seconds_between(&self.heartbeat_at, now)
    }

    /// Whether this record may be reclaimed at `now`.
    ///
    /// A heartbeat in the future is not stale: a machine whose clock has just been
    /// corrected backwards would otherwise evict a perfectly live holder.
    #[must_use]
    pub fn is_stale(&self, now: &str, stale_after: Duration) -> bool {
        match self.silence_seconds(now) {
            Some(silence) => silence > stale_after.as_secs() as i64,
            // An unreadable heartbeat is a damaged record. It is reclaimed by the age of
            // the file itself, not here; see `reclaimable`.
            None => false,
        }
    }
}

/// The outcome of trying to become the writer.
#[derive(Debug)]
pub enum Acquisition {
    /// We are the writer. Hold on to the [`LimitsLock`]; dropping it releases the lock.
    Held(LimitsLock),
    /// Somebody else is, and this is what they wrote about themselves. `None` when the
    /// record could not be read at all, which still means "not ours to write".
    Taken(Option<LockRecord>),
}

impl Acquisition {
    /// The lock, if we got it.
    #[must_use]
    pub fn held(self) -> Option<LimitsLock> {
        match self {
            Acquisition::Held(lock) => Some(lock),
            Acquisition::Taken(_) => None,
        }
    }
}

/// What a heartbeat found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heartbeat {
    /// Still ours. Carry on writing.
    Held,
    /// The file now names somebody else, or is gone and could not be retaken. Stop writing.
    Lost,
}

/// A held lock. Releases itself when dropped.
#[derive(Debug)]
pub struct LimitsLock {
    path: PathBuf,
    record: LockRecord,
}

impl LimitsLock {
    /// Try to become the writer of `path`.
    ///
    /// `now` is RFC 3339 in UTC and is both the heartbeat this process writes and the
    /// instant an existing record is judged against.
    pub fn acquire(path: &Path, now: &str) -> Result<Acquisition> {
        LimitsLock::acquire_as(
            path,
            std::process::id(),
            &crate::clock::process_start_rfc3339(),
            now,
            STALE_AFTER,
        )
    }

    /// [`acquire`](LimitsLock::acquire) with everything about "this process" handed in.
    ///
    /// The tests' way in: two "processes" in one test binary need two identities, and a
    /// stale record needs a threshold that does not take five minutes to reach.
    pub fn acquire_as(
        path: &Path,
        pid: u32,
        started_at: &str,
        now: &str,
        stale_after: Duration,
    ) -> Result<Acquisition> {
        Ok(match claim(path, pid, started_at, now, stale_after)? {
            Ok(record) => Acquisition::Held(LimitsLock {
                path: path.to_path_buf(),
                record,
            }),
            Err(holder) => Acquisition::Taken(holder),
        })
    }

    /// The file this lock is on.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The record this process wrote.
    #[must_use]
    pub fn record(&self) -> &LockRecord {
        &self.record
    }

    /// Say we are still here.
    ///
    /// Rewrites the record atomically and checks, in the same breath, that the file still
    /// names us. A holder that has been evicted — its heartbeat missed for five minutes on
    /// a suspended machine, the lock reclaimed by a new instance — finds out here and stops
    /// writing, which is what keeps "one writer" true after a sleep as well as before it.
    pub fn heartbeat(&mut self, now: &str) -> Result<Heartbeat> {
        match read_record(&self.path) {
            Some(current) if self.wrote(&current) => {}
            Some(_) => return Ok(Heartbeat::Lost),
            // The file vanished — something tidied `~/.nazar` — so take it again rather
            // than carry on as a writer with no lock.
            None if !self.path.exists() => {
                let claimed = claim(
                    &self.path,
                    self.record.pid,
                    &self.record.started_at,
                    now,
                    STALE_AFTER,
                )?;
                return Ok(match claimed {
                    Ok(record) => {
                        self.record = record;
                        Heartbeat::Held
                    }
                    Err(_) => Heartbeat::Lost,
                });
            }
            // There is a file, and it is not a record. Somebody else wrote it.
            None => return Ok(Heartbeat::Lost),
        }

        self.record.heartbeat_at = now.to_owned();
        let text = render(&self.record)?;
        atomic::write_bytes(&self.path, text.as_bytes())?;
        Ok(Heartbeat::Held)
    }

    /// Whether `record` is the one this process wrote.
    fn wrote(&self, record: &LockRecord) -> bool {
        record.pid == self.record.pid && record.started_at == self.record.started_at
    }

    /// Release the lock now rather than at the end of the scope.
    pub fn release(self) {
        drop(self);
    }
}

impl Drop for LimitsLock {
    fn drop(&mut self) {
        // Only ours to remove. A lock that was reclaimed while we were suspended belongs to
        // somebody else now, and deleting it would hand a third process a lock two others
        // think they hold.
        if read_record(&self.path).is_some_and(|current| self.wrote(&current)) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// The acquisition itself, without the guard that would release it.
///
/// `Ok(record)` means the file is ours and holds that record; `Err(holder)` means somebody
/// else has it, and `holder` is what they wrote about themselves when that could be read.
fn claim(
    path: &Path,
    pid: u32,
    started_at: &str,
    now: &str,
    stale_after: Duration,
) -> Result<std::result::Result<LockRecord, Option<LockRecord>>> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
    }

    for _ in 0..ATTEMPTS {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut file) => {
                let record = LockRecord::new(pid, started_at, now);
                let text = render(&record)?;
                // Written through the handle we already hold exclusively. Every later write
                // goes through the atomic writer instead, so a reader never sees half a
                // record once there is one to see.
                file.write_all(text.as_bytes())
                    .and_then(|()| file.sync_all())
                    .map_err(|source| Error::io(path, source))?;
                return Ok(Ok(record));
            }
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                let existing = read_record(path);
                if reclaimable(path, existing.as_ref(), now, stale_after) {
                    // Best effort: if the delete fails, the next attempt reads the record
                    // again and reports it as taken rather than spinning.
                    let _ = std::fs::remove_file(path);
                    continue;
                }
                return Ok(Err(existing));
            }
            Err(source) => return Err(Error::io(path, source)),
        }
    }
    Ok(Err(read_record(path)))
}

/// Read the record without taking anything.
///
/// `None` for a file that is not there, cannot be read, or is not a record — a caller that
/// wants to tell those apart is asking the wrong question, because all three mean "there is
/// nothing here that says who the writer is".
#[must_use]
pub fn read_record(path: &Path) -> Option<LockRecord> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether an existing lock file may be deleted and retaken.
///
/// Two ways in:
///
/// * the record is readable and its heartbeat is older than `stale_after`;
/// * the record is **not** readable, and the file itself has not been touched for
///   `stale_after` either. The second case is what makes the acquisition race safe: between
///   `create_new` and the record being written there is a moment when the file is empty,
///   and a competitor that read it then must wait rather than evict a holder that is one
///   instruction from being alive.
fn reclaimable(path: &Path, record: Option<&LockRecord>, now: &str, stale_after: Duration) -> bool {
    match record {
        Some(record) => record.is_stale(now, stale_after),
        None => match file_age_seconds(path, now) {
            Some(age) => age > stale_after.as_secs() as i64,
            None => false,
        },
    }
}

/// How many seconds ago the file was last written, from its own modification time.
fn file_age_seconds(path: &Path, now: &str) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let now_seconds = unix_seconds_from_rfc3339(now)?;
    Some(now_seconds - crate::clock::system_time_seconds(modified))
}

/// The record as it is written: pretty, with a trailing newline.
fn render(record: &LockRecord) -> Result<String> {
    let mut text = serde_json::to_string_pretty(record)?;
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    const START: &str = "2026-09-07T10:00:00Z";

    fn lock_path(dir: &TempDir) -> PathBuf {
        dir.join("limits.lock")
    }

    #[test]
    fn the_first_process_gets_the_lock_and_the_second_is_told_who_has_it() {
        let dir = TempDir::new("lock-one-writer");
        let path = lock_path(&dir);

        let first = LimitsLock::acquire_as(&path, 111, START, "2026-09-07T10:00:00Z", STALE_AFTER)
            .unwrap()
            .held()
            .expect("the first process must get the lock");
        assert_eq!(first.record().pid, 111);

        let second =
            LimitsLock::acquire_as(&path, 222, START, "2026-09-07T10:00:01Z", STALE_AFTER).unwrap();
        match second {
            Acquisition::Held(_) => panic!("two processes must not both hold the lock"),
            Acquisition::Taken(Some(record)) => assert_eq!(record.pid, 111),
            Acquisition::Taken(None) => panic!("the loser should be told who holds it"),
        }
    }

    #[test]
    fn releasing_the_lock_lets_the_next_process_take_it() {
        let dir = TempDir::new("lock-release");
        let path = lock_path(&dir);

        let first = LimitsLock::acquire_as(&path, 111, START, "2026-09-07T10:00:00Z", STALE_AFTER)
            .unwrap()
            .held()
            .unwrap();
        first.release();
        assert!(!path.exists(), "a released lock leaves no file behind");

        assert!(
            LimitsLock::acquire_as(&path, 222, START, "2026-09-07T10:00:02Z", STALE_AFTER)
                .unwrap()
                .held()
                .is_some()
        );
    }

    #[test]
    fn a_record_whose_holder_stopped_beating_is_reclaimed() {
        let dir = TempDir::new("lock-stale");
        let path = lock_path(&dir);

        // A holder that died five and a half minutes ago: the file is there, the process
        // that wrote it is not, and nothing has rewritten the heartbeat since.
        let dead = LockRecord::new(4242, START, "2026-09-07T10:00:00Z");
        std::fs::write(&path, render(&dead).unwrap()).unwrap();

        let taken =
            LimitsLock::acquire_as(&path, 999, START, "2026-09-07T10:05:30Z", STALE_AFTER).unwrap();
        let lock = taken.held().expect("a stale lock must be reclaimable");
        assert_eq!(lock.record().pid, 999);
        assert_eq!(read_record(&path).unwrap().pid, 999);
    }

    #[test]
    fn a_record_that_is_still_beating_is_not_reclaimed() {
        let dir = TempDir::new("lock-alive");
        let path = lock_path(&dir);

        let alive = LockRecord::new(4242, START, "2026-09-07T10:00:00Z");
        std::fs::write(&path, render(&alive).unwrap()).unwrap();

        // Four and a half minutes of silence is inside the five-minute grace.
        let taken =
            LimitsLock::acquire_as(&path, 999, START, "2026-09-07T10:04:30Z", STALE_AFTER).unwrap();
        assert!(matches!(taken, Acquisition::Taken(Some(_))));
        assert_eq!(read_record(&path).unwrap().pid, 4242);
    }

    #[test]
    fn a_heartbeat_from_the_future_does_not_evict_a_live_holder() {
        let record = LockRecord::new(1, START, "2026-09-07T10:10:00Z");
        assert!(!record.is_stale("2026-09-07T10:00:00Z", STALE_AFTER));
        assert_eq!(record.silence_seconds("2026-09-07T10:00:00Z"), Some(-600));
    }

    #[test]
    fn an_empty_lock_file_is_respected_until_it_ages_out() {
        let dir = TempDir::new("lock-empty");
        let path = lock_path(&dir);
        // The moment between `create_new` and the record being written.
        std::fs::write(&path, "").unwrap();

        let now = crate::timefmt::now_rfc3339();
        let taken = LimitsLock::acquire_as(&path, 7, START, &now, STALE_AFTER).unwrap();
        assert!(
            matches!(taken, Acquisition::Taken(None)),
            "a lock file that was created a moment ago must not be evicted mid-write"
        );

        // The same file, an hour later. A husk nobody has touched since is reclaimable.
        let an_hour_on = crate::timefmt::rfc3339_from_unix_seconds(
            crate::timefmt::unix_seconds_from_rfc3339(&now).unwrap() + 3600,
        );
        let taken = LimitsLock::acquire_as(&path, 7, START, &an_hour_on, STALE_AFTER).unwrap();
        assert!(
            taken.held().is_some(),
            "a long-abandoned husk is reclaimable"
        );
    }

    #[test]
    fn a_heartbeat_keeps_the_lock_and_moves_the_stamp() {
        let dir = TempDir::new("lock-heartbeat");
        let path = lock_path(&dir);

        let mut lock =
            LimitsLock::acquire_as(&path, 111, START, "2026-09-07T10:00:00Z", STALE_AFTER)
                .unwrap()
                .held()
                .unwrap();

        assert_eq!(
            lock.heartbeat("2026-09-07T10:01:00Z").unwrap(),
            Heartbeat::Held
        );
        let on_disk = read_record(&path).unwrap();
        assert_eq!(on_disk.heartbeat_at, "2026-09-07T10:01:00Z");
        assert_eq!(on_disk.started_at, START, "the start does not move");
        assert_eq!(on_disk.pid, 111);
    }

    #[test]
    fn a_holder_that_was_evicted_learns_it_on_the_next_heartbeat() {
        let dir = TempDir::new("lock-evicted");
        let path = lock_path(&dir);

        let mut lock =
            LimitsLock::acquire_as(&path, 111, START, "2026-09-07T10:00:00Z", STALE_AFTER)
                .unwrap()
                .held()
                .unwrap();

        // A second instance found the record stale and took over.
        let usurper = LockRecord::new(222, "2026-09-07T10:06:00Z", "2026-09-07T10:06:00Z");
        std::fs::write(&path, render(&usurper).unwrap()).unwrap();

        assert_eq!(
            lock.heartbeat("2026-09-07T10:07:00Z").unwrap(),
            Heartbeat::Lost
        );
        // And dropping ours must not take theirs with it.
        drop(lock);
        assert_eq!(read_record(&path).unwrap().pid, 222);
    }

    #[test]
    fn a_lock_file_that_vanished_is_taken_again() {
        let dir = TempDir::new("lock-vanished");
        let path = lock_path(&dir);

        let mut lock =
            LimitsLock::acquire_as(&path, 111, START, "2026-09-07T10:00:00Z", STALE_AFTER)
                .unwrap()
                .held()
                .unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(
            lock.heartbeat("2026-09-07T10:01:00Z").unwrap(),
            Heartbeat::Held
        );
        assert_eq!(read_record(&path).unwrap().pid, 111);
    }

    #[test]
    fn the_record_carries_three_facts_and_no_paths() {
        let dir = TempDir::new("lock-content");
        let path = lock_path(&dir);
        let _lock = LimitsLock::acquire_as(&path, 111, START, "2026-09-07T10:00:00Z", STALE_AFTER)
            .unwrap()
            .held()
            .unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with('\n'), "got {text:?}");
        for expected in [
            "\"schemaVersion\"",
            "\"pid\"",
            "\"startedAt\"",
            "\"heartbeatAt\"",
        ] {
            assert!(text.contains(expected), "got {text}");
        }
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value.as_object().unwrap().len(),
            4,
            "the lock record holds four keys and nothing that identifies a machine: {text}"
        );
    }

    #[test]
    fn unknown_fields_in_a_record_survive_a_heartbeat() {
        let dir = TempDir::new("lock-forward");
        let path = lock_path(&dir);
        let newer = r#"{"schemaVersion":1,"pid":111,"startedAt":"2026-09-07T10:00:00Z",
            "heartbeatAt":"2026-09-07T10:00:00Z","writerBuild":"9.9.9"}"#;
        std::fs::write(&path, newer).unwrap();

        let record = read_record(&path).unwrap();
        assert_eq!(record.extra["writerBuild"], Value::from("9.9.9"));
        let rewritten: LockRecord = serde_json::from_str(&render(&record).unwrap()).unwrap();
        assert_eq!(rewritten, record);
    }
}
