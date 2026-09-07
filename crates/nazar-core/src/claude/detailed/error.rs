//! What can go wrong in the detailed-windows mode, said out loud without saying too much.
//!
//! Two rules, both from the audit of the retired prototype:
//!
//! 1. **Never mix a response body into error text.** The endpoint is undocumented; nobody
//!    knows what a future error body contains, and an error string ends up in a screenshot
//!    or a bug report. So a failure carries a status code and a category, never bytes from
//!    the wire. Finding B10.
//! 2. **Never name an absolute path.** "Claude Code is not signed in on this machine" is
//!    the same information as the same sentence with a home directory in it, minus the
//!    user name in the screenshot. Finding B11.
//!
//! Neither the token nor anything derived from it can reach these variants: none of them
//! holds a string that came from the credential file or from the response.

use std::fmt;
use std::time::Duration;

/// The one message that tells the user what to do, and the reason this mode can
/// self-heal: Claude Code refreshes the token on its next run.
pub const TOKEN_EXPIRED: &str = "token expired; run Claude Code once to refresh";

/// Why an attempt at the usage endpoint did not produce numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailedError {
    /// No sign-in on this machine, or a file that is not a sign-in at all.
    NotSignedIn,
    /// The stored token's own expiry has passed, so no request was sent. This is the
    /// cheap half of [`DetailedError::Unauthorized`]: it costs no round trip and it is
    /// what stops the "401 every half hour for three hours" the audit found in the logs.
    Expired,
    /// The endpoint rejected the token. Same message as [`DetailedError::Expired`],
    /// because the user's move is the same one.
    Unauthorized,
    /// HTTP 429. `retry_after` is the server's own `Retry-After` when it sent one.
    RateLimited {
        /// How long the server asked us to wait, when it said.
        retry_after: Option<Duration>,
    },
    /// Any other status the endpoint returned.
    Status {
        /// The status code, and nothing else from the response.
        code: u16,
    },
    /// The request never got an answer.
    Network {
        /// Which kind of failure, from a closed list. Never the underlying message.
        kind: NetworkErrorKind,
    },
    /// HTTP 200 with a body this build cannot read.
    UnexpectedShape {
        /// What was looked for and not found. A fixed phrase, never part of the body.
        expected: &'static str,
    },
    /// A previous failure is still being backed off from, so nothing was attempted.
    BackingOff {
        /// How much of the wait is left.
        remaining: Duration,
    },
}

/// The categories of "the request did not complete".
///
/// A closed list on purpose. The HTTP client's own error message is not used: it is a
/// third party's string, it can change with a version bump, and a category is all a user
/// or a bug report can act on anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkErrorKind {
    /// The 20-second budget ran out.
    Timeout,
    /// No route, refused, or DNS said no.
    Connect,
    /// The TLS handshake failed.
    Tls,
    /// Something answered, but not with HTTP this client understands.
    Protocol,
    /// Anything else.
    Other,
}

impl NetworkErrorKind {
    /// A short, fixed phrase for the message.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            NetworkErrorKind::Timeout => "timed out",
            NetworkErrorKind::Connect => "could not connect",
            NetworkErrorKind::Tls => "TLS handshake failed",
            NetworkErrorKind::Protocol => "the answer was not HTTP this build understands",
            NetworkErrorKind::Other => "the request failed",
        }
    }
}

impl DetailedError {
    /// Whether the numbers this failure replaced should be kept and marked stale.
    ///
    /// Everything except "there is no sign-in here" should: a network hiccup or a 429 does
    /// not make yesterday's percentage wrong, only old. A machine with no sign-in has
    /// nothing to be stale about, and keeping a previous account's numbers there is
    /// finding B25.
    #[must_use]
    pub fn keeps_last_good(&self) -> bool {
        !matches!(self, DetailedError::NotSignedIn)
    }

    /// Whether this failure should push the next attempt further away.
    ///
    /// All of them do, including the two authorisation ones. A token that expired at
    /// 09:00 is still expired at 09:30, and the prototype's logs show it asking anyway,
    /// seven times over three hours. The credential file changing underneath us clears
    /// the wait, so the moment Claude Code refreshes the token the mode comes back.
    #[must_use]
    pub fn backs_off(&self) -> bool {
        !matches!(self, DetailedError::BackingOff { .. })
    }
}

impl fmt::Display for DetailedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DetailedError::NotSignedIn => {
                f.write_str("Claude Code is not signed in on this machine")
            }
            DetailedError::Expired | DetailedError::Unauthorized => f.write_str(TOKEN_EXPIRED),
            DetailedError::RateLimited { retry_after: None } => {
                f.write_str("the usage endpoint is rate-limiting this machine (HTTP 429)")
            }
            DetailedError::RateLimited {
                retry_after: Some(wait),
            } => write!(
                f,
                "the usage endpoint is rate-limiting this machine (HTTP 429, retry after {} s)",
                wait.as_secs()
            ),
            DetailedError::Status { code } => {
                write!(f, "the usage endpoint returned HTTP {code}")
            }
            DetailedError::Network { kind } => {
                write!(f, "the usage endpoint {}", kind.as_str())
            }
            DetailedError::UnexpectedShape { expected } => write!(
                f,
                "the usage endpoint answered with a shape this build does not know ({expected})"
            ),
            DetailedError::BackingOff { remaining } => write!(
                f,
                "waiting {} s before asking the usage endpoint again",
                remaining.as_secs()
            ),
        }
    }
}

impl std::error::Error for DetailedError {}
