//! Where the totals live, and why a crash cannot count anything twice.
//!
//! Transcripts are not an archive. Claude Code prunes them — the maintainer's machine had
//! six days of them behind a setting that says ninety — so a panel that read them directly
//! would answer "all time" with "the last few days" and would answer differently every
//! week. The totals are therefore kept here, in this product's own files, and the
//! transcripts are only ever the source they were built from.
//!
//! **`docs/usage-contract.md` is the document this module writes**, and it was written
//! before this code was. What is here is the implementation of that page: the shape, the
//! spellings, the hourly UTC grain, the rule that a bucket never goes down, and the rule
//! that a damaged month is reported and left alone rather than repaired.
//!
//! ```json
//! {
//!   "version": 1,
//!   "month": "2026-09",
//!   "since": "2026-09-08T04:00:00Z",
//!   "scanned_at": "2026-09-13T02:31:07Z",
//!   "providers": {
//!     "claude": {
//!       "applied_through": 12,
//!       "buckets": {
//!         "2026-09-13T02": {
//!           "claude-opus-5": {"input": 2, "output": 328, "cache_create": 24843,
//!                             "cache_read": 35613, "requests": 1}
//!         }
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! Beside them, `<state dir>/usage/cursors-claude.json` and `cursors-codex.json`, one
//! document per reader: where each log had been read up to. It is this module's own
//! bookkeeping rather than part of the contract, and it names nothing — a log is filed
//! under a hash of its path, not the path, because `~/.claude/projects/` is named after
//! every working directory somebody has opened a session in; a credited message is filed
//! under a hash of its identifiers, not the identifiers; and a rollout's events are filed
//! under a hash of their timestamps and counters, not either.
//!
//! # The invariant
//!
//! **Every record is credited to exactly one month document exactly once, and neither a
//! crash between two writes nor a re-scan of the same bytes can change that.**
//!
//! It holds because of two facts and the order they are written in.
//!
//! 1. *A record is read once.* The cursor that covers a record and the totals computed
//!    from it are written **in the same atomic write** — the cursor document carries the
//!    scan's totals in [`Cursors::pending`] as it advances its offsets. So there is no
//!    state in which the offset moved past a record whose totals were not recorded.
//! 2. *A pending total is applied once.* Every scan that has something to file takes the
//!    next [`Cursors::generation`], and each provider's block in a month document records
//!    the generation it last absorbed in `applied_through`. A pending entry is added only
//!    when that stamp is older, so replaying the same entry is a no-operation.
//!
//! The write order is: apply anything left outstanding from last time → read the transcripts
//! → **write the cursor with the new offsets and the new pending entries** (this is the
//! commit point) → add each entry to its month → write the cursor again with the entries
//! that were filed removed. A crash before the commit loses a pass that will simply be
//! repeated; a crash after it leaves entries the next pass replays, skipping the months that
//! already carry their generation.
//!
//! **The journal is per month, and an entry that cannot be filed stays in it.** That is the
//! second half of the invariant and it was missing from the first version: a month whose
//! document no longer parses is skipped by [`apply`], and clearing the journal anyway threw
//! away totals whose bytes were already behind a cursor that had moved. Repairing the month
//! afterwards could not bring them back, because nothing would ever read those bytes again.
//! Now the entry waits — through any number of scans — and the next pass that finds the
//! month readable files it.
//!
//! Two states this cannot repair on its own, both deliberate:
//!
//! * A cursor document that is no longer readable JSON is an **error**, not a fresh start.
//!   Defaulting it would reset every offset to zero and count every surviving transcript
//!   into months that already hold it.
//! * A month document that is no longer readable JSON is **reported and left exactly as it
//!   is** — never replaced by an empty one and never repaired. The months beside it keep
//!   working. A lost month may not exist anywhere else, so deleting it is a thing the user
//!   does once they know.
//!
//! [`rebuild`] is the escape hatch both point at: it removes the cursor and the month
//! documents together, so the next scan counts whatever the transcripts still hold from
//! the top. Together, because removing either alone is the one way to double count.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::dedupe::Credit;
use super::scan::{Usage, fnv1a};
use crate::atomic;
use crate::error::{Error, Result};

/// Schema version written by this build. Its own number, unrelated to `limits.json`'s.
pub const VERSION: u32 = 1;

/// The provider the transcript scanner reads.
pub const PROVIDER: &str = "claude";

/// The provider the rollout scanner reads.
///
/// The two spellings `limits.json` already uses, and the two keys of `providers` in the
/// contract. Each reader keeps **its own cursor document** — see [`cursors_path`] — because
/// a pass replaces the set of files it knows about, and a reader sharing that set with
/// another would forget every offset the other had just written.
pub const PROVIDER_CODEX: &str = "codex";

/// The provider key days that come from Claude Code's own statistics cache are filed under.
///
/// **Never merged into [`PROVIDER`]**, and that is the whole of the design. Those days are
/// not a reading of anything this product did: they are one number per model per day as
/// another program computed it, for days whose transcripts no longer exist here, and the one
/// counter they carry is [`Bucket::reported_total`] rather than any of the five. A key of
/// their own is what lets the panel draw them differently, lets a reader sum the real
/// providers without them, and lets the whole block be thrown away and written again from the
/// file every time the setting that fills it is on. See [`super::reported`].
pub const PROVIDER_REPORTED: &str = "claude_reported";

/// One model's totals for one UTC hour.
///
/// The counter names are the ones the sources already use, rather than `limits.json`'s
/// camelCase: this file is meant to be readable next to a raw `message.usage` block, and
/// the contract page argues the trade.
///
/// **All five counters are always present, and a counter nothing reported is `0`.** The
/// first version let an unreported counter stay absent, on the argument that absent and zero
/// are different facts — which is true of one record and false of a *bucket*, where the
/// number being described is a sum over many. The contract says five non-negative integers,
/// and a document that sometimes omitted two of them did not write what the contract
/// promised: every reader would have needed the same `?? 0` the writer was avoiding. Where
/// the distinction is real it is kept — [`super::scan::Usage`] still says `None` for a
/// counter a line never named, which is how a line with no numbers at all is recognised and
/// skipped rather than counted as four zeroes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bucket {
    /// `input_tokens`, summed.
    #[serde(default)]
    pub input: u64,
    /// `output_tokens`, summed.
    #[serde(default)]
    pub output: u64,
    /// `cache_creation_input_tokens`, summed.
    #[serde(default)]
    pub cache_create: u64,
    /// `cache_read_input_tokens`, summed. Around 98.5% of the raw total on a real machine,
    /// which is why the panel shows it beside the headline and never inside it.
    #[serde(default)]
    pub cache_read: u64,
    /// Distinct messages, after dedupe.
    #[serde(default)]
    pub requests: u64,
    /// The same four token counters with **no dedupe at all**: every line added up.
    ///
    /// What Claude Code's `/usage` shows and what `~/.claude/stats-cache.json` holds, digit
    /// for digit. Optional, and its absence is a statement rather than a hole: **a bucket
    /// with no `raw` has a per-line sum equal to its five counters.** That is exactly true
    /// for Codex, which writes each event once and has no copies to collapse, and it is what
    /// a bucket written before T-WP22 is read as — the bytes those lines were in are behind a
    /// cursor that has already moved, so the real per-line sum is not recoverable and a
    /// factor of 1.7 applied to it would be an invented number.
    ///
    /// It is written the moment anything credits a per-line sum to the bucket, seeded from
    /// the five counters so that the rule above keeps holding; see [`Bucket::add`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Raw>,
    /// One number another program reported for a whole day, when this bucket is one of those.
    ///
    /// Only ever present under [`PROVIDER_REPORTED`], where it is the **only** number: the
    /// five counters are `0` there because `stats-cache.json` holds one total per model per
    /// day and no split, and writing a guess at the split would be inventing four numbers out
    /// of one. See [`super::reported`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_total: Option<u64>,
    /// Fields a future version added. Preserved verbatim.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

/// The four token counters as the lines carried them, before any copy was collapsed.
///
/// No `requests`: a request is a message and not a line, so the count beside the per-line
/// numbers is the deduplicated one in both readings. The contract says so where it says what
/// `requests` means, and a per-line count would be a count of content blocks under a label
/// that promises replies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Raw {
    /// `input_tokens`, summed over every line.
    #[serde(default)]
    pub input: u64,
    /// `output_tokens`, summed over every line.
    #[serde(default)]
    pub output: u64,
    /// `cache_creation_input_tokens`, summed over every line.
    #[serde(default)]
    pub cache_create: u64,
    /// `cache_read_input_tokens`, summed over every line.
    #[serde(default)]
    pub cache_read: u64,
}

impl Raw {
    /// Add a reading, treating an absent counter as nothing.
    fn add(&mut self, usage: &Usage) {
        self.input = self.input.saturating_add(usage.input.unwrap_or(0));
        self.output = self.output.saturating_add(usage.output.unwrap_or(0));
        self.cache_create = self
            .cache_create
            .saturating_add(usage.cache_create.unwrap_or(0));
        self.cache_read = self
            .cache_read
            .saturating_add(usage.cache_read.unwrap_or(0));
    }

    /// Add another set of per-line counters.
    fn absorb(&mut self, other: Raw) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_create = self.cache_create.saturating_add(other.cache_create);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
    }

    /// The four counters added together.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_create)
            .saturating_add(self.cache_read)
    }
}

impl Bucket {
    /// Add a credited reading to this bucket.
    ///
    /// `fresh` says whether this is a message nothing has counted before; a reading that
    /// is the rest of a message counted in an earlier pass adds its tokens without adding
    /// a request. A counter the reading never carried adds nothing, which is the one place
    /// absent becomes zero and the place the contract says it does.
    ///
    /// `raw` is the same reading with no dedupe — every line of that message added up — or
    /// `None` from a reader that has no copies to collapse, which is Codex's whole side of
    /// the store. `None` leaves an absent `raw` absent, and that is what keeps "absent means
    /// the same as the five counters" true rather than merely claimed.
    pub fn add(&mut self, usage: &Usage, raw: Option<&Usage>, fresh: bool) {
        // Before the five counters move, because an absent `raw` is seeded from them: it is
        // the statement "these were the same number", and the moment one of them grows the
        // other has to be written down.
        match raw {
            Some(raw) => {
                let mut counters = self.raw_counters();
                counters.add(raw);
                self.raw = Some(counters);
            }
            None => {
                if let Some(counters) = self.raw.as_mut() {
                    counters.add(usage);
                }
            }
        }

        self.input = self.input.saturating_add(usage.input.unwrap_or(0));
        self.output = self.output.saturating_add(usage.output.unwrap_or(0));
        self.cache_create = self
            .cache_create
            .saturating_add(usage.cache_create.unwrap_or(0));
        self.cache_read = self
            .cache_read
            .saturating_add(usage.cache_read.unwrap_or(0));
        if fresh {
            self.requests = self.requests.saturating_add(1);
        }
    }

    /// The per-line counters, or the five counters when this bucket has none.
    ///
    /// The one place the rule "absent `raw` means the same as the five counters" is read, so
    /// that nothing else has to remember it.
    #[must_use]
    pub fn raw_counters(&self) -> Raw {
        self.raw.unwrap_or(Raw {
            input: self.input,
            output: self.output,
            cache_create: self.cache_create,
            cache_read: self.cache_read,
        })
    }

    /// Add another bucket's totals to this one.
    pub fn absorb(&mut self, other: &Bucket) {
        // Absent plus absent stays absent — two buckets that both said "the same as the five
        // counters" add up to a third that says it too — and anything else is written out,
        // because one of the two sides knew something the five counters do not carry.
        if self.raw.is_some() || other.raw.is_some() {
            let mut counters = self.raw_counters();
            counters.absorb(other.raw_counters());
            self.raw = Some(counters);
        }
        if let Some(reported) = other.reported_total {
            self.reported_total = Some(self.reported_total.unwrap_or(0).saturating_add(reported));
        }
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_create = self.cache_create.saturating_add(other.cache_create);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.requests = self.requests.saturating_add(other.requests);
    }
}

/// Model name to totals, within one hour.
pub type Models = BTreeMap<String, Bucket>;
/// UTC hour (`YYYY-MM-DDTHH`) to the models seen in it.
pub type Hours = BTreeMap<String, Models>;
/// UTC month (`YYYY-MM`) to its hours.
pub type Months = BTreeMap<String, Hours>;

/// Add one credited reading to a set of hourly buckets.
///
/// `raw` is the same reading with no dedupe, or `None` from a reader whose source writes each
/// record once. See [`Bucket::add`].
pub fn credit(
    hours: &mut Hours,
    hour: &str,
    model: &str,
    usage: &Usage,
    raw: Option<&Usage>,
    fresh: bool,
) {
    hours
        .entry(hour.to_owned())
        .or_default()
        .entry(model.to_owned())
        .or_default()
        .add(usage, raw, fresh);
}

/// The earliest hour a set of buckets holds, as an RFC 3339 instant.
fn earliest_hour(hours: &Hours) -> Option<String> {
    hours.keys().next().map(|hour| format!("{hour}:00:00Z"))
}

/// One provider's totals within a month.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderTotals {
    /// The newest scan generation whose totals are already in `buckets`. This module's
    /// bookkeeping, per provider, so that two readers filing into the same month never
    /// share a stamp.
    #[serde(default)]
    pub applied_through: u64,
    /// UTC hour to model to totals. An hour in which nothing happened is absent, never a
    /// row of zeroes.
    #[serde(default)]
    pub buckets: Hours,
    /// Fields a future version added. Preserved verbatim.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

/// One month's totals, for every provider that has been read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Month {
    /// Schema version. See [`VERSION`].
    pub version: u32,
    /// The UTC month, `YYYY-MM`, and the same value as the file name.
    pub month: String,
    /// The earliest instant **the whole store** holds anything for, RFC 3339 UTC.
    ///
    /// Not the earliest in this file: it is what the panel's "since {date}" line reads,
    /// and it is the honest boundary of the words "all time".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// When the scan that produced this document finished, RFC 3339 UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanned_at: Option<String>,
    /// `claude`, `codex`. A provider that has never been read has no key at all.
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderTotals>,
    /// Fields a future version added. Preserved verbatim.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

impl Month {
    /// An empty document for `month`.
    #[must_use]
    pub fn new(month: &str) -> Self {
        Month {
            version: VERSION,
            month: month.to_owned(),
            since: None,
            scanned_at: None,
            providers: BTreeMap::new(),
            extra: Map::new(),
        }
    }

    /// One provider's hourly buckets, if it has any here.
    #[must_use]
    pub fn buckets(&self, provider: &str) -> Option<&Hours> {
        self.providers
            .get(provider)
            .map(|totals| &totals.buckets)
            .filter(|hours| !hours.is_empty())
    }

    /// The earliest hour any provider holds in this document, as an RFC 3339 instant.
    ///
    /// **[`PROVIDER_REPORTED`] is not a provider for this purpose**, and leaving it out is
    /// load-bearing rather than tidy. `since` is *the earliest instant this store measured
    /// anything*, the boundary the backfill fills days **before** — so counting a backfilled
    /// day in it would move the boundary back behind itself, empty the block on the next
    /// pass, move it forward again, and leave the store oscillating between two answers for
    /// as long as the setting is on.
    #[must_use]
    pub fn earliest(&self) -> Option<String> {
        self.measured()
            .filter_map(|(_, totals)| earliest_hour(&totals.buckets))
            .min()
    }

    /// The providers that are a reading of this machine's own logs, in key order.
    ///
    /// Everything except [`PROVIDER_REPORTED`], which is another program's arithmetic about
    /// days this store never saw.
    pub fn measured(&self) -> impl Iterator<Item = (&String, &ProviderTotals)> {
        self.providers
            .iter()
            .filter(|(provider, _)| provider.as_str() != PROVIDER_REPORTED)
    }
}

/// One month's totals from one pass, written with the cursor before they reach the month.
///
/// This is the write-ahead half of the invariant in the module documentation. **Per month
/// rather than per pass**, because that is the granularity at which filing can fail: a
/// damaged September must not take August's totals down with it, and must not lose its own.
/// An entry lives on disk from the commit until the month it names has absorbed it, which is
/// usually the next few milliseconds and is however long the user takes if the month is
/// damaged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    /// The UTC month these totals belong to, `YYYY-MM`.
    pub month: String,
    /// The generation they belong to. Compared with the month's `applied_through`.
    pub generation: u64,
    /// When the pass that read them ran, RFC 3339 UTC.
    pub scanned_at: String,
    /// Hour to model to totals.
    #[serde(default)]
    pub hours: Hours,
}

impl Pending {
    /// `true` when there is nothing to apply.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hours.is_empty()
    }

    /// The earliest hour in this entry, as an RFC 3339 instant.
    #[must_use]
    pub fn earliest(&self) -> Option<String> {
        earliest_hour(&self.hours)
    }

    /// Fold another pass's totals for the same month into this entry.
    ///
    /// What keeps the journal bounded while a month stays damaged: a scan every five
    /// minutes would otherwise leave an entry every five minutes, for as long as nobody
    /// fixes the file. The merged entry carries the **newer** generation, so filing it fills
    /// the month up to that generation in one step.
    ///
    /// The limit, written down rather than discovered: a user who "repairs" a damaged month
    /// by restoring an *older* copy of it — one whose `applied_through` is behind some of
    /// the generations merged here — gets those generations counted twice. Nothing can tell
    /// that document apart from the one that was damaged, which is why the contract's answer
    /// to a damaged month is a rebuild rather than surgery.
    fn absorb(&mut self, other: Pending) {
        for (hour, models) in other.hours {
            let into = self.hours.entry(hour).or_default();
            for (model, bucket) in models {
                into.entry(model).or_default().absorb(&bucket);
            }
        }
        self.generation = self.generation.max(other.generation);
        self.scanned_at = other.scanned_at;
    }
}

/// Where one log had been read up to.
///
/// Three of the six fields are one reader's and are absent in the other's document: a
/// transcript carries dedupe keys because Claude Code writes a message several times, and
/// a rollout carries its events' fingerprints and a model because Codex writes neither an id
/// nor a model on the event that spends the tokens.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCursor {
    /// The file's identity when it was last read. A different one means a different file.
    pub identity: String,
    /// Offset of the byte after the last complete line read.
    pub offset: u64,
    /// Hash of the bytes immediately before `offset`. A different one means the offset is
    /// no longer the place it was, even when the identity says the file is the same file —
    /// a rewrite in place. See [`super::scan::Resume`].
    #[serde(default)]
    pub fingerprint: u64,
    /// **Every** dedupe key credited from this transcript, and the most that was credited
    /// for each. The seed of the next pass's deduper, including a pass that had to start at
    /// byte zero — which is the case it exists for. See [`super::dedupe`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credited: Vec<Credit>,
    /// Fingerprints of **every** event credited from this rollout, in the order they were
    /// credited. Codex writes no event id, so this stands in for one: a log read again from
    /// byte zero recognises the events it has already counted, and a log whose *opening run*
    /// repeats another's is a fork of it (see [`super::codex`], which compares only the
    /// first [`super::codex::PREFIX_EVENTS`] of them). Hashes rather than the values they
    /// stand for, so the document still names nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<u64>,
    /// The model in force at the offset — the last one this log named before it. Carried
    /// because a `token_count` event does not name the model that produced it and the next
    /// pass starts after the `turn_context` that did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl FileCursor {
    /// This cursor as the reader takes it: identity, offset and fingerprint.
    #[must_use]
    pub fn resume(&self) -> super::scan::Resume<'_> {
        super::scan::Resume {
            identity: self.identity.as_str(),
            offset: self.offset,
            fingerprint: self.fingerprint,
        }
    }
}

/// The cursor document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursors {
    /// Schema version. See [`VERSION`].
    pub version: u32,
    /// Which provider these cursors are for.
    pub provider: String,
    /// The newest scan generation. Each pass that has something to file takes the next one.
    #[serde(default)]
    pub generation: u64,
    /// Totals committed but not yet added to their month documents, at most one entry per
    /// month. Normally empty; an entry survives a scan only when its month could not be
    /// written, which is the journal doing the thing it exists for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<Pending>,
    /// Hash of a transcript's path to where it had been read up to.
    #[serde(default)]
    pub files: BTreeMap<String, FileCursor>,
    /// Fields a future version added. Preserved verbatim.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

impl Cursors {
    /// The empty cursor document of a provider that has never been scanned.
    #[must_use]
    pub fn new(provider: &str) -> Self {
        Cursors {
            version: VERSION,
            provider: provider.to_owned(),
            generation: 0,
            pending: Vec::new(),
            files: BTreeMap::new(),
            extra: Map::new(),
        }
    }

    /// Add a month's totals to the journal, merging with an entry already outstanding.
    pub fn enqueue(&mut self, entry: Pending) {
        if let Some(outstanding) = self
            .pending
            .iter_mut()
            .find(|existing| existing.month == entry.month)
        {
            outstanding.absorb(entry);
        } else {
            self.pending.push(entry);
        }
    }
}

/// The key a transcript's cursor is filed under.
///
/// A hash, not the path: `~/.claude/projects/` names a directory after every working
/// directory somebody has opened a session in, and none of that belongs in a file this
/// product writes. The path is re-derived from the walk on every scan, so nothing is lost
/// by not keeping it.
#[must_use]
pub fn path_key(path: &Path) -> String {
    format!("{:016x}", fnv1a(path.to_string_lossy().as_bytes()))
}

/// `<state dir>/usage` — where the month documents and the cursor live.
#[must_use]
pub fn usage_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("usage")
}

/// `<state dir>/usage/YYYY-MM.json`.
#[must_use]
pub fn month_path(state_dir: &Path, month: &str) -> PathBuf {
    usage_dir(state_dir).join(format!("{month}.json"))
}

/// `<state dir>/usage/cursors-<provider>.json`.
///
/// One document per provider rather than one shared one. A pass writes the whole set of
/// files it walked, so two readers sharing a document would take turns forgetting each
/// other's offsets and counting the same bytes again; and the `generation` stamp each pass
/// takes is compared against a stamp kept **per provider** inside a month document, so two
/// sequences never meet. The provider name is a constant of this crate, never a value read
/// off a disk, which is what makes it safe in a file name.
///
/// Named after its provider on both sides, where the first reader used to have the bare
/// `cursors.json`: two documents that do the same job should not have two kinds of name, and
/// a reader looking at the directory should not have to know which one came first.
#[must_use]
pub fn cursors_path(state_dir: &Path, provider: &str) -> PathBuf {
    usage_dir(state_dir).join(format!("cursors-{provider}.json"))
}

/// Every cursor document in the store, whichever reader wrote it.
///
/// The bare `cursors.json` is in the list and is not written by anything: it is what the
/// transcript reader's cursor was called before it was named after its provider, and a
/// rebuild that left one behind would be a rebuild that did not reset the store.
fn cursor_documents(state_dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = [PROVIDER, PROVIDER_CODEX]
        .iter()
        .map(|provider| cursors_path(state_dir, provider))
        .collect();
    found.push(usage_dir(state_dir).join("cursors.json"));
    found
}

/// Read one provider's cursor document.
///
/// A document that is not there is a provider that has never been scanned, and yields the
/// empty one. A document that is there and is not readable is an error, deliberately: see
/// the invariant in the module documentation.
pub fn read_cursors(state_dir: &Path, provider: &str) -> Result<Cursors> {
    let path = cursors_path(state_dir, provider);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Cursors::new(provider));
        }
        Err(source) => return Err(Error::io(&path, source)),
    };
    serde_json::from_str(&text).map_err(|source| Error::json(&path, source))
}

/// Write the cursor document, atomically.
///
/// Compact rather than indented, unlike the month documents beside it: a month document is
/// the contract and somebody will open it, while this one is bookkeeping with an entry per
/// transcript and up to sixteen carried keys in each. Over 127 transcripts it was 302 KB
/// indented and 163 KB compact, and it is rewritten twice per scan.
pub fn write_cursors(state_dir: &Path, cursors: &Cursors) -> Result<()> {
    let mut text = serde_json::to_string(cursors)?;
    text.push('\n');
    atomic::write_bytes(&cursors_path(state_dir, &cursors.provider), text.as_bytes())
}

/// What reading a month document produced.
#[derive(Debug)]
pub enum Read {
    /// The document, as it is on disk.
    Document(Box<Month>),
    /// There is no document for that month.
    Absent,
    /// There is one and it does not parse. It is left exactly as it is.
    Damaged,
}

/// Read one month document, never failing on a damaged one.
pub fn read_month(state_dir: &Path, month: &str) -> Result<Read> {
    let path = month_path(state_dir, month);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Read::Absent),
        Err(source) => return Err(Error::io(&path, source)),
    };
    match serde_json::from_str::<Month>(&text) {
        Ok(document) => Ok(Read::Document(Box::new(document))),
        Err(_) => Ok(Read::Damaged),
    }
}

/// Write one month document, atomically, and only when it would differ from what is there.
///
/// Returns `true` when something was written. Comparing first is what makes a scan that
/// found nothing leave the directory byte for byte as it was, and it costs one read of a
/// file this pass has already read.
pub fn write_month(state_dir: &Path, document: &Month) -> Result<bool> {
    let path = month_path(state_dir, &document.month);
    let text = document_text(document)?;
    if std::fs::read_to_string(&path).is_ok_and(|existing| existing == text) {
        return Ok(false);
    }
    atomic::write_bytes(&path, text.as_bytes())?;
    Ok(true)
}

fn document_text<T: Serialize>(document: &T) -> Result<String> {
    let mut text = serde_json::to_string_pretty(document)?;
    text.push('\n');
    Ok(text)
}

/// Every month a document exists for, oldest first.
///
/// Only files named the way [`month_path`] names them count, so `cursors.json` and
/// anything a user dropped in the directory are passed over rather than parsed.
pub fn months(state_dir: &Path) -> Result<Vec<String>> {
    let dir = usage_dir(state_dir);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(Error::io(&dir, source)),
    };
    let mut found = Vec::new();
    for entry in entries.filter_map(std::result::Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if is_month(stem) {
            found.push(stem.to_owned());
        }
    }
    found.sort();
    Ok(found)
}

/// Whether `text` is a `YYYY-MM` month key.
#[must_use]
pub fn is_month(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 7
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..].iter().all(u8::is_ascii_digit)
}

/// What filing a pending block did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Months whose documents were written.
    pub written: Vec<String>,
    /// Months whose documents do not parse. Left exactly as they are, and skipped.
    pub damaged: Vec<String>,
}

/// Add outstanding journal entries to their month documents, skipping any that already
/// carry them.
///
/// Idempotent by construction: a provider block whose `applied_through` is at or past an
/// entry's generation is left exactly as it is, and a document that would not change is not
/// rewritten. The store-wide `since` is recomputed here and stamped into every readable
/// month, which is what makes that field mean what the contract says it means.
///
/// **A month that could not be read is named in [`Applied::damaged`] and nothing else
/// happens to it** — its entry is not applied and the caller is expected to keep it. Every
/// other entry is done with: absorbed, or skipped because the month already carries its
/// generation, which is the same thing from the journal's point of view.
pub fn apply(state_dir: &Path, provider: &str, pending: &[Pending]) -> Result<Applied> {
    let mut outcome = Applied::default();
    let mut damaged = BTreeSet::new();
    let mut documents: BTreeMap<String, Month> = BTreeMap::new();

    let known = months(state_dir)?;
    for month in known.iter().chain(pending.iter().map(|entry| &entry.month)) {
        if documents.contains_key(month) || damaged.contains(month) {
            continue;
        }
        match read_month(state_dir, month)? {
            Read::Document(document) => {
                documents.insert(month.clone(), *document);
            }
            Read::Absent => {}
            Read::Damaged => {
                damaged.insert(month.clone());
            }
        }
    }

    for entry in pending {
        if damaged.contains(&entry.month) {
            continue;
        }
        let document = documents
            .entry(entry.month.clone())
            .or_insert_with(|| Month::new(&entry.month));
        let totals = document.providers.entry(provider.to_owned()).or_default();
        if totals.applied_through >= entry.generation {
            continue;
        }
        for (hour, models) in &entry.hours {
            let into = totals.buckets.entry(hour.clone()).or_default();
            for (model, bucket) in models {
                into.entry(model.clone()).or_default().absorb(bucket);
            }
        }
        totals.applied_through = entry.generation;
        document.scanned_at = Some(entry.scanned_at.clone());
    }

    let since = documents.values().filter_map(Month::earliest).min();
    for (month, document) in &mut documents {
        document.version = VERSION;
        document.month = month.clone();
        document.since = since.clone();
        if write_month(state_dir, document)? {
            outcome.written.push(month.clone());
        }
    }
    outcome.damaged = damaged.into_iter().collect();
    Ok(outcome)
}

/// Remove every cursor document and every month document, so the next scan starts from the top.
///
/// The escape hatch for a damaged store, and the only safe way to ask for one: removing
/// the month documents without the cursors leaves a store that will never see those months
/// again, and removing the cursors without the month documents counts every surviving log
/// into months that already hold it. Whatever the transcripts no longer hold is
/// gone — which is the thing the store exists to avoid, so this is a decision somebody
/// makes rather than something a reader does to recover.
///
/// Returns the number of documents removed.
pub fn rebuild(state_dir: &Path) -> Result<usize> {
    let mut removed = 0;
    // Every provider's cursor, not one of them: a rebuild that cleared the months and left
    // a reader's offsets behind would lose that reader's history and never read it again.
    for cursors in cursor_documents(state_dir) {
        match std::fs::remove_file(&cursors) {
            Ok(()) => removed += 1,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(Error::io(&cursors, source)),
        }
    }
    for month in months(state_dir)? {
        let path = month_path(state_dir, &month);
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(Error::io(&path, source)),
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn usage(input: u64, output: u64) -> Usage {
        Usage {
            input: Some(input),
            output: Some(output),
            cache_create: None,
            cache_read: None,
        }
    }

    fn pending(generation: u64, month: &str, hour: &str, usage: &Usage) -> Vec<Pending> {
        let mut hours = Hours::new();
        credit(&mut hours, hour, "claude-opus-5", usage, None, true);
        vec![Pending {
            month: month.to_owned(),
            generation,
            scanned_at: "2026-09-13T02:31:07Z".to_owned(),
            hours,
        }]
    }

    fn document(dir: &Path, month: &str) -> Month {
        match read_month(dir, month).unwrap() {
            Read::Document(document) => *document,
            other => panic!("expected a document for {month}, got {other:?}"),
        }
    }

    fn bucket(dir: &Path, month: &str, hour: &str) -> Bucket {
        document(dir, month).buckets(PROVIDER).unwrap()[hour]["claude-opus-5"].clone()
    }

    #[test]
    fn a_counter_nothing_reported_is_zero_and_is_written() {
        let mut bucket = Bucket::default();
        bucket.add(
            &Usage {
                input: Some(2),
                output: None,
                cache_create: None,
                cache_read: None,
            },
            None,
            true,
        );
        assert_eq!(bucket.input, 2);
        assert_eq!(bucket.output, 0);
        assert_eq!(bucket.requests, 1);

        // The contract says five non-negative integers. All five are in the document.
        let text = serde_json::to_string(&bucket).unwrap();
        for key in [
            "\"input\"",
            "\"output\"",
            "\"cache_create\"",
            "\"cache_read\"",
            "\"requests\"",
        ] {
            assert!(
                text.contains(key),
                "the bucket does not spell {key}: {text}"
            );
        }
    }

    #[test]
    fn a_reading_that_completes_an_earlier_one_adds_no_request() {
        let mut bucket = Bucket::default();
        bucket.add(&usage(1, 10), None, true);
        bucket.add(&usage(0, 5), None, false);
        assert_eq!(bucket.output, 15);
        assert_eq!(bucket.requests, 1);
    }

    #[test]
    fn a_missing_cursor_document_is_an_empty_one() {
        let dir = TempDir::new("usage-store-missing");
        let cursors = read_cursors(&dir.path, PROVIDER).unwrap();
        assert_eq!(cursors.generation, 0);
        assert!(cursors.files.is_empty());
    }

    #[test]
    fn a_broken_cursor_document_is_an_error_not_a_fresh_start() {
        let dir = TempDir::new("usage-store-broken");
        std::fs::create_dir_all(usage_dir(&dir.path)).unwrap();
        std::fs::write(cursors_path(&dir.path, PROVIDER), b"{not json").unwrap();

        let outcome = read_cursors(&dir.path, PROVIDER);
        assert!(
            outcome.is_err(),
            "defaulting would re-count every transcript into months that already hold it"
        );
    }

    #[test]
    fn unknown_keys_survive_a_round_trip() {
        let dir = TempDir::new("usage-store-forward");
        let mut document = Month::new("2026-09");
        let totals = document.providers.entry(PROVIDER.to_owned()).or_default();
        credit(
            &mut totals.buckets,
            "2026-09-13T02",
            "claude-opus-5",
            &usage(2, 328),
            None,
            true,
        );
        document
            .extra
            .insert("writer_build".to_owned(), "9.9".into());
        assert!(write_month(&dir.path, &document).unwrap());

        let read = self::document(&dir.path, "2026-09");
        assert_eq!(read.extra["writer_build"], Value::from("9.9"));
        assert_eq!(bucket(&dir.path, "2026-09", "2026-09-13T02").output, 328);
    }

    #[test]
    fn the_counters_keep_the_spelling_the_sources_use() {
        let dir = TempDir::new("usage-store-spelling");
        apply(
            &dir.path,
            PROVIDER,
            &pending(
                1,
                "2026-09",
                "2026-09-13T02",
                &Usage {
                    input: Some(1),
                    output: Some(2),
                    cache_create: Some(3),
                    cache_read: Some(4),
                },
            ),
        )
        .unwrap();

        let text = std::fs::read_to_string(month_path(&dir.path, "2026-09")).unwrap();
        for key in [
            "\"cache_create\"",
            "\"cache_read\"",
            "\"scanned_at\"",
            "\"providers\"",
            "\"buckets\"",
        ] {
            assert!(text.contains(key), "the document does not spell {key}");
        }
    }

    #[test]
    fn applying_the_same_generation_twice_changes_nothing() {
        let dir = TempDir::new("usage-store-idempotent");
        let block = pending(1, "2026-09", "2026-09-13T02", &usage(2, 328));

        assert_eq!(
            apply(&dir.path, PROVIDER, &block).unwrap().written,
            vec!["2026-09"]
        );
        let text = std::fs::read_to_string(month_path(&dir.path, "2026-09")).unwrap();

        assert!(
            apply(&dir.path, PROVIDER, &block)
                .unwrap()
                .written
                .is_empty(),
            "the second apply must be a no-operation"
        );
        assert_eq!(
            std::fs::read_to_string(month_path(&dir.path, "2026-09")).unwrap(),
            text,
            "and must leave the document byte for byte as it was"
        );
        assert_eq!(bucket(&dir.path, "2026-09", "2026-09-13T02").output, 328);
    }

    #[test]
    fn a_later_generation_adds_on_top() {
        let dir = TempDir::new("usage-store-later");
        apply(
            &dir.path,
            PROVIDER,
            &pending(1, "2026-09", "2026-09-13T02", &usage(2, 328)),
        )
        .unwrap();
        apply(
            &dir.path,
            PROVIDER,
            &pending(2, "2026-09", "2026-09-13T02", &usage(1, 2)),
        )
        .unwrap();

        let bucket = bucket(&dir.path, "2026-09", "2026-09-13T02");
        assert_eq!(bucket.output, 330);
        assert_eq!(bucket.requests, 2);
        assert_eq!(
            document(&dir.path, "2026-09").since.as_deref(),
            Some("2026-09-13T02:00:00Z")
        );
    }

    #[test]
    fn since_is_the_whole_store_in_every_month_that_holds_it() {
        let dir = TempDir::new("usage-store-since");
        apply(
            &dir.path,
            PROVIDER,
            &pending(1, "2026-09", "2026-09-05T10", &usage(1, 1)),
        )
        .unwrap();
        assert_eq!(
            document(&dir.path, "2026-09").since.as_deref(),
            Some("2026-09-05T10:00:00Z")
        );

        // An older month arrives; the newer document's `since` moves with it.
        apply(
            &dir.path,
            PROVIDER,
            &pending(2, "2026-08", "2026-08-02T03", &usage(1, 1)),
        )
        .unwrap();
        for month in ["2026-08", "2026-09"] {
            assert_eq!(
                document(&dir.path, month).since.as_deref(),
                Some("2026-08-02T03:00:00Z"),
                "{month} carries the store's earliest instant, not its own"
            );
        }
    }

    #[test]
    fn two_providers_share_a_month_without_sharing_a_stamp() {
        let dir = TempDir::new("usage-store-providers");
        apply(
            &dir.path,
            PROVIDER,
            &pending(1, "2026-09", "2026-09-13T02", &usage(2, 328)),
        )
        .unwrap();

        let mut hours = Hours::new();
        credit(
            &mut hours,
            "2026-09-13T02",
            "gpt-6-astra",
            &usage(9, 9),
            None,
            true,
        );
        apply(
            &dir.path,
            PROVIDER_CODEX,
            &[Pending {
                month: "2026-09".to_owned(),
                generation: 1,
                scanned_at: "2026-09-13T02:40:00Z".to_owned(),
                hours,
            }],
        )
        .unwrap();

        let read = document(&dir.path, "2026-09");
        assert_eq!(read.providers.len(), 2);
        assert_eq!(
            read.buckets("claude").unwrap()["2026-09-13T02"]["claude-opus-5"].output,
            328
        );
        assert_eq!(
            read.buckets("codex").unwrap()["2026-09-13T02"]["gpt-6-astra"].output,
            9
        );
    }

    #[test]
    fn an_entry_for_a_damaged_month_is_the_callers_to_keep() {
        let dir = TempDir::new("usage-store-outstanding");
        apply(
            &dir.path,
            PROVIDER,
            &pending(1, "2026-08", "2026-08-31T23", &usage(1, 1)),
        )
        .unwrap();
        std::fs::write(month_path(&dir.path, "2026-08"), "{ not a month").unwrap();

        // One pass, two months: the damaged one is named and the other is filed.
        let mut entries = pending(2, "2026-08", "2026-08-31T23", &usage(5, 5));
        entries.extend(pending(2, "2026-09", "2026-09-01T00", &usage(7, 7)));
        let outcome = apply(&dir.path, PROVIDER, &entries).unwrap();
        assert_eq!(outcome.damaged, vec!["2026-08"]);
        assert_eq!(outcome.written, vec!["2026-09"]);

        // What a caller does with that: keep what could not be filed, drop the rest.
        entries.retain(|entry| outcome.damaged.contains(&entry.month));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].month, "2026-08");

        // The month is repaired — here, by being removed, which is what a user does — and
        // the entry that waited is filed, once.
        std::fs::remove_file(month_path(&dir.path, "2026-08")).unwrap();
        let outcome = apply(&dir.path, PROVIDER, &entries).unwrap();
        assert!(outcome.damaged.is_empty());
        assert_eq!(bucket(&dir.path, "2026-08", "2026-08-31T23").output, 5);

        let again = apply(&dir.path, PROVIDER, &entries).unwrap();
        assert!(again.written.is_empty(), "and replaying it changes nothing");
        assert_eq!(bucket(&dir.path, "2026-08", "2026-08-31T23").output, 5);
    }

    #[test]
    fn two_entries_for_one_month_merge_and_keep_the_newer_generation() {
        let mut outstanding = Cursors::new(PROVIDER);
        for entry in pending(4, "2026-09", "2026-09-01T00", &usage(1, 1)) {
            outstanding.enqueue(entry);
        }
        for entry in pending(5, "2026-09", "2026-09-01T01", &usage(2, 2)) {
            outstanding.enqueue(entry);
        }
        for entry in pending(5, "2026-10", "2026-10-01T00", &usage(3, 3)) {
            outstanding.enqueue(entry);
        }

        assert_eq!(outstanding.pending.len(), 2, "one entry per month");
        let september = &outstanding.pending[0];
        assert_eq!(september.month, "2026-09");
        assert_eq!(september.generation, 5);
        assert_eq!(september.hours.len(), 2);
    }

    #[test]
    fn a_damaged_month_is_reported_and_left_exactly_as_it_is() {
        let dir = TempDir::new("usage-store-damaged");
        apply(
            &dir.path,
            PROVIDER,
            &pending(1, "2026-08", "2026-08-31T23", &usage(1, 1)),
        )
        .unwrap();

        let broken = "{ this was a month once";
        std::fs::write(month_path(&dir.path, "2026-08"), broken).unwrap();

        let outcome = apply(
            &dir.path,
            PROVIDER,
            &pending(2, "2026-08", "2026-08-31T23", &usage(5, 5)),
        )
        .unwrap();
        assert_eq!(outcome.damaged, vec!["2026-08"]);
        assert_eq!(
            std::fs::read_to_string(month_path(&dir.path, "2026-08")).unwrap(),
            broken,
            "never replaced by an empty one and never repaired"
        );

        // And the months beside it keep working.
        apply(
            &dir.path,
            PROVIDER,
            &pending(3, "2026-09", "2026-09-01T00", &usage(1, 1)),
        )
        .unwrap();
        assert_eq!(bucket(&dir.path, "2026-09", "2026-09-01T00").output, 1);
    }

    #[test]
    fn only_month_shaped_names_are_month_documents() {
        let dir = TempDir::new("usage-store-months");
        apply(
            &dir.path,
            PROVIDER,
            &pending(1, "2026-08", "2026-08-31T23", &usage(1, 1)),
        )
        .unwrap();
        apply(
            &dir.path,
            PROVIDER,
            &pending(2, "2026-09", "2026-09-01T00", &usage(1, 1)),
        )
        .unwrap();
        write_cursors(&dir.path, &Cursors::new(PROVIDER)).unwrap();
        std::fs::write(usage_dir(&dir.path).join("notes.json"), b"{}").unwrap();

        assert_eq!(months(&dir.path).unwrap(), vec!["2026-08", "2026-09"]);
        assert!(is_month("2026-09"));
        assert!(!is_month("cursors"));
        assert!(!is_month("2026-9"));
    }

    #[test]
    fn rebuilding_takes_the_cursor_and_the_months_together() {
        let dir = TempDir::new("usage-store-rebuild");
        apply(
            &dir.path,
            PROVIDER,
            &pending(1, "2026-09", "2026-09-01T00", &usage(1, 1)),
        )
        .unwrap();
        write_cursors(&dir.path, &Cursors::new(PROVIDER)).unwrap();

        assert_eq!(rebuild(&dir.path).unwrap(), 2);
        assert!(months(&dir.path).unwrap().is_empty());
        assert_eq!(read_cursors(&dir.path, PROVIDER).unwrap().generation, 0);
        assert_eq!(rebuild(&dir.path).unwrap(), 0, "and is safe to repeat");
    }

    #[test]
    fn a_cursor_document_names_no_directory_anybody_works_in() {
        let key = path_key(Path::new(
            "/home/somebody/.claude/projects/a-project/x.jsonl",
        ));
        assert_eq!(key.len(), 16);
        assert!(!key.contains("project"));
    }
}
