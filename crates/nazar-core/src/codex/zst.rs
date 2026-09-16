//! Reading a rollout log Codex has compressed.
//!
//! Codex has shipped a worker since 0.153.4 that rewrites every rollout whose modification
//! time is more than seven days old as `<name>.jsonl.zst` and **deletes the plain file**.
//! Until T-WP26 this crate knew the name and nothing else: [`crate::codex`] answered
//! "the rollouts are compressed" instead of "there are none", and [`crate::usage`] counted
//! them instead of walking past them. This module is the other half — the one that opens
//! them.
//!
//! ## Why decode the whole file rather than a window
//!
//! The plain reader never starts at byte zero: a live rollout grows to tens of megabytes
//! and the newest quota line sits within two kilobytes of the end, so [`super::tail`] opens
//! a window at the end and walks forward. A zstd frame cannot be entered in the middle —
//! every block depends on the window of decoded bytes before it — so the equivalent trick
//! does not exist here. It is also not needed: a compressed rollout is by definition one
//! Codex has not touched for a week, so it is **cold** — it will never grow, it is read
//! exactly once, and it is small. The largest rollout measured on the maintainer's machine
//! is 12 MB plain, which is 1.2 MB on disk at Codex's compression level and about as much
//! work as reading the plain file's tail twice.
//!
//! ## What is bounded
//!
//! [`MAX_DECODED`] bytes of output, the same 64 MiB ceiling [`super::tail::FULL_WINDOW`]
//! puts on a full pass over a plain log. It is a guard against a file that is not what we
//! think it is — a compressed stream is a few hundred bytes per megabyte it expands to, so
//! without a ceiling a tiny file can ask for all of memory. A rollout that reaches the
//! ceiling is **refused rather than truncated**: half a log read from its beginning is the
//! wrong half for the quota reader, which wants the last line, and a partial count with a
//! finished cursor is worse than no count for the usage reader.
//!
//! ## What is verified
//!
//! Whatever the frame itself offers. A zstd frame may carry a 32-bit content checksum;
//! the `zstd` command line writes one by default and the library Codex calls does not
//! (`zstd::stream::write::Encoder::new(out, 3)`, `codex-rs/rollout/src/compression.rs`,
//! which leaves `ZSTD_c_checksumFlag` at libzstd's default of off). When one is there it
//! is compared, and a mismatch is an error rather than a line of plausible rubbish handed
//! to a JSON parser. When it is not, structural damage still fails: a byte flipped inside
//! a compressed block breaks the entropy tables that decode the rest of it.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use ruzstd::decoding::{FrameDecoder, StreamingDecoder};

use crate::error::{Error, Result};
use crate::usage::scan::log_label;

/// Suffix of a rollout log Codex has compressed.
///
/// One constant, used by both walks: [`super::locate`] for the quota reader and
/// [`crate::usage::codex`] for the usage reader. `.jsonl.zst` does not end in `.jsonl`, so
/// the two tests those walks make are exclusive and the order of them says nothing.
pub const COMPRESSED_SUFFIX: &str = ".jsonl.zst";

/// Most decompressed bytes this module will produce from one file.
///
/// The same ceiling [`super::tail::FULL_WINDOW`] puts on a full pass over a plain rollout,
/// so the two readers refuse the same size of log for the same reason.
pub const MAX_DECODED: u64 = 64 * 1024 * 1024;

/// Bytes pulled out of the decoder per call.
const CHUNK: usize = 256 * 1024;

/// Longest line kept while waiting for its newline.
///
/// A rollout line runs to a few hundred kilobytes at the outside. Beyond this the file is
/// not what we think it is, so the fragment is dropped and counted rather than grown — the
/// same rule, and the same number, as [`crate::usage::scan`].
const MAX_LINE: usize = 8 * 1024 * 1024;

/// What one pass over a compressed rollout produced.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Decoded {
    /// Bytes of plain text the frames expanded to.
    pub decoded: u64,
    /// Complete lines handed to the caller.
    pub lines: u64,
    /// Fragments dropped for being longer than a line could be.
    pub dropped: u64,
}

/// Whether `name` is the name Codex leaves behind after compressing a rollout.
#[must_use]
pub fn is_compressed(name: &str) -> bool {
    name.ends_with(COMPRESSED_SUFFIX)
}

/// The name this log has when it is not compressed.
///
/// `rollout-….jsonl.zst` and `rollout-….jsonl` are the same session in the two states
/// Codex keeps it in, and something that has to recognise it across a sweep — the usage
/// store's cursor, for one — asks for this rather than for the name on disk.
#[must_use]
pub fn plain_name(name: &str) -> &str {
    name.strip_suffix(".zst").unwrap_or(name)
}

/// Decode `path` and hand every complete line to `on_line`, in file order.
///
/// The line arrives without its terminator and with a trailing `\r` still on it: stripping
/// that is the caller's business, because the two callers already do it on the plain path
/// and doing it here as well would be a second place for the rule to live.
///
/// Errors carry the log's redacted label rather than its path, like every other read of a
/// file this crate does not own. A damaged frame, a checksum that does not match and a
/// file past [`MAX_DECODED`] all arrive as [`Error::Log`]: the caller's question is only
/// ever whether the log could be read.
pub fn read_lines(path: &Path, on_line: &mut dyn FnMut(&[u8])) -> Result<Decoded> {
    let label = log_label(path);
    let file = File::open(path).map_err(|source| Error::log(label.clone(), source))?;
    let mut source = BufReader::with_capacity(CHUNK, file);

    let mut found = Decoded::default();
    let mut buffer = vec![0u8; CHUNK];
    let mut carry: Vec<u8> = Vec::new();
    let mut discarding = false;

    // A zstd archive is a sequence of frames, and a `StreamingDecoder` decodes exactly one.
    // Codex writes a single frame and so does the `zstd` command line, but concatenating
    // two archives is a valid one, and reading the first frame of such a file and calling
    // it the whole log would be the silent half-answer this package exists to remove.
    loop {
        let remaining = source
            .fill_buf()
            .map_err(|source| Error::log(label.clone(), source))?;
        if remaining.is_empty() {
            break;
        }

        let mut decoder = StreamingDecoder::new(&mut source)
            .map_err(|error| Error::log(label.clone(), undecodable(&error)))?;

        loop {
            let left = MAX_DECODED - found.decoded;
            if left == 0 {
                // The ceiling is reached, which is not the same as passed: a log that
                // expands to exactly this many bytes is a log that fits. One more byte is
                // the question, and the answer to it is the whole difference.
                let mut probe = [0u8; 1];
                let more = decoder
                    .read(&mut probe)
                    .map_err(|error| Error::log(label.clone(), undecodable(&error)))?;
                if more == 0 {
                    break;
                }
                return Err(Error::log(label.clone(), too_big()));
            }
            let want = CHUNK.min(usize::try_from(left).unwrap_or(CHUNK));
            let read = decoder
                .read(&mut buffer[..want])
                .map_err(|error| Error::log(label.clone(), undecodable(&error)))?;
            if read == 0 {
                break;
            }
            found.decoded += read as u64;
            split(
                &buffer[..read],
                &mut carry,
                &mut discarding,
                &mut found,
                on_line,
            );
        }

        verify(&decoder.decoder, &label)?;
    }

    // A rollout Codex compressed mid-write has no newline after its last line. The plain
    // readers leave such a fragment for the next poll, because more bytes may be coming;
    // here nothing more is ever coming, so the line is finished rather than dropped.
    if !discarding && !carry.is_empty() {
        found.lines += 1;
        on_line(&carry);
    }

    Ok(found)
}

/// Compare the frame's own checksum against the one decoding it produced.
///
/// Both are `None` when the frame carries no checksum, which is the normal case for a file
/// Codex compressed: it calls libzstd with default parameters and the default is off.
fn verify(decoder: &FrameDecoder, label: &str) -> Result<()> {
    let (Some(claimed), Some(calculated)) = (
        decoder.get_checksum_from_data(),
        decoder.get_calculated_checksum(),
    ) else {
        return Ok(());
    };
    if claimed == calculated {
        return Ok(());
    }
    Err(Error::log(
        label.to_owned(),
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the zstd frame's content checksum does not match what decoding it produced",
        ),
    ))
}

/// Split `chunk` into lines, joining the fragment left over from the previous chunk.
///
/// The same shape as [`crate::usage::scan`]'s loop, minus the byte offsets: there is no
/// cursor into a decompressed stream, because there is no way to resume one.
fn split(
    chunk: &[u8],
    carry: &mut Vec<u8>,
    discarding: &mut bool,
    found: &mut Decoded,
    on_line: &mut dyn FnMut(&[u8]),
) {
    let mut rest = chunk;
    while let Some(index) = rest.iter().position(|byte| *byte == b'\n') {
        let (line, tail) = rest.split_at(index);
        rest = &tail[1..];

        if *discarding {
            *discarding = false;
            continue;
        }
        found.lines += 1;
        if carry.is_empty() {
            on_line(line);
        } else {
            carry.extend_from_slice(line);
            let joined = std::mem::take(carry);
            on_line(&joined);
        }
    }

    if rest.is_empty() {
        return;
    }
    if *discarding || carry.len() + rest.len() > MAX_LINE {
        // Not a line we could use even if it ever ended.
        if !*discarding {
            found.dropped += 1;
        }
        carry.clear();
        *discarding = true;
    } else {
        carry.extend_from_slice(rest);
    }
}

/// A zstd error, as the caller's error type carries it.
///
/// The decoder's own message names a byte offset and a table, never a byte of the log, so
/// it is safe to carry and worth carrying: *could not be decoded* is the answer, and how is
/// what makes a bug report about a format change different from one about a half-written
/// file. One sentence covers a bad magic number, a block that ran out of source and a
/// disk that failed under the decoder, because from here they are one fact: this archive
/// did not yield a log.
fn undecodable(error: &impl core::fmt::Display) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("the zstd stream could not be decoded: {error}"),
    )
}

/// The file expanded past what this module will hold.
fn too_big() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("the rollout expands past the {MAX_DECODED} byte ceiling"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    /// The committed archive, produced by `zstd -3` from `rollout-sample.jsonl`.
    const ARCHIVE: &[u8] = include_bytes!("../../../../fixtures/codex/rollout-sample.jsonl.zst");
    /// The same thirteen quota lines, plain.
    const PLAIN: &str = include_str!("../../../../fixtures/codex/rollout-sample.jsonl");

    fn write(dir: &TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn lines_of(path: &Path) -> (Decoded, Vec<String>) {
        let mut lines = Vec::new();
        let found = read_lines(path, &mut |line| {
            lines.push(String::from_utf8_lossy(line).into_owned());
        })
        .unwrap();
        (found, lines)
    }

    #[test]
    fn the_committed_archive_decodes_to_the_committed_fixture_byte_for_byte() {
        let dir = TempDir::new("zst-fixture");
        let path = write(&dir, "rollout-sample.jsonl.zst", ARCHIVE);

        let (found, lines) = lines_of(&path);

        // The whole point of the fixture: the archive is the plain file, so every test
        // that reads one can be compared against the same test reading the other.
        assert_eq!(
            lines.join("\n") + "\n",
            PLAIN,
            "the archive and the plain fixture have drifted apart"
        );
        assert_eq!(found.decoded, PLAIN.len() as u64);
        assert_eq!(found.lines, PLAIN.lines().count() as u64);
        assert_eq!(found.dropped, 0);
    }

    #[test]
    fn a_frame_that_was_never_zstd_is_an_error_and_not_an_empty_file() {
        let dir = TempDir::new("zst-not-an-archive");
        let path = write(&dir, "rollout-x.jsonl.zst", b"not an archive at all\n");

        let error = read_lines(&path, &mut |_| {}).unwrap_err();

        let text = error.to_string();
        assert!(
            text.contains("could not be decoded"),
            "expected an undecodable frame: {text}"
        );
        // The log's path is never in the message; its label is.
        assert!(!text.contains("rollout-x"), "the path leaked: {text}");
    }

    #[test]
    fn an_archive_cut_in_half_is_an_error_rather_than_half_a_log() {
        let dir = TempDir::new("zst-truncated");
        let path = write(&dir, "rollout-cut.jsonl.zst", &ARCHIVE[..ARCHIVE.len() / 2]);

        let mut seen = 0usize;
        let error = read_lines(&path, &mut |_| seen += 1).unwrap_err();

        assert!(
            error.to_string().contains("could not be decoded"),
            "{error}"
        );
        assert!(
            seen < PLAIN.lines().count(),
            "a truncated archive cannot have produced the whole log"
        );
    }

    #[test]
    fn a_flipped_byte_in_the_payload_is_caught() {
        let dir = TempDir::new("zst-flipped");
        let mut damaged = ARCHIVE.to_vec();
        // Two thirds in: past the frame header, well before the checksum.
        let at = damaged.len() * 2 / 3;
        damaged[at] ^= 0x40;
        let path = write(&dir, "rollout-bitrot.jsonl.zst", &damaged);

        let error = read_lines(&path, &mut |_| {}).unwrap_err();

        // Either the entropy tables stop making sense or the content checksum does not
        // match; both are the same answer to the caller and both are named.
        let text = error.to_string();
        assert!(
            text.contains("could not be decoded") || text.contains("checksum"),
            "a flipped byte must not decode into plausible rubbish: {text}"
        );
    }

    #[test]
    fn two_archives_end_to_end_are_read_as_one_log() {
        let dir = TempDir::new("zst-two-frames");
        let mut both = ARCHIVE.to_vec();
        both.extend_from_slice(ARCHIVE);
        let path = write(&dir, "rollout-two.jsonl.zst", &both);

        let (found, lines) = lines_of(&path);

        assert_eq!(found.lines, 2 * PLAIN.lines().count() as u64);
        assert_eq!(lines.len(), 2 * PLAIN.lines().count());
    }

    #[test]
    fn a_missing_file_is_an_error_with_a_label_and_no_path() {
        let dir = TempDir::new("zst-missing");
        let error = read_lines(&dir.join("rollout-nope.jsonl.zst"), &mut |_| {}).unwrap_err();
        let text = error.to_string();
        assert!(text.contains("could not read log"), "{text}");
        assert!(!text.contains("rollout-nope"), "the path leaked: {text}");
    }

    #[test]
    fn the_two_names_of_one_session() {
        assert!(is_compressed("rollout-2026-09-07-abc.jsonl.zst"));
        assert!(!is_compressed("rollout-2026-09-07-abc.jsonl"));
        assert_eq!(
            plain_name("rollout-2026-09-07-abc.jsonl.zst"),
            "rollout-2026-09-07-abc.jsonl"
        );
        assert_eq!(
            plain_name("rollout-2026-09-07-abc.jsonl"),
            "rollout-2026-09-07-abc.jsonl"
        );
    }
}
