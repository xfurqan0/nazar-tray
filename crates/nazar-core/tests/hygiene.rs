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

/// The one directory allowed to name a sign-in file, relative to the repository root.
///
/// The opt-in detailed-windows mode (WP2b) is the single sanctioned exception to "this
/// product reads no credentials". It is an exception the user turns on, it is behind the
/// `detailed-windows` cargo feature as well as the runtime flag, and it is confined to one
/// directory so that the gate below keeps holding for every other file in the workspace.
const DETAILED_WINDOWS_DIR: &str = "crates/nazar-core/src/claude/detailed";

/// Words that mean a file is reaching for sign-in material.
///
/// Assembled at run time so this file does not match itself.
fn credential_needles() -> Vec<String> {
    vec![
        format!("{}{}", "auth", ".json"),
        format!("{}{}", "credential", "s"),
        format!("{}{}", ".credential", "s.json"),
        format!("{}{}", "access", "_token"),
        format!("{}{}", "refresh", "_token"),
        format!("{}{}", "id", "_token"),
        format!("{}{}", "Bearer ", ""),
        format!("{}{}", "api", "_key"),
    ]
}

/// Every `.rs` file in the workspace's three crates.
fn workspace_sources() -> Vec<PathBuf> {
    let root = repo_root();
    let mut sources = Vec::new();
    for crate_name in ["nazar-core", "nazar-statusline", "nazar-tray"] {
        sources.extend(files_under(
            &root.join("crates").join(crate_name).join("src"),
            &["rs"],
        ));
    }
    sources
}

/// Whether a path is inside the one directory that is allowed the exception.
fn is_allow_listed(path: &Path) -> bool {
    let root = repo_root().join(DETAILED_WINDOWS_DIR);
    path.starts_with(&root)
}

/// No source file outside the opt-in mode may name a sign-in file.
///
/// The retired prototype read `~/.codex/auth.json` and the Claude sign-in file to call an
/// endpoint, on every run, for everybody. The passive design needs neither, and this test
/// is what keeps "does not need" from drifting into "does not currently".
///
/// The allow-list is a **directory**, not a list of needles: dropping a needle would have
/// let the exception spread silently across the workspace, while an allow-listed path
/// makes any new file that wants sign-in material an obvious diff — it has to be created
/// in one specific place, next to the tests that keep it honest.
#[test]
fn no_source_file_outside_the_opt_in_mode_names_a_credential_file() {
    let needles = credential_needles();
    let sources = workspace_sources();
    assert!(
        sources.len() >= 14,
        "the gate found almost no source to check, which means it is not working"
    );

    let mut hits = Vec::new();
    for path in &sources {
        if is_allow_listed(path) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
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
        "outside the opt-in detailed-windows mode, nazar-tray reads no credentials; \
         these files say otherwise:\n  {}",
        hits.join("\n  ")
    );
}

/// The exception has to still be where the allow-list says it is.
///
/// A directory-shaped allow-list has one failure mode: the code moves, the entry stays,
/// and the gate silently guards nothing. So the directory must exist, and something in it
/// must actually trip a needle — if the opt-in mode ever stops reading sign-in material,
/// this test fails and the allow-list gets deleted along with it.
#[test]
fn the_allow_listed_directory_is_the_only_place_that_needs_the_exception() {
    let root = repo_root().join(DETAILED_WINDOWS_DIR);
    assert!(
        root.is_dir(),
        "{DETAILED_WINDOWS_DIR} is not there; move the allow-list or delete it"
    );

    let needles = credential_needles();
    let mut tripped = Vec::new();
    for path in files_under(&root, &["rs"]) {
        let text = std::fs::read_to_string(&path).unwrap().to_lowercase();
        if needles
            .iter()
            .any(|needle| text.contains(&needle.to_lowercase()))
        {
            tripped.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    assert!(
        !tripped.is_empty(),
        "nothing in {DETAILED_WINDOWS_DIR} reads sign-in material any more, so the \
         allow-list is guarding nothing and should go"
    );
}

/// The opt-in mode prints nothing, ever.
///
/// A token that never reaches a file can still reach a terminal. There is no logging
/// framework in this crate to configure away, so the rule is mechanical: the module that
/// handles the token contains no printing macro at all. The tray, which does have a
/// console in debug builds, gets its diagnostics from returned values.
#[test]
fn the_opt_in_mode_contains_no_printing_at_all() {
    let root = repo_root().join(DETAILED_WINDOWS_DIR);
    // Assembled so this file does not match itself.
    let macros = [
        format!("{}{}", "print", "ln!"),
        format!("{}{}", "eprint", "ln!"),
        format!("{}{}", "e", "print!"),
        format!("{}{}", "db", "g!"),
    ];

    let mut hits = Vec::new();
    for path in files_under(&root, &["rs"]) {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // The mock server is test scaffolding and prints nothing either, but the rule is
        // simplest when it has no exceptions at all.
        let text = std::fs::read_to_string(&path).unwrap();
        for macro_name in &macros {
            if text.contains(macro_name.as_str()) {
                hits.push(format!("{name} uses {macro_name}"));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "the opt-in mode must not print; found:\n  {}",
        hits.join("\n  ")
    );
}

/// The token is read out of its wrapper in exactly one place.
///
/// `Secret::expose_for_one_request` is named to be greppable, and this is the grep. One
/// call in the client and one in each of the two tests that check the wrapper works: a
/// fourth call site is a decision somebody should have to argue for in review.
#[test]
fn the_token_is_exposed_in_one_place_in_the_shipping_code() {
    let method = format!("{}{}", "expose_for", "_one_request");
    let mut call_sites = Vec::new();
    for path in workspace_sources() {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // The definition itself, and the tests, are not call sites in shipping code.
        if name == "secret.rs" || name == "tests.rs" {
            continue;
        }
        let uses = text.matches(method.as_str()).count();
        if uses > 0 {
            call_sites.push(format!("{name} ({uses})"));
        }
    }
    assert_eq!(
        call_sites,
        vec!["client.rs (1)".to_owned()],
        "the token should leave its wrapper once, in the one place that builds the header"
    );
}

/// Nothing derives a time from the machine's time zone.
///
/// The contract stores UTC and the state model counts down between two instants, so a
/// countdown is the same number wherever it is computed. `state/tests.rs` proves the
/// arithmetic; this proves there is no second path — nothing reads `TZ`, and nothing calls a
/// local-time conversion — because a property test cannot see a dependency that has not been
/// written yet. Rendering local time is the panel's job, in JavaScript, where it is one call.
#[test]
fn nothing_in_the_workspace_asks_the_machine_what_time_zone_it_is_in() {
    // Assembled at run time so this file does not match itself.
    let needles = [
        format!("\"{}\"", "TZ"),
        format!("{}{}", "local", "time"),
        format!("{}{}", "Local", "time"),
        format!("{}{}", "to_", "local"),
        format!("{}{}", "utc_", "offset"),
    ];

    let mut hits = Vec::new();
    for path in workspace_sources() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for needle in &needles {
            if text.contains(needle.as_str()) {
                hits.push(format!(
                    "{} mentions {needle:?}",
                    path.file_name().unwrap().to_string_lossy()
                ));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "reset arithmetic happens on UTC instants and nowhere else; these files say \
         otherwise:\n  {}",
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
        assert_carries_no_identity(&path);
    }
}

/// The Claude fixtures have to clear the same bar, and one more.
///
/// A status-line payload is *made* of paths — `cwd`, `transcript_path`, the workspace and
/// its repository — so its fixture is the one most likely to carry a machine out of the
/// house. The settings fixtures matter for the same reason: one of them is the shape of
/// the maintainer's own file, and only the shape is allowed to travel.
#[test]
fn the_claude_fixtures_carry_nothing_that_identifies_a_machine() {
    let root = repo_root();
    let fixtures = files_under(&root.join("fixtures").join("claude"), &["json"]);
    assert!(
        fixtures.len() >= 6,
        "expected the six Claude fixtures, found {}",
        fixtures.len()
    );

    for path in &fixtures {
        assert_carries_no_identity(path);
    }

    // The payload fixtures must be payloads, and the settings fixtures settings: a
    // fixture that quietly stops being what its name says stops testing what it claims.
    let mut payloads = 0;
    let mut settings = 0;
    for path in &fixtures {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(path).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|_| panic!("{name} is not valid JSON"));
        assert!(value.is_object(), "{name} is not a JSON object");
        assert!(text.ends_with('\n'), "{name} has no trailing newline");

        if name.starts_with("statusline-payload") {
            payloads += 1;
            assert!(value.get("session_id").is_some(), "{name} is not a payload");
            // No sign-in material and no account identity — the claim the whole design
            // rests on, checked against the file rather than asserted in a README.
            // Token *counts* are fine and are all over this payload; the needles below
            // are assembled at run time so the shape of the check cannot match itself.
            let forbidden: Vec<String> = vec![
                format!("{}{}", "access", "_token"),
                format!("{}{}", "refresh", "_token"),
                format!("{}{}", "id", "_token"),
                format!("{}{}", "api", "_key"),
                format!("{}{}", "Bearer", " "),
                format!("{}{}", "o", "auth"),
                format!("{}{}", "account", "_id"),
                format!("{}{}", "e-", "mail"),
            ];
            let lowered = text.to_lowercase();
            for needle in &forbidden {
                assert!(
                    !lowered.contains(&needle.to_lowercase()),
                    "{name} mentions {needle:?}"
                );
            }
        } else if name.starts_with("settings-") {
            settings += 1;
        }
    }
    assert_eq!(payloads, 3, "three payload shapes are pinned");
    assert_eq!(settings, 3, "three settings shapes are pinned");
}

/// The usage fixtures are transcripts, and a transcript is made of the things this gate
/// looks for.
///
/// Which is why not one line of them was copied off a machine: every shape in them was
/// observed, and then written out again with nothing in it. The gate is what keeps that
/// true when the next one is added — pasting a real line in to make a test pass would fail
/// here rather than ship.
#[test]
fn the_usage_fixtures_carry_nothing_that_identifies_a_machine() {
    let root = repo_root()
        .join("crates")
        .join("nazar-core")
        .join("fixtures")
        .join("usage");
    // `json` as well as `jsonl` since T-WP22: `stats-cache.json` is a whole document rather
    // than a stream of lines, and it is the one usage fixture that stands for a file Claude
    // Code keeps about the machine itself — sessions, message counts, hours of the day. All
    // the more reason for it to be a shape with nothing in it.
    let fixtures = files_under(&root, &["jsonl", "json"]);
    assert!(
        fixtures.len() >= 9,
        "expected the usage fixtures — four transcripts, four rollouts and a statistics \
         cache — found {}",
        fixtures.len()
    );

    for path in &fixtures {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert_carries_no_identity(path);

        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.ends_with('\n'), "{name} has no trailing newline");
        // A fixture is a shape, not a recording. Anything this size stopped being one.
        assert!(
            text.len() < 64 * 1024,
            "{name} is too big to be a shape rather than a recording"
        );
    }
}

/// The captured fixtures are real files off a running installation, and the bar is higher.
///
/// `crates/nazar-core/fixtures/captured/` holds `~/.nazar` as it stood on one machine on
/// 2026-09-09: five status-line captures and the `limits.json` written from them. Their
/// whole value is that nothing about their *shape* was invented — which means nothing about
/// their shape may be edited to make a test pass either. The only thing that was changed in
/// one is identity, so identity is the only thing this checks, plus the placeholders that
/// prove the sanitising was run at all.
#[test]
fn the_captured_fixtures_carry_nothing_that_identifies_a_machine() {
    let root = repo_root()
        .join("crates")
        .join("nazar-core")
        .join("fixtures")
        .join("captured");
    let fixtures = files_under(&root, &["json"]);
    assert!(
        fixtures.len() >= 6,
        "expected the six captured fixtures, found {}",
        fixtures.len()
    );

    for path in &fixtures {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert_carries_no_identity(path);

        let text = std::fs::read_to_string(path).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|_| panic!("{name} is not valid JSON"));
        assert!(value.is_object(), "{name} is not a JSON object");
        assert!(text.ends_with('\n'), "{name} has no trailing newline");

        // The placeholders the sanitiser writes. A capture added later and committed raw
        // is then a failing test rather than a file nobody looked at twice.
        if name.starts_with("statusline-") {
            assert!(
                text.contains("C:\\\\work\\\\project"),
                "{name} does not carry the placeholder working directory"
            );
            assert!(
                !text.contains("\"repo\"") || text.contains("example-owner"),
                "{name} names a real repository"
            );
            assert!(
                !text.contains("session_name") || text.contains("\"session-a\""),
                "{name} carries a real session name"
            );
        }
    }
}

/// The shared bar: no home path, no e-mail, no long identifier, not the tester's name.
fn assert_carries_no_identity(path: &Path) {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| panic!("cannot read {name}"));

    assert!(!text.contains('~'), "{name} contains a home-relative path");
    for fragment in [
        "C:\\Users\\",
        "C:/Users/",
        // The same path as JSON writes it, with every separator escaped. A status-line
        // payload is made of Windows paths and every one of them arrives in this spelling,
        // so a gate that knew only the unescaped one would have waved them all through.
        "C:\\\\Users\\\\",
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
