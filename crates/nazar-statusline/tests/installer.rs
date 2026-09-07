//! `install`, `uninstall` and `status`, exercised as a real process against three real
//! settings shapes.
//!
//! The acceptance criterion for this package is one sentence: *works with no status line,
//! with ccstatusline, and with the maintainer's custom script; uninstall restores the
//! previous command exactly.* The three fixtures are those three shapes, and the
//! round-trip test is that sentence.
//!
//! Every one of these runs against a temporary home directory. The machine's own
//! `~/.claude` is never opened, let alone written; see `tests/common/mod.rs`.

mod common;

use common::{
    SETTINGS_CCSTATUSLINE, SETTINGS_CUSTOM, SETTINGS_NONE, Sandbox, digest, stderr, stdout,
};

/// The three shapes the package has to survive.
fn fixtures() -> [(&'static str, &'static str); 3] {
    [
        ("no status line", SETTINGS_NONE),
        ("ccstatusline", SETTINGS_CCSTATUSLINE),
        ("a custom node script", SETTINGS_CUSTOM),
    ]
}

#[test]
fn install_then_uninstall_returns_all_three_fixtures_to_their_exact_bytes() {
    for (name, original) in fixtures() {
        let sandbox = Sandbox::new("round-trip");
        sandbox.write_settings(original);

        let installed = sandbox.run(&[
            "install",
            "--config-dir",
            &sandbox.config_dir().display().to_string(),
        ]);
        assert!(installed.status.success(), "{name}: {}", stderr(&installed));
        let report = stdout(&installed);
        assert!(
            report.contains("--- a/settings.json"),
            "{name}: no diff was printed\n{report}"
        );
        assert!(report.contains("+++ b/settings.json"), "{name}\n{report}");

        // The file now points at us, and nothing else about it moved.
        let after_install: serde_json::Value =
            serde_json::from_str(&sandbox.read_settings()).unwrap();
        let command = after_install["statusLine"]["command"].as_str().unwrap();
        assert!(
            command.contains("nazar-statusline"),
            "{name}: got {command}"
        );

        let before_keys: Vec<String> = keys(original);
        let after_keys: Vec<String> = keys(&sandbox.read_settings());
        if before_keys.contains(&"statusLine".to_owned()) {
            assert_eq!(before_keys, after_keys, "{name}: the key order moved");
        } else {
            assert_eq!(
                after_keys.last().map(String::as_str),
                Some("statusLine"),
                "{name}: a new key belongs at the end"
            );
        }

        let uninstalled = sandbox.run(&[
            "uninstall",
            "--config-dir",
            &sandbox.config_dir().display().to_string(),
        ]);
        assert!(
            uninstalled.status.success(),
            "{name}: {}",
            stderr(&uninstalled)
        );

        let restored = sandbox.read_settings();
        // Semantic equality is the promise; byte equality is what actually happens, and
        // the test says which so that a change to either is visible rather than assumed.
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&restored).unwrap(),
            serde_json::from_str::<serde_json::Value>(original).unwrap(),
            "{name}: the restored document is not the one we started with"
        );
        assert_eq!(
            restored, original,
            "{name}: the round trip was not byte-for-byte"
        );

        // The backup stays where it is.
        let backups = sandbox.backups();
        assert_eq!(
            backups.len(),
            1,
            "{name}: expected one backup, got {backups:?}"
        );
        assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), original);
    }
}

/// The keys of a JSON object, in file order.
fn keys(text: &str) -> Vec<String> {
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    value
        .as_object()
        .unwrap()
        .keys()
        .map(ToOwned::to_owned)
        .collect()
}

#[test]
fn the_chain_record_holds_the_previous_status_line_exactly() {
    let sandbox = Sandbox::new("chain-record");
    sandbox.write_settings(SETTINGS_CUSTOM);
    let original: serde_json::Value = serde_json::from_str(SETTINGS_CUSTOM).unwrap();

    let output = sandbox.run(&[
        "install",
        "--config-dir",
        &sandbox.config_dir().display().to_string(),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));

    let chain = sandbox.chain();
    assert_eq!(chain["previous"], original["statusLine"]);
    assert_eq!(chain["schemaVersion"], serde_json::json!(1));
    assert!(chain["backupPath"].as_str().unwrap().contains("nazar-bak-"));
    assert!(
        chain["installedCommand"]
            .as_str()
            .unwrap()
            .contains("nazar-statusline")
    );
}

#[test]
fn padding_and_refresh_interval_survive_the_install() {
    let sandbox = Sandbox::new("preserve-knobs");
    sandbox.write_settings(SETTINGS_CUSTOM);
    sandbox.run(&[
        "install",
        "--config-dir",
        &sandbox.config_dir().display().to_string(),
    ]);

    let after: serde_json::Value = serde_json::from_str(&sandbox.read_settings()).unwrap();
    assert_eq!(after["statusLine"]["padding"], serde_json::json!(0));
    assert_eq!(
        after["statusLine"]["refreshInterval"],
        serde_json::json!(30)
    );
    assert_eq!(after["statusLine"]["type"], serde_json::json!("command"));
    // Every other key of the document is untouched.
    let original: serde_json::Value = serde_json::from_str(SETTINGS_CUSTOM).unwrap();
    for (key, value) in original.as_object().unwrap() {
        if key == "statusLine" {
            continue;
        }
        assert_eq!(&after[key], value, "{key} was changed");
    }
}

#[test]
fn installing_twice_changes_nothing_the_second_time() {
    let sandbox = Sandbox::new("idempotent");
    sandbox.write_settings(SETTINGS_CCSTATUSLINE);
    let config = sandbox.config_dir().display().to_string();

    sandbox.run(&["install", "--config-dir", &config]);
    let after_first = digest(&sandbox.settings());
    let chain_after_first = sandbox.chain();

    let second = sandbox.run(&["install", "--config-dir", &config]);
    assert!(second.status.success());
    assert!(
        stdout(&second).contains("Already installed"),
        "{}",
        stdout(&second)
    );
    assert_eq!(
        digest(&sandbox.settings()),
        after_first,
        "the second install wrote"
    );
    assert_eq!(
        sandbox.chain(),
        chain_after_first,
        "the second install rewrote chain.json"
    );
    assert_eq!(
        sandbox.backups().len(),
        1,
        "the second install must not take a second backup"
    );
    // And the record still points at the user's real status line, not at us.
    assert_eq!(
        chain_after_first["previous"]["command"],
        serde_json::json!("npx ccstatusline@latest")
    );
}

#[test]
fn a_settings_file_that_is_not_valid_json_is_reported_and_left_alone() {
    let sandbox = Sandbox::new("broken-json");
    let broken = "{\n  \"model\": \"x\",\n  oops\n}\n";
    sandbox.write_settings(broken);
    let before = digest(&sandbox.settings());

    let output = sandbox.run(&[
        "install",
        "--config-dir",
        &sandbox.config_dir().display().to_string(),
    ]);

    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(message.contains("not valid JSON"), "{message}");
    assert!(message.contains("Nothing was changed"), "{message}");
    assert_eq!(
        digest(&sandbox.settings()),
        before,
        "the broken file was written to"
    );
    assert!(
        sandbox.backups().is_empty(),
        "a refused install left a backup behind"
    );
    assert!(!sandbox.captures().join("chain.json").exists());
}

#[test]
fn a_lock_file_stops_the_installer_before_it_reads_anything() {
    let sandbox = Sandbox::new("locked");
    sandbox.write_settings(SETTINGS_CUSTOM);
    std::fs::write(sandbox.config_dir().join("settings.json.lock"), "").unwrap();
    let before = digest(&sandbox.settings());

    let output = sandbox.run(&[
        "install",
        "--config-dir",
        &sandbox.config_dir().display().to_string(),
    ]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("settings.json.lock"),
        "{}",
        stderr(&output)
    );
    assert_eq!(digest(&sandbox.settings()), before);
}

#[test]
fn a_dry_run_writes_absolutely_nothing() {
    let sandbox = Sandbox::new("dry-run");
    sandbox.write_settings(SETTINGS_CUSTOM);
    let before = digest(&sandbox.settings());
    let config = sandbox.config_dir().display().to_string();

    let output = sandbox.run(&["install", "--dry-run", "--config-dir", &config]);

    assert!(output.status.success(), "{}", stderr(&output));
    let report = stdout(&output);
    assert!(report.contains("Would change"), "{report}");
    assert!(report.contains("--- a/settings.json"), "{report}");
    assert!(report.contains("nazar-statusline"), "{report}");
    assert!(
        report.contains("--dry-run: nothing was written"),
        "{report}"
    );

    assert_eq!(
        digest(&sandbox.settings()),
        before,
        "the dry run wrote the settings file"
    );
    assert!(sandbox.backups().is_empty(), "the dry run took a backup");
    assert!(
        !sandbox.captures().join("chain.json").exists(),
        "the dry run wrote chain.json"
    );

    // And the diff a dry run shows is the diff a real install produces.
    let real = sandbox.run(&["install", "--config-dir", &config]);
    let hunk = |text: &str| {
        text.lines()
            .skip_while(|line| !line.starts_with("--- a/"))
            .take_while(|line| {
                !line.starts_with("--dry-run")
                    && !line.starts_with("Backed up")
                    && !line.starts_with("No settings file")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(hunk(&report), hunk(&stdout(&real)));
}

#[test]
fn a_second_install_after_an_uninstall_never_overwrites_the_first_backup() {
    let sandbox = Sandbox::new("two-backups");
    sandbox.write_settings(SETTINGS_CUSTOM);
    let config = sandbox.config_dir().display().to_string();

    sandbox.run(&["install", "--config-dir", &config]);
    sandbox.run(&["uninstall", "--config-dir", &config]);
    sandbox.run(&["install", "--config-dir", &config]);

    let backups = sandbox.backups();
    assert_eq!(backups.len(), 2, "got {backups:?}");
    for backup in &backups {
        assert_eq!(
            std::fs::read_to_string(backup).unwrap(),
            SETTINGS_CUSTOM,
            "a backup was written over"
        );
    }
}

#[test]
fn installing_where_there_is_no_settings_file_creates_one_and_uninstall_empties_it() {
    let sandbox = Sandbox::new("no-file");
    let config = sandbox.config_dir().display().to_string();
    assert!(!sandbox.settings().exists());

    let installed = sandbox.run(&["install", "--config-dir", &config]);
    assert!(installed.status.success(), "{}", stderr(&installed));
    assert!(
        stdout(&installed).contains("nothing to back up"),
        "{}",
        stdout(&installed)
    );

    let after: serde_json::Value = serde_json::from_str(&sandbox.read_settings()).unwrap();
    assert_eq!(after.as_object().unwrap().len(), 1);
    assert!(
        after["statusLine"]["command"]
            .as_str()
            .unwrap()
            .contains("nazar-statusline")
    );

    let uninstalled = sandbox.run(&["uninstall", "--config-dir", &config]);
    assert!(uninstalled.status.success(), "{}", stderr(&uninstalled));
    assert_eq!(sandbox.read_settings(), "{}\n", "the key is gone again");
}

#[test]
fn uninstalling_when_nothing_is_installed_is_a_no_op() {
    let sandbox = Sandbox::new("uninstall-noop");
    sandbox.write_settings(SETTINGS_CCSTATUSLINE);
    let before = digest(&sandbox.settings());

    let output = sandbox.run(&[
        "uninstall",
        "--config-dir",
        &sandbox.config_dir().display().to_string(),
    ]);

    assert!(output.status.success());
    assert!(
        stdout(&output).contains("not installed"),
        "{}",
        stdout(&output)
    );
    assert_eq!(digest(&sandbox.settings()), before);
}

#[test]
fn uninstalling_without_the_chain_record_refuses_rather_than_guesses() {
    let sandbox = Sandbox::new("uninstall-no-chain");
    sandbox.write_settings(SETTINGS_CUSTOM);
    let config = sandbox.config_dir().display().to_string();
    sandbox.run(&["install", "--config-dir", &config]);

    std::fs::remove_file(sandbox.captures().join("chain.json")).unwrap();
    let before = digest(&sandbox.settings());

    let output = sandbox.run(&["uninstall", "--config-dir", &config]);

    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(
        message.contains("nazar-bak-"),
        "the way out must be named: {message}"
    );
    assert_eq!(digest(&sandbox.settings()), before);
}

#[test]
fn uninstalling_when_the_record_and_the_backup_disagree_refuses() {
    let sandbox = Sandbox::new("uninstall-mismatch");
    sandbox.write_settings(SETTINGS_CUSTOM);
    let config = sandbox.config_dir().display().to_string();
    sandbox.run(&["install", "--config-dir", &config]);

    // Somebody edited the record. The backup is the second witness and disagrees.
    let path = sandbox.captures().join("chain.json");
    let mut record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    record["previous"]["command"] = serde_json::json!("something-else-entirely");
    std::fs::write(&path, serde_json::to_string_pretty(&record).unwrap()).unwrap();
    let before = digest(&sandbox.settings());

    let output = sandbox.run(&["uninstall", "--config-dir", &config]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("does not match"),
        "{}",
        stderr(&output)
    );
    assert_eq!(digest(&sandbox.settings()), before);
}

#[test]
fn an_uninstall_dry_run_writes_nothing_either() {
    let sandbox = Sandbox::new("uninstall-dry-run");
    sandbox.write_settings(SETTINGS_CUSTOM);
    let config = sandbox.config_dir().display().to_string();
    sandbox.run(&["install", "--config-dir", &config]);
    let before = digest(&sandbox.settings());

    let output = sandbox.run(&["uninstall", "--dry-run", "--config-dir", &config]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("--dry-run: nothing was written"));
    assert_eq!(digest(&sandbox.settings()), before);
}

#[test]
fn status_says_what_is_installed_what_it_chains_to_and_how_old_the_captures_are() {
    let sandbox = Sandbox::new("status");
    sandbox.write_settings(SETTINGS_CUSTOM);
    let config = sandbox.config_dir().display().to_string();

    let before = stdout(&sandbox.run(&["status", "--config-dir", &config]));
    assert!(before.contains("installed: no"), "{before}");
    assert!(
        before.contains("never installed on this machine"),
        "{before}"
    );
    assert!(before.contains("0 files"), "{before}");

    sandbox.run(&["install", "--config-dir", &config]);
    sandbox.run_with_stdin(&[], common::PAYLOAD_BOTH.as_bytes());

    let after = stdout(&sandbox.run(&["status", "--config-dir", &config]));
    assert!(after.contains("installed: yes"), "{after}");
    assert!(after.contains("nazar-statusline"), "{after}");
    assert!(
        after.contains("statusline.js"),
        "the chain target is named: {after}"
    );
    assert!(after.contains("nazar-bak-"), "{after}");
    assert!(after.contains("1 file, newest"), "{after}");
}

#[test]
fn a_status_line_of_a_type_we_do_not_know_is_preserved_and_not_run() {
    let sandbox = Sandbox::new("unknown-type");
    let exotic =
        "{\n  \"statusLine\": {\n    \"type\": \"somethingNew\",\n    \"widget\": \"x\"\n  }\n}\n";
    sandbox.write_settings(exotic);
    let config = sandbox.config_dir().display().to_string();

    sandbox.run(&["install", "--config-dir", &config]);
    assert_eq!(
        sandbox.chain()["previous"]["type"],
        serde_json::json!("somethingNew")
    );

    // The wrapper has nothing runnable to chain to, so it prints its own line.
    let output = sandbox.run_with_stdin(&[], common::PAYLOAD_BOTH.as_bytes());
    assert!(output.status.success());
    assert!(stdout(&output).contains("Fable 5.1"), "{}", stdout(&output));

    sandbox.run(&["uninstall", "--config-dir", &config]);
    assert_eq!(
        sandbox.read_settings(),
        exotic,
        "the exotic shape came back exactly"
    );
}

#[test]
fn the_installed_command_is_an_absolute_path_without_the_verbatim_prefix() {
    let sandbox = Sandbox::new("absolute-path");
    sandbox.write_settings(SETTINGS_NONE);
    sandbox.run(&[
        "install",
        "--config-dir",
        &sandbox.config_dir().display().to_string(),
    ]);

    let after: serde_json::Value = serde_json::from_str(&sandbox.read_settings()).unwrap();
    let command = after["statusLine"]["command"].as_str().unwrap();
    assert!(!command.starts_with(r"\\?\"), "got {command}");
    let path = command.trim_matches('"');
    assert!(
        std::path::Path::new(path).is_absolute(),
        "the settings file must name a path that works from any directory, got {command}"
    );
    assert!(std::path::Path::new(path).exists(), "got {command}");
}
