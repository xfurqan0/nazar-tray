//! `nazar-statusline` — the shared status-line wrapper.
//!
//! Claude Code runs one command every time it redraws its status line and hands it a JSON
//! payload on standard input. That payload carries, among much else, the two quota
//! windows the server last reported. This program is installed as that command. It writes
//! the payload down and then runs whatever command the user had before, with the same
//! bytes, forwarding its output and its exit code — so from the user's side nothing has
//! changed, and a file has appeared that two other programs can read.
//!
//! ```text
//!  Claude Code ──payload──▶ nazar-statusline ──▶ ~/.nazar/statusline/<session_id>.json
//!                                  │                        │
//!                                  │                        ├──▶ nazar-tray  (this repo)
//!                                  │                        └──▶ Nazar       (the canvas)
//!                                  └──same payload──▶ the status line you already had
//! ```
//!
//! **One owner, one contract, two consumers.** The wrapper lives here because nazar-tray
//! is the project whose whole claim is a single binary with no prerequisites; making the
//! wrapper an npm package would put Node in front of that, and Node's start-up alone would
//! spend the time budget. What the two projects share is the file format, not the code.
//!
//! ## The parts
//!
//! | Module | Job |
//! |---|---|
//! | [`capture`] | The hot path: read the payload, write it, chain, or print a minimal line |
//! | [`chain`] | The status line that was there before, and running it |
//! | [`settings`] | Parsing and rewriting `settings.json` without damaging it |
//! | [`install`] | `install`, `uninstall`, `status`, and the order they do things in |
//! | [`diff`] | The unified diff every edit is shown as, before it happens |
//!
//! ## The rules it is built around
//!
//! 1. **The status line is never broken by us.** Every failure path still runs the user's
//!    own command. There is no input this program refuses to hand on.
//! 2. **The settings file is never damaged.** Invalid JSON is reported, never repaired.
//!    An untouched copy is taken first and never written over. Only `statusLine` is
//!    touched, and every other key — and the order of all of them — survives.
//! 3. **Nothing leaves the machine, and nothing is read that was not offered.** The
//!    payload arrives on standard input; the only other file read is `settings.json`.
//! 4. **The user sees the edit before it happens**, as a diff, in a dry run and a real
//!    run alike.

pub mod capture;
pub mod chain;
pub mod diff;
pub mod fail;
pub mod install;
pub mod settings;
pub mod testutil;

pub use fail::{Failure, Result};
