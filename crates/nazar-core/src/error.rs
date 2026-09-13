//! Error type for the core crate.
//!
//! Deliberately small, and deliberately free of file contents: an error names the path
//! and the operation that failed, never the bytes it was reading or writing. The audit
//! called this out ("never mix response bodies into error text") and the same rule holds
//! for local files, because `limits.json` sits next to files that may hold user data.
//!
//! One path does not even get named: a **log this crate reads and never writes** fails as
//! [`Error::Log`], which carries a redacted label. The files this product writes are its
//! own and saying where they are helps; a transcript's path is made of the directories
//! somebody works in, and saying where *that* is helps nobody.

use std::fmt;
use std::path::PathBuf;

/// Result alias used across the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong in the core crate.
#[derive(Debug)]
pub enum Error {
    /// A filesystem operation failed. `path` is the file or directory involved.
    Io {
        /// Path the failing operation was pointed at.
        path: PathBuf,
        /// Underlying operating-system error.
        source: std::io::Error,
    },
    /// JSON could not be parsed or produced.
    Json {
        /// Path the JSON came from, when it came from a file.
        path: Option<PathBuf>,
        /// Underlying serde error. It carries a line and column, not the document.
        source: serde_json::Error,
    },
    /// A log this crate only ever reads could not be read.
    ///
    /// Named by a **label** rather than by a path, which is the whole reason the variant
    /// exists: `~/.claude/projects/` is named after every working directory somebody has
    /// opened a session in, and `~/.codex/sessions/` after the days they worked. A caller
    /// that logs this learns *which* log failed — the label is the same hash the cursor
    /// document files it under — and nothing about the machine it is on.
    Log {
        /// A stable, redacted name for the log. Never a path.
        label: String,
        /// Underlying operating-system error.
        source: std::io::Error,
    },
    /// A target path has no parent directory, so no temporary file can sit beside it.
    NoParentDirectory {
        /// The offending path.
        path: PathBuf,
    },
    /// The user's home directory could not be determined from the environment.
    NoHomeDirectory,
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        Error::Json {
            path: Some(path.into()),
            source,
        }
    }

    pub(crate) fn log(label: impl Into<String>, source: std::io::Error) -> Self {
        Error::Log {
            label: label.into(),
            source,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => {
                write!(f, "file operation failed on {}: {source}", path.display())
            }
            Error::Json {
                path: Some(path),
                source,
            } => write!(f, "invalid JSON in {}: {source}", path.display()),
            Error::Json { path: None, source } => write!(f, "invalid JSON: {source}"),
            Error::Log { label, source } => write!(f, "could not read {label}: {source}"),
            Error::NoParentDirectory { path } => {
                write!(f, "{} has no parent directory", path.display())
            }
            Error::NoHomeDirectory => {
                f.write_str("could not determine the home directory from the environment")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } | Error::Log { source, .. } => Some(source),
            Error::Json { source, .. } => Some(source),
            Error::NoParentDirectory { .. } | Error::NoHomeDirectory => None,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(source: serde_json::Error) -> Self {
        Error::Json { path: None, source }
    }
}
