//! Atomic file writes: temporary file in the same directory, then rename.
//!
//! `limits.json` has exactly one writer (the tray process) and at least two readers (the
//! tray's own panel and the Nazar canvas). A reader must never observe a prefix of a
//! document, and a write that dies half way must never leave one on disk. Both follow
//! from writing somewhere else first and renaming over the target, which is a single
//! filesystem operation.
//!
//! The temporary file is created in the **same directory** as the target on purpose: a
//! rename across volumes is a copy, and a copy is not atomic.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use crate::error::{Error, Result};

/// Windows can refuse a rename with a sharing violation while another process still has
/// the target open. Readers hold it open for a few microseconds, so a short retry loop
/// turns a rare failure into a rare delay.
const RENAME_ATTEMPTS: u32 = 6;
const RENAME_BACKOFF: Duration = Duration::from_millis(25);

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path`, atomically.
///
/// Creates the parent directory if it is missing. On success the target holds exactly
/// `bytes`; on failure it is untouched and no temporary file remains.
pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    write_with(path, |file| file.write_all(bytes))
}

/// Same as [`write_bytes`], but the caller fills the temporary file.
///
/// Used by the tests to simulate a write that is interrupted part way through.
pub(crate) fn write_with<F>(path: &Path, fill: F) -> Result<()>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    let parent = match path.parent() {
        Some(parent) if parent.as_os_str().is_empty() => Path::new("."),
        Some(parent) => parent,
        None => {
            return Err(Error::NoParentDirectory {
                path: path.to_path_buf(),
            });
        }
    };
    fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;

    let temp = TempFile::create(parent, path)?;

    {
        // Scoped so the handle is closed before the rename; Windows will not rename a
        // file that is still open for writing.
        let mut file = temp.open_for_write()?;
        fill(&mut file).map_err(|source| Error::io(&temp.path, source))?;
        file.flush()
            .map_err(|source| Error::io(&temp.path, source))?;
        file.sync_all()
            .map_err(|source| Error::io(&temp.path, source))?;
    }

    temp.rename_onto(path)
}

/// A temporary file that deletes itself unless it is renamed into place.
///
/// The `Drop` impl is what guarantees "an interrupted write leaves nothing behind": it
/// runs on an early return and on a panic alike.
struct TempFile {
    path: PathBuf,
    file: Option<File>,
}

impl TempFile {
    fn create(parent: &Path, target: &Path) -> Result<Self> {
        let stem = target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "nazar".to_owned());

        // Process id plus a per-process counter: two trays writing the same file at the
        // same time (which the single-instance guard forbids, but still) never collide.
        for attempt in 0..16u32 {
            let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(".{stem}.tmp-{}-{serial}", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    return Ok(TempFile {
                        path,
                        file: Some(file),
                    });
                }
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists && attempt < 15 => {
                    continue;
                }
                Err(source) => return Err(Error::io(path, source)),
            }
        }
        unreachable!("the loop above either returns or exhausts its attempts with an error")
    }

    fn open_for_write(&self) -> Result<File> {
        self.file
            .as_ref()
            .expect("the handle is taken only by rename_onto, which consumes self")
            .try_clone()
            .map_err(|source| Error::io(&self.path, source))
    }

    fn rename_onto(mut self, target: &Path) -> Result<()> {
        // Close our handle first: on Windows the rename fails while it is open.
        self.file = None;

        let mut last = None;
        for attempt in 0..RENAME_ATTEMPTS {
            match fs::rename(&self.path, target) {
                Ok(()) => {
                    // Renamed away; nothing left for Drop to clean up.
                    self.path = PathBuf::new();
                    return Ok(());
                }
                Err(source) => {
                    last = Some(source);
                    if attempt + 1 < RENAME_ATTEMPTS {
                        thread::sleep(RENAME_BACKOFF);
                    }
                }
            }
        }
        Err(Error::io(
            target,
            last.unwrap_or_else(|| io::Error::other("rename failed without an error")),
        ))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        self.file = None;
        if !self.path.as_os_str().is_empty() {
            // Best effort: the write already failed, and a leftover temporary file is
            // not worth a second error the caller cannot act on.
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp-"))
            .collect()
    }

    #[test]
    fn writes_the_bytes_and_creates_the_directory() {
        let dir = TempDir::new("atomic-basic");
        let path = dir.join("nested/deeper/limits.json");

        write_bytes(&path, b"hello").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"hello");
        assert!(leftovers(path.parent().unwrap()).is_empty());
    }

    #[test]
    fn overwrites_an_existing_file() {
        let dir = TempDir::new("atomic-overwrite");
        let path = dir.join("limits.json");

        write_bytes(&path, b"first").unwrap();
        write_bytes(&path, b"second write, longer").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"second write, longer");
        assert!(leftovers(&dir.path).is_empty());
    }

    #[test]
    fn an_interrupted_write_leaves_the_previous_file_intact() {
        let dir = TempDir::new("atomic-interrupted");
        let path = dir.join("limits.json");
        write_bytes(&path, b"the good document").unwrap();

        // Write half a document, then fail the way a crashing serialiser would.
        let outcome = write_with(&path, |file| {
            file.write_all(b"the truncated docu")?;
            Err(io::Error::other("simulated interruption"))
        });

        assert!(outcome.is_err());
        assert_eq!(
            fs::read(&path).unwrap(),
            b"the good document",
            "the target must still hold the previous document"
        );
        assert!(
            leftovers(&dir.path).is_empty(),
            "the temporary file must be gone, found {:?}",
            leftovers(&dir.path)
        );
    }

    #[test]
    fn a_panicking_write_leaves_nothing_behind() {
        let dir = TempDir::new("atomic-panic");
        let path = dir.join("limits.json");
        write_bytes(&path, b"the good document").unwrap();

        let panicked = std::panic::catch_unwind(|| {
            write_with(&path, |file| {
                file.write_all(b"the truncated docu").unwrap();
                panic!("simulated crash mid-write");
            })
        });

        assert!(panicked.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"the good document");
        assert!(
            leftovers(&dir.path).is_empty(),
            "found {:?}",
            leftovers(&dir.path)
        );
    }

    #[test]
    fn a_reader_never_sees_a_partial_document() {
        let dir = TempDir::new("atomic-concurrent");
        let path = dir.join("limits.json");

        let short = b"{\"n\":1}".to_vec();
        let long = format!("{{\"n\":2,\"pad\":\"{}\"}}", "x".repeat(200_000)).into_bytes();
        write_bytes(&path, &short).unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let reader = {
            let path = path.clone();
            let stop = Arc::clone(&stop);
            let (short, long) = (short.clone(), long.clone());
            thread::spawn(move || {
                let mut reads = 0u32;
                while !stop.load(Ordering::Relaxed) {
                    // A reader may fail to open the file mid-rename; it may never see a
                    // document that is neither of the two we write.
                    if let Ok(seen) = fs::read(&path) {
                        assert!(
                            seen == short || seen == long,
                            "read a document of {} bytes that was never written whole",
                            seen.len()
                        );
                        reads += 1;
                    }
                }
                reads
            })
        };

        for round in 0..200 {
            write_bytes(&path, if round % 2 == 0 { &long } else { &short }).unwrap();
        }
        stop.store(true, Ordering::Relaxed);

        let reads = reader.join().expect("the reader thread must not panic");
        assert!(reads > 0, "the reader never managed to open the file");
        assert!(leftovers(&dir.path).is_empty());
    }
}
