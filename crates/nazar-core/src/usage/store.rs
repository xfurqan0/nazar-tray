//! Where the totals live, and why a crash cannot count anything twice.
//!
//! Transcripts are not an archive. Claude Code prunes them — the maintainer's machine had
//! six days of them behind a setting that says ninety — so a panel that read them directly
//! would answer "all time" with "the last few days" and would answer differently every
//! week. The totals are therefore kept here, in this product's own files, and the
//! transcripts are only ever the source they were built from.
//!
//! # The documents
//!
//! One file per **UTC** month, `<state dir>/usage/YYYY-MM.json`, each written whole and
//! atomically ([`crate::atomic`]) the way everything else in this crate is written:
//!
//! ```json
//! {
//!   "version": 1,
//!   "provider": "claude",
//!   "month": "2026-09",
//!   "since": "2026-09-08T04:00:00Z",
//!   "scannedAt": "2026-09-13T02:31:07Z",
//!   "appliedThrough": 12,
//!   "hours": {
//!     "2026-09-13T02": {
//!       "claude-opus-5": {"input": 2, "output": 328, "cacheCreate": 24843,
//!                         "cacheRead": 35613, "requests": 1}
//!     }
//!   }
//! }
//! ```
//!
//! Hours rather than days, and UTC rather than anything else, for one reason each. UTC
//! because this crate is forbidden to ask the machine what time zone it is in — resets and
//! instants are arithmetic on UTC and rendering a calendar is the panel's job, in
//! JavaScript, where it is one call. Hours because a row that is a UTC *day* cannot be
//! re-cut into the reader's day, and a row that is a UTC *hour* can: every offset the world
//! uses is a whole number of hours or a half of one, and a half-hour offset moves whole
//! hours between days without splitting one.
//!
//! Beside them, `<state dir>/usage/cursors.json`: where each transcript had been read up
//! to, keyed by a hash of its path rather than the path, so the file cannot be read as a
//! list of the directories somebody works in.
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
//! 2. *A pending total is applied once.* Every scan takes the next [`Cursors::generation`],
//!    and a month document records the generation it last absorbed in `appliedThrough`. A
//!    pending block is added to a month only when the month's stamp is older, so replaying
//!    the same block is a no-operation.
//!
//! The write order is: apply anything left pending from last time → read the transcripts →
//! **write the cursor with the new offsets and the new pending block** (this is the commit
//! point) → add the pending block to each month → write the cursor again with the pending
//! block cleared. A crash before the commit loses a pass that will simply be repeated; a
//! crash after it leaves a pending block that the next pass replays, skipping the months
//! that already carry its generation.
//!
//! The one state this cannot repair is a `cursors.json` that is no longer readable JSON.
//! Defaulting it would reset every offset to zero and count every surviving transcript into
//! months that already hold it, so it is an error instead, and recovering is a decision
//! somebody makes: delete the usage directory and rebuild from whatever transcripts are
//! still on the machine.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::dedupe::Credit;
use super::scan::{Usage, fnv1a};
use crate::atomic;
use crate::error::{Error, Result};

/// Schema version written by this build.
pub const VERSION: u32 = 1;

/// The provider these documents describe.
pub const PROVIDER: &str = "claude";

/// One model's totals for one UTC hour.
///
/// Each measurement is absent until a record reported it, so a bucket built from lines
/// that never named an input count says so rather than claiming zero. `requests` counts
/// distinct messages, after dedupe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// which is why the panel shows it on its own line instead of inside the headline.
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

/// One month's totals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Month {
    /// Schema version. See [`VERSION`].
    pub version: u32,
    /// Which provider these totals are for.
    pub provider: String,
    /// The UTC month, `YYYY-MM`.
    pub month: String,
    /// The earliest instant this document holds anything for, RFC 3339 UTC.
    ///
    /// The panel needs it: "all time" means "since the first scan, plus however far back
    /// the transcripts went that day", and a label that does not say which is dishonest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// When a scan last added to this document, RFC 3339 UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanned_at: Option<String>,
    /// The newest scan generation whose totals are already in `hours`.
    #[serde(default)]
    pub applied_through: u64,
    /// UTC hour to model to totals.
    #[serde(default)]
    pub hours: Hours,
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
            provider: PROVIDER.to_owned(),
            month: month.to_owned(),
            since: None,
            scanned_at: None,
            applied_through: 0,
            hours: Hours::new(),
            extra: Map::new(),
        }
    }

    /// The earliest hour this document holds, as an RFC 3339 instant.
    #[must_use]
    pub fn earliest(&self) -> Option<String> {
        self.hours
            .keys()
            .next()
            .map(|hour| format!("{hour}:00:00Z"))
    }
}

/// A pass's totals, written with the cursor before they are written to the months.
///
/// This is the write-ahead half of the invariant in the module documentation: it exists on
/// disk only between the commit and the moment every month has absorbed it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    /// The generation these totals belong to.
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
}

/// Where one transcript had been read up to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct Cursors {
    /// Schema version. See [`VERSION`].
    pub version: u32,
    /// Which provider these cursors are for.
    pub provider: String,
    /// The newest scan generation. Each pass takes the next one.
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
pub fn write_cursors(state_dir: &Path, cursors: &Cursors) -> Result<()> {
    write_document(&cursors_path(state_dir), cursors)
}

/// Read one month document, if it exists.
pub fn read_month(state_dir: &Path, month: &str) -> Result<Option<Month>> {
    let path = month_path(state_dir, month);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(Error::io(&path, source)),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|source| Error::json(&path, source))
}

/// Write one month document, atomically.
pub fn write_month(state_dir: &Path, month: &Month) -> Result<()> {
    write_document(&month_path(state_dir, &month.month), month)
}

fn write_document<T: Serialize>(path: &Path, document: &T) -> Result<()> {
    let mut text = serde_json::to_string_pretty(document)?;
    text.push('\n');
    atomic::write_bytes(path, text.as_bytes())
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

/// Add a pending block to its month documents, skipping any that already carry it.
///
/// Idempotent by construction: a month whose `appliedThrough` is at or past the block's
/// generation is left exactly as it is. Returns the months that were written.
pub fn apply(state_dir: &Path, pending: &Pending) -> Result<Vec<String>> {
    let mut written = Vec::new();
    for (month, hours) in &pending.months {
        let mut document = read_month(state_dir, month)?.unwrap_or_else(|| Month::new(month));
        if document.applied_through >= pending.generation {
            continue;
        }
        for (hour, models) in hours {
            let into = document.hours.entry(hour.clone()).or_default();
            for (model, bucket) in models {
                into.entry(model.clone()).or_default().absorb(bucket);
            }
        }
        document.version = VERSION;
        document.provider = PROVIDER.to_owned();
        document.month = month.clone();
        document.applied_through = pending.generation;
        document.scanned_at = Some(pending.scanned_at.clone());
        document.since = document.earliest();
        write_month(state_dir, &document)?;
        written.push(month.clone());
    }
    Ok(written)
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
            generation,
            scanned_at: "2026-09-13T02:31:07Z".to_owned(),
            months,
        }
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
        credit(
            &mut document.hours,
            "2026-09-13T02",
            "claude-opus-5",
            &usage(2, 328),
            true,
        );
        document
            .extra
            .insert("writerBuild".to_owned(), "9.9".into());
        write_month(&dir.path, &document).unwrap();

        let read = read_month(&dir.path, "2026-09").unwrap().unwrap();
        assert_eq!(read.extra["writerBuild"], Value::from("9.9"));
        assert_eq!(
            read.hours["2026-09-13T02"]["claude-opus-5"].output,
            Some(328)
        );
    }

    #[test]
    fn applying_the_same_generation_twice_changes_nothing() {
        let dir = TempDir::new("usage-store-idempotent");
        let block = pending(1, "2026-09", "2026-09-13T02", &usage(2, 328));

        assert_eq!(apply(&dir.path, &block).unwrap(), vec!["2026-09"]);
        assert!(
            apply(&dir.path, &block).unwrap().is_empty(),
            "the second apply must be a no-operation"
        );

        let read = read_month(&dir.path, "2026-09").unwrap().unwrap();
        assert_eq!(
            read.hours["2026-09-13T02"]["claude-opus-5"].output,
            Some(328)
        );
        assert_eq!(read.applied_through, 1);
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

        let read = read_month(&dir.path, "2026-09").unwrap().unwrap();
        let bucket = &read.hours["2026-09-13T02"]["claude-opus-5"];
        assert_eq!(bucket.output, Some(330));
        assert_eq!(bucket.requests, 2);
        assert_eq!(read.since.as_deref(), Some("2026-09-13T02:00:00Z"));
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
    fn a_cursor_document_names_no_directory_anybody_works_in() {
        let key = path_key(Path::new(
            "/home/somebody/.claude/projects/a-project/x.jsonl",
        ));
        assert_eq!(key.len(), 16);
        assert!(!key.contains("project"));
    }
}
