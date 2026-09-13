//! Pure-Rust core of nazar-tray.
//!
//! This crate owns the `limits.json` contract described in `docs/limits-contract.md` —
//! the serde types, the atomic writer and the reader — and the readers that fill it in.
//! It knows nothing about Tauri, the tray, or any UI, so it builds and tests on every
//! platform.
//!
//! Four rules shape everything here:
//!
//! 1. **Never invent a number.** A window whose value could not be read has no `percent`
//!    at all and a `state` of [`WindowState::Error`]. Consumers render "unknown", never
//!    a reassuring `0`.
//! 2. **Never leave a half-written file.** Every write goes to a temporary file in the
//!    same directory and is renamed into place ([`atomic`]).
//! 3. **Never read sign-in material, and never copy source text.** The readers open logs
//!    the user's own tools wrote, take the handful of numbers they came for, and leave the
//!    rest of the file where it is. `docs/pinned-internal-formats.md` lists what is read
//!    and what is off limits; two tests enforce it.
//! 4. **Never store what can be derived.** `limits.json` holds what a source measured;
//!    which window binds, how long until it resets, how old the reading is and how alarming
//!    it is all change with the clock rather than with the data, so they are worked out at
//!    read time in [`state`] and stored nowhere.
//!
//! The moving parts, in the order data goes through them: the readers ([`codex`],
//! [`claude`]) turn somebody else's files into provider blocks, [`refresh`] runs them on one
//! thread and decides when, [`writer`] writes the result only when it differs from the last
//! one, [`lock`] makes sure exactly one process is doing that, [`state`] answers what any of
//! it means right now, and [`alerts`] decides which of those meanings is worth interrupting
//! somebody about — once, and not again until the window resets.
//!
//! There is exactly one sanctioned exception to rule 3, and it is a switch the user turns:
//! [`claude::detailed`], the opt-in detailed-windows mode, which reads Claude Code's OAuth
//! token into memory for a single request to the official usage endpoint and never writes
//! it anywhere. It is off by default, it lives behind the `detailed-windows` cargo feature
//! as well as the runtime flag, and `docs/detailed-windows.md` is the whole story.

pub mod alerts;
pub mod atomic;
pub mod claude;
pub mod clock;
pub mod codex;
pub mod config;
pub mod error;
pub mod limits;
pub mod lock;
pub mod paths;
pub mod refresh;
pub mod state;
pub mod timefmt;
pub mod usage;
pub mod writer;

/// The readers, run against payloads a real installation produced rather than against
/// payloads their author imagined. See the module's own documentation.
#[cfg(test)]
mod captured;
#[cfg(test)]
pub(crate) mod testutil;

pub use alerts::{Alert, AlertLog, AlertRules, Alerts};
pub use claude::ClaudeReader;
#[cfg(feature = "detailed-windows")]
pub use claude::detailed::{DetailedWindows, should_suggest_detailed};
#[cfg(feature = "detailed-windows")]
pub use claude::merge::merge as merge_claude;
pub use clock::{Clock, SystemClock};
pub use codex::CodexReader;
pub use config::{Config, Invalid, ProviderSwitches, QuietHours};
pub use error::{Error, Result};
pub use limits::{
    Limits, Provider, Providers, SCHEMA_VERSION, Source, Window, WindowState, read_limits,
    write_limits,
};
pub use lock::{Acquisition, LimitsLock};
pub use refresh::{Cause, Engine, Event, LoopHandle, ReaderSet, Warnings};
pub use state::{Freshness, Rules, Severity, Snapshot, SnapshotView};
pub use timefmt::now_rfc3339;
pub use usage::{UsageSummary, UsageView, scan_claude, scan_codex};
pub use writer::{LimitsWriter, Written};
