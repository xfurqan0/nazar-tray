//! Shared scaffolding for the integration tests.
//!
//! Every helper here exists to make one guarantee true: **no test touches the machine it
//! runs on**. A child process gets a home directory inside a temporary folder, a
//! `CLAUDE_CONFIG_DIR` inside it, and a `NAZAR_HOME` inside it, so even a test that forgot
//! to pass `--config-dir` cannot reach the real `~/.claude` or the real `~/.nazar`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub use nazar_statusline::testutil::TempDir;

/// The three payload fixtures, as text.
pub const PAYLOAD: &str = include_str!("../../../../fixtures/claude/statusline-payload.json");
pub const PAYLOAD_BOTH: &str =
    include_str!("../../../../fixtures/claude/statusline-payload-both-windows.json");

/// The three settings fixtures the acceptance criteria name.
pub const SETTINGS_NONE: &str =
    include_str!("../../../../fixtures/claude/settings-no-statusline.json");
pub const SETTINGS_CCSTATUSLINE: &str =
    include_str!("../../../../fixtures/claude/settings-ccstatusline.json");
pub const SETTINGS_CUSTOM: &str =
    include_str!("../../../../fixtures/claude/settings-custom-node.json");

/// A sandbox: a home directory, a Claude config directory and a `~/.nazar`, all temporary.
pub struct Sandbox {
    pub root: TempDir,
}

impl Sandbox {
    pub fn new(label: &str) -> Self {
        let root = TempDir::new(label);
        let sandbox = Sandbox { root };
        std::fs::create_dir_all(sandbox.config_dir()).unwrap();
        std::fs::create_dir_all(sandbox.nazar_home()).unwrap();
        sandbox
    }

    /// The fake `~/.claude`.
    pub fn config_dir(&self) -> PathBuf {
        self.root.join("claude")
    }

    /// The fake `~/.nazar`.
    pub fn nazar_home(&self) -> PathBuf {
        self.root.join("nazar")
    }

    /// The fake `~/.nazar/statusline`.
    pub fn captures(&self) -> PathBuf {
        self.nazar_home().join("statusline")
    }

    /// The fake `~/.claude/settings.json`.
    pub fn settings(&self) -> PathBuf {
        self.config_dir().join("settings.json")
    }

    /// Put a settings fixture in place.
    pub fn write_settings(&self, text: &str) {
        std::fs::write(self.settings(), text).unwrap();
    }

    /// The settings file as it is now.
    pub fn read_settings(&self) -> String {
        std::fs::read_to_string(self.settings()).unwrap()
    }

    /// Every backup the installer has left behind, sorted.
    pub fn backups(&self) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(self.config_dir())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(nazar_statusline::install::BACKUP_PREFIX))
            })
            .collect();
        found.sort();
        found
    }

    /// The chain record the installer wrote.
    pub fn chain(&self) -> serde_json::Value {
        let text = std::fs::read_to_string(self.captures().join("chain.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    /// Run the binary with the sandbox's environment and no standard input.
    pub fn run(&self, arguments: &[&str]) -> Output {
        self.run_with_stdin(arguments, b"")
    }

    /// Run the binary with the sandbox's environment and `stdin` on standard input.
    pub fn run_with_stdin(&self, arguments: &[&str], stdin: &[u8]) -> Output {
        let mut child = self
            .command()
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the binary must start");
        {
            use std::io::Write;
            let mut handle = child.stdin.take().unwrap();
            handle.write_all(stdin).unwrap();
        }
        child.wait_with_output().expect("the binary must finish")
    }

    /// A command pointed at the binary, with the sandbox's environment already set.
    pub fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nazar-statusline"));
        // Belt and braces: even a test that forgets `--config-dir` lands inside the
        // sandbox, because the home directory itself is inside the sandbox.
        command.env("HOME", &self.root.path);
        command.env("USERPROFILE", &self.root.path);
        command.env("CLAUDE_CONFIG_DIR", self.config_dir());
        command.env("NAZAR_HOME", self.nazar_home());
        command
    }
}

/// Standard output of a finished process, as text.
pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Standard error of a finished process, as text.
pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A stable digest of a file, for "this was not written to" assertions.
///
/// FNV-1a over the bytes: not a security hash, and it does not need to be. It exists so a
/// test can say "these bytes are the same bytes" without printing a settings file into the
/// test output when it fails.
pub fn digest(path: &Path) -> u64 {
    let bytes = std::fs::read(path).unwrap();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

/// Write a script that copies its standard input to `record`, prints markers on both
/// output streams and exits with `exit_code`, and return the command line that runs it.
///
/// This stands in for "the status line the user already had". It is a script file rather
/// than a one-liner because the shape that matters — the maintainer's own — is an
/// interpreter followed by a **quoted absolute path**, which is exactly the case that
/// breaks when a wrapper hands a command line to the wrong shell.
pub fn chained_script(directory: &Path, record: &Path, exit_code: i32) -> String {
    std::fs::create_dir_all(directory).unwrap();

    #[cfg(windows)]
    {
        let script = directory.join("previous-status-line.ps1");
        std::fs::write(
            &script,
            format!(
                "$text = [Console]::In.ReadToEnd()\n\
                 [IO.File]::WriteAllText('{}', $text)\n\
                 [Console]::Out.Write('CHAINED-STDOUT')\n\
                 [Console]::Error.Write('CHAINED-STDERR')\n\
                 exit {exit_code}\n",
                record.display().to_string().replace('\'', "''")
            ),
        )
        .unwrap();
        format!(
            "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{}\"",
            script.display()
        )
    }

    #[cfg(not(windows))]
    {
        let script = directory.join("previous-status-line.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncat > \"{}\"\nprintf 'CHAINED-STDOUT'\nprintf 'CHAINED-STDERR' >&2\nexit {exit_code}\n",
                record.display()
            ),
        )
        .unwrap();
        format!("sh \"{}\"", script.display())
    }
}
