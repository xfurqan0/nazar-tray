//! The one command-line mode the tray has: `nazar-tray --print`.
//!
//! It builds the `limits.json` document from whichever readers exist today and writes it
//! to standard output. Nothing else happens: no window, no tray icon, and **no write to
//! `~/.nazar/limits.json`** — the file has exactly one writer and wiring it up is WP3's
//! job, not a side effect of a command that says "print".
//!
//! Why it exists before the package that needs it (WP8, the Nazar handoff): a reader is
//! only finished when something outside the test suite can see what it produces. This is
//! how the Codex reader gets checked against the real logs on a real machine, and it is
//! how Nazar and the Linux story will read the same numbers later.
//!
//! One limitation, deliberate and documented rather than worked around here: the tray is
//! a GUI-subsystem binary on Windows, so a release build launched from an interactive
//! console has no console to print to. Redirected output — `nazar-tray --print > out.json`
//! or a pipe, which is how a consumer actually calls it — works in every build. Attaching
//! to the parent console belongs with the rest of the command-line surface in WP8.
//!
//! ## `--detailed`
//!
//! Turns the opt-in detailed-windows mode on **for one run**, without touching
//! `config.json`. It is here so the mode can be checked against a real account without
//! first switching it on for the machine, and so a script can ask for the model-scoped
//! numbers once. Everything the mode does with it on is what `docs/detailed-windows.md`
//! describes: one request, a token held in memory, nothing written.

use std::io::Write;

use nazar_core::claude::ClaudeReader;
use nazar_core::claude::detailed::{DetailedWindows, SystemClock};
use nazar_core::claude::merge::merge;
use nazar_core::codex::CodexReader;
use nazar_core::{Config, Limits, Provider, now_rfc3339};

/// The flag that turns the tray into a one-shot printer.
const PRINT_FLAG: &str = "--print";

/// The flag that turns the opt-in mode on for this run only.
const DETAILED_FLAG: &str = "--detailed";

/// Run a command-line mode if one was asked for.
///
/// Returns `true` when the process has done its job and should exit without starting the
/// tray. Unknown arguments are ignored: this is a tray application that happens to have a
/// flag, not a command-line tool that happens to have a tray, so an argument it does not
/// understand must not stop it from starting.
pub fn run_if_requested() -> bool {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if !arguments.iter().any(|argument| argument == PRINT_FLAG) {
        return false;
    }

    let document = snapshot(arguments.iter().any(|argument| argument == DETAILED_FLAG));
    let text = match document.to_json() {
        Ok(text) => text,
        Err(error) => {
            // The document is built from our own types, so this cannot happen; saying so
            // out loud is still better than an empty stdout and a zero exit code.
            eprintln!("nazar-tray: could not render limits.json: {error}");
            std::process::exit(1);
        }
    };

    let mut stdout = std::io::stdout().lock();
    if stdout.write_all(text.as_bytes()).is_err() {
        // A closed pipe (`nazar-tray --print | head`) is not an error worth a message.
        std::process::exit(0);
    }
    let _ = stdout.flush();
    true
}

/// Build the `limits.json` document from the readers that exist.
///
/// Both providers are read from local files their own tools wrote: Codex from its newest
/// `rollout-*.jsonl`, Claude from the status-line captures `nazar-statusline` leaves in
/// `~/.nazar/statusline`. Both keys are always present — the contract says a provider's
/// key never disappears, so a consumer can tell "not set up on this machine" from "this
/// build does not know about that provider" without special cases.
///
/// `force_detailed` is the `--detailed` flag. Without it the opt-in mode does exactly what
/// `config.json` says, which on a machine nobody has configured is nothing at all.
#[must_use]
pub fn snapshot(force_detailed: bool) -> Limits {
    // Settings that cannot be read are settings that were never written: the defaults,
    // which have the mode off. A damaged `config.json` must not be a reason for `--print`
    // to fail, and it must certainly not be a reason to turn something on.
    let configured = Config::load().is_ok_and(|config| config.detailed_windows);
    build(&mut DetailedWindows::from_config(
        force_detailed || configured,
    ))
}

/// The snapshot with the opt-in mode handed in rather than discovered.
///
/// This is what the tests call, with a [`DetailedWindows`] that is off. A test suite that
/// went through [`snapshot`] would ask the real endpoint on the machine of anybody who had
/// turned the mode on for themselves, which is precisely the thing this repository's tests
/// are not allowed to do.
fn build(mode: &mut DetailedWindows) -> Limits {
    let now = now_rfc3339();
    let mut limits = Limits::new(now.clone());

    limits.providers.codex = match CodexReader::discover() {
        Ok(mut reader) => reader.refresh(),
        // No home directory at all: an unusual environment, and the honest answer is the
        // same as an uninstalled Codex.
        Err(_) => Provider::default(),
    };

    let passive = match ClaudeReader::discover() {
        Ok(mut reader) => reader.refresh(),
        Err(_) => Provider::default(),
    };
    let outcome = mode.refresh(&SystemClock);
    let reading = outcome
        .as_ref()
        .and_then(|outcome| outcome.reading.as_ref());

    limits.providers.claude = merge(passive, reading, &now);
    limits
}

#[cfg(test)]
mod tests {
    use super::*;
    use nazar_core::SCHEMA_VERSION;

    /// A snapshot with the opt-in mode off, whatever this machine's settings say.
    ///
    /// Every test here goes through this. Nothing in this repository's test suite is
    /// allowed to read a token or open a socket, and a test that read `config.json` would
    /// do both on the machine of anyone who had turned the mode on.
    fn offline_snapshot() -> Limits {
        build(&mut DetailedWindows::default())
    }

    #[test]
    fn the_snapshot_is_a_valid_contract_document() {
        let limits = offline_snapshot();
        assert_eq!(limits.schema_version, SCHEMA_VERSION);
        assert!(
            nazar_core::timefmt::sanitize_timestamp(&limits.updated_at).is_some(),
            "updatedAt must be a timestamp, got {}",
            limits.updated_at
        );

        // Round-trips through the contract's own parser, whatever this machine holds.
        let text = limits.to_json().unwrap();
        let parsed = Limits::from_json(&text).expect("the printed document must parse");
        assert_eq!(parsed, limits);
    }

    /// Both provider keys are always in the document, whatever this machine holds.
    ///
    /// The assertion cannot be "Claude is configured" or "Claude is not": the answer
    /// depends on whether the wrapper is installed on the machine running the tests, and a
    /// test that demands one of those is a test that fails on somebody else's computer.
    /// What the contract actually promises is that the key is there either way, and that
    /// an unconfigured provider carries no windows.
    #[test]
    fn both_providers_are_present_however_the_machine_is_set_up() {
        let limits = offline_snapshot();
        for provider in [&limits.providers.claude, &limits.providers.codex] {
            if !provider.configured {
                assert!(provider.windows.is_empty());
                assert_eq!(provider.binding, None);
            }
        }
    }

    #[test]
    fn the_printed_document_never_carries_a_percentage_it_did_not_read() {
        let limits = offline_snapshot();
        for (key, provider) in [
            ("claude", &limits.providers.claude),
            ("codex", &limits.providers.codex),
        ] {
            for (name, window) in &provider.windows {
                if window.state == nazar_core::WindowState::Error {
                    assert_eq!(
                        window.percent, None,
                        "{key}.{name} has a percentage it could not read"
                    );
                }
            }
        }
    }
}
