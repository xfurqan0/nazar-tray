//! A string that must not be printed, written, or left in memory.
//!
//! One value in this whole product needs this treatment: the OAuth access token that the
//! opt-in detailed-windows mode puts in a single `Authorization` header. [`Secret`] is
//! deliberately awkward to use — no `Display`, no `Clone`, no `Serialize`, and a `Debug`
//! that prints nothing — so that the only way to get at the characters is a method whose
//! name says what it is for and shows up in a grep.
//!
//! What it does guarantee: the buffer this type owns is overwritten before it is freed.
//! What it cannot guarantee: that no copy was ever made anywhere else. That is why the
//! reader in [`super::credentials`] also wipes the file text and the parsed document it
//! took the token out of, and why nothing else in this crate is ever handed one of these.

use std::fmt;

use zeroize::Zeroize;

/// An access token, held for the length of one request.
pub struct Secret {
    value: String,
}

impl Secret {
    /// Take ownership of a secret string.
    #[must_use]
    pub fn new(value: String) -> Self {
        Secret { value }
    }

    /// The characters, for the one place that needs them: an `Authorization` header.
    ///
    /// Named the way it is so that a second caller is a visible decision rather than an
    /// accident. There is exactly one call site in this crate, and a test counts them.
    #[must_use]
    pub fn expose_for_one_request(&self) -> &str {
        &self.value
    }

    /// How long the token is. Safe to log; the length of a JWT is not a secret.
    #[must_use]
    pub fn len(&self) -> usize {
        self.value.len()
    }

    /// Whether the token is empty, which means the file had a key with nothing behind it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }
}

/// Prints nothing but the type.
///
/// Derived `Debug` on any struct that holds one of these is therefore safe, which matters:
/// the alternative — no `Debug` at all — would push callers into writing their own, and a
/// hand-written one is where a token gets printed by accident.
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}
