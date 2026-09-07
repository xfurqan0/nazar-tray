//! Pure-Rust core of nazar-tray.
//!
//! This crate owns the `limits.json` contract described in `docs/limits-contract.md` —
//! the serde types, the atomic writer and the reader — and the readers that fill it in.
//! It knows nothing about Tauri, the tray, or any UI, so it builds and tests on every
//! platform.
//!
//! Three rules shape everything here:
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
//!
//! There is exactly one sanctioned exception to rule 3, and it is a switch the user turns:
//! [`claude::detailed`], the opt-in detailed-windows mode, which reads Claude Code's OAuth
//! token into memory for a single request to the official usage endpoint and never writes
//! it anywhere. It is off by default, it lives behind the `detailed-windows` cargo feature
//! as well as the runtime flag, and `docs/detailed-windows.md` is the whole story.

pub mod atomic;
pub mod claude;
pub mod codex;
pub mod config;
pub mod error;
pub mod limits;
pub mod paths;
pub mod timefmt;

#[cfg(test)]
pub(crate) mod testutil;

pub use claude::ClaudeReader;
#[cfg(feature = "detailed-windows")]
pub use claude::detailed::{DetailedWindows, should_suggest_detailed};
#[cfg(feature = "detailed-windows")]
pub use claude::merge::merge as merge_claude;
pub use codex::CodexReader;
pub use config::Config;
pub use error::{Error, Result};
pub use limits::{
    Limits, Provider, Providers, SCHEMA_VERSION, Source, Window, WindowState, read_limits,
    write_limits,
};
pub use timefmt::now_rfc3339;
