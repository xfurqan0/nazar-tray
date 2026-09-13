//! `get_usage` — the one question the panel asks about the usage store.
//!
//! T-WP13 built the store: a scan of Claude Code's transcripts, deduplicated, folded into
//! hourly UTC buckets under `<settings dir>/usage/YYYY-MM.json`. `docs/usage-contract.md`
//! is the document it writes. Nothing read it. This is the bridge, and it is deliberately
//! thin: it decides **when** a scan may run, hands the store's own answer back unchanged,
//! and turns the three ways this can fail into something a panel can put on screen.
//!
//! ```text
//!   panel  ──▶ get_usage { range, from, to, force }
//!                  │
//!                  ├── validate the range name and the two instants
//!                  ├── scan   (writer only · at most every five minutes · or force)
//!                  └── query  ──▶ hourly buckets, exactly as the store holds them
//! ```
//!
//! # Three rules, and the reason for each
//!
//! **1. The window arrives from the panel, already in UTC.** A week starts on Monday in the
//! reader's own time zone, and this crate is not allowed to know what that is —
//! `crates/nazar-core/tests/hygiene.rs` greps the whole workspace for the names a time-zone
//! conversion would have to use and fails the build on a hit. So the panel, which is
//! JavaScript and gets a zone offset for one call, computes the two instants and sends
//! them. `range` comes along as a **name**, not as arithmetic: it is echoed back so the
//! answer says which question it answers, and it is validated so a panel that invents a
//! fourth range gets an error rather than a silent empty view.
//!
//! **2. The scan is not on the refresh path.** Quota is why this application exists; it
//! reads two small files in milliseconds and must never queue behind a scan that reads
//! hundreds of megabytes. So the refresh loop does not scan at all. This does, when the
//! panel opens the usage view, and [`Throttle`] is what keeps "when the panel opens it"
//! from meaning "every time somebody clicks the tray icon": at most one scan every five
//! minutes, unless the caller forces one.
//!
//! **3. One writer.** The scan writes to the same `<settings dir>/usage/` that `limits.json`'s
//! single-writer rule protects, and it runs only in the process that holds
//! `~/.nazar/limits.lock`. A second tray instance — or one whose lock was reclaimed while
//! its machine slept — still *reads* the store and draws the view; it simply does not add to
//! it. That is the same "one writer, many readers" the contract rests on, with no second
//! lock and no second discipline.
//!
//! # What this module does not do
//!
//! No user interface: T-WP16 draws the view. No Codex scan: `nazar_core::usage` has no
//! `scan_codex` yet — T-WP14 is writing it — and the one line that will call it is marked
//! below rather than guessed at.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use nazar_core::clock::{Clock, SystemClock};
use nazar_core::error::Error;
use nazar_core::paths;
use nazar_core::usage::{Hours, UsageSummary};
use serde::{Deserialize, Serialize};

/// The range names the panel may ask for.
///
/// A list rather than an enum on the wire, because the answer echoes the string back and an
/// enum would put a Rust spelling in a JSON document that `docs/usage-contract.md` already
/// spells out. The validation is the same either way; this way the error message can name
/// what was actually sent.
pub const RANGES: [&str; 3] = ["week", "month", "all"];

/// How long the store is left alone after a scan, in milliseconds.
///
/// Five minutes, from `docs/usage-contract.md`: *"It runs once at start-up, when the usage
/// view is opened, and at most once every five minutes."* The same five minutes as
/// [`nazar_core::lock::STALE_AFTER`] by coincidence rather than by design — one is about a
/// dead process, this is about a big read — so they are two constants.
pub const SCAN_INTERVAL_MS: u64 = 5 * 60 * 1000;

/// What the panel sends.
///
/// `from` and `to` are RFC 3339 instants in UTC, half-open: an hour is in the answer when
/// **its start** falls inside `[from, to)`. The panel computes them because it is the only
/// side that may ask the machine which zone it is in; see rule 1 above.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageRequest {
    /// `week`, `month` or `all`. Echoed back; see [`RANGES`].
    pub range: String,
    /// The first instant of the window, RFC 3339 UTC.
    pub from: String,
    /// The instant the window stops before, RFC 3339 UTC.
    pub to: String,
    /// Skip the throttle and scan now. The panel's own Refresh button, and nothing else.
    #[serde(default)]
    pub force: bool,
}

/// What one scan of the transcripts did, for a diagnostic line.
///
/// Four of [`UsageSummary`]'s twenty fields: enough to say *"412 files, 90 210 lines, 51 344
/// duplicates, 1.8 s"* and not enough to be a second contract. The rest stays in the core
/// crate, where the tests that prove it live.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageScan {
    /// Transcript files found.
    pub files_seen: u64,
    /// Lines that carried a usage object.
    pub lines: u64,
    /// Copies of a message that had already been counted.
    pub duplicates: u64,
    /// How long the scan took, in milliseconds.
    pub took_ms: u64,
}

impl UsageScan {
    /// The four numbers the panel shows, taken out of the summary.
    fn of(summary: &UsageSummary, took_ms: u64) -> Self {
        UsageScan {
            files_seen: summary.files_seen,
            lines: summary.lines,
            duplicates: summary.duplicates,
            took_ms,
        }
    }
}

/// The answer.
///
/// **Every key here is `snake_case`, and that is deliberate.** The rest of this application's
/// bridge is camelCase, because `limits.json` is a contract with another program and renames
/// what it reads into one spelling. The usage store does the opposite on purpose — its
/// counters keep the shape `message.usage` already has, so a reader comparing this document
/// with a raw transcript line does not have to hold a rename in their head — and
/// `docs/usage-contract.md` argues that trade at length. Renaming it on the way to the panel
/// would mean this application spelled the same five counters two ways in two files, which is
/// the thing the contract was written to avoid.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageResponse {
    /// The range that was asked for, as it was asked for.
    pub range: String,
    /// The first instant of the window, as it was asked for.
    pub from: String,
    /// The instant the window stops before, as it was asked for.
    pub to: String,
    /// The earliest instant the store holds anything for, RFC 3339 UTC.
    ///
    /// The panel's *since {date}* line, and the reason "all time" is an honest label. Absent
    /// on a machine whose store is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// When a scan last wrote to the store, RFC 3339 UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanned_at: Option<String>,
    /// Provider (`claude`, `codex`) to UTC hour `YYYY-MM-DDTHH` to model to its counters.
    ///
    /// Handed back exactly as the store holds it. The cutting into local days, and into
    /// weeks that start on a Monday, happens in the panel — that is the whole reason the
    /// grain is an hour.
    pub providers: BTreeMap<String, Hours>,
    /// The `YYYY-MM` months whose documents do not parse, and so are missing from the
    /// answer.
    ///
    /// **Not an error.** The months beside a damaged one still load, so the honest answer is
    /// the numbers that survive plus the name of what is missing. A damaged month is
    /// reported and left alone, never repaired and never replaced by an empty one; deleting
    /// it is a thing the user does once they know.
    pub damaged: Vec<String>,
    /// What the scan this call ran did, or absent when it did not run one.
    ///
    /// Absent means one of three things, none of which is a failure: the throttle said not
    /// yet, another instance holds the advisory lock, or the transcripts could not be
    /// located. In all three the store still answers from what it already holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan: Option<UsageScan>,
}

/// Why there is no answer.
///
/// A `kind` and a `detail`, the same shape [`crate::statusline::Outcome`] uses and for the
/// same reason. **The kind is a message key, never a sentence**: the panel looks it up in
/// its locale catalogue, so what the user reads is translated like every other word on the
/// page rather than being English in an otherwise Korean panel. The detail is the
/// diagnostic line beside it, and it names the value that was refused or repeats what a
/// reader said — the one kind of text this application does print verbatim, because no
/// catalogue can know in advance which file said what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageError {
    /// `bad_range`, `bad_window`, `no_state_dir`, `scan_failed` or `store_unreadable`.
    pub kind: String,
    /// The offending value, or the reader's own words, with the home directory collapsed.
    pub detail: String,
}

impl UsageError {
    /// An error with a detail of our own.
    fn new(kind: &str, detail: impl Into<String>) -> Self {
        UsageError {
            kind: kind.to_owned(),
            detail: detail.into(),
        }
    }

    /// An error from the core crate, with the user's name taken out of it.
    fn from_core(kind: &str, error: &Error) -> Self {
        UsageError::new(kind, describe(error))
    }
}

/// When the last scan ran, and the rule that says whether another may.
///
/// Monotonic milliseconds rather than a wall clock: a clock correction — or a laptop coming
/// back from sleep with the time set by NTP — must not make a scan due a hundred times, and
/// `Instant` is the reading that only moves forward. It is the same reasoning
/// [`nazar_core::clock`] gives for the refresh loop's scheduler.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Throttle {
    /// When the last scan started. `None` before the first one.
    last: Option<u64>,
}

impl Throttle {
    /// Whether a scan may start now.
    ///
    /// `force` is the panel's Refresh button: a user who has just finished a long session
    /// and wants to see it should not be told to wait five minutes for a file that is
    /// already on disk. It is reachable from nowhere else.
    #[must_use]
    pub fn due(&self, now_ms: u64, force: bool) -> bool {
        if force {
            return true;
        }
        match self.last {
            None => true,
            // `saturating_sub` rather than a subtraction that could wrap: the monotonic
            // clock only moves forward, and a reading that somehow went backwards has to
            // mean "not yet" rather than "immediately", which is what an underflow to
            // `u64::MAX` would have meant.
            Some(last) => now_ms.saturating_sub(last) >= SCAN_INTERVAL_MS,
        }
    }

    /// Remember that a scan started at `now_ms`.
    ///
    /// Called **whether or not the scan succeeded**. A store this build cannot read is a
    /// state the user has to fix — `cursors.json` that is no longer JSON is an error rather
    /// than a fresh start, on purpose — and retrying it on every panel open would turn one
    /// broken file into a scan on every click.
    pub fn ran(&mut self, now_ms: u64) {
        self.last = Some(now_ms);
    }
}

/// What the command needs and the application owns.
pub struct UsageState {
    /// Whether this process holds `~/.nazar/limits.lock` and may therefore write.
    ///
    /// Decided once, in `main`, by the same acquisition that decides whether this instance
    /// writes `limits.json`. A tray that loses the lock later stops writing that file at the
    /// same moment; this one keeps its answer for the life of the process, which is the
    /// conservative direction — the loser of a race never starts writing.
    writer: bool,
    /// The throttle, and the thing that makes two panel opens at once mean one scan.
    ///
    /// Held **across** the scan rather than only around the check. That serialises scans
    /// inside this process for free, which is what "one writer" means here: two threads that
    /// both found the store due would otherwise read the same transcripts twice and race for
    /// the same cursor document.
    throttle: Mutex<Throttle>,
}

impl UsageState {
    /// The state for a process that may or may not write.
    #[must_use]
    pub fn new(writer: bool) -> Self {
        UsageState {
            writer,
            throttle: Mutex::new(Throttle::default()),
        }
    }

    /// Whether this process may add to the store.
    #[must_use]
    pub fn writes(&self) -> bool {
        self.writer
    }
}

/// The usage store, for one range.
///
/// Runs a scan first when it is allowed to and one is due, then reads the store back for the
/// window the panel asked for. Marked `async` so Tauri runs it off the main thread: the scan
/// reads hundreds of megabytes on a machine with a long history, and a panel whose window
/// froze while it drew a chart would be a worse bug than a chart that took a second to
/// arrive.
#[tauri::command(async)]
pub fn get_usage(
    state: tauri::State<'_, UsageState>,
    request: UsageRequest,
) -> Result<UsageResponse, UsageError> {
    let (from, to) = window(&request)?;
    let state_dir =
        paths::settings_dir().map_err(|error| UsageError::from_core("no_state_dir", &error))?;

    let scanned = if state.writes() {
        scan_if_due(&state.throttle, &state_dir, request.force)?
    } else {
        None
    };

    let view = nazar_core::usage::query(&state_dir, &from, &to)
        .map_err(|error| UsageError::from_core("store_unreadable", &error))?;

    // A month the scan could not parse and a month the query could not parse are the same
    // month; a scan that wrote outside this window still found it, and the user should be
    // told either way.
    let mut damaged = view.damaged;
    if let Some(scanned) = scanned.as_ref() {
        damaged.extend(scanned.damaged.iter().cloned());
    }
    damaged.sort();
    damaged.dedup();

    Ok(UsageResponse {
        range: request.range,
        from,
        to,
        since: view.since,
        scanned_at: view.scanned_at,
        providers: view.providers,
        damaged,
        scan: scanned.map(|scanned| scanned.scan),
    })
}

/// What one scan of the transcripts produced.
struct Scanned {
    /// The four numbers the panel shows.
    scan: UsageScan,
    /// The `YYYY-MM` months the scan found it could not parse.
    damaged: Vec<String>,
}

/// Scan, if this process may and the throttle says it is time.
///
/// `None` when no scan ran. The lock is held for the whole call on purpose; see
/// [`UsageState::throttle`].
fn scan_if_due(
    throttle: &Mutex<Throttle>,
    state_dir: &Path,
    force: bool,
) -> Result<Option<Scanned>, UsageError> {
    let mut throttle = throttle.lock().unwrap_or_else(PoisonError::into_inner);
    let now_ms = SystemClock.monotonic_millis();
    if !throttle.due(now_ms, force) {
        return Ok(None);
    }
    let Some(home) = claude_home() else {
        // The transcripts cannot be pointed at from here; see [`home_for`]. The store still
        // answers from whatever it already holds, and the throttle is left alone so that
        // fixing the environment takes effect on the next panel open rather than in five
        // minutes.
        return Ok(None);
    };

    // Marked before the scan rather than after it: a scan that fails is a scan that ran, and
    // retrying a broken store on every panel open would be worse than the failure.
    throttle.ran(now_ms);

    let started = Instant::now();
    let summary = nazar_core::usage::scan_claude(&home, state_dir)
        .map_err(|error| UsageError::from_core("scan_failed", &error))?;
    // T-WP14 hook: `nazar_core::usage::scan_codex(codex_home, state_dir)` goes here, and its
    // summary is added to the one below. It does not exist yet — this is the only line that
    // changes when it does.
    let took_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    Ok(Some(Scanned {
        scan: UsageScan::of(&summary, took_ms),
        damaged: summary.damaged,
    }))
}

/// The window to read, once the request has been checked.
///
/// Two checks and no arithmetic — the arithmetic that turns *this week* into two instants is
/// the panel's, and rule 1 above is why.
///
/// * The **range name** has to be one this build knows. A panel asking for a fourth range is
///   a bug in the panel, and an error says so where an empty week would not.
/// * The **window** has to be two instants the core crate can read, with `to` after `from`.
///   [`nazar_core::usage::query`] answers an unreadable range with an empty view on purpose —
///   right for a store, wrong for a command whose caller would then have no idea why its
///   chart was blank.
///
/// Any legal RFC 3339 spelling is accepted, including a numeric offset, because an offset
/// names an instant unambiguously and [`nazar_core::timefmt`] converts it. What comes back
/// is always the contract's spelling — UTC, whole seconds, `…Z`, the same rule
/// `docs/limits-contract.md` rule 6 applies to every timestamp this product writes — so a
/// `+03:00` the panel happened to send does not come back out as one.
fn window(request: &UsageRequest) -> Result<(String, String), UsageError> {
    if !RANGES.contains(&request.range.as_str()) {
        return Err(UsageError::new("bad_range", format!("{:?}", request.range)));
    }

    let asked = || format!("{:?} .. {:?}", request.from, request.to);
    let (Some(from), Some(to)) = (
        nazar_core::timefmt::unix_seconds_from_rfc3339(&request.from),
        nazar_core::timefmt::unix_seconds_from_rfc3339(&request.to),
    ) else {
        return Err(UsageError::new("bad_window", asked()));
    };
    // The two instants rather than the two strings: an offset gives one moment two
    // spellings, and comparing those spellings as text puts them in the wrong order.
    if to <= from {
        return Err(UsageError::new("bad_window", asked()));
    }

    Ok((
        nazar_core::timefmt::rfc3339_from_unix_seconds(from),
        nazar_core::timefmt::rfc3339_from_unix_seconds(to),
    ))
}

/// The home directory to hand [`nazar_core::usage::scan_claude`].
///
/// `CLAUDE_CONFIG_DIR` is honoured **here** rather than in the core crate, which takes a home
/// and derives `<home>/.claude/projects` from it. See [`home_for`] for the one shape it
/// cannot express.
fn claude_home() -> Option<PathBuf> {
    home_for(&paths::claude_config_dir().ok()?)
}

/// Which home yields `configured` as its Claude directory, if any.
///
/// [`nazar_core::usage::projects_dir`] builds `<home>/.claude/projects`, so pointing a scan
/// at a configured directory means handing over the home that would produce it:
///
/// * unset — `configured` is already `<home>/.claude`, and its parent is the home;
/// * set to a directory **named `.claude`** — its parent is the home that yields it, which
///   is the shape a second checkout or a throwaway profile actually takes;
/// * set to a directory named anything else — **no home yields it**, and rather than scan
///   `~/.claude` behind the user's back this answers `None` and no scan runs. The store
///   still reads back whatever it holds.
///
/// That last case is a real gap and it is the core crate's to close: a `scan_claude_in(
/// projects_dir, state_dir)` beside the existing entry point would take the directory
/// directly and this function would collapse to one line. It is written down in
/// `docs/PROJECT.md` §9 rather than worked around, because a scan of the wrong tree reports
/// numbers that are wrong without looking wrong.
fn home_for(configured: &Path) -> Option<PathBuf> {
    if configured.file_name() == Some(OsStr::new(".claude")) {
        return configured.parent().map(Path::to_path_buf);
    }
    None
}

/// An error message with the home directory collapsed to `~`.
///
/// [`Error`] carries the path it failed on, which on Windows begins
/// `C:\Users\<name>\`. This text goes on screen, and screenshots of this panel end up in the
/// documentation — the settings page already collapses every path it shows for exactly that
/// reason, and an error line is no different.
fn describe(error: &Error) -> String {
    let text = error.to_string();
    let Ok(home) = paths::home_dir() else {
        return text;
    };
    let home = home.display().to_string();
    if home.is_empty() {
        return text;
    }
    text.replace(&home, "~")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request that would pass, for a test to spoil one field of.
    fn request() -> UsageRequest {
        UsageRequest {
            range: "week".to_owned(),
            from: "2026-09-07T21:00:00Z".to_owned(),
            to: "2026-09-14T21:00:00Z".to_owned(),
            force: false,
        }
    }

    #[test]
    fn the_three_ranges_are_accepted_and_their_window_comes_back_unchanged() {
        for range in RANGES {
            let asked = UsageRequest {
                range: range.to_owned(),
                ..request()
            };
            let window = window(&asked).expect("a known range with a readable window");
            assert_eq!(window, (asked.from.clone(), asked.to.clone()));
        }
    }

    #[test]
    fn a_window_sent_with_an_offset_is_answered_in_the_contracts_spelling() {
        // A `+03:00` instant is an instant, and the panel is allowed to send one. What comes
        // back is UTC with a `Z`, like every other timestamp this product writes, so nothing
        // downstream has to work out an offset.
        let asked = UsageRequest {
            from: "2026-09-08T00:00:00+03:00".to_owned(),
            to: "2026-09-15T00:00:00+03:00".to_owned(),
            ..request()
        };
        assert_eq!(
            window(&asked).expect("an offset names an instant"),
            (
                "2026-09-07T21:00:00Z".to_owned(),
                "2026-09-14T21:00:00Z".to_owned()
            )
        );
    }

    #[test]
    fn a_range_this_build_does_not_know_is_an_error_rather_than_an_empty_week() {
        for range in ["", "day", "Week", "year", "all-time"] {
            let asked = UsageRequest {
                range: range.to_owned(),
                ..request()
            };
            let error = window(&asked).expect_err("an unknown range must not be answered");
            assert_eq!(error.kind, "bad_range");
            assert!(
                error.detail.contains(range),
                "the diagnostic should name what was refused: {}",
                error.detail
            );
        }
    }

    #[test]
    fn a_window_that_is_not_two_instants_is_refused() {
        for (from, to) in [
            ("", "2026-09-14T21:00:00Z"),
            ("2026-09-07T21:00:00Z", ""),
            ("2026-09-07", "2026-09-14"),
            ("last monday", "today"),
            ("2026-09-32T21:00:00Z", "2026-09-33T21:00:00Z"),
            ("1757278800", "1757883600"),
        ] {
            let asked = UsageRequest {
                from: from.to_owned(),
                to: to.to_owned(),
                ..request()
            };
            let error = window(&asked).expect_err("{from} .. {to} is not a window");
            assert_eq!(error.kind, "bad_window", "{from} .. {to}");
        }
    }

    #[test]
    fn a_window_that_ends_before_it_starts_is_refused() {
        let asked = UsageRequest {
            from: "2026-09-14T21:00:00Z".to_owned(),
            to: "2026-09-07T21:00:00Z".to_owned(),
            ..request()
        };
        assert_eq!(window(&asked).unwrap_err().kind, "bad_window");

        // Half-open, so an empty window is empty rather than an hour long.
        let asked = UsageRequest {
            from: "2026-09-14T21:00:00Z".to_owned(),
            to: "2026-09-14T21:00:00Z".to_owned(),
            ..request()
        };
        assert_eq!(window(&asked).unwrap_err().kind, "bad_window");
    }

    #[test]
    fn the_first_scan_is_always_due() {
        let throttle = Throttle::default();
        assert!(throttle.due(0, false));
        assert!(throttle.due(9_000_000, false));
    }

    #[test]
    fn a_second_scan_waits_five_minutes() {
        let mut throttle = Throttle::default();
        throttle.ran(1_000_000);

        assert!(!throttle.due(1_000_000, false), "the same instant");
        assert!(!throttle.due(1_000_001, false), "a millisecond later");
        assert!(
            !throttle.due(1_000_000 + SCAN_INTERVAL_MS - 1, false),
            "one millisecond short of five minutes is still short"
        );
        assert!(
            throttle.due(1_000_000 + SCAN_INTERVAL_MS, false),
            "five minutes is due"
        );
        assert!(throttle.due(1_000_000 + SCAN_INTERVAL_MS * 12, false));
    }

    #[test]
    fn five_minutes_is_five_minutes() {
        assert_eq!(SCAN_INTERVAL_MS, 300_000);
    }

    #[test]
    fn force_ignores_the_throttle_and_the_throttle_still_moves() {
        let mut throttle = Throttle::default();
        throttle.ran(1_000_000);
        assert!(throttle.due(1_000_100, true), "force is the Refresh button");

        // Forcing does not reset the rule for everybody else: the next unforced call is
        // measured from the forced scan, not from the one before it.
        throttle.ran(1_000_100);
        assert!(!throttle.due(1_000_100 + SCAN_INTERVAL_MS - 1, false));
        assert!(throttle.due(1_000_100 + SCAN_INTERVAL_MS, false));
    }

    #[test]
    fn a_monotonic_reading_that_went_backwards_means_not_yet() {
        let mut throttle = Throttle::default();
        throttle.ran(9_000_000);
        assert!(
            !throttle.due(1, false),
            "an underflow would have made this due for ever"
        );
    }

    #[test]
    fn the_home_handed_to_the_scanner_is_the_one_that_yields_the_configured_directory() {
        let home = Path::new("/home/someone");
        assert_eq!(
            home_for(&home.join(".claude")),
            Some(home.to_path_buf()),
            "the default shape: <home>/.claude"
        );
        assert_eq!(
            home_for(Path::new("/tmp/profile/.claude")),
            Some(PathBuf::from("/tmp/profile")),
            "an override that names a .claude directory is expressible"
        );
        assert_eq!(
            home_for(Path::new("/opt/claude-config")),
            None,
            "an override no home yields must not quietly scan the real one"
        );
    }

    #[test]
    fn an_error_shown_to_the_user_does_not_carry_their_name() {
        let Ok(home) = paths::home_dir() else {
            return;
        };
        let error = Error::Io {
            path: home.join(".claude").join("projects"),
            source: std::io::Error::other("no"),
        };
        let text = describe(&error);
        assert!(
            !text.contains(&home.display().to_string()),
            "the home directory is still in {text:?}"
        );
        assert!(text.contains('~'), "{text:?} should have collapsed it");
    }

    #[test]
    fn a_response_spells_its_counters_the_way_the_contract_does() {
        let response = UsageResponse {
            range: "week".to_owned(),
            from: "2026-09-07T21:00:00Z".to_owned(),
            to: "2026-09-14T21:00:00Z".to_owned(),
            since: Some("2026-09-07T04:13:52Z".to_owned()),
            scanned_at: Some("2026-09-13T02:31:07Z".to_owned()),
            providers: BTreeMap::new(),
            damaged: Vec::new(),
            scan: Some(UsageScan {
                files_seen: 412,
                lines: 90_210,
                duplicates: 51_344,
                took_ms: 1_840,
            }),
        };
        let json = serde_json::to_string(&response).expect("a response serialises");
        for key in ["scanned_at", "files_seen", "took_ms"] {
            assert!(json.contains(key), "{key} is not in {json}");
        }
        assert!(
            !json.contains("scannedAt"),
            "the usage document is snake_case throughout; see the type's own documentation"
        );
    }

    #[test]
    fn an_absent_scan_is_absent_rather_than_null() {
        let json = serde_json::to_string(&UsageResponse::default()).expect("serialises");
        assert!(!json.contains("scan"), "{json}");
        assert!(!json.contains("null"), "{json}");
    }

    #[test]
    fn a_reader_instance_is_told_it_may_not_write() {
        assert!(UsageState::new(true).writes());
        assert!(!UsageState::new(false).writes());
    }

    /// The command's own path, end to end, against whatever this machine really has.
    ///
    /// Ignored by default: it walks the reader's transcripts, which on a machine with a
    /// history is hundreds of megabytes and a second or two. Run it with
    /// `cargo test -p nazar-tray --release -- --ignored scan_and_query --nocapture`.
    ///
    /// The store it writes is a throwaway directory, so the maintainer's own history is
    /// neither read from nor added to, and a machine with no `~/.claude` is a pass rather
    /// than a failure — that is CI, and the thing under test is the path, not the corpus.
    #[test]
    #[ignore = "walks the machine's own transcripts"]
    fn scan_and_query_the_store_this_machine_actually_has() {
        let state_dir = std::env::temp_dir().join(format!("nazar-wp15-{}", std::process::id()));
        std::fs::create_dir_all(&state_dir).expect("a throwaway store");

        let throttle = Mutex::new(Throttle::default());
        let first = scan_if_due(&throttle, &state_dir, false).expect("a scan that does not panic");
        let Some(first) = first else {
            std::fs::remove_dir_all(&state_dir).ok();
            panic!("the first call must scan: the throttle has never run");
        };
        println!(
            "scan: {} files, {} lines, {} duplicates, {} ms",
            first.scan.files_seen, first.scan.lines, first.scan.duplicates, first.scan.took_ms
        );

        // The second call inside five minutes must not scan again.
        assert!(
            scan_if_due(&throttle, &state_dir, false)
                .expect("a throttled call is not a failure")
                .is_none(),
            "a second scan ran inside the five-minute window"
        );

        let view =
            nazar_core::usage::query(&state_dir, "2000-01-01T00:00:00Z", "2100-01-01T00:00:00Z")
                .expect("the store this scan just wrote must read back");
        println!(
            "store: {} provider(s), since {:?}, damaged {:?}",
            view.providers.len(),
            view.since,
            view.damaged
        );
        assert!(
            view.damaged.is_empty(),
            "a store written now is not damaged"
        );

        std::fs::remove_dir_all(&state_dir).ok();
    }
}
