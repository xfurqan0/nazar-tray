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
//! Beside them, `<state dir>/usage/cursors.json`: where each transcript had been read up
//! to. It is this module's own bookkeeping rather than part of the contract, and it names
//! nothing — a transcript is filed under a hash of its path, not the path, because
//! `~/.claude/projects/` is named after every working directory somebody has opened a
//! session in; and a credited message is filed under a hash of its identifiers, not the
//! identifiers.
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
//!    the generation it last absorbed in `applied_through`. A pending block is added only
//!    when that stamp is older, so replaying the same block is a no-operation.
//!
//! The write order is: apply anything left pending from last time → read the transcripts →
//! **write the cursor with the new offsets and the new pending block** (this is the commit
//! point) → add the pending block to each month → write the cursor again with the pending
//! block cleared. A crash before the commit loses a pass that will simply be repeated; a
//! crash after it leaves a pending block that the next pass replays, skipping the months
//! that already carry its generation.
//!
//! Two states this cannot repair on its own, both deliberate:
//!
//! * A `cursors.json` that is no longer readable JSON is an **error**, not a fresh start.
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

/// The provider this module's scanner reads.
pub const PROVIDER: &str = "claude";

/// One model's totals for one UTC hour.
///
/// The counter names are the ones the sources already use, rather than `limits.json`'s
/// camelCase: this file is meant to be readable next to a raw `message.usage` block, and
/// the contract page argues the trade. Each measurement is absent until a record reported
/// it, so a bucket built from lines that never named an input count says so rather than
/// claiming zero.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bucket {
    /// `input_tokens`, summed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    /// `output_tokens`, summed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    /// `cache_creation_input_tokens`, summed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_create: Option<u64>,
    /// `cache_read_input_tokens`, summed. Around 98.5% of the raw total on a real machine,
    /// which is why the panel shows it beside the headline and never inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    /// Distinct messages, after dedupe.
    #[serde(default)]
    pub requests: u64,
    /// Fields a future version added. Preserved verbatim.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

impl Bucket {
    /// Add a credited reading to this bucket.
    ///
    /// `fresh` says whether this is a message nothing has counted before; a reading that
    /// is the rest of a message counted in an earlier pass adds its tokens without adding
    /// a request.
    pub fn add(&mut self, usage: &Usage, fresh: bool) {
        self.input = sum(self.input, usage.input);
        self.output = sum(self.output, usage.output);
        self.cache_create = sum(self.cache_create, usage.cache_create);
        self.cache_read = sum(self.cache_read, usage.cache_read);
        if fresh {
            self.requests = self.requests.saturating_add(1);
        }
    }

    /// Add another bucket's totals to this one.
    pub fn absorb(&mut self, other: &Bucket) {
        self.input = sum(self.input, other.input);
        self.output = sum(self.output, other.output);
        self.cache_create = sum(self.cache_create, other.cache_create);
        self.cache_read = sum(self.cache_read, other.cache_read);
        self.requests = self.requests.saturating_add(other.requests);
    }
}

/// Two measurements added, where absent is absent and not zero.
fn sum(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (None, None) => None,
        _ => Some(left.unwrap_or(0).saturating_add(right.unwrap_or(0))),
    }
}

/// Model name to totals, within one hour.
pub type Models = BTreeMap<String, Bucket>;
/// UTC hour (`YYYY-MM-DDTHH`) to the models seen in it.
pub type Hours = BTreeMap<String, Models>;
/// UTC month (`YYYY-MM`) to its hours.
pub type Months = BTreeMap<String, Hours>;

/// Add one credited reading to a set of hourly buckets.
pub fn credit(hours: &mut Hours, hour: &str, model: &str, usage: &Usage, fresh: bool) {
    hours
        .entry(hour.to_owned())
        .or_default()
        .entry(model.to_owned())
        .or_default()
        .add(usage, fresh);
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
    #[must_use]
    pub fn earliest(&self) -> Option<String> {
        self.providers
            .values()
            .filter_map(|totals| earliest_hour(&totals.buckets))
            .min()
    }
}

/// A pass's totals, written with the cursor before they are written to the months.
///
/// This is the write-ahead half of the invariant in the module documentation: it exists on
/// disk only between the commit and the moment every month has absorbed it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    /// Which provider's totals these are.
    pub provider: String,
    /// The generation they belong to.
    pub generation: u64,
    /// When the pass ran, RFC 3339 UTC.
    pub scanned_at: String,
    /// Month to hour to model to totals.
    #[serde(default)]
    pub months: Months,
}

impl Pending {
    /// `true` when there is nothing to apply.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.months.is_empty()
    }

    /// The earliest hour anywhere in this block, as an RFC 3339 instant.
    #[must_use]
    pub fn earliest(&self) -> Option<String> {
        self.months.values().filter_map(earliest_hour).min()
    }
}

/// Where one transcript had been read up to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCursor {
    /// The file's identity when it was last read. A different one means a different file.
    pub identity: String,
    /// Offset of the byte after the last complete line read.
    pub offset: u64,
    /// The last few dedupe keys credited from this file, so a message whose content blocks
    /// landed either side of the offset is still counted once. See [`super::dedupe`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<Credit>,
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
    /// Totals committed but not yet added to their month documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Pending>,
    /// Hash of a transcript's path to where it had been read up to.
    #[serde(default)]
    pub files: BTreeMap<String, FileCursor>,
    /// Fields a future version added. Preserved verbatim.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

impl Default for Cursors {
    fn default() -> Self {
        Cursors {
            version: VERSION,
            provider: PROVIDER.to_owned(),
            generation: 0,
            pending: None,
            files: BTreeMap::new(),
            extra: Map::new(),
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

/// `<state dir>/usage/cursors.json`.
#[must_use]
pub fn cursors_path(state_dir: &Path) -> PathBuf {
    usage_dir(state_dir).join("cursors.json")
}

/// Read the cursor document.
///
/// A document that is not there is a machine that has never scanned, and yields the empty
/// one. A document that is there and is not readable is an error, deliberately: see the
/// invariant in the module documentation.
pub fn read_cursors(state_dir: &Path) -> Result<Cursors> {
    let path = cursors_path(state_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Cursors::default());
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
    atomic::write_bytes(&cursors_path(state_dir), text.as_bytes())
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

/// Add a pending block to its month documents, skipping any that already carry it.
///
/// Idempotent by construction: a provider block whose `applied_through` is at or past the
/// block's generation is left exactly as it is, and a document that would not change is
/// not rewritten. The store-wide `since` is recomputed here and stamped into every
/// readable month, which is what makes that field mean what the contract says it means.
pub fn apply(state_dir: &Path, pending: &Pending) -> Result<Applied> {
    let mut outcome = Applied::default();
    let mut damaged = BTreeSet::new();
    let mut documents: BTreeMap<String, Month> = BTreeMap::new();

    let known = months(state_dir)?;
    for month in known.iter().chain(pending.months.keys()) {
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

    for (month, hours) in &pending.months {
        if damaged.contains(month) {
            continue;
        }
        let document = documents
            .entry(month.clone())
            .or_insert_with(|| Month::new(month));
        let totals = document
            .providers
            .entry(pending.provider.clone())
            .or_default();
        if totals.applied_through >= pending.generation {
            continue;
        }
        for (hour, models) in hours {
            let into = totals.buckets.entry(hour.clone()).or_default();
            for (model, bucket) in models {
                into.entry(model.clone()).or_default().absorb(bucket);
            }
        }
        totals.applied_through = pending.generation;
        document.scanned_at = Some(pending.scanned_at.clone());
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

/// Remove the cursor and every month document, so the next scan starts from the top.
///
/// The escape hatch for a damaged store, and the only safe way to ask for one: removing
/// the month documents without the cursor leaves a store that will never see those months
/// again, and removing the cursor without the month documents counts every surviving
/// transcript into months that already hold it. Whatever the transcripts no longer hold is
/// gone — which is the thing the store exists to avoid, so this is a decision somebody
/// makes rather than something a reader does to recover.
///
/// Returns the number of documents removed.
pub fn rebuild(state_dir: &Path) -> Result<usize> {
    let mut removed = 0;
    let cursors = cursors_path(state_dir);
    match std::fs::remove_file(&cursors) {
        Ok(()) => removed += 1,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(Error::io(&cursors, source)),
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

    fn pending(generation: u64, month: &str, hour: &str, usage: &Usage) -> Pending {
        let mut hours = Hours::new();
        credit(&mut hours, hour, "claude-opus-5", usage, true);
        let mut months = Months::new();
        months.insert(month.to_owned(), hours);
        Pending {
            provider: PROVIDER.to_owned(),
            generation,
            scanned_at: "2026-09-13T02:31:07Z".to_owned(),
            months,
        }
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
    fn an_absent_measurement_stays_absent() {
        let mut bucket = Bucket::default();
        bucket.add(
            &Usage {
                input: Some(2),
                output: None,
                cache_create: None,
                cache_read: None,
            },
            true,
        );
        assert_eq!(bucket.input, Some(2));
        assert_eq!(bucket.output, None, "never a reassuring zero");
        assert_eq!(bucket.requests, 1);
    }

    #[test]
    fn a_reading_that_completes_an_earlier_one_adds_no_request() {
        let mut bucket = Bucket::default();
        bucket.add(&usage(1, 10), true);
        bucket.add(&usage(0, 5), false);
        assert_eq!(bucket.output, Some(15));
        assert_eq!(bucket.requests, 1);
    }

    #[test]
    fn a_missing_cursor_document_is_an_empty_one() {
        let dir = TempDir::new("usage-store-missing");
        let cursors = read_cursors(&dir.path).unwrap();
        assert_eq!(cursors.generation, 0);
        assert!(cursors.files.is_empty());
    }

    #[test]
    fn a_broken_cursor_document_is_an_error_not_a_fresh_start() {
        let dir = TempDir::new("usage-store-broken");
        std::fs::create_dir_all(usage_dir(&dir.path)).unwrap();
        std::fs::write(cursors_path(&dir.path), b"{not json").unwrap();

        let outcome = read_cursors(&dir.path);
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
            true,
        );
        document
            .extra
            .insert("writer_build".to_owned(), "9.9".into());
        assert!(write_month(&dir.path, &document).unwrap());

        let read = self::document(&dir.path, "2026-09");
        assert_eq!(read.extra["writer_build"], Value::from("9.9"));
        assert_eq!(
            bucket(&dir.path, "2026-09", "2026-09-13T02").output,
            Some(328)
        );
    }

    #[test]
    fn the_counters_keep_the_spelling_the_sources_use() {
        let dir = TempDir::new("usage-store-spelling");
        apply(
            &dir.path,
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

        assert_eq!(apply(&dir.path, &block).unwrap().written, vec!["2026-09"]);
        let text = std::fs::read_to_string(month_path(&dir.path, "2026-09")).unwrap();

        assert!(
            apply(&dir.path, &block).unwrap().written.is_empty(),
            "the second apply must be a no-operation"
        );
        assert_eq!(
            std::fs::read_to_string(month_path(&dir.path, "2026-09")).unwrap(),
            text,
            "and must leave the document byte for byte as it was"
        );
        assert_eq!(
            bucket(&dir.path, "2026-09", "2026-09-13T02").output,
            Some(328)
        );
    }

    #[test]
    fn a_later_generation_adds_on_top() {
        let dir = TempDir::new("usage-store-later");
        apply(
            &dir.path,
            &pending(1, "2026-09", "2026-09-13T02", &usage(2, 328)),
        )
        .unwrap();
        apply(
            &dir.path,
            &pending(2, "2026-09", "2026-09-13T02", &usage(1, 2)),
        )
        .unwrap();

        let bucket = bucket(&dir.path, "2026-09", "2026-09-13T02");
        assert_eq!(bucket.output, Some(330));
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
            &pending(1, "2026-09", "2026-09-13T02", &usage(2, 328)),
        )
        .unwrap();

        let mut hours = Hours::new();
        credit(
            &mut hours,
            "2026-09-13T02",
            "gpt-6-astra",
            &usage(9, 9),
            true,
        );
        let mut months = Months::new();
        months.insert("2026-09".to_owned(), hours);
        apply(
            &dir.path,
            &Pending {
                provider: "codex".to_owned(),
                generation: 1,
                scanned_at: "2026-09-13T02:40:00Z".to_owned(),
                months,
            },
        )
        .unwrap();

        let read = document(&dir.path, "2026-09");
        assert_eq!(read.providers.len(), 2);
        assert_eq!(
            read.buckets("claude").unwrap()["2026-09-13T02"]["claude-opus-5"].output,
            Some(328)
        );
        assert_eq!(
            read.buckets("codex").unwrap()["2026-09-13T02"]["gpt-6-astra"].output,
            Some(9)
        );
    }

    #[test]
    fn a_damaged_month_is_reported_and_left_exactly_as_it_is() {
        let dir = TempDir::new("usage-store-damaged");
        apply(
            &dir.path,
            &pending(1, "2026-08", "2026-08-31T23", &usage(1, 1)),
        )
        .unwrap();

        let broken = "{ this was a month once";
        std::fs::write(month_path(&dir.path, "2026-08"), broken).unwrap();

        let outcome = apply(
            &dir.path,
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
            &pending(3, "2026-09", "2026-09-01T00", &usage(1, 1)),
        )
        .unwrap();
        assert_eq!(
            bucket(&dir.path, "2026-09", "2026-09-01T00").output,
            Some(1)
        );
    }

    #[test]
    fn only_month_shaped_names_are_month_documents() {
        let dir = TempDir::new("usage-store-months");
        apply(
            &dir.path,
            &pending(1, "2026-08", "2026-08-31T23", &usage(1, 1)),
        )
        .unwrap();
        apply(
            &dir.path,
            &pending(2, "2026-09", "2026-09-01T00", &usage(1, 1)),
        )
        .unwrap();
        write_cursors(&dir.path, &Cursors::default()).unwrap();
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
            &pending(1, "2026-09", "2026-09-01T00", &usage(1, 1)),
        )
        .unwrap();
        write_cursors(&dir.path, &Cursors::default()).unwrap();

        assert_eq!(rebuild(&dir.path).unwrap(), 2);
        assert!(months(&dir.path).unwrap().is_empty());
        assert_eq!(read_cursors(&dir.path).unwrap().generation, 0);
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
