//! Writing `~/.nazar/limits.json`, and mostly not writing it.
//!
//! The tray refreshes every sixty seconds. The numbers it reads change far less often than
//! that: a five-hour window moves when you send a prompt, not when a timer fires. So the
//! writer compares what it is about to write with what it wrote last time and, when they
//! are the same document, does nothing at all — no temporary file, no rename, no
//! modification time, nothing for a watching consumer to wake up for.
//!
//! The comparison ignores `updatedAt`, because otherwise every document would differ from
//! every other one and the test would never pass. Which means `updatedAt` moves **only when
//! the content moves**, and that is the honest reading of what the field has always said:
//! *when the tray last wrote this file*.
//!
//! ```text
//!   refresh ──▶ canonical bytes (updatedAt blanked)
//!                     │
//!                     ├── same as last time ──▶ nothing happens. The file keeps the
//!                     │                          updatedAt it earned.
//!                     └── different ──▶ stamp updatedAt = now, write atomically,
//!                                        remember the new canonical bytes.
//! ```
//!
//! A consumer that wants to know whether the *tray* is alive — as opposed to whether the
//! *numbers* are recent — reads the heartbeat in `~/.nazar/limits.lock` ([`crate::lock`]),
//! which is rewritten on every tick whether or not anything changed. The two questions are
//! different and now they have two answers instead of one that was wrong for both.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::limits::{Limits, write_limits};

/// What a write attempt did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// The document was identical to the last one written. Nothing touched the disk.
    Unchanged,
    /// The document changed and was written, stamped with this `updatedAt`.
    Wrote(String),
}

impl Written {
    /// Whether anything reached the disk.
    #[must_use]
    pub fn changed(&self) -> bool {
        matches!(self, Written::Wrote(_))
    }
}

/// The single writer of one `limits.json`.
///
/// Holds the last document it wrote, in its canonical form, so that "has anything changed"
/// is a string comparison rather than a file read. Constructing it does not touch the disk;
/// [`LimitsWriter::adopting`] does, once, so that a tray restarting over an unchanged file
/// does not rewrite it just to prove it is awake.
#[derive(Debug)]
pub struct LimitsWriter {
    path: PathBuf,
    canonical: Option<String>,
    writes: u64,
}

impl LimitsWriter {
    /// A writer that assumes nothing about what is already on disk.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        LimitsWriter {
            path: path.into(),
            canonical: None,
            writes: 0,
        }
    }

    /// A writer that takes the document already on disk as its starting point.
    ///
    /// A file that is missing, unreadable or not a contract document is simply no starting
    /// point: the first refresh then writes, which is what should happen.
    #[must_use]
    pub fn adopting(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let canonical = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| Limits::from_json(&text).ok())
            .and_then(|limits| canonical_form(&limits).ok());
        LimitsWriter {
            path,
            canonical,
            writes: 0,
        }
    }

    /// The file this writer owns.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many times this writer has actually written.
    #[must_use]
    pub fn writes(&self) -> u64 {
        self.writes
    }

    /// Write the document if it differs from the last one, stamping it with `now`.
    ///
    /// The stamp is applied to a copy: the caller's document is left exactly as it was, so
    /// that the snapshot held in memory and the file on disk cannot drift apart through a
    /// field only one of them has had set.
    pub fn write_if_changed(&mut self, limits: &Limits, now: &str) -> Result<Written> {
        let canonical = canonical_form(limits)?;
        if self.canonical.as_deref() == Some(canonical.as_str()) {
            return Ok(Written::Unchanged);
        }

        let mut stamped = limits.clone();
        stamped.updated_at = now.to_owned();
        write_limits(&self.path, &stamped)?;

        self.canonical = Some(canonical);
        self.writes += 1;
        Ok(Written::Wrote(stamped.updated_at))
    }
}

/// The document as it is compared: exactly what would be written, with `updatedAt` blanked.
///
/// Serialised rather than compared field by field on purpose. `Limits` carries three
/// `extra` maps for fields a newer build wrote, and a structural comparison that forgot one
/// of them would silently stop noticing changes in it.
pub fn canonical_form(limits: &Limits) -> Result<String> {
    let mut stripped = limits.clone();
    stripped.updated_at = String::new();
    stripped.to_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{Provider, Source, Window, read_limits};
    use crate::testutil::TempDir;

    fn document(percent: f64) -> Limits {
        let mut limits = Limits::new("2026-09-07T10:00:00Z");
        let mut windows = std::collections::BTreeMap::new();
        windows.insert(
            "primary".to_owned(),
            Window::ok(percent).with_window_minutes(300),
        );
        limits.providers.codex = Provider {
            configured: true,
            source: Some(Source::Rollout),
            source_at: Some("2026-09-07T09:59:00Z".to_owned()),
            binding: Some("primary".to_owned()),
            windows,
            ..Provider::default()
        };
        limits
    }

    #[test]
    fn the_first_document_is_written_and_stamped() {
        let dir = TempDir::new("writer-first");
        let path = dir.join("limits.json");
        let mut writer = LimitsWriter::new(&path);

        let written = writer
            .write_if_changed(&document(54.0), "2026-09-07T10:00:30Z")
            .unwrap();
        assert_eq!(written, Written::Wrote("2026-09-07T10:00:30Z".to_owned()));
        assert_eq!(
            read_limits(&path).unwrap().updated_at,
            "2026-09-07T10:00:30Z"
        );
        assert_eq!(writer.writes(), 1);
    }

    #[test]
    fn an_unchanged_document_does_not_touch_the_file() {
        let dir = TempDir::new("writer-unchanged");
        let path = dir.join("limits.json");
        let mut writer = LimitsWriter::new(&path);

        writer
            .write_if_changed(&document(54.0), "2026-09-07T10:00:30Z")
            .unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();

        // The same numbers, ten minutes later. The document the caller hands over even
        // carries a different `updatedAt`, and it still must not be written.
        let mut later = document(54.0);
        later.updated_at = "2026-09-07T10:10:00Z".to_owned();
        let written = writer
            .write_if_changed(&later, "2026-09-07T10:10:30Z")
            .unwrap();

        assert_eq!(written, Written::Unchanged);
        assert!(!written.changed());
        assert_eq!(writer.writes(), 1, "the second call wrote nothing");
        assert_eq!(
            read_limits(&path).unwrap().updated_at,
            "2026-09-07T10:00:30Z",
            "updatedAt moves with the content, not with the clock"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before,
            "the file was not even opened"
        );
    }

    #[test]
    fn a_changed_percentage_is_written() {
        let dir = TempDir::new("writer-changed");
        let path = dir.join("limits.json");
        let mut writer = LimitsWriter::new(&path);

        writer
            .write_if_changed(&document(54.0), "2026-09-07T10:00:30Z")
            .unwrap();
        let written = writer
            .write_if_changed(&document(55.0), "2026-09-07T10:01:30Z")
            .unwrap();

        assert_eq!(written, Written::Wrote("2026-09-07T10:01:30Z".to_owned()));
        assert_eq!(writer.writes(), 2);
        assert_eq!(
            read_limits(&path).unwrap().providers.codex.windows["primary"].percent,
            Some(55.0)
        );
    }

    #[test]
    fn the_callers_document_is_never_stamped_behind_its_back() {
        let dir = TempDir::new("writer-immutable");
        let path = dir.join("limits.json");
        let mut writer = LimitsWriter::new(&path);

        let limits = document(54.0);
        writer
            .write_if_changed(&limits, "2026-09-07T10:00:30Z")
            .unwrap();
        assert_eq!(limits.updated_at, "2026-09-07T10:00:00Z");
    }

    #[test]
    fn a_restart_over_an_unchanged_file_writes_nothing() {
        let dir = TempDir::new("writer-adopting");
        let path = dir.join("limits.json");

        let mut first = LimitsWriter::new(&path);
        first
            .write_if_changed(&document(54.0), "2026-09-07T10:00:30Z")
            .unwrap();
        drop(first);

        // A new process, the same numbers.
        let mut second = LimitsWriter::adopting(&path);
        assert_eq!(
            second
                .write_if_changed(&document(54.0), "2026-09-07T11:00:00Z")
                .unwrap(),
            Written::Unchanged
        );
        assert_eq!(
            read_limits(&path).unwrap().updated_at,
            "2026-09-07T10:00:30Z"
        );
    }

    #[test]
    fn adopting_a_file_that_is_not_a_document_starts_from_nothing() {
        let dir = TempDir::new("writer-adopting-damaged");
        let path = dir.join("limits.json");
        std::fs::write(&path, "{ this is not JSON").unwrap();

        let mut writer = LimitsWriter::adopting(&path);
        assert!(
            writer
                .write_if_changed(&document(54.0), "2026-09-07T10:00:30Z")
                .unwrap()
                .changed()
        );
    }

    #[test]
    fn a_field_only_a_newer_build_understands_still_counts_as_a_change() {
        let dir = TempDir::new("writer-extra");
        let path = dir.join("limits.json");
        let mut writer = LimitsWriter::new(&path);

        let first = document(54.0);
        writer
            .write_if_changed(&first, "2026-09-07T10:00:30Z")
            .unwrap();

        let mut second = document(54.0);
        second
            .providers
            .codex
            .extra
            .insert("quotaPool".to_owned(), serde_json::Value::from("shared"));
        assert!(
            writer
                .write_if_changed(&second, "2026-09-07T10:01:30Z")
                .unwrap()
                .changed(),
            "the comparison must see every field the writer would write"
        );
    }

    #[test]
    fn the_canonical_form_differs_only_by_the_stamp() {
        let mut early = document(54.0);
        early.updated_at = "2026-09-07T10:00:00Z".to_owned();
        let mut late = document(54.0);
        late.updated_at = "2027-01-01T00:00:00Z".to_owned();

        assert_eq!(
            canonical_form(&early).unwrap(),
            canonical_form(&late).unwrap()
        );
        assert_ne!(
            canonical_form(&early).unwrap(),
            canonical_form(&document(55.0)).unwrap()
        );
    }
}
