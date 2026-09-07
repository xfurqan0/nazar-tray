//! The wrapper itself: read the payload, write it down, hand it on.
//!
//! This is what runs on every status-line refresh, so it is written for the clock. It
//! reads standard input once, parses it once, writes one file, and starts the chained
//! command. There is no configuration file to read, no directory to walk on the hot path,
//! and nothing to log.
//!
//! ```text
//! stdin ──▶ parse ──▶ ~/.nazar/statusline/<session_id>.json   (whole payload, atomically)
//!   │
//!   └──────────────▶ the user's own status-line command, same bytes, output forwarded
//! ```
//!
//! ## What is written, and why all of it
//!
//! The capture holds the **whole** payload. Not because everything in it is needed —
//! today only `rate_limits` is — but because the alternative is for this program to
//! decide, on every refresh, which of Claude Code's fields the two consumers of the file
//! will ever want, and to be wrong the first time one of them wants another. The payload
//! carries no token and no account identifier; it does carry paths (`cwd`,
//! `transcript_path`, the workspace), which is why the file lives under `~/.nazar` with
//! the home directory's permissions and is never uploaded anywhere.
//!
//! ## What happens when things are odd
//!
//! * **Empty or unparseable input:** write nothing, still chain, exit `0`. A status line
//!   that fails because its wrapper did not like the input is worse than a wrapper that
//!   quietly captures nothing this tick.
//! * **No `session_id`:** the capture goes to `unknown.json`. Every payload observed has
//!   one; this is the case that is not supposed to happen, and it is better for it to
//!   land in one predictable file than to be dropped.
//! * **Nothing to chain to:** print a minimal line of our own, so a user who had no
//!   status line before gets something rather than a blank prompt line.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::chain;
use crate::fail::Result;

/// Version of the capture envelope. Bumping it is a change for Nazar as well.
pub const CAPTURE_SCHEMA_VERSION: u64 = nazar_core::claude::CAPTURE_SCHEMA_VERSION;

/// Longest session identifier accepted as a file name.
const MAX_KEY_LEN: usize = 128;

/// Where a payload with no usable session id goes.
const UNKNOWN_KEY: &str = "unknown";

/// How long a capture file is kept once its session has stopped writing to it.
///
/// One file per session, a few kilobytes each, so this is tidiness rather than pressure.
/// Long enough that a machine left alone over a week still shows its last known numbers
/// with an honest age next to them.
pub const CAPTURE_LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The whole wrapper, start to finish. Returns the process exit code.
///
/// Nothing here can stop the status line from being drawn. A machine with no home
/// directory, a directory that cannot be written, a payload that is not JSON: each of
/// them skips the capture and goes straight on to the user's own command.
pub fn run() -> i32 {
    let payload = read_stdin();

    // Parsing decides only *whether* to capture. A failure here is never a failure of the
    // status line: the user's own command still runs, with the same bytes.
    if let Some(document) = parse(&payload) {
        if let Ok(directory) = nazar_core::paths::statusline_dir() {
            let first_write = capture(&directory, &document, &payload).unwrap_or(false);
            if first_write {
                // Once per session rather than once per refresh: the scan is cheap, but
                // the hot path is measured in milliseconds and this is not part of it.
                prune(&directory, CAPTURE_LIFETIME);
            }
        }
    }

    match chain::load().ok().flatten() {
        Some(chain) => match chain.command() {
            Some(command) => chain::run(command, &payload),
            None => default_line(&payload),
        },
        None => default_line(&payload),
    }
}

/// Read every byte of standard input.
///
/// A payload is a couple of kilobytes. The cap is here so that a pipe that never ends
/// cannot turn the status line into a hang; a payload larger than this is not one.
fn read_stdin() -> Vec<u8> {
    const MAX_PAYLOAD: u64 = 8 * 1024 * 1024;
    let mut buffer = Vec::with_capacity(8 * 1024);
    let _ = std::io::stdin()
        .lock()
        .take(MAX_PAYLOAD)
        .read_to_end(&mut buffer);
    buffer
}

/// The payload as JSON text: valid UTF-8, no byte-order mark, no surrounding blank space.
///
/// The byte-order mark is not hypothetical. Anything that writes to this program's
/// standard input through a text stream configured for UTF-8 can put one there — a
/// PowerShell `StreamWriter` does it by default — and `serde_json` refuses a document that
/// starts with one. Costing a status line its capture over three invisible bytes would be
/// a silly way to lose data.
#[must_use]
fn payload_text(payload: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(payload)
        .ok()?
        .trim_start_matches('\u{feff}')
        .trim();
    (!text.is_empty()).then_some(text)
}

/// Parse the payload, or decide there is nothing to capture.
#[must_use]
pub fn parse(payload: &[u8]) -> Option<Value> {
    let value: Value = serde_json::from_str(payload_text(payload)?).ok()?;
    value.is_object().then_some(value)
}

/// Write the capture. `Ok(true)` when the file was not there before.
///
/// Atomic — a temporary file beside the target, then a rename — because the tray and the
/// Nazar canvas read this directory while a status line is writing to it, and a reader
/// must see the previous capture or the new one, never half of either.
pub fn capture(directory: &Path, document: &Value, payload: &[u8]) -> Result<bool> {
    let key = capture_key(document);
    let path = directory.join(format!("{key}.json"));
    let existed = path.exists();
    nazar_core::atomic::write_bytes(&path, &envelope(&key, payload))?;
    Ok(!existed)
}

/// The capture file for a payload: `<session_id>.json`, sanitised.
///
/// The session id becomes a file name, so it is checked rather than trusted: letters,
/// digits, dot, dash and underscore only, nothing that starts with a dot, and never the
/// name of the wrapper's own state file.
#[must_use]
pub fn capture_key(document: &Value) -> String {
    let raw = document
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();

    let usable = !raw.is_empty()
        && raw.len() <= MAX_KEY_LEN
        && !raw.starts_with('.')
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));

    if !usable {
        return UNKNOWN_KEY.to_owned();
    }
    // `chain.json` is the wrapper's own state file. A session that happened to be called
    // `chain` would otherwise overwrite it.
    if raw.eq_ignore_ascii_case("chain") {
        return format!("{raw}-session");
    }
    raw.to_owned()
}

/// Wrap the payload with the little the reader needs and nothing else.
///
/// The payload's own bytes go in verbatim: it is already JSON, re-serialising it would
/// cost time and could change a number's spelling, and "the whole payload" should mean
/// the bytes Claude Code sent.
#[must_use]
pub fn envelope(key: &str, payload: &[u8]) -> Vec<u8> {
    let trimmed = payload_text(payload).unwrap_or("{}");
    let session = serde_json::to_string(key).unwrap_or_else(|_| "\"unknown\"".to_owned());
    let mut out = Vec::with_capacity(trimmed.len() + 256);
    out.extend_from_slice(
        format!(
            "{{\n  \"schemaVersion\": {CAPTURE_SCHEMA_VERSION},\n  \"updatedAt\": \"{}\",\n  \
             \"wrapper\": \"nazar-statusline/{}\",\n  \"sessionId\": {session},\n  \
             \"payload\": ",
            nazar_core::now_rfc3339(),
            env!("CARGO_PKG_VERSION"),
        )
        .as_bytes(),
    );
    out.extend_from_slice(trimmed.as_bytes());
    out.extend_from_slice(b"\n}\n");
    out
}

/// Delete captures nobody has written to for `lifetime`.
///
/// Best effort throughout: a file that will not go is a file the next run tries again.
pub fn prune(directory: &Path, lifetime: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.filter_map(std::result::Result::ok) {
        let path: PathBuf = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str())
            == Some(nazar_core::claude::CHAIN_FILE_NAME)
        {
            continue;
        }
        let too_old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > lifetime);
        if too_old && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Print a minimal status line, for a user who had none before.
///
/// Model, effort and whichever quota windows the payload carried. Nothing is invented: a
/// field that is not there is a field that is not printed, and a payload with nothing
/// worth printing prints nothing at all rather than a lonely separator.
fn default_line(payload: &[u8]) -> i32 {
    let Some(document) = parse(payload) else {
        return 0;
    };
    let text = render_default_line(&document);
    if !text.is_empty() {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{text}");
        let _ = stdout.flush();
    }
    0
}

/// The text of the built-in status line. Separated from the printing so it can be tested.
#[must_use]
pub fn render_default_line(document: &Value) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(4);

    if let Some(model) = document
        .get("model")
        .and_then(|model| model.get("display_name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        parts.push(model.to_owned());
    }
    if let Some(effort) = document
        .get("effort")
        .and_then(|effort| effort.get("level"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|level| !level.is_empty())
    {
        parts.push(effort.to_owned());
    }
    for (label, key) in [("5h", "five_hour"), ("7d", "seven_day")] {
        let Some(percent) = document
            .get("rate_limits")
            .and_then(|limits| limits.get(key))
            .and_then(|window| window.get("used_percentage"))
            .and_then(Value::as_f64)
            .filter(|percent| percent.is_finite())
        else {
            continue;
        };
        parts.push(format!("{label} {}%", percent.round() as i64));
    }

    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    const REAL: &str = include_str!("../../../fixtures/claude/statusline-payload.json");
    const BOTH: &str =
        include_str!("../../../fixtures/claude/statusline-payload-both-windows.json");

    fn document(text: &str) -> Value {
        parse(text.as_bytes()).expect("the fixture must parse")
    }

    #[test]
    fn the_capture_holds_the_whole_payload_and_a_stamp() {
        let dir = TempDir::new("capture-whole");
        let payload = REAL.as_bytes();
        let first = capture(&dir.path, &document(REAL), payload).unwrap();
        assert!(first, "the first write reports itself as the first");

        let path = dir.join("00000000-0000-4000-8000-000000000001.json");
        let text = std::fs::read_to_string(&path).unwrap();
        let envelope: Value = serde_json::from_str(&text).unwrap();

        assert_eq!(
            envelope["schemaVersion"],
            Value::from(CAPTURE_SCHEMA_VERSION)
        );
        assert_eq!(
            envelope["sessionId"],
            Value::from("00000000-0000-4000-8000-000000000001")
        );
        assert!(
            envelope["wrapper"]
                .as_str()
                .unwrap()
                .starts_with("nazar-statusline/")
        );
        assert!(
            nazar_core::timefmt::sanitize_timestamp(envelope["updatedAt"].as_str().unwrap())
                .is_some()
        );

        // Every key of the payload survived, byte-identically as a value.
        let original: Value = serde_json::from_str(REAL).unwrap();
        assert_eq!(envelope["payload"], original);
        assert!(!capture(&dir.path, &document(REAL), payload).unwrap());
    }

    #[test]
    fn the_capture_is_the_reader_s_capture() {
        // The other half of the contract: what this writes, `nazar-core` reads.
        let dir = TempDir::new("capture-readable");
        capture(&dir.path, &document(BOTH), BOTH.as_bytes()).unwrap();

        let provider = nazar_core::ClaudeReader::new(&dir.path).refresh();
        assert!(provider.configured);
        assert_eq!(
            provider.windows[nazar_core::claude::WINDOW_FIVE_HOUR].percent,
            Some(12.0)
        );
        assert_eq!(
            provider.windows[nazar_core::claude::WINDOW_SEVEN_DAY].percent,
            Some(31.0)
        );
    }

    #[test]
    fn three_sessions_get_three_files() {
        let dir = TempDir::new("capture-three");
        for session in ["session-a", "session-b", "session-c"] {
            let payload = format!(r#"{{"session_id":"{session}","rate_limits":{{}}}}"#);
            capture(&dir.path, &document(&payload), payload.as_bytes()).unwrap();
        }
        let count = std::fs::read_dir(&dir.path).unwrap().count();
        assert_eq!(count, 3, "one capture per session, never one shared file");
    }

    #[test]
    fn a_session_id_that_would_escape_the_directory_does_not() {
        for (raw, expected) in [
            ("../../etc/passwd", UNKNOWN_KEY),
            ("..", UNKNOWN_KEY),
            (".hidden", UNKNOWN_KEY),
            ("with/slash", UNKNOWN_KEY),
            ("with\\backslash", UNKNOWN_KEY),
            ("with space", UNKNOWN_KEY),
            ("with:colon", UNKNOWN_KEY),
            ("", UNKNOWN_KEY),
            ("chain", "chain-session"),
            ("CHAIN", "CHAIN-session"),
            (
                "00000000-0000-4000-8000-000000000001",
                "00000000-0000-4000-8000-000000000001",
            ),
        ] {
            let document = serde_json::json!({ "session_id": raw });
            assert_eq!(capture_key(&document), expected, "session id {raw:?}");
        }
        assert_eq!(capture_key(&serde_json::json!({})), UNKNOWN_KEY);
        assert_eq!(
            capture_key(&serde_json::json!({"session_id": 42})),
            UNKNOWN_KEY
        );
        assert_eq!(
            capture_key(&serde_json::json!({"session_id": "x".repeat(MAX_KEY_LEN + 1)})),
            UNKNOWN_KEY
        );
    }

    #[test]
    fn the_wrapper_state_file_can_never_be_overwritten_by_a_session() {
        let dir = TempDir::new("capture-chain-guard");
        let payload = r#"{"session_id":"chain"}"#;
        capture(&dir.path, &document(payload), payload.as_bytes()).unwrap();
        assert!(!dir.join(nazar_core::claude::CHAIN_FILE_NAME).exists());
        assert!(dir.join("chain-session.json").exists());
    }

    #[test]
    fn input_that_is_not_a_payload_is_not_captured() {
        for text in [
            "",
            "   \n",
            "not json",
            "[1,2,3]",
            "\"a string\"",
            "42",
            "null",
        ] {
            assert_eq!(
                parse(text.as_bytes()),
                None,
                "{text:?} must not be captured"
            );
        }
        assert_eq!(parse(&[0xff, 0xfe, 0x00]), None, "invalid UTF-8");
    }

    #[test]
    fn a_byte_order_mark_does_not_cost_the_capture() {
        let mut with_bom = vec![0xef, 0xbb, 0xbf];
        with_bom.extend_from_slice(REAL.as_bytes());

        let document = parse(&with_bom).expect("a payload with a byte-order mark still parses");
        assert_eq!(
            capture_key(&document),
            "00000000-0000-4000-8000-000000000001"
        );

        // And the envelope it produces is valid JSON, with no mark left inside it.
        let text = String::from_utf8(envelope("session-a", &with_bom)).unwrap();
        assert!(!text.contains('\u{feff}'));
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            parsed["payload"],
            serde_json::from_str::<Value>(REAL).unwrap()
        );
    }

    #[test]
    fn pruning_removes_the_old_and_keeps_the_new_and_the_state_file() {
        let dir = TempDir::new("capture-prune");
        let old = dir.join("old-session.json");
        let fresh = dir.join("fresh-session.json");
        let state = dir.join(nazar_core::claude::CHAIN_FILE_NAME);
        for path in [&old, &fresh, &state] {
            std::fs::write(path, "{}").unwrap();
        }
        std::fs::write(dir.join("notes.txt"), "not ours").unwrap();

        let long_ago = SystemTime::now() - Duration::from_secs(30 * 24 * 60 * 60);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&state)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();

        assert_eq!(prune(&dir.path, CAPTURE_LIFETIME), 1);
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(state.exists(), "the wrapper's own state is not a capture");
        assert!(dir.join("notes.txt").exists());
    }

    #[test]
    fn the_default_line_prints_what_it_has_and_nothing_it_does_not() {
        assert_eq!(
            render_default_line(&document(BOTH)),
            "Fable 5.1 · high · 5h 12% · 7d 31%"
        );
        assert_eq!(
            render_default_line(&document(REAL)),
            "Fable 5.1 · high · 7d 31%",
            "a window the payload did not carry is not printed"
        );
        assert_eq!(
            render_default_line(&serde_json::json!({"session_id": "x"})),
            "",
            "nothing known means nothing printed"
        );
        assert_eq!(
            render_default_line(&serde_json::json!({"model": {"display_name": "Fable 5.1"}})),
            "Fable 5.1"
        );
        assert_eq!(
            render_default_line(&serde_json::json!({
                "rate_limits": {"five_hour": {"used_percentage": 12.4}}
            })),
            "5h 12%",
            "the percentage is rounded at display time, which is here"
        );
    }
}
