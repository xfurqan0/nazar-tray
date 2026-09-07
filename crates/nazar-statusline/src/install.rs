//! Putting the wrapper into `settings.json`, and taking it back out.
//!
//! The order of operations is the whole design, because the failure everybody else in
//! this niche has is the same one: they write the user's settings file first and think
//! about the consequences afterwards. Here nothing is written until every reason to stop
//! has been checked, and what is written is written in the order that leaves the least
//! damage if the machine dies half way.
//!
//! ```text
//!   resolve directory ─▶ refuse on a lock file
//!                     ─▶ parse settings.json      (invalid JSON: report, touch nothing)
//!                     ─▶ already installed?       (yes: say so, touch nothing)
//!                     ─▶ build the new document   (only `statusLine` differs)
//!                     ─▶ print the diff           (always, terminal or not)
//!   ── --dry-run stops here, having written nothing ──
//!                     ─▶ copy settings.json to settings.json.nazar-bak-<stamp>
//!                     ─▶ write chain.json         (what we are about to replace)
//!                     ─▶ write settings.json      (atomically)
//! ```
//!
//! `chain.json` is written **before** the settings file on purpose. A crash between the
//! two leaves a record of a status line that was never replaced, which is inert. The
//! other order would leave this program installed with no record of what it displaced,
//! which is the one outcome that cannot be undone from the files on disk.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::chain::{self, CHAIN_SCHEMA_VERSION, Chain};
use crate::diff;
use crate::fail::{Failure, Result};
use crate::settings::{self, SETTINGS_FILE, Settings};

/// Prefix of the untouched copy taken before an edit.
pub const BACKUP_PREFIX: &str = "settings.json.nazar-bak-";

/// How the three subcommands were asked to behave.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// `--config-dir`: where `settings.json` lives. `CLAUDE_CONFIG_DIR`, then `~/.claude`.
    pub config_dir: Option<PathBuf>,
    /// `--dry-run`: print the diff, write nothing at all.
    pub dry_run: bool,
    /// The command to install. Defaults to this executable's own absolute path; tests
    /// set it so that they never depend on where the test runner happens to live.
    pub program: Option<String>,
}

/// Install the wrapper. Returns the process exit code.
pub fn install(options: &Options, out: &mut dyn Write) -> Result<i32> {
    let directory = settings::config_dir(options.config_dir.as_deref())?;
    refuse_on_lock(&directory)?;

    let path = settings::settings_path(&directory);
    let current = Settings::load(&path)?;

    if let Some(command) = current.status_line_command() {
        if settings::is_our_command(command) {
            let _ = writeln!(out, "Already installed. Nothing to do.");
            let _ = writeln!(out, "  settings:  {}", path.display());
            let _ = writeln!(out, "  command:   {command}");
            if let Some(previous) = chain::load()?.as_ref().and_then(Chain::command) {
                let _ = writeln!(out, "  chains to: {previous}");
            }
            return Ok(0);
        }
    }

    let program = settings::quote_program(&program_path(options)?);
    let previous = current.status_line().cloned();
    let patched =
        current.with_status_line(Some(settings::status_line_for(&program, previous.as_ref())));

    let before = current.before_text();
    let after = patched.render();
    print_plan(out, &path, &before, &after, options.dry_run);

    if options.dry_run {
        let _ = writeln!(
            out,
            "--dry-run: nothing was written. {} is unchanged.",
            path.display()
        );
        return Ok(0);
    }

    let backup = if current.exists() {
        let backup = write_backup(&path, before.as_bytes())?;
        let _ = writeln!(out, "Backed up to {}", backup.display());
        Some(backup)
    } else {
        let _ = writeln!(
            out,
            "No settings file existed; a new one was created, so there is nothing to back up."
        );
        None
    };

    let record = Chain {
        schema_version: CHAIN_SCHEMA_VERSION,
        installed_at: nazar_core::now_rfc3339(),
        settings_path: path.display().to_string(),
        backup_path: backup.as_ref().map(|path| path.display().to_string()),
        installed_command: program.clone(),
        previous,
    };
    let record_path = chain::store(&record)?;
    let _ = writeln!(
        out,
        "Recorded the previous status line in {}",
        record_path.display()
    );

    nazar_core::atomic::write_bytes(&path, after.as_bytes())?;
    let _ = writeln!(out, "Installed. {} now runs {program}.", SETTINGS_FILE);
    match record.command() {
        Some(command) => {
            let _ = writeln!(out, "Your own status line still runs after it: {command}");
        }
        None => {
            let _ = writeln!(
                out,
                "You had no status line before, so a minimal one is printed instead."
            );
        }
    }
    Ok(0)
}

/// Take the wrapper back out, restoring exactly what was there.
pub fn uninstall(options: &Options, out: &mut dyn Write) -> Result<i32> {
    let directory = settings::config_dir(options.config_dir.as_deref())?;
    refuse_on_lock(&directory)?;

    let path = settings::settings_path(&directory);
    let current = Settings::load(&path)?;

    let installed = current
        .status_line_command()
        .is_some_and(settings::is_our_command);
    if !installed {
        let _ = writeln!(
            out,
            "nazar-statusline is not installed in {}. Nothing to do.",
            path.display()
        );
        return Ok(0);
    }

    let Some(record) = chain::load()? else {
        return Err(Failure::new(format!(
            "{} says nazar-statusline is installed, but {} is missing, so the status line \
             it replaced is not known. Restore it by hand from one of the {BACKUP_PREFIX}* \
             copies beside the settings file, or remove the statusLine key.",
            path.display(),
            chain::chain_path()?.display()
        )));
    };

    let patched = current.with_status_line(record.previous.clone());

    // The backup is the second witness. chain.json is derived from it, so the two can
    // only disagree if something edited one of them, and that is a state to report rather
    // than to write through.
    if let Some(backup) = &record.backup_path {
        match Settings::load(Path::new(backup)) {
            Ok(backup_settings) => {
                if backup_settings.status_line() != patched.status_line() {
                    return Err(Failure::new(format!(
                        "the status line recorded in {} does not match the one in the \
                         backup at {backup}. Nothing was changed; compare the two by hand.",
                        chain::chain_path()?.display()
                    )));
                }
                let _ = writeln!(out, "Checked against the backup at {backup}.");
            }
            Err(error) => {
                let _ = writeln!(
                    out,
                    "Note: the backup at {backup} could not be read ({error}); restoring \
                     from chain.json alone."
                );
            }
        }
    }

    let before = current.before_text();
    let after = patched.render();
    print_plan(out, &path, &before, &after, options.dry_run);

    if options.dry_run {
        let _ = writeln!(
            out,
            "--dry-run: nothing was written. {} is unchanged.",
            path.display()
        );
        return Ok(0);
    }

    nazar_core::atomic::write_bytes(&path, after.as_bytes())?;
    match record.command() {
        Some(command) => {
            let _ = writeln!(out, "Uninstalled. Your status line is {command} again.");
        }
        None => {
            let _ = writeln!(
                out,
                "Uninstalled. There was no status line before, so the key is gone again."
            );
        }
    }
    if let Some(backup) = &record.backup_path {
        let _ = writeln!(out, "The backup at {backup} was left in place.");
    }
    Ok(0)
}

/// Say what is installed, what it chains to, and what the captures look like.
pub fn status(options: &Options, out: &mut dyn Write) -> Result<i32> {
    let directory = settings::config_dir(options.config_dir.as_deref())?;
    let path = settings::settings_path(&directory);

    let _ = writeln!(out, "settings:  {}", path.display());
    let loaded = Settings::load(&path);
    match &loaded {
        Err(error) => {
            let _ = writeln!(out, "installed: unknown — {error}");
        }
        Ok(current) => match current.status_line_command() {
            Some(command) if settings::is_our_command(command) => {
                let _ = writeln!(out, "installed: yes");
                let _ = writeln!(out, "command:   {command}");
            }
            Some(command) => {
                let _ = writeln!(out, "installed: no");
                let _ = writeln!(out, "command:   {command}");
            }
            None => {
                let _ = writeln!(out, "installed: no");
                let _ = writeln!(out, "command:   (no statusLine in this file)");
            }
        },
    }

    match chain::load() {
        Ok(Some(record)) => {
            let _ = writeln!(
                out,
                "chains to: {}",
                record
                    .command()
                    .unwrap_or("(nothing — there was no status line before)")
            );
            if let Some(backup) = &record.backup_path {
                let _ = writeln!(out, "backup:    {backup}");
            }
        }
        Ok(None) => {
            let _ = writeln!(out, "chains to: (never installed on this machine)");
        }
        Err(error) => {
            let _ = writeln!(out, "chains to: unknown — {error}");
        }
    }

    let captures = nazar_core::paths::statusline_dir()?;
    let (count, newest) = capture_summary(&captures);
    let _ = writeln!(out, "captures:  {}", captures.display());
    let _ = writeln!(
        out,
        "           {count} file{}, newest {}",
        if count == 1 { "" } else { "s" },
        newest.map_or_else(|| "—".to_owned(), |age| format!("{}s old", age.as_secs()))
    );
    Ok(0)
}

/// How many captures there are and how old the newest one is.
fn capture_summary(directory: &Path) -> (usize, Option<std::time::Duration>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return (0, None);
    };
    let now = SystemTime::now();
    let mut count = 0;
    let mut newest: Option<std::time::Duration> = None;
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str())
            == Some(nazar_core::claude::CHAIN_FILE_NAME)
        {
            continue;
        }
        count += 1;
        if let Some(age) = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
        {
            newest = Some(newest.map_or(age, |seen| seen.min(age)));
        }
    }
    (count, newest)
}

/// Print the header and the diff. Identical in a dry run and a real one, on purpose:
/// what the user reviews has to be what the user gets.
fn print_plan(out: &mut dyn Write, path: &Path, before: &str, after: &str, dry_run: bool) {
    let _ = writeln!(
        out,
        "{} {}",
        if dry_run { "Would change" } else { "Changing" },
        path.display()
    );
    let text = diff::unified(
        &format!("a/{SETTINGS_FILE}"),
        &format!("b/{SETTINGS_FILE}"),
        before,
        after,
    );
    if text.is_empty() {
        let _ = writeln!(out, "(no change)");
    } else {
        let _ = write!(out, "{text}");
    }
}

/// Stop before anything is read in earnest if another writer is in the directory.
fn refuse_on_lock(directory: &Path) -> Result<()> {
    match settings::lock_in_the_way(directory) {
        None => Ok(()),
        Some(lock) => Err(Failure::new(format!(
            "{} exists, so something else is editing the settings. Nothing was changed; \
             try again once it is gone.",
            lock.display()
        ))),
    }
}

/// The command to install: whatever the caller asked for, or this executable.
fn program_path(options: &Options) -> Result<String> {
    if let Some(program) = &options.program {
        return Ok(program.clone());
    }
    let exe = std::env::current_exe()
        .map_err(|error| Failure::new(format!("could not find my own path: {error}")))?;
    // Canonicalising resolves a relative launch and any symlink, so the settings file
    // records where this program really is rather than how it happened to be started.
    let resolved = std::fs::canonicalize(&exe).unwrap_or(exe);
    Ok(plain_path(&resolved))
}

/// A path without Windows's verbatim prefix.
///
/// `canonicalize` returns `\\?\C:\…`, which works everywhere except in a file a person
/// has to read, and which some shells refuse outright.
#[must_use]
pub fn plain_path(path: &Path) -> String {
    let text = path.display().to_string();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
}

/// Copy the settings file to `settings.json.nazar-bak-<stamp>`, never over an existing one.
///
/// The stamp has one-second resolution, so two installs in the same second would collide;
/// the counter is what makes "never overwrite a backup" true rather than nearly true.
fn write_backup(path: &Path, contents: &[u8]) -> Result<PathBuf> {
    let directory = path.parent().unwrap_or(Path::new("."));
    let stamp = nazar_core::now_rfc3339().replace(['-', ':'], "");

    for attempt in 0..100u32 {
        let name = if attempt == 0 {
            format!("{BACKUP_PREFIX}{stamp}")
        } else {
            format!("{BACKUP_PREFIX}{stamp}-{attempt}")
        };
        let candidate = directory.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                file.write_all(contents).map_err(|error| {
                    Failure::new(format!(
                        "could not write the backup {}: {error}",
                        candidate.display()
                    ))
                })?;
                file.sync_all().map_err(|error| {
                    Failure::new(format!(
                        "could not flush the backup {}: {error}",
                        candidate.display()
                    ))
                })?;
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(Failure::new(format!(
                    "could not create the backup {}: {error}",
                    candidate.display()
                )));
            }
        }
    }
    Err(Failure::new(
        "could not find a free backup name; nothing was changed",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verbatim_prefix_is_stripped_from_a_path_a_person_has_to_read() {
        assert_eq!(
            plain_path(Path::new(r"\\?\C:\bin\nazar-statusline.exe")),
            r"C:\bin\nazar-statusline.exe"
        );
        assert_eq!(
            plain_path(Path::new(r"\\?\UNC\server\share\nazar-statusline.exe")),
            r"\\server\share\nazar-statusline.exe"
        );
        assert_eq!(
            plain_path(Path::new("/usr/local/bin/nazar-statusline")),
            "/usr/local/bin/nazar-statusline"
        );
    }

    #[test]
    fn a_backup_is_never_written_over_an_existing_one() {
        let dir = crate::testutil::TempDir::new("backup");
        let path = dir.join(SETTINGS_FILE);
        std::fs::write(&path, "{}").unwrap();

        let first = write_backup(&path, b"first").unwrap();
        let second = write_backup(&path, b"second").unwrap();
        let third = write_backup(&path, b"third").unwrap();

        assert_ne!(first, second);
        assert_ne!(second, third);
        assert_eq!(std::fs::read(&first).unwrap(), b"first");
        assert_eq!(std::fs::read(&second).unwrap(), b"second");
        assert_eq!(std::fs::read(&third).unwrap(), b"third");
        assert!(
            first
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(BACKUP_PREFIX)
        );
    }
}
