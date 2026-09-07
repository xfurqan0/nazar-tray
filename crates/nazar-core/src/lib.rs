//! Pure-Rust core of nazar-tray.
//!
//! This crate owns the `limits.json` contract described in `docs/limits-contract.md`:
//! the serde types, the atomic writer and the reader. It knows nothing about Tauri, the
//! tray, or any UI, so it builds and tests on every platform.
//!
//! Two rules shape everything here:
//!
//! 1. **Never invent a number.** A window whose value could not be read has no `percent`
//!    at all and a `state` of [`WindowState::Error`]. Consumers render "unknown", never
//!    a reassuring `0`.
//! 2. **Never leave a half-written file.** Every write goes to a temporary file in the
//!    same directory and is renamed into place ([`atomic`]).

pub mod atomic;
pub mod error;
pub mod limits;
pub mod paths;

#[cfg(test)]
pub(crate) mod testutil;

pub use error::{Error, Result};
pub use limits::{
    Limits, Provider, Providers, SCHEMA_VERSION, Source, Window, WindowState, read_limits,
    write_limits,
};
