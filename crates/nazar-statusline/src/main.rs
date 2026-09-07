//! The `nazar-statusline` binary.
//!
//! With no arguments it is the wrapper and reads a payload from standard input. With a
//! subcommand it is the installer. The two share a binary because a user has to be able
//! to point Claude Code's `statusLine.command` at one absolute path and be done, and
//! because the thing that installs the wrapper should be the thing that *is* the wrapper —
//! it writes its own path into the settings file, so the two can never disagree.

use std::path::PathBuf;
use std::process::ExitCode;

use nazar_statusline::install::{self, Options};
use nazar_statusline::{Failure, Result, capture};

/// Exit code for a command that could not do what it was asked.
const FAILED: u8 = 2;

const USAGE: &str = "\
nazar-statusline — records Claude Code's status-line payload and runs the status line you
already had.

USAGE:
  nazar-statusline                       read a payload on stdin, capture it, chain on
  nazar-statusline install [OPTIONS]     make it Claude Code's statusLine command
  nazar-statusline uninstall [OPTIONS]   put the previous status line back
  nazar-statusline status [OPTIONS]      what is installed, what it chains to
  nazar-statusline --help | --version

OPTIONS:
  --dry-run              print the diff and write nothing (install, uninstall)
  --config-dir <PATH>    where settings.json lives; defaults to CLAUDE_CONFIG_DIR,
                         then ~/.claude

FILES:
  ~/.nazar/statusline/<session_id>.json  one capture per Claude Code session
  ~/.nazar/statusline/chain.json         the status line install replaced, verbatim
  NAZAR_HOME                             moves ~/.nazar somewhere else
";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();

    // No arguments at all is the case that runs thousands of times a day. It is checked
    // first and costs one comparison.
    if arguments.is_empty() {
        return code(capture::run());
    }

    match run(&arguments) {
        Ok(status) => code(status),
        Err(failure) => {
            eprintln!("nazar-statusline: {failure}");
            ExitCode::from(FAILED)
        }
    }
}

/// Dispatch a subcommand.
fn run(arguments: &[String]) -> Result<i32> {
    let mut out = std::io::stdout().lock();
    match arguments[0].as_str() {
        "--help" | "-h" | "help" => {
            print!("{USAGE}");
            Ok(0)
        }
        "--version" | "-V" => {
            println!("nazar-statusline {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        "install" => install::install(&options(&arguments[1..])?, &mut out),
        "uninstall" => install::uninstall(&options(&arguments[1..])?, &mut out),
        "status" => install::status(&options(&arguments[1..])?, &mut out),
        other => Err(Failure::new(format!(
            "unknown command {other:?}. Run `nazar-statusline --help`."
        ))),
    }
}

/// Parse the flags a subcommand accepts.
///
/// Written by hand rather than with a parser crate: five flags do not justify a
/// dependency in a binary whose start-up time is the feature.
fn options(arguments: &[String]) -> Result<Options> {
    let mut options = Options::default();
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--dry-run" => options.dry_run = true,
            "--config-dir" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return Err(Failure::new("--config-dir needs a path after it"));
                };
                options.config_dir = Some(PathBuf::from(path));
            }
            other => {
                if let Some(path) = other.strip_prefix("--config-dir=") {
                    options.config_dir = Some(PathBuf::from(path));
                } else {
                    return Err(Failure::new(format!(
                        "unknown option {other:?}. Run `nazar-statusline --help`."
                    )));
                }
            }
        }
        index += 1;
    }
    Ok(options)
}

/// Turn a chained command's exit code into this process's.
///
/// `ExitCode` is a byte, and a status line's command can return anything an operating
/// system lets it. Anything that does not fit is reported as a plain failure rather than
/// truncated into a number that means something else.
fn code(status: i32) -> ExitCode {
    match u8::try_from(status) {
        Ok(byte) => ExitCode::from(byte),
        Err(_) => ExitCode::from(1),
    }
}
