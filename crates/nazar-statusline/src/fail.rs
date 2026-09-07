//! The error type for the installer, and the one rule it enforces.
//!
//! A failure carries a sentence meant for the person who ran the command, and it names
//! the file and the operation rather than quoting what was in the file. `settings.json`
//! sits in a directory full of things that are none of this program's business, so an
//! error message is the last place its contents should end up.

use std::fmt;

/// Result alias for the installer.
pub type Result<T> = std::result::Result<T, Failure>;

/// Something went wrong, and here is what to tell the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    message: String,
}

impl Failure {
    /// A failure with a message for the user.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Failure {
            message: message.into(),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failure {}

impl From<nazar_core::Error> for Failure {
    fn from(error: nazar_core::Error) -> Self {
        Failure::new(error.to_string())
    }
}
