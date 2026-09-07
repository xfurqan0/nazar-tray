//! Two gates that fail the build rather than a review.
//!
//! nazar-tray's whole claim is "no credentials, no network". A claim that lives only in a
//! README survives exactly until the first well-meaning patch that adds a fallback. These
//! tests make it survive longer:
//!
//! * **The credential gate** greps the workspace's source for the name of any credential
//!   file. A reader that wants a token has to delete a test to get one, which is a thing
//!   a reviewer notices.
//! * **The fixture gate** greps the committed fixtures for anything that identifies the
//!   machine they were captured on. The fixtures are sanitised copies of real session
//!   logs, and the sanitising has to keep working as new ones are added.
//!
//! Both gates build the strings they look for out of fragments, so the test file does not
//! trip itself.

use std::path::{Path, PathBuf};

/// Repository root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/nazar-core is two levels below the repository root")
        .to_path_buf()
}

/// Every file under `dir` whose extension is in `extensions`.
fn files_under(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path, extensions));
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extensions.contains(&extension))
        {
            found.push(path);
        }
    }
    found
}

/// The reader must never name a credential file.
///
/// The retired prototype read `~/.codex/auth.json` and `~/.claude/.credentials.json` to
/// call an endpoint. The passive design does not need either, and this test is what keeps
/// "does not need" from drifting into "does not currently".
///
/// The opt-in detailed-windows mode (WP2b) is the one sanctioned exception, and it will
/// live in its own crate behind its own feature so that this gate keeps holding for
/// everything else. When that lands, this test grows an explicit allow-list rather than
/// losing a needle.
#[test]
fn no_source_file_names_a_credential_file() {
    // Assembled at run time so this file does not match itself.
    let needles: Vec<String> = vec![
        format!("{}{}", "auth", ".json"),
        format!("{}{}", "credential", "s"),
        format!("{}{}", ".credential", "s.json"),
        format!("{}{}", "access", "_token"),
        format!("{}{}", "refresh", "_token"),
        format!("{}{}", "id", "_token"),
        format!("{}{}", "Bearer ", ""),
        format!("{}{}", "api", "_key"),
    ];

    let root = repo_root();
    let mut sources = Vec::new();
    for crate_name in ["nazar-core", "nazar-tray"] {
        sources.extend(files_under(
            &root.join("crates").join(crate_name).join("src"),
            &["rs"],
        ));
    }
    assert!(
        sources.len() >= 8,
        "the gate found almost no source to check, which means it is not working"
    );

    let mut hits = Vec::new();
    for path in sources {
        // This file is the only one allowed to hold the needles, and it is not in `src`.
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lowered = text.to_lowercase();
        for needle in &needles {
            if lowered.contains(&needle.to_lowercase()) {
                hits.push(format!(
                    "{} mentions {needle:?}",
                    path.file_name().unwrap().to_string_lossy()
                ));
            }
        }
    }

    assert!(
        hits.is_empty(),
        "nazar-tray reads no credentials; these files say otherwise:\n  {}",
        hits.join("\n  ")
    );
}

/// The committed fixtures must not identify the machine they were captured on.
///
/// A fixture is a real session log with its text replaced. What has to be gone: home
/// directories, user names, e-mail addresses, and the long hexadecimal identifiers that
/// Codex uses for sessions and threads. What stays: the quota numbers, which are usage
/// figures, and the tier names `plus` and `codex`, which are not identity.
#[test]
fn the_codex_fixtures_carry_nothing_that_identifies_a_machine() {
    let root = repo_root();
    let fixtures = files_under(&root.join("fixtures").join("codex"), &["jsonl"]);
    assert!(
        fixtures.len() >= 4,
        "expected the four Codex fixtures, found {}",
        fixtures.len()
    );

    for path in fixtures {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("cannot read {name}"));

        assert!(!text.contains('~'), "{name} contains a home-relative path");
        for fragment in [
            "C:\\Users\\",
            "C:/Users/",
            "/home/",
            "/Users/",
            "%USERPROFILE%",
            "$HOME",
        ] {
            assert!(
                !text.contains(fragment),
                "{name} contains the path fragment {fragment:?}"
            );
        }
        assert!(
            !looks_like_an_email(&text),
            "{name} contains an e-mail address"
        );
        assert!(
            !contains_long_hex(&text),
            "{name} contains a 32-character hexadecimal identifier"
        );

        // Whoever runs the tests: their own name must not be in a file they are about to
        // publish. Read from the environment so the name itself is never committed.
        for variable in ["USERNAME", "USER", "LOGNAME"] {
            let Some(value) = std::env::var_os(variable) else {
                continue;
            };
            let value = value.to_string_lossy().into_owned();
            if value.len() < 3 {
                continue;
            }
            assert!(
                !text.to_lowercase().contains(&value.to_lowercase()),
                "{name} contains the current user's name"
            );
        }
    }
}

/// A crude but sufficient e-mail shape: `word@word.word`.
fn looks_like_an_email(text: &str) -> bool {
    let bytes = text.as_bytes();
    let atom = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+');
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'@' || index == 0 || index + 1 >= bytes.len() {
            continue;
        }
        if !atom(bytes[index - 1]) {
            continue;
        }
        // A dot somewhere in the next few characters, all of them atom characters.
        let end = (index + 1 + 64).min(bytes.len());
        let domain = &bytes[index + 1..end];
        let run = domain
            .iter()
            .position(|byte| !atom(*byte))
            .unwrap_or(domain.len());
        if run >= 3 && domain[..run].contains(&b'.') {
            return true;
        }
    }
    false
}

/// A run of 32 or more hexadecimal characters: a session or thread identifier.
///
/// The placeholders the fixture generator writes are words, not hex, so a hit here means
/// a real identifier survived the sanitising.
fn contains_long_hex(text: &str) -> bool {
    let mut run = 0usize;
    for byte in text.bytes() {
        if byte.is_ascii_hexdigit() {
            run += 1;
            if run >= 32 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

#[test]
fn the_hex_and_email_detectors_actually_detect() {
    assert!(contains_long_hex(&"a".repeat(32)));
    assert!(contains_long_hex("01a078d4e6477033a6c956235a65cdd7"));
    assert!(!contains_long_hex("01a078d4-e647-7033-a6c9-56235a65cdd7"));
    assert!(!contains_long_hex("[redacted]"));

    assert!(looks_like_an_email("write to someone@example.com please"));
    assert!(!looks_like_an_email("a plain @ sign"));
    assert!(!looks_like_an_email("\"used_percent\":54.0"));
}
