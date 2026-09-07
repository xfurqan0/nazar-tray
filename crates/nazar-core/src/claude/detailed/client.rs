//! The one HTTP request this product makes.
//!
//! `GET https://api.anthropic.com/api/oauth/usage`, with the three headers Claude Code's
//! own `/usage` panel sends, a twenty-second budget and a bounded body. No other method,
//! no other host, no redirect off it, and nothing at all unless the user turned the
//! detailed-windows mode on.
//!
//! ## What the client is not allowed to do
//!
//! * **Put the token anywhere but the header.** It is borrowed from a [`Secret`] for the
//!   length of the call and never copied into a string this module keeps.
//! * **Let a response body into an error.** The endpoint is undocumented; a future error
//!   body could hold anything, and error text ends up in screenshots. A failure here is a
//!   status code or one of five categories, and nothing else (audit finding B10).
//! * **Read an unbounded body.** A quarter of a megabyte is more than a hundred times the
//!   real answer.
//!
//! The HTTP client is `ureq` with `rustls`. It was picked over `reqwest`, which Tauri
//! already links, by measuring both against this workspace's lock file: `ureq` adds four
//! packages, `reqwest` with its blocking client adds seventeen including `aws-lc-sys` and
//! a `cmake` build. It is also synchronous, which keeps an async runtime out of the crate
//! the status-line wrapper links. Written up in `docs/detailed-windows.md`.

use std::time::Duration;

use super::error::NetworkErrorKind;
use super::secret::Secret;

/// Claude Code's usage endpoint.
pub const USAGE_ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";

/// The beta header the endpoint requires alongside an OAuth token.
const BETA_HEADER: &str = "anthropic-beta";
/// Its value, as Claude Code sends it.
const BETA_VALUE: &str = "oauth-2025-04-20";

/// How long the whole call may take.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Largest response body read. The real one is about two kilobytes.
const MAX_BODY_BYTES: u64 = 256 * 1024;

/// Longest `Retry-After` honoured, so a header cannot park the mode for a day.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30 * 60);

/// What came back: a status, the one header worth reading, and the body.
#[derive(Debug, Clone)]
pub struct Answer {
    /// The HTTP status.
    pub status: u16,
    /// `Retry-After`, in seconds, when the server sent one in that form.
    pub retry_after: Option<Duration>,
    /// The body, capped. Never copied into an error.
    pub body: String,
}

/// A client pointed at one URL.
///
/// The URL is a field rather than a constant so the tests can point it at a socket on
/// this machine. Nothing configurable reaches it: the tray always builds it with
/// [`UsageClient::official`], and `--detailed` does not change that.
#[derive(Debug)]
pub struct UsageClient {
    agent: ureq::Agent,
    endpoint: String,
}

impl UsageClient {
    /// A client for the real endpoint.
    #[must_use]
    pub fn official() -> Self {
        UsageClient::new(USAGE_ENDPOINT)
    }

    /// A client for an explicit URL.
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .user_agent(user_agent())
            // A status is an answer, not a failure: 429 carries `Retry-After` and 401
            // carries the reason this mode has a self-healing story. Both are needed as
            // values rather than as an error the client invented.
            .http_status_as_error(false)
            // Nothing this endpoint returns should move us to another URL, and following
            // one would carry the `Authorization` header somewhere it was not meant for.
            .max_redirects(0)
            .build();

        UsageClient {
            agent: ureq::Agent::new_with_config(config),
            endpoint: endpoint.into(),
        }
    }

    /// The URL this client will call.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Make the request.
    ///
    /// The token is read out of the [`Secret`] exactly here, for exactly one header.
    pub fn get(&self, token: &Secret) -> std::result::Result<Answer, NetworkErrorKind> {
        let mut response = self
            .agent
            .get(&self.endpoint)
            .header(
                "Authorization",
                &format!("Bearer {}", token.expose_for_one_request()),
            )
            .header(BETA_HEADER, BETA_VALUE)
            .header("Accept", "application/json")
            .call()
            .map_err(classify)?;

        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(parse_retry_after);

        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_string()
            .unwrap_or_default();

        Ok(Answer {
            status,
            retry_after,
            body,
        })
    }
}

/// `nazar-tray/<version>`, so the endpoint's operator can tell who is calling.
fn user_agent() -> String {
    format!("nazar-tray/{}", env!("CARGO_PKG_VERSION"))
}

/// Turn a transport failure into one of five categories.
///
/// The client's own message is deliberately dropped. It belongs to a third party, it
/// changes with a version bump, and a category is the whole of what a user or a bug report
/// can act on. Nothing in a `ureq` error carries request headers, but not depending on
/// that is cheaper than depending on it.
fn classify(error: ureq::Error) -> NetworkErrorKind {
    match error {
        ureq::Error::Timeout(_) => NetworkErrorKind::Timeout,
        ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => NetworkErrorKind::Connect,
        ureq::Error::Io(source) => match source.kind() {
            std::io::ErrorKind::TimedOut => NetworkErrorKind::Timeout,
            std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::AddrNotAvailable => NetworkErrorKind::Connect,
            _ => NetworkErrorKind::Other,
        },
        ureq::Error::Tls(_) => NetworkErrorKind::Tls,
        ureq::Error::Protocol(_) | ureq::Error::BadUri(_) | ureq::Error::TooManyRedirects => {
            NetworkErrorKind::Protocol
        }
        _ => NetworkErrorKind::Other,
    }
}

/// `Retry-After: 120`.
///
/// The header may also be an HTTP date. That form is not read: it needs a date parser this
/// crate does not have, and an unread `Retry-After` falls back to the doubling delay,
/// which is the behaviour the header was going to produce anyway. A value that is not a
/// plain number of seconds is therefore ignored rather than guessed at.
fn parse_retry_after(value: &str) -> Option<Duration> {
    let seconds: u64 = value.trim().parse().ok()?;
    Some(Duration::from_secs(seconds).min(MAX_RETRY_AFTER))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_official_endpoint_is_the_one_claude_codes_own_panel_uses() {
        assert_eq!(
            UsageClient::official().endpoint(),
            "https://api.anthropic.com/api/oauth/usage"
        );
        assert!(USAGE_ENDPOINT.starts_with("https://"), "never plain HTTP");
    }

    #[test]
    fn the_user_agent_names_this_product_and_its_version() {
        let agent = user_agent();
        assert!(agent.starts_with("nazar-tray/"), "got {agent}");
        assert!(
            agent.len() > "nazar-tray/".len(),
            "the version is missing from {agent}"
        );
    }

    #[test]
    fn retry_after_is_read_as_seconds_and_capped() {
        assert_eq!(parse_retry_after("120"), Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after("  7 "), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after("0"), Some(Duration::ZERO));
        assert_eq!(parse_retry_after("100000"), Some(MAX_RETRY_AFTER));

        // An HTTP date is legal and is not read; the doubling delay takes over.
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
        assert_eq!(parse_retry_after(""), None);
        assert_eq!(parse_retry_after("-5"), None);
    }
}
