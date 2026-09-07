//! Detailed windows: the opt-in mode, off by default.
//!
//! Everything else in nazar-tray reads files that are already on the disk. This module is
//! the one exception, and it exists because the passive path cannot see the number that
//! actually constrains some people: Claude Code's status line reports a five-hour window
//! and one **global** weekly window, while a Max subscriber also has **model-scoped**
//! weekly caps. Measured on the maintainer's machine on 2026-09-07: the status line said
//! the weekly window was at 18 %, and the official endpoint said the Fable weekly window
//! was at 23 %. A tray that showed 18 % would have been wrong about the thing it exists to
//! be right about.
//!
//! So: a switch the user turns on, which reads the token Claude Code already stored, holds
//! it in memory for a single request, and never writes it anywhere. Off unless it is
//! turned on; see `docs/detailed-windows.md` for what that means in plain words, including
//! the terms-of-service note.
//!
//! ```text
//!  config.json                    .credentials.json                 the endpoint
//!  detailedWindows: true  ──▶  claudeAiOauth.accessToken  ──▶  GET /api/oauth/usage
//!         │                            (in memory,                      │
//!         │                             one request,                    ▼
//!         │                             then wiped)              limits[] → windows
//!         ▼                                                             │
//!    off  →  nothing here runs, nothing is opened                       ▼
//!                                            merge with the status-line numbers
//! ```
//!
//! ## The five things this module promises
//!
//! 1. **Nothing happens with the mode off.** The caller checks the flag; with it off the
//!    credential file is never opened, which a test proves by poisoning that file and
//!    watching the refresh not care.
//! 2. **The token is never written, logged, or put in an error.** It lives in a [`Secret`]
//!    that wipes itself, it is read once for one header, and no error variant in this
//!    module can hold a string that came from the file or the wire.
//! 3. **A failure keeps the last good numbers and says they are old.** Old and honest
//!    beats blank, and blank beats invented.
//! 4. **A failure pushes the next attempt away.** `1 s, 2 s, 4 s …` capped at half an
//!    hour, or whatever `Retry-After` asked for. The endpoint is undocumented and this
//!    product is not going to be the reason it starts refusing people.
//! 5. **The response body never reaches an error message.** Status codes and categories
//!    only.
//!
//! ## Who does what
//!
//! | Module | Job |
//! |---|---|
//! | [`secret`] | the token wrapper that wipes itself and prints nothing |
//! | [`credentials`] | reading four values out of `.credentials.json` |
//! | [`client`] | the single `GET`, its headers, its timeout and its body cap |
//! | [`backoff`] | when to try again, and the clock the tests can move |
//! | [`map`] | `limits[]` to `limits.json` windows, and the plan name |
//! | [`error`] | what can go wrong, said without saying too much |
//! | [`super::merge`] | which number wins when both paths have one |

pub mod backoff;
pub mod client;
pub mod credentials;
pub mod error;
pub mod map;
pub mod secret;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::limits::Window;

pub use backoff::{Backoff, Clock, SystemClock};
pub use client::{REQUEST_TIMEOUT, USAGE_ENDPOINT, UsageClient};
pub use credentials::{SignIn, credentials_path};
pub use error::{DetailedError, NetworkErrorKind};
pub use map::{Mapped, map_response, normalise_plan};
pub use secret::Secret;

/// How long an endpoint reading counts as current.
///
/// Past this the numbers are still shown — they are the best anyone has — but they are
/// marked stale, so the tray can say "this is a quarter of an hour old" instead of
/// implying it just looked. The refresh loop that decides how often to *ask* is WP3's;
/// this is only the age at which an answer stops claiming to be fresh.
pub const FRESH_FOR: Duration = Duration::from_secs(15 * 60);

/// One reading from the endpoint, fresh or remembered.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// The plan name the sign-in's hints normalised to, if any.
    pub plan: Option<String>,
    /// When the request that produced these numbers came back, RFC 3339 in UTC.
    pub fetched_at: String,
    /// The windows, keyed the way `limits.json` keys them.
    pub windows: BTreeMap<String, Window>,
    /// Whether these numbers were remembered rather than just fetched.
    pub remembered: bool,
    /// Why they were remembered. Never a response body; see [`error`].
    pub reason: Option<String>,
}

/// What one refresh produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// The numbers, if there are any. `None` means the endpoint has never answered and
    /// the passive path is all there is.
    pub reading: Option<Reading>,
    /// What went wrong this time, if anything. `None` on a clean fetch.
    pub error: Option<DetailedError>,
}

impl Outcome {
    /// Whether this refresh produced numbers straight from the endpoint.
    #[must_use]
    pub fn is_fresh(&self) -> bool {
        self.reading
            .as_ref()
            .is_some_and(|reading| !reading.remembered)
    }
}

/// Whether to offer the detailed-windows mode.
///
/// A pure decision, deliberately: the wiring — a dialog, a toast, a line in the settings
/// panel — is WP5's, and the rule should be readable without any of it.
///
/// The rule is narrow on purpose. Only a Max plan is offered the mode, because only a Max
/// plan has model-scoped weekly caps for it to reveal; on Pro the endpoint would report
/// the two windows the status line already reports, and asking for a token to learn
/// nothing is a bad trade. And it is offered **once**: `already_asked` is
/// `config.detailedSuggested`, set the moment the offer is shown, so a user who said no is
/// not asked again on the next start.
///
/// One honest gap, which is WP5's to close rather than this module's to paper over: with
/// the mode off there is nowhere to learn the plan from. The status-line payload does not
/// carry one (verified in WP2), and the only file that does is the very file this mode
/// exists to ask permission before opening. So `plan_hint` is `None` on a machine that has
/// never turned the mode on, and this function says "do not ask". The alternatives are to
/// read an account file before being allowed to, which is the thing this product does not
/// do, or to put the toggle in settings and let people find it — which is what happens
/// until WP5 decides otherwise.
#[must_use]
pub fn should_suggest_detailed(plan_hint: Option<&str>, already_asked: bool) -> bool {
    if already_asked {
        return false;
    }
    matches!(plan_hint, Some("max_20x" | "max_5x"))
}

/// The mode itself: a reader when it is on, nothing at all when it is off.
///
/// This is the type the switch turns. With `detailedWindows` false there is no reader,
/// [`DetailedWindows::refresh`] returns `None` without looking at anything, and no code
/// path in this module runs — which is the difference between a product that does not read
/// credentials and one that has decided not to today.
#[derive(Debug, Default)]
pub struct DetailedWindows {
    reader: Option<DetailedReader>,
}

impl DetailedWindows {
    /// On or off, with the reader built only when it is on.
    ///
    /// The closure is what makes "off means nothing happens" mechanical rather than
    /// promised: with the mode off it is never called, so there is not even a path to
    /// resolve.
    pub fn new(enabled: bool, build: impl FnOnce() -> Option<DetailedReader>) -> Self {
        DetailedWindows {
            reader: if enabled { build() } else { None },
        }
    }

    /// The mode as `config.detailedWindows` left it, pointed at the real endpoint.
    pub fn from_config(enabled: bool) -> Self {
        DetailedWindows::new(enabled, || DetailedReader::discover().ok())
    }

    /// Whether the mode is on.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.reader.is_some()
    }

    /// Ask the endpoint, or do nothing at all.
    pub fn refresh(&mut self, clock: &dyn Clock) -> Option<Outcome> {
        self.reader.as_mut().map(|reader| reader.refresh(clock))
    }
}

/// The endpoint half of the Claude provider.
///
/// Holds the last good reading and the backoff, and nothing else: no token, no account,
/// nothing on disk. Dropping it forgets everything, which is the intended way to turn the
/// mode off at run time.
#[derive(Debug)]
pub struct DetailedReader {
    client: UsageClient,
    credentials_path: PathBuf,
    backoff: Backoff,
    last_good: Option<Reading>,
    seen: Option<credentials::Stamp>,
}

impl DetailedReader {
    /// A reader for the real endpoint and this machine's Claude Code directory.
    pub fn discover() -> crate::error::Result<Self> {
        Ok(DetailedReader::new(
            UsageClient::official(),
            credentials_path()?,
        ))
    }

    /// A reader with an explicit client and credential path. The tests' way in.
    #[must_use]
    pub fn new(client: UsageClient, credentials_path: impl Into<PathBuf>) -> Self {
        DetailedReader {
            client,
            credentials_path: credentials_path.into(),
            backoff: Backoff::new(),
            last_good: None,
            seen: None,
        }
    }

    /// The file this reader will open, and only while the mode is on.
    #[must_use]
    pub fn credentials_path(&self) -> &Path {
        &self.credentials_path
    }

    /// The backoff, for a caller that wants to show "trying again in …".
    #[must_use]
    pub fn backoff(&self) -> &Backoff {
        &self.backoff
    }

    /// Ask the endpoint, or explain why not.
    ///
    /// The whole flow, in the order the checks are cheapest:
    ///
    /// 1. Has the sign-in file changed since last time? Then whatever we were waiting out
    ///    may be over.
    /// 2. Are we still waiting? Then do not ask.
    /// 3. Is there a sign-in at all, and has it already expired by its own clock? An
    ///    expired token is a 401 that has not been sent yet.
    /// 4. Ask. Read the answer. Map it.
    ///
    /// Anything that fails keeps the last good numbers and marks them remembered — except
    /// "there is no sign-in here", which throws them away, because numbers from an account
    /// that is no longer signed in are worse than none.
    pub fn refresh(&mut self, clock: &dyn Clock) -> Outcome {
        let now_millis = clock.monotonic_millis();
        self.notice_new_sign_in();

        if let Some(remaining) = self.backoff.remaining(now_millis) {
            return self.remembered(DetailedError::BackingOff { remaining });
        }

        let sign_in = match credentials::read_sign_in(&self.credentials_path) {
            Ok(sign_in) => sign_in,
            Err(error) => return self.failed(now_millis, error, None),
        };

        // The two plan hints are the only strings that outlive the sign-in, and neither is
        // a secret: `default_claude_max_20x` and `max` are the two observed values.
        let tier = sign_in.rate_limit_tier.clone();
        let subscription = sign_in.subscription_type.clone();

        // A sign-in for a different plan than the one the remembered numbers came from is
        // a different account. Keeping the old percentages under the new plan's name is
        // audit finding B25.
        let plan = normalise_plan(tier.as_deref(), subscription.as_deref());
        if self
            .last_good
            .as_ref()
            .is_some_and(|reading| reading.plan != plan)
        {
            self.last_good = None;
        }

        let now = clock.now_rfc3339();
        if let Some(seconds) = crate::timefmt::unix_seconds_from_rfc3339(&now) {
            if sign_in.is_expired(seconds) {
                return self.failed(now_millis, DetailedError::Expired, None);
            }
        }

        let answer = match self.client.get(&sign_in.token) {
            Ok(answer) => answer,
            Err(kind) => return self.failed(now_millis, DetailedError::Network { kind }, None),
        };
        // The token has done its one job. Everything below this line works on numbers.
        drop(sign_in);

        match answer.status {
            200 => match map_response(&answer.body, tier.as_deref(), subscription.as_deref()) {
                Ok(mapped) => self.succeeded(now, mapped),
                Err(error) => self.failed(now_millis, error, None),
            },
            401 | 403 => self.failed(now_millis, DetailedError::Unauthorized, None),
            429 => self.failed(
                now_millis,
                DetailedError::RateLimited {
                    retry_after: answer.retry_after,
                },
                answer.retry_after,
            ),
            code => self.failed(now_millis, DetailedError::Status { code }, None),
        }
    }

    /// Clear the wait when Claude Code has rewritten the sign-in file.
    ///
    /// The commonest reason it changes is a token refresh, which is exactly the event that
    /// makes a 401 stop being true. Waiting out half an hour of backoff after the problem
    /// has been fixed is what the audit saw in the prototype's logs.
    fn notice_new_sign_in(&mut self) {
        let stamp = credentials::stamp(&self.credentials_path);
        if self.seen.is_some() && self.seen != stamp {
            self.backoff.cleared();
        }
        self.seen = stamp;
    }

    /// Record a fetch and hand back its numbers.
    fn succeeded(&mut self, now: String, mapped: Mapped) -> Outcome {
        self.backoff.succeeded();
        let reading = Reading {
            plan: mapped.plan,
            fetched_at: now,
            windows: mapped.windows,
            remembered: false,
            reason: None,
        };
        self.last_good = Some(reading.clone());
        Outcome {
            reading: Some(reading),
            error: None,
        }
    }

    /// Record a failure, start the wait, and hand back what is still known.
    fn failed(
        &mut self,
        now_millis: u64,
        error: DetailedError,
        retry_after: Option<Duration>,
    ) -> Outcome {
        if error.backs_off() {
            self.backoff.failed(now_millis, retry_after);
        }
        if !error.keeps_last_good() {
            self.last_good = None;
        }
        self.remembered(error)
    }

    /// The last good numbers, marked as remembered, with the reason.
    fn remembered(&self, error: DetailedError) -> Outcome {
        let reading = self.last_good.as_ref().map(|reading| Reading {
            remembered: true,
            reason: Some(error.to_string()),
            ..reading.clone()
        });
        Outcome {
            reading,
            error: Some(error),
        }
    }
}

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;
