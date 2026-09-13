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
//! No user interface: T-WP16 draws the view.
//!
//! # T-WP17 adds two things
//!
//! **The Codex half of a scan.** T-WP14 landed [`nazar_core::usage::scan_codex_home`], and
//! [`scan_if_due`] now runs it beside the Claude one, under the same throttle and the same
//! writer rule. The counters the panel sees are the two summed, with
//! [`UsageScan::providers_scanned`] saying which readers were behind them — a machine with
//! no Codex on it reports `["claude"]` rather than half a number nobody can account for.
//!
//! **The tray tooltip's second line.** [`week_usage`] folds the store into one headline and
//! one model name for *this week*; [`crate::tray::tooltip`] writes them. The week it means
//! is the one described on [`week_window`], which is the only interesting problem in this
//! file: this crate may not ask the machine which time zone it is in, and a week starts on
//! a Monday somewhere.
//!
//! # T-WP20b changes one number
//!
//! [`headline`] is all four counters — `input + output + cache_read + cache_create` — which is
//! what Claude Code's `/usage` calls *total tokens* and what the panel has shown since T-WP20.
//! The tooltip and the view now answer the same question with the same number; the argument
//! that once made this the narrower sum is on [`headline`], and what it argues for now is the
//! panel's breakdown.
//!
//! # T-WP22 adds two switches, and neither of them is a default
//!
//! **The default is still the deduplicated spend**, because it is the one number here that is
//! a measurement of what this machine used. The two settings trade a property of it for
//! agreement with another program, and each says so on screen:
//!
//! * `usage.countLikeClaudeCode` swaps every counter for its per-line twin — the sum Claude
//!   Code's own `/usage` shows, about 1.7× the real spend, because it counts a message once
//!   per content block. The store holds both since T-WP22; this decides which one leaves, and
//!   [`UsageResponse::mode`] says which one did, so the panel can label it rather than guess.
//!   The tooltip follows the same setting, so the two surfaces still cannot disagree.
//! * `usage.fillHistoryFromStats` runs [`nazar_core::usage::backfill_claude_stats_from`] in
//!   the same throttled pass and hands back [`UsageResponse::reported`] — one total per model
//!   for each day older than the transcripts, keyed by the **date** it belongs to rather than a
//!   UTC hour, because it is a day another program computed and not an hour anything happened
//!   in.
//!
//! **`claude_reported` never reaches [`UsageResponse::providers`].** It is stripped here
//! whether the setting is on or off, so nothing downstream can add another program's
//! arithmetic into a counter this one promises to have measured.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use nazar_core::clock::{self, Clock, SystemClock};
use nazar_core::error::Error;
use nazar_core::paths;
use nazar_core::timefmt;
use nazar_core::usage::{Bucket, Hours, PROVIDER, PROVIDER_CODEX, PROVIDER_REPORTED, UsageSummary};
use serde::{Deserialize, Serialize};

/// The range names the panel may ask for.
///
/// A list rather than an enum on the wire, because the answer echoes the string back and an
/// enum would put a Rust spelling in a JSON document that `docs/usage-contract.md` already
/// spells out. The validation is the same either way; this way the error message can name
/// what was actually sent.
pub const RANGES: [&str; 3] = ["week", "month", "all"];

/// What [`UsageResponse::mode`] says when the counters are the deduplicated ones.
pub const MODE_DEDUPED: &str = "deduped";

/// What it says when they are the per-line ones Claude Code's `/usage` shows.
pub const MODE_PER_LINE: &str = "per_line";

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

/// What one scan did, for a diagnostic line.
///
/// Five of [`UsageSummary`]'s twenty fields: enough to say *"412 files, 90 210 lines, 51 344
/// duplicates, 1.8 s"* and not enough to be a second contract. The rest stays in the core
/// crate, where the tests that prove it live.
///
/// **The counters are the readers' sums, and [`providers_scanned`] is why that is honest.**
/// A pass runs Claude's reader and Codex's, and adding their file counts without saying so
/// would make *"412 files"* a number nobody could take apart again. The list names exactly
/// the readers that ran, so a machine with no Codex on it says `["claude"]` and a pass where
/// one reader's tree could not be resolved says which one was left out.
///
/// [`providers_scanned`]: UsageScan::providers_scanned
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageScan {
    /// Transcript and rollout files found, both readers together.
    pub files_seen: u64,
    /// Lines that carried a usage object, both readers together.
    pub lines: u64,
    /// Copies of a message that had already been counted.
    ///
    /// Claude's only: a rollout event carries no identifier, so the Codex reader's whole
    /// dedupe is its byte cursor and it has nothing to count here.
    pub duplicates: u64,
    /// How long the whole pass took, in milliseconds.
    pub took_ms: u64,
    /// Lines the server answered with an error and billed for none of, skipped by name.
    ///
    /// T-WP13b's finding: an `isApiErrorMessage` line carries a full set of counters. All
    /// eleven on the machine this was measured on were also `<synthetic>`, so no total moved
    /// — which is exactly why the number is reported rather than assumed to stay zero.
    pub skipped_api_errors: u64,
    /// The readers that ran, in the order they ran: `claude`, then `codex`.
    pub providers_scanned: Vec<String>,
    /// Days filled from Claude Code's statistics cache, or `0` when that setting is off.
    ///
    /// Not a count of anything this pass measured, which is why it is its own field and not
    /// folded into [`UsageScan::files_seen`]: it is how many days older than the transcripts
    /// were copied in from another program's arithmetic.
    #[serde(default)]
    pub reported_days: u64,
}

impl UsageScan {
    /// Add one reader's summary to the pass.
    fn absorb(&mut self, provider: &str, summary: &UsageSummary) {
        self.files_seen = self.files_seen.saturating_add(summary.files_seen);
        self.lines = self.lines.saturating_add(summary.lines);
        self.duplicates = self.duplicates.saturating_add(summary.duplicates);
        self.skipped_api_errors = self
            .skipped_api_errors
            .saturating_add(summary.skipped_api_errors);
        self.providers_scanned.push(provider.to_owned());
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
    /// Which of the two counts the buckets above hold: `deduped` or `per_line`.
    ///
    /// Sent on every answer rather than only when it is unusual, so the panel labels what it
    /// is drawing from the answer rather than from a setting it read separately and might
    /// have read at a different moment.
    pub mode: String,
    /// Local day (`YYYY-MM-DD`) to model to the one total another program reported for it.
    ///
    /// The days **older than the transcripts**, from `~/.claude/stats-cache.json`, and empty
    /// unless the setting that fills them is on. Keyed by date rather than by UTC hour
    /// because that is what it is: a day Claude Code added up, with no hour inside it and no
    /// split into the four counters. The panel draws these apart from the measured days and
    /// says where they came from; `docs/usage-contract.md` is the whole of the rule.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reported: BTreeMap<String, BTreeMap<String, u64>>,
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
    /// state the user has to fix — a `cursors-claude.json` that is no longer JSON is an error
    /// rather than a fresh start, on purpose — and retrying it on every panel open would turn
    /// one broken file into a scan on every click.
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
    /// Where the reader's own week begins, as seconds into a week from the epoch.
    ///
    /// [`UTC_MONDAY_PHASE`] until the panel says otherwise, and then whatever the panel's
    /// last *week* request implied. See [`week_window`] for the whole argument.
    phase: Mutex<i64>,
}

impl UsageState {
    /// The state for a process that may or may not write.
    #[must_use]
    pub fn new(writer: bool) -> Self {
        UsageState {
            writer,
            throttle: Mutex::new(Throttle::default()),
            phase: Mutex::new(UTC_MONDAY_PHASE),
        }
    }

    /// Whether this process may add to the store.
    #[must_use]
    pub fn writes(&self) -> bool {
        self.writer
    }

    /// Where the tooltip's week begins, as seconds into a week from the epoch.
    #[must_use]
    pub fn phase(&self) -> i64 {
        *self.phase.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Remember the week the panel just asked about.
    ///
    /// Advisory, like everything else behind this lock: a poisoned mutex means a thread
    /// panicked while holding it, and the worst that costs here is a tooltip whose week
    /// starts at the wrong hour.
    fn remember_week(&self, phase: i64) {
        *self.phase.lock().unwrap_or_else(PoisonError::into_inner) = phase;
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
    app: tauri::AppHandle,
    state: tauri::State<'_, UsageState>,
    request: UsageRequest,
) -> Result<UsageResponse, UsageError> {
    let (from, to) = window(&request)?;
    let state_dir =
        paths::settings_dir().map_err(|error| UsageError::from_core("no_state_dir", &error))?;

    // The panel has just told this process where its week begins, in the only way that is
    // allowed to: by naming the instant. See [`week_window`].
    if request.range == "week"
        && let Some(start) = timefmt::unix_seconds_from_rfc3339(&from)
    {
        state.remember_week(week_phase(start));
    }

    // The two settings, read once and used three times: whether a scan also refreshes the
    // reported days, which counters leave this function, and what the tooltip redrawn at the
    // end of it says. Reading them once is what keeps those three from disagreeing.
    let settings = usage_settings(&app);

    let scanned = if state.writes() {
        scan_if_due(
            &state.throttle,
            &state_dir,
            request.force,
            settings.fill_history_from_stats,
        )?
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

    // The scan that just ran is the only thing that moves the store, so this is where the
    // tooltip's second line is newest. The refresh tick redraws it again every pass, from
    // the store and never from a scan; `main.rs` is where that is wired.
    crate::tray::refresh_tooltip(&app);

    // `claude_reported` leaves by its own door or not at all: it is another program's
    // arithmetic about days this store never measured, and a reader that found it among the
    // providers would add it to a total that promises to be a measurement.
    let mut providers = view.providers;
    let reported = match providers.remove(PROVIDER_REPORTED) {
        Some(hours) if settings.fill_history_from_stats => reported_days(&hours),
        _ => BTreeMap::new(),
    };
    if settings.count_like_claude_code {
        providers = providers
            .into_iter()
            .map(|(provider, hours)| (provider, per_line(hours)))
            .collect();
    }

    Ok(UsageResponse {
        range: request.range,
        from,
        to,
        since: view.since,
        scanned_at: view.scanned_at,
        providers,
        mode: mode_name(settings.count_like_claude_code).to_owned(),
        reported,
        damaged,
        scan: scanned.map(|scanned| scanned.scan),
    })
}

/// Which of the two counts a setting asks for, as the answer spells it.
#[must_use]
pub fn mode_name(count_like_claude_code: bool) -> &'static str {
    if count_like_claude_code {
        MODE_PER_LINE
    } else {
        MODE_DEDUPED
    }
}

/// The usage settings, or the shipped defaults when there is no state to ask.
///
/// `None` only before [`crate::state::AppState`] is managed, which in a real run is a moment
/// no command can be called in. The defaults are both `false`, so the fallback is the honest
/// answer rather than an accident.
fn usage_settings(app: &tauri::AppHandle) -> nazar_core::config::UsageSwitches {
    use tauri::Manager as _;
    app.try_state::<crate::state::AppState>()
        .map(|state| state.config().usage)
        .unwrap_or_default()
}

/// Every bucket's per-line counters, in the same shape the deduplicated ones came in.
///
/// **The same shape on purpose.** The panel's arithmetic, its calendar, its weeks list and
/// its detail views are one set of functions over one bucket type, and a second shape for the
/// other count would have meant a second copy of all of it — with two chances to disagree
/// about what a week adds up to. What changes is the four numbers; `requests` does not,
/// because a request is a message in both readings and a count of content blocks under that
/// label would be a number the label lies about.
fn per_line(hours: Hours) -> Hours {
    hours
        .into_iter()
        .map(|(hour, models)| {
            let models = models
                .into_iter()
                .map(|(model, bucket)| {
                    let raw = bucket.raw_counters();
                    (
                        model,
                        Bucket {
                            input: raw.input,
                            output: raw.output,
                            cache_create: raw.cache_create,
                            cache_read: raw.cache_read,
                            requests: bucket.requests,
                            raw: None,
                            reported_total: None,
                            extra: bucket.extra,
                        },
                    )
                })
                .collect();
            (hour, models)
        })
        .collect()
}

/// The reported block as days rather than hours: `YYYY-MM-DD` to model to one total.
///
/// The store keeps each of these at hour `T00` of the day it names, because the store keeps
/// hours; what it is, though, is a **day** another program added up. Handing the panel the
/// date means nothing between here and the screen converts a time zone, and so nothing
/// between here and the screen can be an hour wrong about a number that never had an hour.
fn reported_days(hours: &Hours) -> BTreeMap<String, BTreeMap<String, u64>> {
    let mut days: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    for (hour, models) in hours {
        if hour.len() < 10 {
            continue;
        }
        let day = days.entry(hour[..10].to_owned()).or_default();
        for (model, bucket) in models {
            let Some(total) = bucket.reported_total else {
                continue;
            };
            let into = day.entry(model.clone()).or_default();
            *into = into.saturating_add(total);
        }
    }
    days.retain(|_, models| !models.is_empty());
    days
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
///
/// **Both readers, one throttle.** Claude's transcripts and Codex's rollout logs are scanned
/// in the same pass and written into the same month documents, because they answer one
/// question — *what has this machine spent this week* — and a panel that could see one half
/// updated and the other five minutes behind would be showing a week that never happened.
///
/// **Either reader may be absent, and that is not a failure.** A machine with no Codex on it
/// resolves no Codex home, and a pass that resolves neither does not count as a pass: the
/// throttle is left alone so that installing one of them takes effect on the next panel open
/// rather than in five minutes.
///
/// **A reader that fails does not stop the other.** Each commits its own totals before it
/// returns, so a Codex tree that cannot be walked must not throw away a Claude scan that
/// would have worked. The first error is kept and reported after both have been tried, which
/// is the same "every reader answers for itself" the quota side has always had.
fn scan_if_due(
    throttle: &Mutex<Throttle>,
    state_dir: &Path,
    force: bool,
    backfill: bool,
) -> Result<Option<Scanned>, UsageError> {
    let mut throttle = throttle.lock().unwrap_or_else(PoisonError::into_inner);
    let now_ms = SystemClock.monotonic_millis();
    if !throttle.due(now_ms, force) {
        return Ok(None);
    }

    // Each reader's tree, named rather than derived: `CLAUDE_CONFIG_DIR` through
    // [`claude_projects`] and `CODEX_HOME` through the core crate's own resolver, which the
    // quota reader already uses. Neither variable is read here a second time — one spelling
    // of an environment variable per program.
    let claude = claude_projects();
    let codex = nazar_core::codex::codex_home().ok();
    if claude.is_none() && codex.is_none() {
        return Ok(None);
    }

    // Marked before the scan rather than after it: a scan that fails is a scan that ran, and
    // retrying a broken store on every panel open would be worse than the failure.
    throttle.ran(now_ms);

    let started = Instant::now();
    let mut scan = UsageScan::default();
    let mut damaged = Vec::new();
    let mut failure = None;

    if let Some(projects) = claude {
        match nazar_core::usage::scan_claude_in(&projects, state_dir) {
            Ok(summary) => {
                scan.absorb(PROVIDER, &summary);
                damaged.extend(summary.damaged);
            }
            Err(error) => failure = Some(UsageError::from_core("scan_failed", &error)),
        }
    }
    if let Some(home) = codex {
        match nazar_core::usage::scan_codex_home(&home, state_dir) {
            Ok(summary) => {
                scan.absorb(PROVIDER_CODEX, &summary);
                damaged.extend(summary.damaged);
            }
            Err(error) => {
                failure.get_or_insert_with(|| UsageError::from_core("scan_failed", &error));
            }
        }
    }
    // The reported days, in the same pass and behind the same throttle, because they answer
    // the same question and a panel that could see one half refreshed and the other five
    // minutes behind would be showing a history that never happened. It runs **after** the
    // transcripts, so the boundary it fills up to is the one this pass just measured.
    //
    // A failure here does not fail the call. Every number the user asked for is already in
    // the store; a statistics cache that could not be read costs the days before it, which
    // are the days this product never measured anyway.
    if backfill
        && failure.is_none()
        && let Some(cache) = claude_stats_cache()
        && let Ok(filled) = nazar_core::usage::backfill_claude_stats_from(&cache, state_dir)
    {
        scan.reported_days = filled.days;
    }

    if let Some(failure) = failure {
        return Err(failure);
    }

    scan.took_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(Some(Scanned { scan, damaged }))
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

/// The `projects` directory to hand [`nazar_core::usage::scan_claude_in`].
///
/// `CLAUDE_CONFIG_DIR` is honoured **here** rather than in the core crate, which takes the
/// directory it is told to walk and asks no questions about where it came from.
/// [`paths::claude_config_dir`] is the one answer this product has about that variable, and
/// the quota reader uses the same one; `projects` under it is the shape Claude Code writes,
/// whatever the configuration directory is called.
///
/// **This used to be a gap and now is not.** T-WP15 could only name a *home* and derive
/// `<home>/.claude/projects` from it, so a `CLAUDE_CONFIG_DIR` pointing at a directory not
/// called `.claude` — a second checkout, a throwaway profile — meant no scan at all, because
/// scanning `~/.claude` behind the user's back would have been worse. T-WP13b added the entry
/// point that takes the directory, and this is it being used.
fn claude_projects() -> Option<PathBuf> {
    Some(paths::claude_config_dir().ok()?.join("projects"))
}

/// The `stats-cache.json` to hand [`nazar_core::usage::backfill_claude_stats_from`].
///
/// Beside [`claude_projects`] and for the same reason: `CLAUDE_CONFIG_DIR` names a
/// configuration directory that need not be called `.claude`, and a path derived from a home
/// directory cannot express it. One spelling of that variable per program, resolved here, so
/// the transcripts and the statistics that stand in for the ones that are gone are read out of
/// the same directory rather than out of two.
fn claude_stats_cache() -> Option<PathBuf> {
    Some(paths::claude_config_dir().ok()?.join("stats-cache.json"))
}

// ----------------------------------------------------------------- this week, for a tooltip

/// Seconds in a week.
const WEEK_SECONDS: i64 = 7 * 24 * 60 * 60;

/// Where a Monday-start week begins in UTC, as seconds into a week from the epoch.
///
/// 1970-01-01 was a Thursday, so the first Monday 00:00 UTC is unix second 345 600 and every
/// Monday since is that plus a whole number of weeks.
const UTC_MONDAY_PHASE: i64 = 4 * 24 * 60 * 60;

/// What the tooltip says about this week.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeekUsage {
    /// `input + output + cache_read + cache_create`, summed over the week and over both
    /// providers.
    ///
    /// All four counters, which is what Claude Code's `/usage` calls *total tokens* — the same
    /// rule the panel's headline follows, for the same reason. The argument for the narrower
    /// number, and why it became the panel's breakdown instead, is on [`headline`].
    pub headline: u64,
    /// The model with the largest headline, as the source spelled it.
    ///
    /// Raw and never translated: a model id is data, and this build cannot know next
    /// quarter's names. Summed across providers, because the question is *which model*, not
    /// *which model on which side*.
    pub model: String,
}

/// Where the reader's week begins, from an instant that is one of its Mondays.
///
/// A week is 604 800 seconds long and every Monday 00:00 in a given zone is the same number
/// of seconds into one, so the remainder is the whole of what has to be remembered.
#[must_use]
pub fn week_phase(monday: i64) -> i64 {
    monday.rem_euclid(WEEK_SECONDS)
}

/// The week `now` falls in, as a half-open range of UTC instants.
///
/// # Why this is not simply Monday in UTC
///
/// A week starts on Monday **where the reader is**, and `crates/nazar-core/tests/hygiene.rs`
/// fails the build on anything in this workspace that asks the machine what time zone it is
/// in. The panel is allowed to ask — it is JavaScript, and it is one call — and T-WP15 built
/// the command around exactly that: the panel computes its local Monday and sends the
/// instant. So the instant is already arriving here, once per look at the usage view, and
/// nothing new has to be invented to find out where the reader's week begins.
///
/// `phase` is that instant with the weeks divided out of it: *how far into a week a Monday
/// falls*, which for a reader at `+03:00` is Sunday 21:00 UTC and for one at `-05:00` is
/// Monday 05:00 UTC. Given the phase, every week boundary before and after it is arithmetic
/// on UTC seconds, and this crate never learns which zone produced it.
///
/// # What it costs
///
/// * **Before the panel has ever opened the usage view**, the phase is [`UTC_MONDAY_PHASE`]
///   and the tooltip means a UTC week. A reader at `+03:00` who hovers the icon between
///   Monday 00:00 and 03:00 local sees last week's number for those three hours, once a
///   week, until the first time they open the panel. Nothing is wrong afterwards, and
///   nothing is ever wrong by more than one zone offset.
/// * **A daylight-saving change moves the phase by an hour**, and this crate does not know
///   it happened. The panel reports the new one the next time it asks, so the window is
///   right again at the first look; between the change and that look it is an hour out at
///   the boundary. Both are written down in `docs/PROJECT.md` §9 rather than hidden.
///
/// The alternative — a second Tauri command, or the offset persisted in the config file —
/// would buy those hours at the price of a wider bridge and a fourth thing that can disagree
/// with the panel. The tooltip is a glance surface; the panel is where the exact week lives.
#[must_use]
pub fn week_window(now: i64, phase: i64) -> (i64, i64) {
    let start = now - (now - phase).rem_euclid(WEEK_SECONDS);
    (start, start + WEEK_SECONDS)
}

/// A token count short enough for a tooltip: `843`, `22.3K`, `22.3M`, `1.5B`.
///
/// **Latin magnitude marks, unlike the panel.** `ui/src/usage.ts` hands the number to
/// `Intl.NumberFormat`, which writes `22.3M` in English and `2230만` in Korean, and T-WP16
/// put no magnitude mark in a locale file on purpose. There is no `Intl` in Rust and the
/// tooltip has 127 characters for everything, so this writes the mark itself. It is the one
/// place in this application where a number is not spelled the way the language spells it,
/// and the trade is deliberate: four locale keys per language for `K`, `M`, `B` and `T`
/// would be twenty-four strings to get wrong for a line the shell draws on hover.
///
/// **Floored to the precision it is shown at**, the same rule as a quota percentage and as
/// the panel's own `formatTokens`: 22.39 M is not 22.4 M, because the store did not measure
/// the difference. A trailing `.0` is dropped, which is what `Intl` does with
/// `maximumFractionDigits: 1`, so `1000` is `1K` on both sides.
#[must_use]
pub fn compact(value: u64) -> String {
    const MARKS: [(u64, char); 4] = [
        (1_000_000_000_000, 'T'),
        (1_000_000_000, 'B'),
        (1_000_000, 'M'),
        (1_000, 'K'),
    ];

    let Some(&(unit, mark)) = MARKS.iter().find(|(unit, _)| value >= *unit) else {
        // Anything under a thousand is printed as itself: 843 is not 0.8K.
        return value.to_string();
    };
    let whole = value / unit;
    // One decimal below a hundred, none above it — three significant figures either way, and
    // five characters or fewer for anything a week could actually reach.
    let tenths = if whole < 100 {
        (value % unit) * 10 / unit
    } else {
        0
    };
    if tenths == 0 {
        format!("{whole}{mark}")
    } else {
        format!("{whole}.{tenths}{mark}")
    }
}

/// `input + output + cache_read + cache_create`.
///
/// **All four, which is what Claude Code's `/usage` calls *total tokens*.** T-WP16 left
/// `cache_read` out here and in the panel, on a measurement that is still true: cache reads
/// were 98.5 % of the raw total over six days of real work, so a headline with them folded in
/// is a number about the cache. T-WP20 put the two windows side by side and found the other
/// half of that argument — 1.4 M against 1.0 B, a tray answering `/usage`'s question with
/// 0.1 % of `/usage`'s answer, which does not read as careful, it reads as broken. The panel
/// moved then; this is the tooltip catching up, so that the two surfaces of this application
/// cannot answer one question with two numbers.
///
/// What the measurement argued for is the panel's **four-way breakdown**, where a reader can
/// see for themselves that the billion is the cache. A tooltip has 127 characters and no room
/// for it; what it has instead is a number that matches the view one click away.
///
/// All five counters are present on every stored bucket since T-WP13b — absent against zero is
/// a distinction about one record, not about a sum over many — so this is an addition rather
/// than a decision, and what "nothing here" means is decided once, in [`fold`], where it is a
/// total of zero.
/// `per_line` picks the same four counters with no dedupe, which is the whole of what the
/// *Count like Claude Code* setting does to this side: the tooltip and the panel are handed
/// the same choice from the same place, so a user comparing them sees one number twice.
fn headline(bucket: &Bucket, per_line: bool) -> u64 {
    if per_line {
        return bucket.raw_counters().total();
    }
    bucket
        .input
        .saturating_add(bucket.output)
        .saturating_add(bucket.cache_read)
        .saturating_add(bucket.cache_create)
}

/// The headline and the busiest model across a set of hourly buckets.
///
/// `None` when the headline comes to zero, which since T-WP20b means exactly one thing: a week
/// with no tokens of any kind in it, because all four counters are in the sum now. A week of
/// nothing but cache reads used to land here too and no longer does — it is a week that spent
/// something, `/usage` says so, and the tooltip says the same.
///
/// A tie goes to the model that sorts first, so the same store always produces the same
/// tooltip. There is no honest tie-break between two models that spent the same amount, and
/// a tooltip that flickered between them on every pass would be worse than an arbitrary
/// rule written down.
#[must_use]
pub fn fold(providers: &BTreeMap<String, Hours>, per_line: bool) -> Option<WeekUsage> {
    let mut total = 0u64;
    let mut per_model: BTreeMap<&str, u64> = BTreeMap::new();

    for (provider, hours) in providers {
        // The reported days are not this week and are not a measurement; a tooltip has one
        // line and no room to say which half of its number came from somewhere else.
        if provider == PROVIDER_REPORTED {
            continue;
        }
        for models in hours.values() {
            for (model, bucket) in models {
                let headline = headline(bucket, per_line);
                if headline == 0 {
                    continue;
                }
                total = total.saturating_add(headline);
                let entry = per_model.entry(model.as_str()).or_default();
                *entry = entry.saturating_add(headline);
            }
        }
    }
    if total == 0 {
        return None;
    }

    let mut best: Option<(&str, u64)> = None;
    for (model, spent) in per_model {
        if best.is_none_or(|(_, highest)| spent > highest) {
            best = Some((model, spent));
        }
    }
    best.map(|(model, _)| WeekUsage {
        headline: total,
        model: model.to_owned(),
    })
}

/// This week, read back from the store.
///
/// **Reads, never scans.** The tray redraws its tooltip on every refresh pass, and the pass
/// exists to read two small files in milliseconds; a scan reads hundreds of megabytes and
/// stays where T-WP15 put it, behind the panel and behind a five-minute throttle. What this
/// opens is the one or two month documents the week touches, which is the same work the
/// panel already does every time it is shown.
///
/// `None` for every way this can come to nothing — no settings directory, a clock that is
/// not a timestamp, a store that will not read, a week with nothing in it — because a
/// tooltip has no room to explain itself and the panel says all four properly.
#[must_use]
pub fn week_usage(state: &UsageState, per_line: bool) -> Option<WeekUsage> {
    let state_dir = paths::settings_dir().ok()?;
    let now = clock::wall_seconds(&SystemClock)?;
    let (from, to) = week_window(now, state.phase());
    let view = nazar_core::usage::query(
        &state_dir,
        &timefmt::rfc3339_from_unix_seconds(from),
        &timefmt::rfc3339_from_unix_seconds(to),
    )
    .ok()?;
    fold(&view.providers, per_line)
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
    fn the_directory_handed_to_the_scanner_is_projects_under_the_configured_one() {
        // `CLAUDE_CONFIG_DIR` is read by the process and cannot be moved from a test without
        // making the suite unsound, so what is checked is the shape this builds around
        // whatever that answer is: `projects` under the configuration directory, never a
        // guess derived from a home. Every override that used to be inexpressible — a
        // directory not called `.claude` — is expressible now, which is T-WP13b's half of
        // this and the reason `home_for` is gone.
        let configured = paths::claude_config_dir().expect("a machine with a home directory");
        let scanned = claude_projects().expect("the same answer, one segment longer");

        assert_eq!(scanned, configured.join("projects"));
        assert_eq!(scanned.parent(), Some(configured.as_path()));
        assert_eq!(
            scanned.file_name(),
            Some(std::ffi::OsStr::new("projects")),
            "the directory Claude Code writes under, whatever its parent is called"
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
            mode: MODE_DEDUPED.to_owned(),
            reported: BTreeMap::new(),
            damaged: Vec::new(),
            scan: Some(UsageScan {
                files_seen: 412,
                lines: 90_210,
                duplicates: 51_344,
                took_ms: 1_840,
                skipped_api_errors: 11,
                providers_scanned: vec![PROVIDER.to_owned(), PROVIDER_CODEX.to_owned()],
                reported_days: 0,
            }),
        };
        let json = serde_json::to_string(&response).expect("a response serialises");
        for key in ["scanned_at", "files_seen", "took_ms", "providers_scanned"] {
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
        assert!(
            !json.contains("reported"),
            "a machine with the setting off carries no empty reported map: {json}"
        );
    }

    #[test]
    fn the_mode_switch_hands_back_the_other_numbers_in_the_same_shape() {
        let mut bucket = spent(2, 328, 24_843, 35_613);
        bucket.raw = Some(nazar_core::usage::Raw {
            input: 4,
            output: 656,
            cache_create: 49_686,
            cache_read: 71_226,
        });
        let hours = hours("2026-09-09T12", "claude-opus-5", bucket.clone());

        let swapped = per_line(hours.clone());
        let out = &swapped["2026-09-09T12"]["claude-opus-5"];
        assert_eq!((out.input, out.output), (4, 656));
        assert_eq!((out.cache_create, out.cache_read), (49_686, 71_226));
        assert_eq!(
            out.requests, 1,
            "a request is a message in both readings, so this one does not move"
        );
        assert_eq!(out.raw, None, "and the answer carries one count, not two");

        // The tooltip follows the same setting, so the two surfaces cannot disagree.
        let mut providers = BTreeMap::new();
        providers.insert(PROVIDER.to_owned(), hours);
        assert_eq!(
            fold(&providers, false).expect("a week").headline,
            2 + 328 + 24_843 + 35_613
        );
        assert_eq!(
            fold(&providers, true).expect("a week").headline,
            4 + 656 + 49_686 + 71_226
        );
        assert_eq!(mode_name(false), MODE_DEDUPED);
        assert_eq!(mode_name(true), MODE_PER_LINE);
    }

    #[test]
    fn a_bucket_with_no_per_line_counters_reads_as_the_five_it_has() {
        // Codex writes one of these for every event, and so does every month document
        // written before T-WP22. Turning the setting on must show them, not zero them.
        let hours = hours("2026-09-09T12", "gpt-5.6-sol", spent(1000, 300, 0, 3000));
        let swapped = per_line(hours);
        let out = &swapped["2026-09-09T12"]["gpt-5.6-sol"];
        assert_eq!((out.input, out.output, out.cache_read), (1000, 300, 3000));
    }

    #[test]
    fn the_reported_block_reaches_the_panel_as_days_rather_than_hours() {
        let mut models = nazar_core::usage::Models::new();
        models.insert(
            "claude-opus-5".to_owned(),
            Bucket {
                reported_total: Some(10_000),
                ..Bucket::default()
            },
        );
        models.insert(
            "claude-fable-5-1".to_owned(),
            Bucket {
                reported_total: Some(2000),
                ..Bucket::default()
            },
        );
        // A bucket with no reported total is not a day with a zero in it.
        models.insert("claude-sonnet-5".to_owned(), Bucket::default());

        let mut hours = Hours::new();
        hours.insert("2026-08-30T00".to_owned(), models);

        let days = reported_days(&hours);
        assert_eq!(days.len(), 1);
        let day = &days["2026-08-30"];
        assert_eq!(day["claude-opus-5"], 10_000);
        assert_eq!(day["claude-fable-5-1"], 2000);
        assert!(!day.contains_key("claude-sonnet-5"));
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
        let first =
            scan_if_due(&throttle, &state_dir, false, false).expect("a scan that does not panic");
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
            scan_if_due(&throttle, &state_dir, false, false)
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

        // T-WP22: the same window in both counts, and what the two settings are worth on
        // this machine. The per-line number is the one `/usage` prints; the ratio between
        // them is the whole of why the setting exists.
        let deduped = fold(&view.providers, false).map_or(0, |week| week.headline);
        let counted = fold(&view.providers, true).map_or(0, |week| week.headline);
        println!(
            "all time: deduped {deduped}, per_line {counted}, ratio {:.3}",
            if deduped == 0 {
                0.0
            } else {
                counted as f64 / deduped as f64
            }
        );
        assert!(
            counted >= deduped,
            "the per-line count is never the smaller of the two"
        );

        if let Some(cache) = claude_stats_cache() {
            let filled = nazar_core::usage::backfill_claude_stats_from(&cache, &state_dir)
                .expect("a backfill that does not panic");
            println!(
                "reported: {} day(s), {} model row(s), {} tokens, before {:?}, cache v{:?} \
                 computed to {:?}",
                filled.days,
                filled.models,
                filled.total,
                filled.boundary,
                filled.version,
                filled.last_computed
            );
            // Twice, because the whole block is a copy rather than an accumulation.
            let again = nazar_core::usage::backfill_claude_stats_from(&cache, &state_dir)
                .expect("a second backfill");
            assert_eq!(again.total, filled.total, "a copy is the same copy twice");
            assert!(
                again.months.is_empty(),
                "and writes no bytes the second time"
            );
        }

        std::fs::remove_dir_all(&state_dir).ok();
    }

    // ------------------------------------------------------- T-WP17: the scan, and the week

    /// A summary with the three counters a pass reports, and nothing else.
    fn summary(files_seen: u64, lines: u64, duplicates: u64) -> UsageSummary {
        UsageSummary {
            files_seen,
            lines,
            duplicates,
            ..UsageSummary::default()
        }
    }

    #[test]
    fn a_pass_reports_the_two_readers_added_up_and_says_which_ran() {
        let mut scan = UsageScan::default();
        scan.absorb(PROVIDER, &summary(412, 90_210, 51_344));
        scan.absorb(PROVIDER_CODEX, &summary(21, 3_180, 0));

        assert_eq!(scan.files_seen, 433);
        assert_eq!(scan.lines, 93_390);
        assert_eq!(
            scan.duplicates, 51_344,
            "only Claude has duplicates to count"
        );
        assert_eq!(scan.providers_scanned, ["claude", "codex"]);
    }

    #[test]
    fn a_machine_with_only_one_of_the_two_says_so_rather_than_reporting_half_a_sum() {
        // Why the list is in the answer at all: 412 files with no Codex on the machine and
        // 412 files with a Codex nobody could resolve are the same number, and only one of
        // them is a complete one.
        let mut scan = UsageScan::default();
        scan.absorb(PROVIDER, &summary(412, 90_210, 51_344));

        assert_eq!(scan.providers_scanned, ["claude"]);
        assert_eq!(scan.files_seen, 412);
    }

    #[test]
    fn a_reader_that_ran_and_found_nothing_still_ran() {
        let mut scan = UsageScan::default();
        scan.absorb(PROVIDER, &summary(0, 0, 0));
        scan.absorb(PROVIDER_CODEX, &summary(21, 3_180, 0));

        assert_eq!(scan.files_seen, 21);
        assert_eq!(scan.providers_scanned, ["claude", "codex"]);
    }

    #[test]
    fn a_compact_number_is_the_one_the_panel_would_have_written() {
        // The right-hand column is what `Intl.NumberFormat`'s compact notation produces in
        // English for the same value, which is what `ui/src/usage.ts` draws. Anything under
        // a thousand is itself, the value is floored to the digit it is shown at, and a
        // trailing `.0` is dropped because `maximumFractionDigits: 1` drops it too.
        for (value, expected) in [
            (0u64, "0"),
            (1, "1"),
            (843, "843"),
            (999, "999"),
            (1_000, "1K"),
            (1_100, "1.1K"),
            (1_199, "1.1K"),
            (22_345_678, "22.3M"),
            (22_399_999, "22.3M"),
            (99_999_999, "99.9M"),
            (100_000_000, "100M"),
            (999_999_999, "999M"),
            (1_500_000_000, "1.5B"),
            (1_500_000_000_000, "1.5T"),
            (u64::MAX, "18446744T"),
        ] {
            assert_eq!(compact(value), expected, "compact({value})");
        }
    }

    #[test]
    fn a_compact_number_never_rounds_a_week_up() {
        // The same rule as a quota percentage: 22.39 M is not 22.4 M, because the store did
        // not measure the difference and a reader would take the larger number for a fact.
        assert_eq!(compact(22_390_000), "22.3M");
        assert_eq!(compact(999_999), "999K");
        assert_eq!(compact(1_999_999_999), "1.9B");
    }

    #[test]
    fn a_compact_number_stays_short_enough_for_a_tooltip() {
        // Five characters for anything below a thousand of the largest mark, which is every
        // week a machine could produce. `tray.rs` covers `u64::MAX` against the real limit.
        for value in [
            0,
            1,
            999,
            1_000,
            22_345_678,
            99_999_999_999_999,
            999_999_999_999_999,
        ] {
            let text = compact(value);
            assert!(text.chars().count() <= 5, "compact({value}) is {text:?}");
        }
    }

    #[test]
    fn a_utc_monday_is_where_a_week_begins_before_the_panel_has_said_otherwise() {
        // 1970-01-01 was a Thursday, so the first Monday is the fourth day after it.
        assert_eq!(UTC_MONDAY_PHASE, 345_600);
        let monday = timefmt::unix_seconds_from_rfc3339("1970-01-05T00:00:00Z").unwrap();
        assert_eq!(monday, UTC_MONDAY_PHASE);
        assert_eq!(week_phase(monday), UTC_MONDAY_PHASE);
    }

    #[test]
    fn the_phase_is_the_same_whichever_monday_the_panel_names() {
        // What is remembered is *how far into a week* a Monday falls, so any Monday of any
        // year gives the same number and the tooltip does not drift across a year end.
        let at = |text: &str| timefmt::unix_seconds_from_rfc3339(text).unwrap();
        for monday in [
            "2026-09-07T00:00:00Z",
            "2026-12-28T00:00:00Z",
            "2027-01-04T00:00:00Z",
            "1970-01-05T00:00:00Z",
        ] {
            assert_eq!(week_phase(at(monday)), UTC_MONDAY_PHASE, "{monday}");
        }
    }

    #[test]
    fn a_readers_own_monday_is_a_phase_this_crate_never_names_a_zone_for() {
        // The instants the panel actually sends. At +03:00 a local Monday starts on Sunday
        // at 21:00 UTC; at -05:00 it starts on Monday at 05:00. Neither is a time zone as
        // far as this file is concerned — both are a number of seconds into a week.
        let at = |text: &str| timefmt::unix_seconds_from_rfc3339(text).unwrap();
        let now = at("2026-09-09T12:00:00Z");

        let istanbul = week_phase(at("2026-09-07T00:00:00+03:00"));
        assert_eq!(istanbul, UTC_MONDAY_PHASE - 3 * 3_600);
        let (from, _) = week_window(now, istanbul);
        assert_eq!(
            timefmt::rfc3339_from_unix_seconds(from),
            "2026-09-06T21:00:00Z"
        );

        let chicago = week_phase(at("2026-09-07T00:00:00-05:00"));
        assert_eq!(chicago, UTC_MONDAY_PHASE + 5 * 3_600);
        let (from, _) = week_window(now, chicago);
        assert_eq!(
            timefmt::rfc3339_from_unix_seconds(from),
            "2026-09-07T05:00:00Z"
        );

        // Half an hour is a zone too, and a store of whole hours still answers for it: the
        // hours that *start* inside the window are the week, which is the same rule the
        // panel applies to the same buckets.
        let kolkata = week_phase(at("2026-09-07T00:00:00+05:30"));
        let (from, _) = week_window(now, kolkata);
        assert_eq!(
            timefmt::rfc3339_from_unix_seconds(from),
            "2026-09-06T18:30:00Z"
        );
    }

    #[test]
    fn the_week_window_is_seven_days_long_and_holds_the_instant_it_was_asked_about() {
        let at = |text: &str| timefmt::unix_seconds_from_rfc3339(text).unwrap();

        for now in [
            "2026-09-07T00:00:00Z",
            "2026-09-07T00:00:01Z",
            "2026-09-09T12:00:00Z",
            "2026-09-13T23:59:59Z",
        ] {
            let (from, to) = week_window(at(now), UTC_MONDAY_PHASE);
            assert_eq!(to - from, WEEK_SECONDS, "{now}");
            assert!(from <= at(now) && at(now) < to, "{now}");
            assert_eq!(
                timefmt::rfc3339_from_unix_seconds(from),
                "2026-09-07T00:00:00Z",
                "{now}"
            );
        }

        // Half-open at the far end: the first instant of the next Monday is next week.
        let (from, _) = week_window(at("2026-09-14T00:00:00Z"), UTC_MONDAY_PHASE);
        assert_eq!(
            timefmt::rfc3339_from_unix_seconds(from),
            "2026-09-14T00:00:00Z"
        );
    }

    #[test]
    fn a_week_before_the_epoch_does_not_wrap_the_arithmetic() {
        // `rem_euclid` rather than `%`: a negative instant with a plain remainder would put
        // the start of the week *after* the instant it is supposed to contain.
        let (from, to) = week_window(-1, UTC_MONDAY_PHASE);
        assert!(from <= -1 && -1 < to);
        assert_eq!(to - from, WEEK_SECONDS);
    }

    /// One hour of one model's buckets, for the folding tests.
    fn hours(hour: &str, model: &str, bucket: Bucket) -> Hours {
        let mut models = nazar_core::usage::Models::new();
        models.insert(model.to_owned(), bucket);
        let mut hours = Hours::new();
        hours.insert(hour.to_owned(), models);
        hours
    }

    /// A bucket with all five counters, which since T-WP13b is every stored bucket.
    fn spent(input: u64, output: u64, cache_create: u64, cache_read: u64) -> Bucket {
        Bucket {
            input,
            output,
            cache_create,
            cache_read,
            requests: 1,
            ..Bucket::default()
        }
    }

    #[test]
    fn the_headline_is_all_four_counters_the_way_usage_totals_them() {
        let mut providers = BTreeMap::new();
        providers.insert(
            PROVIDER.to_owned(),
            hours(
                "2026-09-09T12",
                "claude-opus-5",
                spent(2, 328, 24_843, 35_613),
            ),
        );

        let week = fold(&providers, false).expect("a week with work in it");
        assert_eq!(
            week.headline,
            2 + 328 + 24_843 + 35_613,
            "cache_read is inside the headline, because /usage puts it there"
        );
        assert_eq!(week.model, "claude-opus-5");
    }

    #[test]
    fn both_providers_are_added_together_and_so_is_a_model_seen_in_both() {
        // The question the tooltip answers is "what has this machine spent", not "what has
        // it spent on each side", so the providers are summed. A model id that turned up
        // under both is one model: it is the same name, and naming it twice would split a
        // total the panel shows whole.
        let mut providers = BTreeMap::new();
        providers.insert(
            PROVIDER.to_owned(),
            hours("2026-09-09T12", "shared", spent(10, 10, 10, 9_999)),
        );
        providers.insert(
            PROVIDER_CODEX.to_owned(),
            hours("2026-09-09T13", "shared", spent(5, 5, 5, 9_999)),
        );

        let week = fold(&providers, false).expect("two providers, one week");
        assert_eq!(week.headline, 30 + 15 + 2 * 9_999);
        assert_eq!(week.model, "shared");
    }

    #[test]
    fn the_model_named_is_the_one_that_spent_most_across_the_whole_week() {
        // Two hours and two providers, so the winner is decided by the sum rather than by
        // whichever bucket the walk happened to reach last.
        let mut claude = hours("2026-09-09T12", "claude-fable-5-1", spent(0, 0, 40, 0));
        claude
            .entry("2026-09-10T09".to_owned())
            .or_default()
            .insert("claude-opus-5".to_owned(), spent(0, 0, 30, 0));
        let mut providers = BTreeMap::new();
        providers.insert(PROVIDER.to_owned(), claude);
        providers.insert(
            PROVIDER_CODEX.to_owned(),
            hours("2026-09-11T08", "claude-opus-5", spent(0, 0, 25, 0)),
        );

        let week = fold(&providers, false).expect("a week with two models in it");
        assert_eq!(week.headline, 95);
        assert_eq!(
            week.model, "claude-opus-5",
            "30 + 25 beats 40, and the sum is what decides"
        );
    }

    #[test]
    fn a_tie_goes_to_the_model_that_sorts_first_so_the_tooltip_does_not_flicker() {
        let mut claude = hours("2026-09-09T12", "bbb", spent(0, 0, 50, 0));
        claude
            .entry("2026-09-09T13".to_owned())
            .or_default()
            .insert("aaa".to_owned(), spent(0, 0, 50, 0));
        let mut providers = BTreeMap::new();
        providers.insert(PROVIDER.to_owned(), claude);

        assert_eq!(
            fold(&providers, false)
                .expect("a tie is still a week")
                .model,
            "aaa"
        );
    }

    #[test]
    fn a_week_with_nothing_in_it_has_nothing_to_say() {
        assert_eq!(fold(&BTreeMap::new(), false), None, "an empty store");

        // A week that was genuinely idle. `0` is honest, but it is not worth a line the
        // quota sentence above it has to make room for. Since T-WP20b this is the *only*
        // way to reach `None`: every counter is in the sum, so a fold that comes to zero is
        // a week nothing was stored for rather than a week whose spending was the wrong
        // shape.
        let mut providers = BTreeMap::new();
        providers.insert(
            PROVIDER.to_owned(),
            hours("2026-09-09T12", "claude-opus-5", spent(0, 0, 0, 0)),
        );
        assert_eq!(fold(&providers, false), None, "an idle week");
    }

    #[test]
    fn a_week_of_nothing_but_cache_reads_is_a_week_that_spent_something() {
        // T-WP16's case, decided the other way. 1.5 billion cache reads used to fold to
        // `None`, on the argument that the number was about the cache rather than about the
        // work — and the tooltip then said nothing at all while `/usage`, one keystroke
        // away, said 1.5B. The panel's breakdown is where the cache is told apart now; the
        // headline is the total both windows are comparing.
        let mut providers = BTreeMap::new();
        providers.insert(
            PROVIDER.to_owned(),
            hours(
                "2026-09-09T12",
                "claude-opus-5",
                Bucket {
                    cache_read: 1_500_000_000,
                    requests: 3,
                    ..Bucket::default()
                },
            ),
        );

        let week = fold(&providers, false).expect("cache reads are tokens /usage counts");
        assert_eq!(week.headline, 1_500_000_000);
        assert_eq!(week.model, "claude-opus-5");
        assert_eq!(
            compact(week.headline),
            "1.5B",
            "and the tooltip can hold it"
        );
    }

    #[test]
    fn the_model_named_is_the_one_with_the_largest_four_way_total() {
        // The pick rides on the same sum as the headline, which reorders it: T-WP20 found
        // the same thing in the panel's rows. `heavy-cache` produces less than `all-work`
        // does on the three narrow counters and reads far more, and the model the tooltip
        // names is the one that moved the number the tooltip carries.
        let mut claude = hours("2026-09-09T12", "all-work", spent(100, 100, 100, 0));
        claude
            .entry("2026-09-09T13".to_owned())
            .or_default()
            .insert("heavy-cache".to_owned(), spent(1, 1, 1, 100_000));
        let mut providers = BTreeMap::new();
        providers.insert(PROVIDER.to_owned(), claude);

        let week = fold(&providers, false).expect("two models, one week");
        assert_eq!(week.headline, 300 + 3 + 100_000);
        assert_eq!(
            week.model, "heavy-cache",
            "the busiest model is the one with the largest total, cache included"
        );
    }

    #[test]
    fn a_model_nobody_named_is_still_a_model() {
        // `UNKNOWN_MODEL` is what the store files a record under when its source named no
        // model. It is data like any other id, printed as it is spelled rather than
        // translated or hidden — the same rule the panel's rows follow.
        let mut providers = BTreeMap::new();
        providers.insert(
            PROVIDER_CODEX.to_owned(),
            hours(
                "2026-09-09T12",
                nazar_core::usage::UNKNOWN_MODEL,
                spent(1, 1, 1, 0),
            ),
        );
        let week = fold(&providers, false).expect("an unnamed model still spent tokens");
        assert_eq!(week.model, "unknown");
    }

    #[test]
    fn the_week_the_panel_asked_about_is_the_week_the_tooltip_means() {
        // The whole design, in the only place it is observable without a panel: a `week`
        // request teaches this process where the reader's Monday is, and nothing else does.
        let state = UsageState::new(true);
        assert_eq!(
            state.phase(),
            UTC_MONDAY_PHASE,
            "before the panel has asked"
        );

        let monday = timefmt::unix_seconds_from_rfc3339("2026-09-07T00:00:00+03:00").unwrap();
        state.remember_week(week_phase(monday));
        assert_eq!(state.phase(), UTC_MONDAY_PHASE - 3 * 3_600);

        let now = timefmt::unix_seconds_from_rfc3339("2026-09-09T12:00:00Z").unwrap();
        let (from, to) = week_window(now, state.phase());
        assert_eq!(
            (
                timefmt::rfc3339_from_unix_seconds(from),
                timefmt::rfc3339_from_unix_seconds(to)
            ),
            (
                "2026-09-06T21:00:00Z".to_owned(),
                "2026-09-13T21:00:00Z".to_owned()
            )
        );
    }
}
