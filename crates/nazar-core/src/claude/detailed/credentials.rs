//! The one file this product reads a token out of, and only while the mode is on.
//!
//! `<CLAUDE_CONFIG_DIR or ~/.claude>/.credentials.json` is written by Claude Code when you
//! sign in, and refreshed by Claude Code when the token expires. **This module only ever
//! opens it for reading.** Nothing here writes it, renames it, or creates it, and the
//! whole module is compiled only when the `detailed-windows` feature is on.
//!
//! Four values are taken out of it and nothing else:
//!
//! | Key | What it is used for |
//! |---|---|
//! | `claudeAiOauth.accessToken` | one `Authorization` header, then wiped |
//! | `claudeAiOauth.expiresAt` | skipping a request that is going to be a 401 |
//! | `claudeAiOauth.rateLimitTier` | the plan name in `limits.json` (`max_20x`) |
//! | `claudeAiOauth.subscriptionType` | the same, when the tier is missing |
//!
//! `refreshToken` and `refreshTokenExpiresAt` are in the file and are **never read**:
//! refreshing a token is Claude Code's job, and a tool that could do it would be a tool
//! that intermediates a sign-in. `scopes` is not read either.
//!
//! ## What happens to the characters
//!
//! The token exists in three buffers on its way to the header: the file text, the parsed
//! document, and the [`Secret`]. All three are overwritten before they are dropped — the
//! first two here, the third by `Secret`'s own `Drop`. This is best effort by nature (an
//! allocator can move a buffer and leave the old bytes behind), and it is worth doing
//! anyway: the tray is a process that runs all day.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;
use zeroize::Zeroize;

use super::error::DetailedError;
use super::secret::Secret;
use crate::error::Result;
use crate::paths::claude_config_dir;

/// Name of the file inside Claude Code's configuration directory.
pub const CREDENTIALS_FILE: &str = ".credentials.json";

/// The object inside it that holds the sign-in.
const OAUTH_KEY: &str = "claudeAiOauth";

/// Largest file this reader will take. The real one is around half a kilobyte.
const MAX_FILE_BYTES: u64 = 256 * 1024;

/// Longest token accepted. Observed: 108 characters.
const MAX_TOKEN_LEN: usize = 8192;

/// Longest plan hint accepted. Observed: `default_claude_max_20x`, 22 characters.
const MAX_HINT_LEN: usize = 64;

/// Seconds of slack allowed against the stored expiry.
///
/// The comparison is between this machine's clock and a timestamp another program wrote,
/// so treating a token as expired one second early would trade a working request for a
/// wrong error message. A minute is enough for clock drift and short of anything that
/// matters: the token lives for hours.
const EXPIRY_SKEW_SECONDS: i64 = 60;

/// `<CLAUDE_CONFIG_DIR or ~/.claude>/.credentials.json`.
pub fn credentials_path() -> Result<PathBuf> {
    Ok(claude_config_dir()?.join(CREDENTIALS_FILE))
}

/// Everything read out of the file. The token is the only part that is protected.
#[derive(Debug)]
pub struct SignIn {
    /// The access token, wiped when this value is dropped.
    pub token: Secret,
    /// `expiresAt`, as Unix seconds. `None` when the file did not carry one.
    pub expires_at: Option<i64>,
    /// `rateLimitTier`, verbatim. Normalised into a plan name by [`super::map`].
    pub rate_limit_tier: Option<String>,
    /// `subscriptionType`, verbatim.
    pub subscription_type: Option<String>,
}

impl SignIn {
    /// Whether the stored expiry has already passed at `now_unix_seconds`.
    ///
    /// `false` when the file carried no expiry: an unknown expiry is not an expired one,
    /// and the endpoint is the authority either way.
    #[must_use]
    pub fn is_expired(&self, now_unix_seconds: i64) -> bool {
        self.expires_at
            .is_some_and(|expiry| now_unix_seconds - EXPIRY_SKEW_SECONDS >= expiry)
    }
}

/// What the file looked like last time, so a change can be noticed.
///
/// Not a hash of the contents and certainly not of the token: size and modification time
/// are enough to answer "did Claude Code rewrite this", which is the only question asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

/// The file's stamp, or `None` when it is not there.
#[must_use]
pub fn stamp(path: &Path) -> Option<Stamp> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(Stamp {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

/// Read the sign-in, or say why there is none.
///
/// Every failure is [`DetailedError::NotSignedIn`]: a missing file, a directory where a
/// file should be, a file that is not JSON, a document with no `claudeAiOauth` in it and
/// one with an empty token are all the same thing to a user — "this machine has not signed
/// in to Claude Code" — and telling them apart would mean putting the reason, and
/// therefore the path, in a message.
pub fn read_sign_in(path: &Path) -> std::result::Result<SignIn, DetailedError> {
    if std::fs::metadata(path).is_ok_and(|meta| meta.len() > MAX_FILE_BYTES) {
        return Err(DetailedError::NotSignedIn);
    }
    let mut text = std::fs::read_to_string(path).map_err(|_| DetailedError::NotSignedIn)?;
    let parsed = serde_json::from_str::<Value>(&text);
    text.zeroize();

    let mut document = parsed.map_err(|_| DetailedError::NotSignedIn)?;
    let sign_in = extract(&mut document);
    wipe_token(&mut document);
    drop(document);

    sign_in.ok_or(DetailedError::NotSignedIn)
}

/// Take the four values out of a parsed document.
fn extract(document: &mut Value) -> Option<SignIn> {
    let oauth = document.get(OAUTH_KEY)?.as_object()?;

    let token = oauth.get("accessToken")?.as_str()?;
    if token.is_empty() || token.len() > MAX_TOKEN_LEN {
        return None;
    }

    Some(SignIn {
        token: Secret::new(token.to_owned()),
        expires_at: oauth.get("expiresAt").and_then(expiry_seconds),
        rate_limit_tier: oauth.get("rateLimitTier").and_then(hint),
        subscription_type: oauth.get("subscriptionType").and_then(hint),
    })
}

/// Overwrite the token inside the parsed document before it is dropped.
fn wipe_token(document: &mut Value) {
    if let Some(Value::String(token)) = document
        .get_mut(OAUTH_KEY)
        .and_then(|oauth| oauth.get_mut("accessToken"))
    {
        token.zeroize();
    }
}

/// `expiresAt`, in seconds, however the file spelled it.
///
/// Claude Code writes **milliseconds** here — 1788785223254, verified on the maintainer's
/// machine — where both it and Codex write seconds for a window's reset. So the magnitude
/// decides, the same rule `crate::timefmt` uses, rather than a constant that would be
/// wrong for one of the two.
fn expiry_seconds(value: &Value) -> Option<i64> {
    let raw = if let Some(number) = value.as_i64() {
        number
    } else {
        let number = value.as_f64()?;
        if !number.is_finite() || number.abs() >= 9e18 {
            return None;
        }
        number as i64
    };
    if raw <= 0 {
        return None;
    }
    Some(if raw >= 1_000_000_000_000 {
        raw / 1000
    } else {
        raw
    })
}

/// A plan hint, if it looks like one.
///
/// Bounded and non-empty; the shape check that decides whether it can become a plan name
/// in `limits.json` is [`super::map::normalise_plan`], which is where the contract's
/// "never invent a number, never forward prose" rule is applied.
fn hint(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    if text.is_empty() || text.len() > MAX_HINT_LEN {
        return None;
    }
    Some(text.to_owned())
}
