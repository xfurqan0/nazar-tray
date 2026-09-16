//! Token usage history, read from the logs Claude Code and Codex already keep.
//!
//! # What this reads, and what it refuses to
//!
//! `~/.claude/projects/**/*.jsonl` is where Claude Code records a session, and every
//! assistant message in it carries the token counts **the server reported for that
//! message**. Six values are taken: `type`, `timestamp`, `requestId`, `message.id`,
//! `message.model` and the four numbers under `message.usage`. Nothing else is named, and
//! what is not named is not built — `message.content`, `cwd`, `gitBranch`, `sessionId` and
//! the rest have no field in any struct here, so they are walked past by the deserialiser
//! and never become a string, a value, or a borrowed slice. `docs/pinned-internal-formats.md`
//! is the inventory; the leak test is the proof.
//!
//! `~/.codex/sessions/**/rollout-*.jsonl` is the second source and obeys the same rules
//! with different field names: every `token_count` event's `last_token_usage`, and the
//! model from the `turn_context` line that last named one. [`codex`] is that reader, and
//! the three things it exists to get right — the per-turn counter rather than the
//! cumulative one, the model that lives on another line, and the missing event id — are
//! written down there.
//!
//! This is the same discipline the Codex reader has had since the first commit, applied to
//! a second file format, and it is a reading of *reported* numbers rather than an estimate
//! of anything. The retired prototype that read this directory counted transcripts in order
//! to **guess** at a quota percentage the server had already answered; that is the thing the
//! project ruled out, and it is not this.
//!
//! # The shape of the thing
//!
//! ```text
//! ~/.claude/projects/**/*.jsonl ──▶ scan  ──▶ dedupe ─┐
//!  ~/.codex/sessions/**/*.jsonl ──▶ codex ────────────┼─▶ UTC-hour buckets
//!      (cursor: identity + byte offset)               │          │
//!                                          <state dir>/usage/YYYY-MM.json
//!                                                              │
//!                                       panel: the reader's own days and weeks
//! ```
//!
//! * [`scan`] walks the transcripts, including the sub-agent ones three levels down that
//!   are 78% of the bytes, and reads only what arrived since last time. Its incremental
//!   reader is shared: [`scan::read_new_lines`] is what both readers stand on.
//! * [`dedupe`] collapses the several lines Claude Code writes per message back into one
//!   message. Skipping this inflates the answer by 1.81× on the maintainer's machine, and
//!   the factor is not a constant that could be divided out afterwards.
//! * [`codex`] walks the rollout logs. Codex writes each event once and gives it no id, so
//!   there is nothing to deduplicate and nothing to deduplicate *with*: the cursor is the
//!   whole guarantee, plus one rule for the log that copies another log's opening events.
//! * [`store`] keeps the totals in monthly documents so that they survive the logs, which
//!   Claude Code prunes and a user may delete, and states the invariant that keeps a crash
//!   from counting anything twice. Each reader has its own cursor document; both file into
//!   the same months, under their own provider key and their own generation stamp.
//!
//! **`docs/usage-contract.md` is the document [`store`] writes**, written before this code
//! was: the shape, the spellings, the hourly UTC grain, what the file never contains, and
//! what happens to a month that no longer parses. `docs/pinned-internal-formats.md` is the
//! matching inventory of what is read out of a transcript and what is off limits.
//!
//! Everything here is UTC. A week that starts on Monday in the reader's own time zone is a
//! drawing decision and belongs in the panel, where asking the operating system for an
//! offset is one call; in this crate it is forbidden, and hourly rows are what make it
//! possible to draw one without ever having stored one.

pub mod codex;
pub mod dedupe;
pub mod reported;
pub mod scan;
pub mod store;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::timefmt::{now_rfc3339, unix_seconds_from_rfc3339};

pub use dedupe::{Credit, Credited, Deduper};
pub use reported::{Backfill, backfill_claude_stats, backfill_claude_stats_from};
pub use scan::{Record, UNKNOWN_MODEL, Usage};
pub use store::{
    Applied, Bucket, Hours, Models, Month, Months, PROVIDER, PROVIDER_CODEX, PROVIDER_REPORTED,
    ProviderTotals, Raw, VERSION, rebuild,
};

/// `<home>/.claude/projects` — where Claude Code keeps its transcripts.
///
/// Derived from the home directory the caller passes rather than from the environment, so
/// a test points the whole scan at a throwaway tree without touching the machine it runs
/// on. A caller that honours `CLAUDE_CONFIG_DIR` names the directory itself and calls
/// [`scan_claude_in`]; see [`crate::paths::claude_config_dir`].
#[must_use]
pub fn projects_dir(home: &Path) -> PathBuf {
    home.join(".claude").join("projects")
}

/// What one scan added, and what it decided not to count.
///
/// Every field is a count of something that happened, which is why they are plain numbers
/// rather than optional ones: a scan that read nothing read nothing, and that is a fact
/// rather than an unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSummary {
    /// Transcript files found under `projects/`.
    pub files_seen: u64,
    /// Files that had bytes nobody had read yet.
    pub files_read: u64,
    /// Files that could not be opened this time. Their cursors are kept as they were.
    pub files_unreadable: u64,
    /// Rollout logs Codex had compressed to `.jsonl.zst`, and this pass decoded.
    ///
    /// Not a count of what was missed. T-WP25 added this field while it was one — the
    /// reader knew the name and could not open the file — and T-WP26 gave it a decoder, so
    /// these logs are now walked, parsed and credited like any other. The number stays
    /// because it is still worth knowing how much of a machine's history is archived: it is
    /// the part of a total that came out of a file nobody can `grep`. Always `0` on the
    /// Claude side, which has no such format.
    #[serde(default)]
    pub files_compressed: u64,
    /// Files that had been replaced or truncated and were read from the top again.
    pub files_restarted: u64,
    /// Bytes read.
    pub bytes_read: u64,
    /// Lines that carried a usage object.
    pub lines: u64,
    /// Lines that carried one and were not JSON this crate could read.
    pub malformed: u64,
    /// Copies of a message that had already been counted.
    pub duplicates: u64,
    /// Messages keyed on `message.id` alone, for want of a `requestId`.
    pub dedupe_fallbacks: u64,
    /// Messages the server never billed, `model` being `<synthetic>`.
    pub synthetic: u64,
    /// Lines marked `isApiErrorMessage`: an error the server answered with, carrying a full
    /// set of counters and billed for none of them.
    #[serde(default)]
    pub skipped_api_errors: u64,
    /// Messages whose source named no model, filed under [`UNKNOWN_MODEL`].
    pub unnamed_model: u64,
    /// Usage lines dropped for want of a message id, a timestamp, or any number at all.
    pub unattributable: u64,
    /// Distinct messages credited.
    pub credited: u64,
    /// What the credited messages added up to.
    pub credited_total: u64,
    /// What the **lines behind them** added up to, with no dedupe at all.
    ///
    /// Not [`UsageSummary::naive_total`], which is every line this pass read including the
    /// ones it refused. This is what actually reached a bucket's `raw`, so on a pass that
    /// re-read a file it is the growth rather than the whole file again — and it is the
    /// number that has to stay equal across a rescan for the per-line side to be idempotent.
    #[serde(default)]
    pub raw_credited_total: u64,
    /// What the same lines would have added up to with no dedupe at all.
    ///
    /// Kept beside the real total so the inflation is observable rather than asserted:
    /// `naive_total / credited_total` is the factor, and on this machine it is not the
    /// same factor twice.
    pub naive_total: u64,
    /// The months whose documents this scan wrote.
    pub months: Vec<String>,
    /// The months whose documents do not parse. Left exactly as they are; see
    /// [`store::rebuild`].
    pub damaged: Vec<String>,
    /// The earliest instant the store holds anything for, RFC 3339 UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// When this scan ran, RFC 3339 UTC.
    pub scanned_at: String,
}

/// Scan Claude Code's transcripts and fold what is new into the monthly documents.
///
/// `home` is the user's home directory; `state_dir` is where this product keeps the
/// user's own files (`%APPDATA%\nazar` on Windows), under which the usage documents live
/// in `usage/`.
///
/// Safe to call repeatedly and safe to interrupt. Calling it twice over an unchanged tree
/// adds nothing the second time, because the cursor that says where each transcript was
/// read up to is committed in the same write as the totals read from it — the invariant is
/// spelled out in [`store`].
pub fn scan_claude(home: &Path, state_dir: &Path) -> Result<UsageSummary> {
    scan_claude_in(&projects_dir(home), state_dir)
}

/// [`scan_claude`] against an explicit `projects` directory.
///
/// The entry point for a caller that honours `CLAUDE_CONFIG_DIR`. That variable names a
/// configuration directory rather than a home, and it need not be called `.claude`, so a
/// scan derived from a home directory cannot express it: the only honest answer was to
/// scan nothing, which is what the tray did and wrote down as a gap. This is the directory
/// being named instead of guessed at.
pub fn scan_claude_in(projects_dir: &Path, state_dir: &Path) -> Result<UsageSummary> {
    let mut cursors = store::read_cursors(state_dir, PROVIDER)?;
    let mut summary = UsageSummary {
        scanned_at: now_rfc3339(),
        ..UsageSummary::default()
    };
    resume(state_dir, &mut cursors, &mut summary)?;

    let mut months = Months::new();
    let mut files: BTreeMap<String, store::FileCursor> = BTreeMap::new();

    for path in scan::transcripts(projects_dir) {
        summary.files_seen += 1;
        let key = store::path_key(&path);
        let previous = cursors.files.get(&key).cloned();

        let pass = match scan::scan_file(&path, previous.as_ref().map(store::FileCursor::resume)) {
            Ok(pass) => pass,
            Err(_) => {
                // A transcript that is locked, or gone between the walk and the read, is a
                // moment rather than a state. Its cursor is kept exactly as it was so the
                // next pass picks up where this one meant to.
                summary.files_unreadable += 1;
                if let Some(cursor) = previous {
                    files.insert(key, cursor);
                }
                continue;
            }
        };

        if pass.bytes > 0 {
            summary.files_read += 1;
        }
        if pass.restarted {
            summary.files_restarted += 1;
        }
        summary.bytes_read += pass.bytes;
        summary.lines += pass.lines;
        summary.malformed += pass.malformed;
        summary.synthetic += pass.skipped(scan::Skipped::Synthetic);
        summary.skipped_api_errors += pass.skipped(scan::Skipped::ApiError);
        summary.unattributable += pass.skipped(scan::Skipped::NoMessageId)
            + pass.skipped(scan::Skipped::NoTimestamp)
            + pass.skipped(scan::Skipped::NoNumbers);

        // Carried whatever happened to the file, and **especially** when the pass had to
        // start at byte zero: a truncated or rewritten transcript hands back records that
        // are already in the months, and this map is the only thing that knows it. A key
        // it does not hold is a message nothing has counted; a key it holds credits the
        // difference, which for the same bytes is nothing.
        let carried = previous
            .as_ref()
            .map(|cursor| cursor.credited.clone())
            .unwrap_or_default();
        // `rereading` is the per-line sum's half of the same idea: the deduplicated side
        // subtracts what it has already credited whichever way the pass arrived, while a sum
        // over *lines* has to know whether these are new bytes or the same bytes again.
        let mut deduper = Deduper::with_credited(&carried).rereading(pass.restarted);
        for item in deduper.reduce(pass.records) {
            let Some(month) = scan::month_of(&item.record.hour) else {
                continue;
            };
            summary.credited += u64::from(item.fresh);
            if item.fresh && item.record.model == UNKNOWN_MODEL {
                summary.unnamed_model += 1;
            }
            summary.credited_total = summary.credited_total.saturating_add(item.delta.total());
            summary.raw_credited_total =
                summary.raw_credited_total.saturating_add(item.raw.total());
            store::credit(
                months.entry(month).or_default(),
                &item.record.hour,
                &item.record.model,
                &item.delta,
                Some(&item.raw),
                item.fresh,
            );
        }
        summary.duplicates += deduper.duplicates;
        summary.dedupe_fallbacks += deduper.fallbacks;
        summary.naive_total = summary.naive_total.saturating_add(deduper.naive_total);

        files.insert(
            key,
            store::FileCursor {
                identity: pass.identity,
                offset: pass.offset,
                fingerprint: pass.fingerprint,
                credited: deduper.credited(),
                events: Vec::new(),
                model: None,
            },
        );
    }

    // Cursors for files that have gone are dropped — but only when the walk found
    // something. A walk that found nothing is far more likely to be a home directory that
    // was not there for a moment than a machine whose transcripts all vanished, and
    // forgetting every offset would count every surviving transcript a second time.
    if summary.files_seen > 0 {
        cursors.files = files;
    }

    commit(state_dir, &mut cursors, months, &mut summary)?;
    summary.since = earliest(state_dir)?;
    Ok(summary)
}

/// Scan Codex's rollout logs and fold what is new into the same monthly documents.
///
/// `home` is the user's home directory, under which Codex keeps `.codex/sessions`;
/// `state_dir` is the same one [`scan_claude`] writes to, and the totals land in the same
/// month documents under the provider key `codex`. A caller that honours `CODEX_HOME`
/// resolves it itself and calls [`scan_codex_home`].
///
/// Safe to call repeatedly and safe to interrupt, for the same reason and by the same
/// mechanism as the transcript scan — with one difference that is Codex's, not ours: a
/// rollout event carries no identifier, so the cursor is the *whole* of what keeps an event
/// from being counted twice. See [`codex`] for the one shape that gets past a byte offset
/// and the rule that catches it.
pub fn scan_codex(home: &Path, state_dir: &Path) -> Result<UsageSummary> {
    scan_codex_home(&codex::codex_dir(home), state_dir)
}

/// [`scan_codex`] against an explicit Codex home directory.
pub fn scan_codex_home(codex_home: &Path, state_dir: &Path) -> Result<UsageSummary> {
    let mut cursors = store::read_cursors(state_dir, PROVIDER_CODEX)?;
    let mut summary = UsageSummary {
        scanned_at: now_rfc3339(),
        ..UsageSummary::default()
    };
    resume(state_dir, &mut cursors, &mut summary)?;

    let mut months = Months::new();
    let mut files: BTreeMap<String, store::FileCursor> = BTreeMap::new();

    // The opening events of every log this store already knows, taken before the walk so
    // that the fork rule answers the same way whichever order the walk happens to produce.
    let known: Vec<(String, Vec<u64>)> = cursors
        .files
        .iter()
        .map(|(key, cursor)| {
            (
                key.clone(),
                cursor
                    .events
                    .iter()
                    .copied()
                    .take(codex::PREFIX_EVENTS)
                    .collect(),
            )
        })
        .collect();

    let walk = codex::find_rollouts(&codex::sessions_dir(codex_home));
    summary.files_compressed = walk.compressed;

    for path in walk.paths {
        summary.files_seen += 1;
        // Filed under the name the log has when it is **not** compressed, so that Codex's
        // sweep — and Codex putting the plain file back to append to it — is a file that
        // was replaced rather than a file nobody has ever read. The second reading is what
        // would count the session twice; see [`codex::cursor_path`].
        let key = store::path_key(&codex::cursor_path(&path));
        let previous = cursors.files.get(&key).cloned();

        let pass = match codex::scan_file(
            &path,
            previous.as_ref().map(store::FileCursor::resume),
            previous.as_ref().and_then(|cursor| cursor.model.as_deref()),
        ) {
            Ok(pass) => pass,
            Err(_) => {
                // A log that is locked, or gone between the walk and the read, is a moment
                // rather than a state. Its cursor is kept exactly as it was.
                summary.files_unreadable += 1;
                if let Some(cursor) = previous {
                    files.insert(key, cursor);
                }
                continue;
            }
        };

        if pass.bytes > 0 {
            summary.files_read += 1;
        }
        if pass.restarted {
            summary.files_restarted += 1;
        }
        summary.bytes_read += pass.bytes;
        summary.lines += pass.lines;
        summary.malformed += pass.malformed;
        summary.unattributable +=
            pass.skipped(codex::Skipped::NoTimestamp) + pass.skipped(codex::Skipped::NoNumbers);

        // Every event this log has already been credited for. A rollout carries no event
        // id, so this stands in for one — and a pass that restarted at byte zero is
        // re-reading events that are already in the months. `remaining` is that set with
        // multiplicity, consumed as the pass matches it, so a log that legitimately holds
        // two identical events is not made to lose one of them.
        let mut credited: Vec<u64> = previous
            .as_ref()
            .map(|cursor| cursor.events.clone())
            .unwrap_or_default();
        let mut remaining: BTreeMap<u64, usize> = BTreeMap::new();
        if pass.restarted {
            for fingerprint in &credited {
                *remaining.entry(*fingerprint).or_default() += 1;
            }
        }

        let seen_before = credited.len();
        let opening: Vec<u64> = credited
            .iter()
            .copied()
            .chain(pass.events.iter().map(|event| event.fingerprint))
            .take(codex::PREFIX_EVENTS)
            .collect();
        // Against every log the store already knew and every log read earlier in this
        // walk: a fork and its parent usually arrive in the same pass, and the parent is
        // read first because the paths sort as dates.
        let copied = codex::copied_prefix(
            &opening,
            known
                .iter()
                .filter(|(other, _)| *other != key)
                .map(|(_, seen)| seen.as_slice())
                .chain(
                    files
                        .iter()
                        .filter(|(other, _)| *other != &key)
                        .map(|(_, cursor)| {
                            let end = cursor.events.len().min(codex::PREFIX_EVENTS);
                            &cursor.events[..end]
                        }),
                ),
        );

        for (at, event) in pass.events.iter().enumerate() {
            summary.naive_total = summary.naive_total.saturating_add(event.usage.total());
            // An event this log has been credited for before, arriving again because the
            // bytes it was written in were read again.
            if let Some(left) = remaining.get_mut(&event.fingerprint)
                && *left > 0
            {
                *left -= 1;
                summary.duplicates += 1;
                continue;
            }
            // An event inside a run this log copied from another one. The tokens are real
            // and are already counted, under the log that was read first.
            if seen_before + at < copied {
                summary.duplicates += 1;
                continue;
            }
            let Some(month) = scan::month_of(&event.hour) else {
                continue;
            };
            credited.push(event.fingerprint);
            summary.credited += 1;
            if event.model == UNKNOWN_MODEL {
                summary.unnamed_model += 1;
            }
            summary.credited_total = summary.credited_total.saturating_add(event.usage.total());
            summary.raw_credited_total = summary
                .raw_credited_total
                .saturating_add(event.usage.total());
            // `None` rather than the same numbers twice: Codex writes each event once, so its
            // per-line sum **is** its deduplicated one, and an absent `raw` is how the store
            // says exactly that. See [`store::Bucket::add`].
            store::credit(
                months.entry(month).or_default(),
                &event.hour,
                &event.model,
                &event.usage,
                None,
                true,
            );
        }

        files.insert(
            key,
            store::FileCursor {
                identity: pass.identity,
                offset: pass.offset,
                fingerprint: pass.fingerprint,
                credited: Vec::new(),
                events: credited,
                model: pass.model,
            },
        );
    }

    // Cursors for logs that have gone are dropped — but only when the walk found
    // something, for the reason [`scan_claude`] gives.
    if summary.files_seen > 0 {
        cursors.files = files;
    }

    commit(state_dir, &mut cursors, months, &mut summary)?;
    summary.since = earliest(state_dir)?;
    Ok(summary)
}

/// File anything a previous run committed but could not finish filing.
///
/// Replaying an entry is a no-operation for a month that already carries its generation, so
/// this is safe however many times it runs — and it runs for as long as it has to: an entry
/// whose month is damaged stays in the journal until the month can be written, which may be
/// the next pass or may be after the user has dealt with the file. See the invariant in
/// [`store`].
fn resume(
    state_dir: &Path,
    cursors: &mut store::Cursors,
    summary: &mut UsageSummary,
) -> Result<()> {
    if cursors.pending.is_empty() {
        return Ok(());
    }
    let applied = store::apply(state_dir, &cursors.provider, &cursors.pending)?;
    cursors
        .pending
        .retain(|entry| applied.damaged.contains(&entry.month));
    summary.months = applied.written;
    summary.damaged = applied.damaged;
    store::write_cursors(state_dir, cursors)?;
    Ok(())
}

/// The commit point both readers go through.
///
/// A generation is a batch of totals, so a pass that found none does not take one: the
/// cursor document of a machine nothing has happened on stays byte for byte as it was.
/// A pass that did take one writes **the new offsets and the totals read from them in the
/// same atomic write**, and only then adds those totals to the month documents — which is
/// the whole of why a crash between the two cannot count anything twice.
///
/// What it then removes from the journal is **only what was filed**. A month that could not
/// be written keeps its entry, and the pass after it, and the one after that, will try
/// again; the alternative is what the first version did, which was to drop totals whose
/// bytes were already behind a moved cursor and could never be read again.
fn commit(
    state_dir: &Path,
    cursors: &mut store::Cursors,
    months: Months,
    summary: &mut UsageSummary,
) -> Result<()> {
    if !months.is_empty() {
        cursors.generation += 1;
        for (month, hours) in months {
            cursors.enqueue(store::Pending {
                month,
                generation: cursors.generation,
                scanned_at: summary.scanned_at.clone(),
                hours,
            });
        }
    }

    // The commit: the offsets and the totals read from them reach the disk together.
    store::write_cursors(state_dir, cursors)?;

    if !cursors.pending.is_empty() {
        let applied = store::apply(state_dir, &cursors.provider, &cursors.pending)?;
        cursors
            .pending
            .retain(|entry| applied.damaged.contains(&entry.month));
        merge(&mut summary.months, applied.written);
        merge(&mut summary.damaged, applied.damaged);
        store::write_cursors(state_dir, cursors)?;
    }
    Ok(())
}

/// Add what a second `apply` reported to what the first one did, without repeating a name.
fn merge(into: &mut Vec<String>, more: Vec<String>) {
    for name in more {
        if !into.contains(&name) {
            into.push(name);
        }
    }
    into.sort();
}

/// The hourly buckets the store holds for `[from, to)`, both RFC 3339 UTC instants.
///
/// Hours, not days and not weeks: the panel knows which offset the reader is in and this
/// crate is not allowed to, so the cutting into days and weeks that starts on a Monday
/// happens there. An hour is in the answer when **its start** falls inside the half-open
/// range.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageView {
    /// The earliest instant the store holds anything for, RFC 3339 UTC. The panel's
    /// "since {date}" line, and the reason "all time" is an honest label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// When a scan last wrote to the store, RFC 3339 UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanned_at: Option<String>,
    /// The start of the range asked for, as it was asked for.
    pub from: String,
    /// The end of the range asked for, as it was asked for.
    pub to: String,
    /// Provider (`claude`, `codex`) to UTC hour to model to totals.
    pub providers: BTreeMap<String, Hours>,
    /// The months whose documents do not parse, and so are missing from the answer.
    pub damaged: Vec<String>,
}

/// Read the store back for one UTC range.
///
/// Only the month documents the range touches are opened, plus the oldest and newest for
/// `since` and `scannedAt`. A range whose ends are not instants this crate can read yields
/// an empty view rather than an error: a panel asking for a week it cannot name is a bug
/// in the panel, and it should draw "no data" rather than a stack trace.
pub fn query(state_dir: &Path, from: &str, to: &str) -> Result<UsageView> {
    let mut view = UsageView {
        from: from.to_owned(),
        to: to.to_owned(),
        ..UsageView::default()
    };

    let known = store::months(state_dir)?;
    if let Some(first) = known.first()
        && let store::Read::Document(document) = store::read_month(state_dir, first)?
    {
        view.since = document.earliest().or(document.since);
    }
    if let Some(last) = known.last()
        && let store::Read::Document(document) = store::read_month(state_dir, last)?
    {
        view.scanned_at = document.scanned_at;
    }

    let (Some(start), Some(end)) = (
        unix_seconds_from_rfc3339(from),
        unix_seconds_from_rfc3339(to),
    ) else {
        return Ok(view);
    };
    if end <= start {
        return Ok(view);
    }

    for month in &known {
        if !touches(month, start, end) {
            continue;
        }
        let document = match store::read_month(state_dir, month)? {
            store::Read::Document(document) => document,
            store::Read::Absent => continue,
            store::Read::Damaged => {
                // The months beside it still load, and the panel is told which one to
                // point the user at. See `store::rebuild`.
                view.damaged.push(month.clone());
                continue;
            }
        };
        for (provider, totals) in document.providers {
            for (hour, models) in totals.buckets {
                let Some(at) = unix_seconds_from_rfc3339(&format!("{hour}:00:00Z")) else {
                    continue;
                };
                if at < start || at >= end {
                    continue;
                }
                let into = view
                    .providers
                    .entry(provider.clone())
                    .or_default()
                    .entry(hour)
                    .or_default();
                for (model, bucket) in models {
                    into.entry(model).or_default().absorb(&bucket);
                }
            }
        }
    }
    Ok(view)
}

/// Whether a `YYYY-MM` month can hold an hour inside `[start, end)`.
fn touches(month: &str, start: i64, end: i64) -> bool {
    let Some(first) = unix_seconds_from_rfc3339(&format!("{month}-01T00:00:00Z")) else {
        return false;
    };
    // The last instant a month can hold is 31 days minus an hour after its first; being
    // generous here costs one extra document read and never a missing row.
    let last = first + 31 * 86_400;
    first < end && last >= start
}

/// The earliest instant the store holds anything for.
fn earliest(state_dir: &Path) -> Result<Option<String>> {
    let known = store::months(state_dir)?;
    let Some(first) = known.first() else {
        return Ok(None);
    };
    match store::read_month(state_dir, first)? {
        store::Read::Document(document) => Ok(document.earliest().or(document.since)),
        store::Read::Absent | store::Read::Damaged => Ok(None),
    }
}

#[cfg(test)]
mod tests;
