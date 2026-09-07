//! Reading the new bytes at the end of a file that another process is appending to.
//!
//! Codex appends to a `rollout-*.jsonl` while it runs, so the reader must be able to poll
//! the same file repeatedly and see only what arrived since last time. Four things make
//! that safe, and each has a test:
//!
//! * **Byte offset.** Every read starts where the last one stopped, so a 12 MB log costs
//!   one `read_dir`, one `metadata` and a few hundred bytes per poll.
//! * **Partial lines.** A poll that lands mid-line keeps the fragment and joins it to the
//!   bytes that arrive next, instead of handing a truncated line to the parser.
//! * **Truncation and rotation.** A file that got shorter is a different file: the offset
//!   resets to the start of the window rather than seeking past the end.
//! * **Line endings.** `\r\n` and `\n` both delimit a line; the `\r` never reaches the
//!   parser. Codex writes `\n` today, but a log copied through a Windows tool will not.
//!
//! The tail also never starts at byte zero of a large file. A session log grows to tens
//! of megabytes and the newest quota line sits within two kilobytes of the end (measured:
//! 1 399–1 780 bytes across the four newest logs on the maintainer's machine), so the
//! first read opens a window at the end and drops the fragment before its first newline.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Bytes read back from the end of a file on the first poll.
///
/// Generous next to the ~2 KB actually needed, and small enough that opening a 12 MB log
/// costs a quarter of a megabyte rather than all of it.
pub const INITIAL_WINDOW: u64 = 256 * 1024;

/// Window used for the fallback pass over a file whose tail held no quota line.
///
/// Still bounded: a corrupt or pathological log must not be read into memory whole.
pub const FULL_WINDOW: u64 = 64 * 1024 * 1024;

/// Longest fragment kept while waiting for a newline.
///
/// A rollout line runs to a few hundred kilobytes at the outside. Beyond this the file is
/// not what we think it is, so the fragment is dropped and counted rather than grown.
const MAX_PARTIAL: usize = 8 * 1024 * 1024;

/// What one poll produced.
#[derive(Debug, Default)]
pub struct Batch {
    /// Complete lines that matched the caller's needle, oldest first, without the
    /// terminator and without a trailing `\r`.
    pub lines: Vec<String>,
    /// `true` when the file was shorter than the last read and the tail started over.
    pub restarted: bool,
    /// Lines dropped because they were not UTF-8, or fragments dropped for being absurd.
    pub dropped: u64,
}

/// An incremental reader over one append-only file.
#[derive(Debug)]
pub struct Tail {
    path: PathBuf,
    window: u64,
    offset: u64,
    /// Bytes after the last newline seen, waiting for the rest of their line.
    partial: Vec<u8>,
    /// `true` until the first newline of a window that began mid-line has been passed.
    skip_to_newline: bool,
    started: bool,
}

impl Tail {
    /// A tail over `path` that begins `window` bytes before the end of the file.
    ///
    /// Nothing is opened here; the file is opened on the first [`Tail::poll`].
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, window: u64) -> Self {
        Tail {
            path: path.into(),
            window,
            offset: 0,
            partial: Vec::new(),
            skip_to_newline: false,
            started: false,
        }
    }

    /// The file this tail is following.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read everything appended since the last call.
    ///
    /// Only lines containing `needle` are returned; the rest are counted as read and
    /// thrown away without being turned into a `String`. An empty needle keeps every
    /// line. The filter is what makes polling a 12 MB log cheap: the quota lines are a
    /// tenth of the entries and a hundredth of the bytes.
    pub fn poll(&mut self, needle: &[u8]) -> Result<Batch> {
        let mut batch = Batch::default();

        let metadata =
            std::fs::metadata(&self.path).map_err(|source| Error::io(&self.path, source))?;
        let length = metadata.len();

        if !self.started {
            self.started = true;
            self.open_window(length);
        } else if length < self.offset {
            // Truncated, rotated, or replaced. Start the window again rather than seek
            // past the end and read nothing forever.
            batch.restarted = true;
            self.partial.clear();
            self.open_window(length);
        }

        if length == self.offset {
            return Ok(batch);
        }

        let mut file = File::open(&self.path).map_err(|source| Error::io(&self.path, source))?;
        file.seek(SeekFrom::Start(self.offset))
            .map_err(|source| Error::io(&self.path, source))?;

        let mut chunk = Vec::new();
        let read = file
            .take(length.saturating_sub(self.offset))
            .read_to_end(&mut chunk)
            .map_err(|source| Error::io(&self.path, source))?;
        self.offset += read as u64;

        self.consume(&chunk, needle, &mut batch);
        Ok(batch)
    }

    /// Point the offset at the start of the window for a file of `length` bytes.
    ///
    /// When the window does not reach the start of the file it opens one byte early and
    /// arms [`Tail::skip_to_newline`]. That extra byte is what tells a window whose edge
    /// happens to fall exactly on a line boundary from one that cuts a line in half: in
    /// the first case the skip consumes the newline it starts on and keeps the whole
    /// line, in the second it discards the fragment.
    fn open_window(&mut self, length: u64) {
        if length > self.window {
            let start = length - self.window;
            self.offset = start.saturating_sub(1);
            self.skip_to_newline = true;
        } else {
            self.offset = 0;
            self.skip_to_newline = false;
        }
    }

    /// Split `chunk` into lines, joining the fragment left over from the previous poll.
    fn consume(&mut self, chunk: &[u8], needle: &[u8], batch: &mut Batch) {
        let mut rest = chunk;

        if self.skip_to_newline {
            match memchr(rest, b'\n') {
                Some(index) => {
                    self.skip_to_newline = false;
                    rest = &rest[index + 1..];
                }
                None => return, // still inside the fragment; wait for more bytes
            }
        }

        while let Some(index) = memchr(rest, b'\n') {
            let (line, tail) = rest.split_at(index);
            rest = &tail[1..];

            if self.partial.is_empty() {
                Self::emit(line, needle, batch);
            } else {
                self.partial.extend_from_slice(line);
                let joined = std::mem::take(&mut self.partial);
                Self::emit(&joined, needle, batch);
            }
        }

        if !rest.is_empty() {
            if self.partial.len() + rest.len() > MAX_PARTIAL {
                // Not a line we could use even if it ever terminated.
                self.partial.clear();
                self.skip_to_newline = true;
                batch.dropped += 1;
            } else {
                self.partial.extend_from_slice(rest);
            }
        }
    }

    /// Hand one complete line to the batch, if it matches and is text.
    fn emit(line: &[u8], needle: &[u8], batch: &mut Batch) {
        let line = match line.strip_suffix(b"\r") {
            Some(stripped) => stripped,
            None => line,
        };
        if line.is_empty() {
            return;
        }
        if !needle.is_empty() && !contains(line, needle) {
            return;
        }
        match std::str::from_utf8(line) {
            Ok(text) => batch.lines.push(text.to_owned()),
            // A log with non-UTF-8 in it is a log we do not understand. Counted, not
            // guessed at: a lossy conversion would hand the parser invented characters.
            Err(_) => batch.dropped += 1,
        }
    }
}

/// Index of the first `byte` in `haystack`.
fn memchr(haystack: &[u8], byte: u8) -> Option<usize> {
    haystack.iter().position(|candidate| *candidate == byte)
}

/// Whether `haystack` contains `needle`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return needle.is_empty();
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::io::Write;

    fn append(path: &Path, bytes: &[u8]) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(bytes).unwrap();
    }

    #[test]
    fn reads_only_what_arrived_since_the_last_poll() {
        let dir = TempDir::new("tail-append");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, b"one\ntwo\n").unwrap();

        let mut tail = Tail::new(&path, INITIAL_WINDOW);
        assert_eq!(tail.poll(b"").unwrap().lines, ["one", "two"]);
        assert!(tail.poll(b"").unwrap().lines.is_empty());

        append(&path, b"three\n");
        assert_eq!(tail.poll(b"").unwrap().lines, ["three"]);
        assert!(tail.poll(b"").unwrap().lines.is_empty());
    }

    #[test]
    fn a_line_split_across_two_polls_arrives_whole() {
        let dir = TempDir::new("tail-partial");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, b"{\"a\":1}\n{\"b\":").unwrap();

        let mut tail = Tail::new(&path, INITIAL_WINDOW);
        assert_eq!(tail.poll(b"").unwrap().lines, ["{\"a\":1}"]);

        append(&path, b"2}");
        assert!(
            tail.poll(b"").unwrap().lines.is_empty(),
            "an unterminated line is not a line yet"
        );

        append(&path, b"\n");
        assert_eq!(tail.poll(b"").unwrap().lines, ["{\"b\":2}"]);
    }

    #[test]
    fn a_line_split_three_ways_still_arrives_whole() {
        let dir = TempDir::new("tail-partial-3");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, b"abc").unwrap();

        let mut tail = Tail::new(&path, INITIAL_WINDOW);
        assert!(tail.poll(b"").unwrap().lines.is_empty());
        append(&path, b"def");
        assert!(tail.poll(b"").unwrap().lines.is_empty());
        append(&path, b"ghi\n");
        assert_eq!(tail.poll(b"").unwrap().lines, ["abcdefghi"]);
    }

    #[test]
    fn truncation_restarts_the_tail() {
        let dir = TempDir::new("tail-truncate");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, b"one\ntwo\nthree\n").unwrap();

        let mut tail = Tail::new(&path, INITIAL_WINDOW);
        assert_eq!(tail.poll(b"").unwrap().lines.len(), 3);

        // A new session reusing the same name, or a rotation.
        std::fs::write(&path, b"fresh\n").unwrap();
        let batch = tail.poll(b"").unwrap();
        assert!(batch.restarted, "the tail must notice a shorter file");
        assert_eq!(batch.lines, ["fresh"]);
    }

    #[test]
    fn truncation_mid_line_drops_the_fragment() {
        let dir = TempDir::new("tail-truncate-partial");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, b"one\nhalf-a-li").unwrap();

        let mut tail = Tail::new(&path, INITIAL_WINDOW);
        assert_eq!(tail.poll(b"").unwrap().lines, ["one"]);

        std::fs::write(&path, b"new\n").unwrap();
        let batch = tail.poll(b"").unwrap();
        assert!(batch.restarted);
        assert_eq!(
            batch.lines,
            ["new"],
            "the old fragment must not be glued on"
        );
    }

    #[test]
    fn crlf_and_lf_are_both_line_endings() {
        let dir = TempDir::new("tail-crlf");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, b"one\r\ntwo\nthree\r\n").unwrap();

        let mut tail = Tail::new(&path, INITIAL_WINDOW);
        assert_eq!(tail.poll(b"").unwrap().lines, ["one", "two", "three"]);
    }

    #[test]
    fn a_crlf_split_between_polls_is_still_one_ending() {
        let dir = TempDir::new("tail-crlf-split");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, b"one\r").unwrap();

        let mut tail = Tail::new(&path, INITIAL_WINDOW);
        assert!(tail.poll(b"").unwrap().lines.is_empty());
        append(&path, b"\ntwo\r\n");
        assert_eq!(tail.poll(b"").unwrap().lines, ["one", "two"]);
    }

    #[test]
    fn blank_lines_are_not_lines() {
        let dir = TempDir::new("tail-blank");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, b"one\n\n\r\ntwo\n").unwrap();

        let mut tail = Tail::new(&path, INITIAL_WINDOW);
        assert_eq!(tail.poll(b"").unwrap().lines, ["one", "two"]);
    }

    #[test]
    fn the_needle_filters_before_a_string_is_allocated() {
        let dir = TempDir::new("tail-needle");
        let path = dir.join("rollout.jsonl");
        std::fs::write(
            &path,
            b"{\"rate_limits\":1}\n{\"other\":2}\n{\"rate_limits\":3}\n",
        )
        .unwrap();

        let mut tail = Tail::new(&path, INITIAL_WINDOW);
        let batch = tail.poll(b"\"rate_limits\"").unwrap();
        assert_eq!(batch.lines, ["{\"rate_limits\":1}", "{\"rate_limits\":3}"]);
    }

    #[test]
    fn a_window_that_cuts_a_line_in_half_drops_that_line() {
        let dir = TempDir::new("tail-window-cut");
        let path = dir.join("rollout.jsonl");
        // Three eleven-byte lines; the file is 33 bytes.
        std::fs::write(&path, b"aaaaaaaaaa\nbbbbbbbbbb\ncccccccccc\n").unwrap();

        // 16 bytes back from the end lands inside the second line.
        let mut tail = Tail::new(&path, 16);
        assert_eq!(
            tail.poll(b"").unwrap().lines,
            ["cccccccccc"],
            "the fragment the window cut in half must be dropped, not parsed"
        );
    }

    #[test]
    fn a_window_that_lands_on_a_line_boundary_keeps_that_line() {
        let dir = TempDir::new("tail-window-boundary");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, b"aaaaaaaaaa\nbbbbbbbbbb\ncccccccccc\n").unwrap();

        // Exactly the last two lines. The line the window starts on is whole, so keeping
        // it is right; a window that always skipped would throw away a good quota line.
        let mut tail = Tail::new(&path, 22);
        assert_eq!(tail.poll(b"").unwrap().lines, ["bbbbbbbbbb", "cccccccccc"]);
    }

    #[test]
    fn a_window_larger_than_the_file_reads_all_of_it() {
        let dir = TempDir::new("tail-window-large");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, b"one\ntwo\n").unwrap();

        let mut tail = Tail::new(&path, INITIAL_WINDOW);
        assert_eq!(tail.poll(b"").unwrap().lines, ["one", "two"]);
    }

    #[test]
    fn invalid_utf8_is_counted_not_guessed() {
        let dir = TempDir::new("tail-utf8");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, b"good\n\xff\xfe bad\nalso good\n").unwrap();

        let mut tail = Tail::new(&path, INITIAL_WINDOW);
        let batch = tail.poll(b"").unwrap();
        assert_eq!(batch.lines, ["good", "also good"]);
        assert_eq!(batch.dropped, 1);
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        let dir = TempDir::new("tail-missing");
        let mut tail = Tail::new(dir.join("nope.jsonl"), INITIAL_WINDOW);
        assert!(tail.poll(b"").is_err());
    }

    #[test]
    fn an_absurd_fragment_is_dropped_rather_than_grown() {
        let dir = TempDir::new("tail-absurd");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, vec![b'x'; MAX_PARTIAL + 16]).unwrap();

        let mut tail = Tail::new(&path, FULL_WINDOW);
        let batch = tail.poll(b"").unwrap();
        assert!(batch.lines.is_empty());
        assert_eq!(batch.dropped, 1);

        // And it recovers on the next real line.
        append(&path, b"\nreal\n");
        assert_eq!(tail.poll(b"").unwrap().lines, ["real"]);
    }
}
