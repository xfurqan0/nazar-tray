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
//! Two things, and the cheap one is asked first.
//!
//! 1. **Is the process there?** [`crate::process`] asks the operating system about the
//!    record's `pid`. A pid the kernel says nothing owns is a holder that is **gone**, and
//!    its record is stale the instant it is read — not five minutes later.
//! 2. **Otherwise, the heartbeat.** The holder is awake every sixty seconds anyway and
//!    rewrites the file while it is there, so a record whose heartbeat is older than
//!    [`STALE_AFTER`] is reclaimable. This is what catches the case a liveness probe
//!    cannot: a holder still running but wedged stops beating and gets replaced, where a
//!    probe alone would wait for ever on a process that will never write again.
//!
//! The probe only ever **shortens** the wait, and it only speaks when it is certain. "No
//! permission to look", "no probe on this platform", "the call failed" are all
//! [`Presence::Unknown`], and unknown is not dead: the heartbeat decides, exactly as it did
//! before the probe existed. Guessing the other way would put two writers on one
//! `limits.json`, which is the failure this whole module is here to prevent.
//!
//! Why it was worth adding, having once been argued against: **an upgrade kills the tray.**
//! The NSIS installer terminates the running process and then offers to launch the new one,
//! seconds later — and the new one used to find a lock whose heartbeat was fresh, say "it
//! is already running", ask a dead process to show a panel, and exit. No tray, no icon, no
//! error anybody could act on, for five minutes, for every upgrading user. Measured on the
//! maintainer's machine upgrading 0.1.0 to 0.2.0.
//!
//! **The pid-reuse guard.** A pid the kernel *does* own is still not proof that it is the
//! same process: ids are reused, quickly on Windows after a kill. So when the platform can
//! name a process's creation time, a holder whose process started **after** the record says
//! it did is a reused id, and the record is stale. When the creation time cannot be read —
//! every POSIX platform — the heartbeat decides, which is the conservative answer.
//!
//! `pid` and `startedAt` are also what a holder recognises its own record by, and what the
//! human reading `~/.nazar` by hand wants first: which process, since when.
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
use crate::process::{BlindProbe, Presence, Probe, SystemProbe};
use crate::timefmt::{seconds_between, unix_seconds_from_rfc3339};

/// Version of the lock record this build writes.
pub const LOCK_SCHEMA_VERSION: u32 = 1;

/// How long a record may go without a heartbeat before it may be reclaimed.
///
/// Five refresh ticks. Long enough that a machine grinding through a big build does not
/// lose its lock to itself, short enough that a tray killed with the task manager does not
/// keep the next one out for a coffee break.
pub const STALE_AFTER: Duration = Duration::from_secs(5 * 60);

/// How much later than its own record a holder's process may have started and still be
/// believed.
///
/// `startedAt` is written from [`crate::clock::process_start_rfc3339`], which is the first
/// time anything in the process asked the clock — a few milliseconds *after* the kernel
/// created it. So a genuine holder's creation time is at or before what its record claims,
/// and the only way round it is a reused id. The slack is for the second the two readings
/// can be floored either side of, not for a doubt about whose process it is.
const START_SLACK: i64 = 2;

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
    swept: Option<Box<LockRecord>>,
}

impl LimitsLock {
    /// Try to become the writer of `path`.
    ///
    /// `now` is RFC 3339 in UTC and is both the heartbeat this process writes and the
    /// instant an existing record is judged against.
    pub fn acquire(path: &Path, now: &str) -> Result<Acquisition> {
        LimitsLock::acquire_probing(
            path,
            std::process::id(),
            &crate::clock::process_start_rfc3339(),
            now,
            STALE_AFTER,
            &SystemProbe,
        )
    }

    /// [`acquire`](LimitsLock::acquire) with everything about "this process" handed in, and
    /// nothing asked of the operating system.
    ///
    /// The tests' way in: two "processes" in one test binary need two identities, and a
    /// stale record needs a threshold that does not take five minutes to reach. Those
    /// identities are invented numbers that name nothing on the machine running the tests,
    /// so this entry probes with [`BlindProbe`] — every holder is
    /// [`Presence::Unknown`] and the heartbeat is the only judge, which is what this
    /// function meant before there was a probe at all. A caller that wants the real answer
    /// wants [`acquire`](LimitsLock::acquire) or
    /// [`acquire_probing`](LimitsLock::acquire_probing).
    pub fn acquire_as(
        path: &Path,
        pid: u32,
        started_at: &str,
        now: &str,
        stale_after: Duration,
    ) -> Result<Acquisition> {
        LimitsLock::acquire_probing(path, pid, started_at, now, stale_after, &BlindProbe)
    }

    /// [`acquire_as`](LimitsLock::acquire_as) with the probe handed in too.
    ///
    /// The whole acquisition, with nothing implicit: which process is claiming, when it
    /// started, what time it is, how much silence is too much, and who to ask about a
    /// holder that is already there.
    pub fn acquire_probing(
        path: &Path,
        pid: u32,
        started_at: &str,
        now: &str,
        stale_after: Duration,
        probe: &dyn Probe,
    ) -> Result<Acquisition> {
        Ok(
            match claim(path, pid, started_at, now, stale_after, probe)? {
                Ok((record, swept)) => Acquisition::Held(LimitsLock {
                    path: path.to_path_buf(),
                    record,
                    swept: swept.map(Box::new),
                }),
                Err(holder) => Acquisition::Taken(holder),
            },
        )
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

    /// The record this acquisition deleted on its way in, if there was one.
    ///
    /// `Some` means a previous holder left its lock behind — killed by an installer
    /// upgrade, by Task Manager, by a power cut. It is worth one line on stderr, because
    /// "the tray took over somebody else's lock" is the whole explanation for a restart
    /// that otherwise looks like nothing happened at all.
    #[must_use]
    pub fn swept(&self) -> Option<&LockRecord> {
        self.swept.as_deref()
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
                    &SystemProbe,
                )?;
                return Ok(match claimed {
                    Ok((record, _)) => {
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

/// What a claim came back with.
///
/// `Ok` is ours: the record written, and the record deleted to make room for it. `Err` is
/// somebody else's, and carries what they wrote about themselves when it could be read.
type Claimed = std::result::Result<(LockRecord, Option<LockRecord>), Option<LockRecord>>;

/// The acquisition itself, without the guard that would release it.
///
/// `Ok((record, swept))` means the file is ours and holds that record, and `swept` is the
/// record this claim deleted to get there — the previous holder, when there was a readable
/// one. `Err(holder)` means somebody else has it, and `holder` is what they wrote about
/// themselves when that could be read.
fn claim(
    path: &Path,
    pid: u32,
    started_at: &str,
    now: &str,
    stale_after: Duration,
    probe: &dyn Probe,
) -> Result<Claimed> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
    }

    let mut swept = None;
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
                return Ok(Ok((record, swept)));
            }
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                let existing = read_record(path);
                if reclaimable(path, existing.as_ref(), now, stale_after, probe) {
                    // Best effort: if the delete fails, the next attempt reads the record
                    // again and reports it as taken rather than spinning.
                    let _ = std::fs::remove_file(path);
                    swept = existing.or(swept);
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
/// Three ways in:
///
/// * the record names a process the operating system says is **gone** — the fast path, and
///   the one an upgrade needs, because a killed tray's heartbeat is seconds old and its
///   pid is nobody's;
/// * the record is readable and its heartbeat is older than `stale_after`;
/// * the record is **not** readable, and the file itself has not been touched for
///   `stale_after` either. The third case is what makes the acquisition race safe: between
///   `create_new` and the record being written there is a moment when the file is empty,
///   and a competitor that read it then must wait rather than evict a holder that is one
///   instruction from being alive. There is no pid to probe in that moment either, which is
///   the same reason.
fn reclaimable(
    path: &Path,
    record: Option<&LockRecord>,
    now: &str,
    stale_after: Duration,
    probe: &dyn Probe,
) -> bool {
    match record {
        Some(record) => holder_is_gone(record, probe) || record.is_stale(now, stale_after),
        None => match file_age_seconds(path, now) {
            Some(age) => age > stale_after.as_secs() as i64,
            None => false,
        },
    }
}

/// Whether the operating system says the process that wrote `record` is no longer there.
///
/// Only a **definite** answer counts. `Unknown` — no permission to look, no probe on this
/// platform, a call that failed — is not "gone"; it hands the decision back to the
/// heartbeat, which is where it was before this function existed.
fn holder_is_gone(record: &LockRecord, probe: &dyn Probe) -> bool {
    match probe.presence(record.pid) {
        Presence::Gone => true,
        Presence::Unknown => false,
        Presence::Running => started_after_its_own_record(record, probe),
    }
}

/// The pid-reuse guard: a running process that started later than the record claims is not
/// the process that wrote it.
///
/// `false` whenever either instant is unreadable, because "cannot tell" must not evict
/// anybody. See [`START_SLACK`] for why later-by-two-seconds is still the same process.
fn started_after_its_own_record(record: &LockRecord, probe: &dyn Probe) -> bool {
    let (Some(actual), Some(claimed)) = (
        probe.start_seconds(record.pid),
        unix_seconds_from_rfc3339(&record.started_at),
    ) else {
        return false;
    };
    actual > claimed + START_SLACK
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
        assert!(
            first.swept().is_none(),
            "a lock taken on an empty directory displaced nobody"
        );

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

    // ------------------------------------------------- the holder the operating system knows

    /// A process that has certainly finished, and the id it had.
    fn a_finished_process() -> u32 {
        let mut child = if cfg!(windows) {
            std::process::Command::new("cmd")
                .args(["/C", "exit"])
                .spawn()
        } else {
            std::process::Command::new("sh")
                .args(["-c", "exit"])
                .spawn()
        }
        .expect("a shell to spawn");
        let pid = child.id();
        child.wait().expect("the child to finish");
        pid
    }

    /// Write `record` to `path` as a holder would have.
    fn lay_down(path: &Path, record: &LockRecord) {
        std::fs::write(path, render(record).unwrap()).unwrap();
    }

    /// **The upgrade bug.** The installer kills the tray and launches the new one seconds
    /// later; the heartbeat is fresh and the process is gone, and the old rule believed the
    /// heartbeat. Five minutes of no tray at all, for every upgrading user.
    #[test]
    fn a_lock_whose_process_is_gone_is_taken_over_at_once() {
        let dir = TempDir::new("lock-dead-pid");
        let path = lock_path(&dir);

        let dead = a_finished_process();
        let now = crate::timefmt::now_rfc3339();
        // Heartbeat written this instant: nothing about the record's age says anything is
        // wrong with it. Only the pid does.
        lay_down(&path, &LockRecord::new(dead, &now, &now));

        let taken = LimitsLock::acquire_probing(&path, 999, START, &now, STALE_AFTER, &SystemProbe)
            .unwrap();
        let lock = taken
            .held()
            .expect("a lock whose process has exited must be reclaimable at once");
        assert_eq!(lock.record().pid, 999);
        assert_eq!(read_record(&path).unwrap().pid, 999);
        assert_eq!(
            lock.swept().map(|record| record.pid),
            Some(dead),
            "the lock remembers whose record it deleted, so the tray can say so"
        );
    }

    /// The other half of the same rule, and the one that keeps "one writer" true.
    #[test]
    fn a_lock_whose_process_is_running_is_left_alone() {
        let dir = TempDir::new("lock-live-pid");
        let path = lock_path(&dir);

        let now = crate::timefmt::now_rfc3339();
        let mine = std::process::id();
        lay_down(
            &path,
            &LockRecord::new(mine, crate::clock::process_start_rfc3339(), &now),
        );

        let taken = LimitsLock::acquire_probing(&path, 999, START, &now, STALE_AFTER, &SystemProbe)
            .unwrap();
        match taken {
            Acquisition::Held(_) => panic!("a running holder must not be pushed aside"),
            Acquisition::Taken(Some(record)) => assert_eq!(record.pid, mine),
            Acquisition::Taken(None) => panic!("the loser should be told who holds it"),
        }
    }

    /// A pid that is alive but is not the one that wrote the record: the id was reused.
    ///
    /// Windows only, because it is the only platform here that can name a process's
    /// creation time. Everywhere else the guard does not fire and the heartbeat decides,
    /// which is what the test below asserts.
    #[cfg(windows)]
    #[test]
    fn a_running_pid_that_started_after_its_record_is_a_reused_id() {
        let dir = TempDir::new("lock-reused-pid");
        let path = lock_path(&dir);

        let now = crate::timefmt::now_rfc3339();
        // This process is alive and its id is in the record — but the record says the
        // holder started in 2000, and this process did not.
        lay_down(
            &path,
            &LockRecord::new(std::process::id(), "2000-01-01T00:00:00Z", &now),
        );

        let taken = LimitsLock::acquire_probing(&path, 999, START, &now, STALE_AFTER, &SystemProbe)
            .unwrap();
        assert!(
            taken.held().is_some(),
            "a live pid whose process is younger than its own record is a reused id"
        );
    }

    /// "Cannot tell" is not "dead".
    ///
    /// The pid in this record belongs to nothing at all, and a probe that knows it would
    /// evict the holder on the spot. [`BlindProbe`] is every platform without a probe and
    /// every process the operating system will not discuss, and it must leave the record
    /// exactly where the heartbeat left it.
    #[test]
    fn a_holder_the_probe_cannot_place_is_judged_by_its_heartbeat_alone() {
        let dir = TempDir::new("lock-unknown-pid");
        let path = lock_path(&dir);

        let dead = a_finished_process();
        let now = crate::timefmt::now_rfc3339();
        lay_down(&path, &LockRecord::new(dead, &now, &now));

        let taken =
            LimitsLock::acquire_probing(&path, 999, START, &now, STALE_AFTER, &BlindProbe).unwrap();
        assert!(
            matches!(taken, Acquisition::Taken(Some(_))),
            "an unknown presence must not evict a fresh heartbeat"
        );
        assert_eq!(read_record(&path).unwrap().pid, dead);

        // And five minutes on, the heartbeat does what it always did.
        let later = crate::timefmt::rfc3339_from_unix_seconds(
            crate::timefmt::unix_seconds_from_rfc3339(&now).unwrap() + 331,
        );
        let taken =
            LimitsLock::acquire_probing(&path, 999, START, &later, STALE_AFTER, &BlindProbe)
                .unwrap();
        assert!(taken.held().is_some(), "the heartbeat still ages out");
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

    // ------------------------------------------------------- the grid over the heartbeat

    /// The heartbeat threshold, walked one named point at a time.
    ///
    /// The same shape of test as `alerts::tests::the_same_period_grid_over_a_weekly_window`
    /// and for the same reason: this is a subtraction against a threshold, and the way a
    /// subtraction against a threshold goes wrong is **at the threshold** or at one of the
    /// handful of instants where arithmetic on time is not arithmetic on numbers. So the
    /// points are named rather than generated — no `proptest`, nothing random, the same
    /// sixteen answers on every machine and every run.
    ///
    /// What is being protected: a holder evicted one second early loses `limits.json` to a
    /// second writer, and a holder never evicted leaves the file unwritten until a reboot.
    #[test]
    fn the_heartbeat_grid_around_the_five_minute_threshold() {
        let beat = "2026-09-07T10:00:00Z";
        let record = LockRecord::new(4242, START, beat);
        let stale_after = STALE_AFTER.as_secs() as i64;

        let at = |silence: i64| {
            crate::timefmt::rfc3339_from_unix_seconds(
                crate::timefmt::unix_seconds_from_rfc3339(beat).unwrap() + silence,
            )
        };

        for (silence, expected, why) in [
            (0, false, "the heartbeat is this instant"),
            (1, false, "one second of silence"),
            (59, false, "just under a minute"),
            (61, false, "just over a minute"),
            (stale_after - 1, false, "one second inside the grace"),
            (
                stale_after,
                false,
                "exactly five minutes is not more than five",
            ),
            (stale_after + 1, true, "one second past it, and no longer"),
            (3600, true, "an hour"),
            (86_400, true, "a day"),
            (-1, false, "a heartbeat one second in the future"),
            (-3600, false, "a clock corrected backwards by an hour"),
            (-86_400, false, "and by a day: still not a reason to evict"),
        ] {
            assert_eq!(
                record.is_stale(&at(silence), STALE_AFTER),
                expected,
                "{silence} s of silence: {why}"
            );
            assert_eq!(
                record.silence_seconds(&at(silence)),
                Some(silence),
                "{silence} s of silence: the gap is signed, not absolute"
            );
        }
    }

    /// The instants where arithmetic on time is not arithmetic on numbers.
    #[test]
    fn the_heartbeat_grid_where_arithmetic_on_instants_goes_wrong() {
        // A zone offset names an instant. A holder that wrote its heartbeat with a local
        // offset — which nothing in this crate does, and a future writer might — is
        // measured on the instant rather than on the digits.
        let offset_beat = LockRecord::new(1, START, "2026-09-07T13:00:00+03:00");
        assert_eq!(
            offset_beat.silence_seconds("2026-09-07T10:00:00Z"),
            Some(0),
            "13:00+03:00 is 10:00Z, so no time has passed at all"
        );
        assert!(!offset_beat.is_stale("2026-09-07T10:04:00Z", STALE_AFTER));
        assert!(offset_beat.is_stale("2026-09-07T10:06:00Z", STALE_AFTER));

        // The night the clocks go forward. Nothing happens in UTC, and the lock is UTC.
        let dst = LockRecord::new(1, START, "2026-03-29T00:59:59Z");
        assert_eq!(dst.silence_seconds("2026-03-29T01:00:00Z"), Some(1));
        assert!(!dst.is_stale("2026-03-29T01:00:00Z", STALE_AFTER));
        // Two local times written on either side of the hour that does not exist locally,
        // two hours and a minute apart on their faces, one minute apart in reality:
        // 01:59+01:00 is 00:59Z and 04:00+03:00 is 01:00Z.
        assert_eq!(
            LockRecord::new(1, START, "2026-03-29T01:59:00+01:00")
                .silence_seconds("2026-03-29T04:00:00+03:00"),
            Some(60)
        );

        // Before the epoch, where a division that rounded towards zero would go wrong.
        let ancient = LockRecord::new(1, START, "1969-12-31T23:59:59Z");
        assert_eq!(ancient.silence_seconds("1970-01-01T00:00:00Z"), Some(1));
        assert!(!ancient.is_stale("1970-01-01T00:00:00Z", STALE_AFTER));
        assert!(ancient.is_stale("1970-01-01T00:06:00Z", STALE_AFTER));

        // Fractional seconds are read and dropped; a heartbeat is not measured in them.
        assert_eq!(
            LockRecord::new(1, START, "2026-09-07T10:00:00.999Z")
                .silence_seconds("2026-09-07T10:00:01Z"),
            Some(1)
        );

        // Text that is not an instant is "cannot tell", and cannot tell is never "dead":
        // a damaged record is reclaimed by the age of its file instead, which is the one
        // measure that does not depend on believing what the record says.
        for unreadable in [
            "",
            "just now",
            "2026-09-07T10:00:00",
            "2026-02-30T10:00:00Z",
            "2026-09-07 10:00:00Z",
        ] {
            let record = LockRecord::new(1, START, unreadable);
            assert_eq!(record.silence_seconds("2026-09-07T10:00:00Z"), None);
            assert!(
                !record.is_stale("2027-01-01T00:00:00Z", STALE_AFTER),
                "{unreadable:?} must not be evicted by this rule"
            );
        }
        // And a `now` nobody can read is the same answer from the other side.
        let live = LockRecord::new(1, START, "2026-09-07T10:00:00Z");
        assert_eq!(live.silence_seconds("whenever"), None);
        assert!(!live.is_stale("whenever", STALE_AFTER));
    }

    /// A threshold of zero, and one of a whole day, mean what they say.
    ///
    /// `STALE_AFTER` is a constant today and a setting the moment somebody needs it to be;
    /// the rule must not have five minutes baked into it anywhere but the constant.
    #[test]
    fn the_heartbeat_threshold_is_the_one_it_is_given() {
        let record = LockRecord::new(1, START, "2026-09-07T10:00:00Z");

        assert!(!record.is_stale("2026-09-07T10:00:00Z", Duration::from_secs(0)));
        assert!(record.is_stale("2026-09-07T10:00:01Z", Duration::from_secs(0)));

        let a_day = Duration::from_secs(86_400);
        assert!(!record.is_stale("2026-09-08T09:59:59Z", a_day));
        assert!(!record.is_stale("2026-09-08T10:00:00Z", a_day));
        assert!(record.is_stale("2026-09-08T10:00:01Z", a_day));
    }
}
