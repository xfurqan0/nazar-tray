//! The wrapper end of `nazar-statusline`, exercised as a real process.
//!
//! Everything here runs the built binary with a temporary home directory. Nothing reads
//! or writes the machine's own `~/.claude` or `~/.nazar`; see `tests/common/mod.rs`.

mod common;

use std::time::Instant;

use common::{PAYLOAD, PAYLOAD_BOTH, Sandbox, chained_script, stderr, stdout};

/// Put a `chain.json` in place without going through the installer.
fn plant_chain(sandbox: &Sandbox, previous: Option<serde_json::Value>) {
    let mut record = serde_json::json!({
        "schemaVersion": 1,
        "installedAt": "2026-09-07T06:40:00Z",
        "settingsPath": sandbox.settings().display().to_string(),
        "installedCommand": "nazar-statusline",
    });
    if let Some(previous) = previous {
        record["previous"] = previous;
    }
    std::fs::create_dir_all(sandbox.captures()).unwrap();
    std::fs::write(
        sandbox.captures().join("chain.json"),
        serde_json::to_string_pretty(&record).unwrap(),
    )
    .unwrap();
}

#[test]
fn the_payload_is_captured_whole_under_its_session_id() {
    let sandbox = Sandbox::new("wrapper-capture");
    let output = sandbox.run_with_stdin(&[], PAYLOAD_BOTH.as_bytes());
    assert!(output.status.success(), "{}", stderr(&output));

    let capture = sandbox
        .captures()
        .join("00000000-0000-4000-8000-000000000001.json");
    assert!(capture.exists(), "the capture is keyed by session id");

    let envelope: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&capture).unwrap()).unwrap();
    let original: serde_json::Value = serde_json::from_str(PAYLOAD_BOTH).unwrap();
    assert_eq!(
        envelope["payload"], original,
        "the whole payload is captured, not a selection of it"
    );
    assert_eq!(envelope["schemaVersion"], serde_json::json!(1));
    assert!(envelope["updatedAt"].as_str().unwrap().ends_with('Z'));
}

#[test]
fn three_sessions_do_not_overwrite_each_other() {
    let sandbox = Sandbox::new("wrapper-sessions");
    for session in ["session-a", "session-b", "session-c"] {
        let payload = PAYLOAD_BOTH.replace("00000000-0000-4000-8000-000000000001", session);
        let output = sandbox.run_with_stdin(&[], payload.as_bytes());
        assert!(output.status.success(), "{}", stderr(&output));
    }
    for session in ["session-a", "session-b", "session-c"] {
        assert!(
            sandbox.captures().join(format!("{session}.json")).exists(),
            "{session} lost its capture to another session"
        );
    }
}

#[test]
fn with_no_chain_a_minimal_line_is_printed() {
    let sandbox = Sandbox::new("wrapper-default-line");
    let output = sandbox.run_with_stdin(&[], PAYLOAD_BOTH.as_bytes());

    assert!(output.status.success());
    assert_eq!(
        stdout(&output).trim_end(),
        "Fable 5.1 · high · 5h 12% · 7d 31%"
    );
}

#[test]
fn a_recorded_status_line_of_none_still_prints_the_minimal_line() {
    let sandbox = Sandbox::new("wrapper-chain-none");
    plant_chain(&sandbox, None);
    let output = sandbox.run_with_stdin(&[], PAYLOAD.as_bytes());

    assert!(output.status.success());
    assert_eq!(stdout(&output).trim_end(), "Fable 5.1 · high · 7d 31%");
}

#[test]
fn the_chained_command_gets_the_same_bytes_and_owns_the_output_and_the_exit_code() {
    let sandbox = Sandbox::new("wrapper-chain");
    let record = sandbox.root.join("what-the-chain-read.txt");
    let command = chained_script(&sandbox.root.join("scripts"), &record, 7);
    plant_chain(
        &sandbox,
        Some(serde_json::json!({"type": "command", "command": command})),
    );

    let output = sandbox.run_with_stdin(&[], PAYLOAD.as_bytes());

    assert_eq!(
        output.status.code(),
        Some(7),
        "the chained command's exit code is this process's exit code"
    );
    assert_eq!(
        stdout(&output),
        "CHAINED-STDOUT",
        "stdout is forwarded verbatim"
    );
    assert_eq!(
        stderr(&output),
        "CHAINED-STDERR",
        "stderr is forwarded verbatim"
    );
    assert_eq!(
        std::fs::read_to_string(&record).unwrap(),
        PAYLOAD,
        "the chained command must see the payload Claude Code sent, byte for byte"
    );
    // And the capture happened anyway.
    assert!(
        sandbox
            .captures()
            .join("00000000-0000-4000-8000-000000000001.json")
            .exists()
    );
}

#[test]
fn a_chained_command_whose_path_contains_a_space_still_runs() {
    // The shape that breaks a wrapper which hands a command line to the wrong shell, and
    // the shape the maintainer's own machine has: an interpreter, then a quoted path.
    let sandbox = Sandbox::new("wrapper-chain-space");
    let record = sandbox.root.join("what-the-chain-read.txt");
    let command = chained_script(&sandbox.root.join("a directory with spaces"), &record, 0);
    plant_chain(
        &sandbox,
        Some(serde_json::json!({"type": "command", "command": command})),
    );

    let output = sandbox.run_with_stdin(&[], PAYLOAD.as_bytes());

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "CHAINED-STDOUT");
    assert_eq!(std::fs::read_to_string(&record).unwrap(), PAYLOAD);
}

#[test]
fn empty_input_writes_nothing_chains_anyway_and_succeeds() {
    let sandbox = Sandbox::new("wrapper-empty");
    let record = sandbox.root.join("what-the-chain-read.txt");
    let command = chained_script(&sandbox.root.join("scripts"), &record, 0);
    plant_chain(
        &sandbox,
        Some(serde_json::json!({"type": "command", "command": command})),
    );

    let output = sandbox.run_with_stdin(&[], b"");

    assert!(output.status.success());
    assert_eq!(stdout(&output), "CHAINED-STDOUT");
    assert_eq!(std::fs::read_to_string(&record).unwrap(), "");

    let captures: Vec<_> = std::fs::read_dir(sandbox.captures())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "chain.json")
        .collect();
    assert!(
        captures.is_empty(),
        "nothing to capture, so nothing captured: {captures:?}"
    );
}

#[test]
fn input_that_is_not_json_does_not_break_the_status_line() {
    let sandbox = Sandbox::new("wrapper-garbage");
    let record = sandbox.root.join("what-the-chain-read.txt");
    let command = chained_script(&sandbox.root.join("scripts"), &record, 0);
    plant_chain(
        &sandbox,
        Some(serde_json::json!({"type": "command", "command": command})),
    );

    let output = sandbox.run_with_stdin(&[], b"this is not JSON at all\n");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "CHAINED-STDOUT");
    assert_eq!(
        std::fs::read_to_string(&record).unwrap(),
        "this is not JSON at all\n",
        "even unparseable input is handed on unchanged"
    );
}

#[test]
fn a_chained_command_that_does_not_exist_is_reported_rather_than_hidden() {
    let sandbox = Sandbox::new("wrapper-chain-missing");
    plant_chain(
        &sandbox,
        Some(serde_json::json!({
            "type": "command",
            "command": "nazar-no-such-program-anywhere --please"
        })),
    );

    let output = sandbox.run_with_stdin(&[], PAYLOAD.as_bytes());
    assert_ne!(
        output.status.code(),
        Some(0),
        "a broken chain is not a success"
    );
    // The capture still happened: the wrapper's own job is done before the chain runs.
    assert!(
        sandbox
            .captures()
            .join("00000000-0000-4000-8000-000000000001.json")
            .exists()
    );
}

/// The budget, measured rather than asserted from a design document.
///
/// The bound checked here is deliberately loose: this is a debug build, on whatever
/// machine happens to be running the tests, and a test that fails when a laptop is busy
/// teaches people to ignore it. The number that matters is printed — run with
/// `cargo test -p nazar-statusline --release -- --nocapture` to see the shipped one.
#[test]
fn the_capture_path_stays_inside_its_time_budget() {
    let sandbox = Sandbox::new("wrapper-timing");

    // One warm-up run so the first measurement is not the binary being paged in.
    sandbox.run_with_stdin(&[], PAYLOAD_BOTH.as_bytes());

    let mut timings = Vec::with_capacity(10);
    for _ in 0..10 {
        let started = Instant::now();
        let output = sandbox.run_with_stdin(&[], PAYLOAD_BOTH.as_bytes());
        timings.push(started.elapsed());
        assert!(output.status.success());
    }
    timings.sort();

    let median = timings[timings.len() / 2];
    println!(
        "nazar-statusline, no chained command, 10 runs: min {:?}, median {:?}, max {:?}",
        timings[0],
        median,
        timings[timings.len() - 1]
    );
    assert!(
        median.as_millis() < 400,
        "the wrapper is far outside any plausible budget: median {median:?}"
    );
}

#[test]
fn help_and_version_do_not_read_standard_input() {
    let sandbox = Sandbox::new("wrapper-help");
    let help = sandbox.run(&["--help"]);
    assert!(help.status.success());
    assert!(stdout(&help).contains("nazar-statusline"));

    let version = sandbox.run(&["--version"]);
    assert!(version.status.success());
    assert!(stdout(&version).starts_with("nazar-statusline "));

    let unknown = sandbox.run(&["frobnicate"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(stderr(&unknown).contains("unknown command"));
}
