//! Asking the operating system whether a process id still names a process.
//!
//! This module exists for exactly one caller — [`crate::lock`], deciding whether the record
//! in `~/.nazar/limits.lock` was written by something that is still there — and its whole
//! design follows from what that caller does with the answer.
//!
//! **Three answers, not two.** A probe that had to choose between "running" and "gone"
//! would have to guess in the cases where the operating system declines to say: a process
//! owned by another account, a platform with no probe at all. Guessing "gone" there means
//! two writers on one `limits.json`, which is the exact failure the lock exists to prevent.
//! So [`Presence::Unknown`] is a real answer, and the lock treats it as "carry on as
//! before" — the heartbeat decides, the way it always did.
//!
//! **No new dependency.** `windows-sys` is already in the lock file, three levels down
//! under Tauri, and adding it here would still be a platform crate in the crate whose claim
//! is that it builds and tests everywhere on two dependencies. Four `kernel32` entry points
//! and one `libc` one are less code than the feature list that selecting them from a crate
//! would need, and they are the only `unsafe` in `nazar-core`.
//!
//! ```text
//!   presence(pid)
//!        ├── Running   the pid names a process that has not exited
//!        ├── Gone      the operating system says there is no such process
//!        └── Unknown   it would not say
//! ```
//!
//! [`start_seconds`] answers the second question a stale lock raises: process ids are
//! reused, so a *running* pid is not proof that the process which wrote the record is the
//! one running under it now. A creation time older than the record settles it. Windows
//! answers that one; the POSIX probe does not, and says so by returning `None`.

/// What the operating system says about a process id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// The id names a process that has not exited.
    Running,
    /// The operating system says there is no such process.
    Gone,
    /// It would not say: no permission to look, or no probe on this platform.
    Unknown,
}

/// Whether `pid` still names a process.
///
/// Never panics and never blocks: one system call, and anything unexpected is
/// [`Presence::Unknown`].
#[must_use]
pub fn presence(pid: u32) -> Presence {
    platform::presence(pid)
}

/// When the process `pid` names was created, as seconds since the Unix epoch.
///
/// `None` when the platform cannot be asked, when the process is gone, or when the call
/// fails. A caller that gets `None` has learnt nothing and must not treat it as a reading.
#[must_use]
pub fn start_seconds(pid: u32) -> Option<i64> {
    platform::start_seconds(pid)
}

/// What [`crate::lock`] asks about a holder, behind a trait so the tests can answer for it.
///
/// The lock's own tests run several "processes" inside one test binary, with invented ids
/// that name nothing on the machine running them. They need a probe that knows nothing;
/// the application needs one that asks the kernel. Both are here.
pub trait Probe {
    /// Whether `pid` still names a process.
    fn presence(&self, pid: u32) -> Presence;

    /// When that process was created, as seconds since the Unix epoch, if that is knowable.
    fn start_seconds(&self, _pid: u32) -> Option<i64> {
        None
    }
}

/// The probe that asks the operating system. What the application runs with.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemProbe;

impl Probe for SystemProbe {
    fn presence(&self, pid: u32) -> Presence {
        presence(pid)
    }

    fn start_seconds(&self, pid: u32) -> Option<i64> {
        start_seconds(pid)
    }
}

/// The probe that knows nothing, and is honest about it.
///
/// Every answer is [`Presence::Unknown`], which is what the lock behaved like before this
/// module existed. It is what [`crate::lock::LimitsLock::acquire_as`] uses, and it is the
/// shape of a platform nobody has written a probe for.
#[derive(Debug, Clone, Copy, Default)]
pub struct BlindProbe;

impl Probe for BlindProbe {
    fn presence(&self, _pid: u32) -> Presence {
        Presence::Unknown
    }
}

// --------------------------------------------------------------------------------- Windows

#[cfg(windows)]
mod platform {
    //! `kernel32`, by hand.
    //!
    //! `OpenProcess` with `PROCESS_QUERY_LIMITED_INFORMATION` is the least a caller can ask
    //! for and the one right that is granted across integrity levels, so a tray started
    //! before an elevated shell can still be seen. Two failures are told apart because they
    //! mean opposite things: `ERROR_INVALID_PARAMETER` is "no process has that id", which
    //! is the answer this module was written for, and `ERROR_ACCESS_DENIED` is "there is
    //! one and you may not look at it", which is not a reason to evict anybody.

    use std::ffi::c_void;

    use super::Presence;

    /// `PROCESS_QUERY_LIMITED_INFORMATION`.
    const QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    /// `STILL_ACTIVE`, the exit code a process that has not exited reports.
    const STILL_ACTIVE: u32 = 259;
    /// `ERROR_INVALID_PARAMETER`, which is what naming a pid that nothing owns produces.
    const ERROR_INVALID_PARAMETER: i32 = 87;

    /// `FILETIME`: 100-nanosecond ticks since 1601-01-01 UTC, split in two words.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit_handle: i32, pid: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetExitCodeProcess(handle: *mut c_void, code: *mut u32) -> i32;
        fn GetProcessTimes(
            handle: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }

    /// An open handle that closes itself. Leaking one would be a handle leak in a process
    /// that runs for weeks and probes on every acquisition.
    struct Opened(*mut c_void);

    impl Opened {
        /// `Err` carries the operating system's error number, which is the whole answer in
        /// the `ERROR_INVALID_PARAMETER` case.
        fn process(pid: u32) -> Result<Opened, Option<i32>> {
            // SAFETY: a call with no pointer arguments. A failure returns null, which is
            // checked before the handle is used or closed.
            let handle = unsafe { OpenProcess(QUERY_LIMITED_INFORMATION, 0, pid) };
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().raw_os_error());
            }
            Ok(Opened(handle))
        }
    }

    impl Drop for Opened {
        fn drop(&mut self) {
            // SAFETY: the handle is non-null (checked in `process`) and is closed once,
            // here, because nothing else owns it.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub(super) fn presence(pid: u32) -> Presence {
        let opened = match Opened::process(pid) {
            Ok(opened) => opened,
            Err(Some(ERROR_INVALID_PARAMETER)) => return Presence::Gone,
            // Access denied, and anything else: there may well be a process there.
            Err(_) => return Presence::Unknown,
        };

        let mut code = 0u32;
        // SAFETY: a live handle from `OpenProcess`, and a pointer to a `u32` this frame
        // owns, which is what the signature asks for.
        if unsafe { GetExitCodeProcess(opened.0, &mut code) } == 0 {
            return Presence::Unknown;
        }
        // A process that exited with 259 is indistinguishable from one that is running.
        // That is the documented shape of this API; the cost of the ambiguity here is one
        // heartbeat window of waiting, which is what the behaviour was before the probe.
        if code == STILL_ACTIVE {
            Presence::Running
        } else {
            Presence::Gone
        }
    }

    pub(super) fn start_seconds(pid: u32) -> Option<i64> {
        let opened = Opened::process(pid).ok()?;
        let mut creation = FileTime::default();
        let mut exit = FileTime::default();
        let mut kernel = FileTime::default();
        let mut user = FileTime::default();
        // SAFETY: a live handle, and four pointers to `FileTime` values this frame owns.
        // All four are out parameters and all four are required by the signature.
        let read =
            unsafe { GetProcessTimes(opened.0, &mut creation, &mut exit, &mut kernel, &mut user) };
        if read == 0 {
            return None;
        }
        Some(unix_seconds(creation))
    }

    /// A `FILETIME` as seconds since the Unix epoch.
    fn unix_seconds(at: FileTime) -> i64 {
        /// Ticks of 100 nanoseconds in a second.
        const TICKS: u64 = 10_000_000;
        /// Seconds between 1601-01-01 and 1970-01-01.
        const EPOCH_DIFFERENCE: i64 = 11_644_473_600;

        let ticks = (u64::from(at.high) << 32) | u64::from(at.low);
        i64::try_from(ticks / TICKS).unwrap_or(i64::MAX) - EPOCH_DIFFERENCE
    }
}

// ----------------------------------------------------------------------------------- POSIX

#[cfg(all(unix, not(windows)))]
mod platform {
    //! `kill(pid, 0)`: the signal that is not sent.
    //!
    //! It performs the permission check and the existence check and then does nothing, so
    //! `ESRCH` is "no such process" and `EPERM` is "there is one, and it is not yours" —
    //! which is still a process, and still a reason not to evict its lock.
    //!
    //! There is no portable POSIX call for a process's creation time, so
    //! [`start_seconds`](super::start_seconds) answers `None` here and the pid-reuse guard
    //! in [`crate::lock`] falls back to the heartbeat. On these platforms the tray is not
    //! shipped yet; the wrapper and the core are, and they take the same lock.

    use super::Presence;

    /// `ESRCH`.
    const NO_SUCH_PROCESS: i32 = 3;
    /// `EPERM`.
    const NOT_PERMITTED: i32 = 1;

    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }

    pub(super) fn presence(pid: u32) -> Presence {
        let Ok(pid) = i32::try_from(pid) else {
            return Presence::Unknown;
        };
        // 0 means "every process in my group" and a negative number means a process group.
        // Neither is a holder, and asking would probe something else entirely.
        if pid <= 0 {
            return Presence::Unknown;
        }
        // SAFETY: two integers by value, no pointers, and signal 0 sends nothing.
        if unsafe { kill(pid, 0) } == 0 {
            return Presence::Running;
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(NO_SUCH_PROCESS) => Presence::Gone,
            Some(NOT_PERMITTED) => Presence::Running,
            _ => Presence::Unknown,
        }
    }

    pub(super) fn start_seconds(_pid: u32) -> Option<i64> {
        None
    }
}

// ------------------------------------------------------------------------------ everywhere
// else

#[cfg(not(any(windows, unix)))]
mod platform {
    //! No probe. Every answer is the one the lock had before this module existed.

    use super::Presence;

    pub(super) fn presence(_pid: u32) -> Presence {
        Presence::Unknown
    }

    pub(super) fn start_seconds(_pid: u32) -> Option<i64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// A process that has certainly finished, and the id it had.
    ///
    /// Reaped before it is returned, so the answer under test is about a process that is
    /// gone rather than one that is about to be.
    fn a_finished_process() -> u32 {
        let mut child = if cfg!(windows) {
            Command::new("cmd").args(["/C", "exit"]).spawn()
        } else {
            Command::new("sh").args(["-c", "exit"]).spawn()
        }
        .expect("a shell to spawn");
        let pid = child.id();
        child.wait().expect("the child to finish");
        pid
    }

    #[test]
    fn this_process_is_running() {
        assert_eq!(presence(std::process::id()), Presence::Running);
    }

    #[test]
    fn a_process_that_has_exited_is_gone() {
        let pid = a_finished_process();
        assert_eq!(
            presence(pid),
            Presence::Gone,
            "pid {pid} exited before it was asked about"
        );
    }

    #[test]
    fn the_blind_probe_never_claims_to_know() {
        let blind = BlindProbe;
        assert_eq!(blind.presence(std::process::id()), Presence::Unknown);
        assert_eq!(blind.presence(a_finished_process()), Presence::Unknown);
        assert_eq!(blind.start_seconds(std::process::id()), None);
    }

    #[test]
    fn the_system_probe_answers_what_the_free_functions_answer() {
        let system = SystemProbe;
        let mine = std::process::id();
        assert_eq!(system.presence(mine), presence(mine));
        assert_eq!(system.start_seconds(mine), start_seconds(mine));
    }

    /// Windows can name a process's creation time; the POSIX probe cannot, and says `None`
    /// rather than something it worked out.
    #[test]
    fn this_process_started_before_now_and_not_in_the_last_century() {
        let started = start_seconds(std::process::id());
        if cfg!(windows) {
            let started = started.expect("Windows can read a creation time");
            let now = crate::timefmt::unix_seconds_from_rfc3339(&crate::timefmt::now_rfc3339())
                .expect("the clock to name an instant");
            assert!(
                started <= now + 2,
                "a process cannot have started after now: {started} vs {now}"
            );
            assert!(
                started > now - 86_400,
                "a test binary that started a day ago is a reading that is wrong: {started}"
            );
        } else {
            assert_eq!(started, None, "there is no portable POSIX creation time");
        }
    }
}
