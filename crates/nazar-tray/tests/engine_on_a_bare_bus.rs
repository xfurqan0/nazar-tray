//! The engine mode, asked of a real session bus rather than of a function.
//!
//! **A stock GNOME session is, as far as this binary can tell, a session bus with nobody on
//! `org.kde.StatusNotifierWatcher`.** `dbus-run-session` makes exactly that: a private bus
//! daemon with no services on it, torn down when the child exits. So the one question the
//! startup entry depends on — does the *same* binary, launched the way the entry launches it
//! and with no `--headless`, fall into the engine on a desktop that has no tray? — is asked
//! here of the real executable and a real bus, not of `desktop::mode` in a unit test that
//! cannot choose which bus it gets.
//!
//! `--print` is the probe because it asks `desktop::mode` the same question a normal
//! launch asks, says the answer on standard error, and then exits without opening a display
//! — which a CI runner has not got. A normal launch would stop at GTK before it got that far.
//!
//! Linux only, like the question. It needs `dbus-run-session`, which every desktop Linux and
//! the CI image have; a developer machine without it skips with a line saying so, and CI is
//! not allowed to skip.
#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::Command;

/// A home directory nobody lives in, so the run reads no real Codex log, no real capture,
/// no real settings, and cannot write anywhere that matters even if it tried.
struct EmptyHome(PathBuf);

impl EmptyHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nazar-tray-bare-bus-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&path).expect("a temporary home must be creatable");
        EmptyHome(path)
    }
}

impl Drop for EmptyHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_plain_binary_is_the_engine_on_a_session_bus_with_no_tray_host() {
    if Command::new("dbus-run-session")
        .arg("--version")
        .output()
        .is_err()
    {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI must have dbus-run-session: this test is the only place the engine fallback \
             is checked against a real bus"
        );
        eprintln!("skipped: dbus-run-session is not installed here");
        return;
    }

    let home = EmptyHome::new();
    let output = Command::new("dbus-run-session")
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_nazar-tray"))
        .arg("--print")
        .env("HOME", &home.0)
        // Every override that would lead the run back to a real directory.
        .env_remove("NAZAR_HOME")
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("XDG_CONFIG_HOME")
        // dbus-run-session sets its own; one inherited from the developer's desktop would be
        // a bus with a watcher on it, which is the other answer.
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .output()
        .expect("dbus-run-session must start the binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "--print exits 0 whatever the desktop is; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("engine mode: no StatusNotifierWatcher on the session bus"),
        "no tray host on the bus, no --headless on the command line, and the binary must \
         still say it is running as the engine; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("--headless"),
        "the reason is the bus, not a flag nobody passed; stderr:\n{stderr}"
    );

    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("standard output is still the document");
    assert!(document.get("schemaVersion").is_some());
    assert!(
        !home.0.join(".nazar").join("limits.json").exists(),
        "--print writes nothing, on this desktop as on every other"
    );
}
