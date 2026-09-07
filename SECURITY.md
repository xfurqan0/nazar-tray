# Security

nazar-tray reads files two AI coding agents leave on your disk, writes one file of its own,
and — if you ask it to from the settings page — edits one line of Claude Code's configuration.
That is a category of tool that can leak your work by accident, so this page is specific about
what it opens, what it refuses to open, and what leaves the machine.

## Reporting a vulnerability

Use GitHub's private reporting: **Security → Report a vulnerability** on
[github.com/xfurqan0/nazar-tray](https://github.com/xfurqan0/nazar-tray/security/advisories/new).
It reaches the maintainer and nobody else. Please do not open a public issue for something
that discloses a credential path, a way to make the tray write outside its own directories, or
a way to get a token into a file.

Expect a first answer within a week. This is a one-person project with no service behind it,
so there is no on-call rotation to promise you — what there is, is a small attack surface and
a short list of files, both below.

Only the latest release is supported. There is no back-porting; the fix is the next version.

## What it reads

| Source | What is taken from it |
|---|---|
| `$CODEX_HOME/sessions/**/rollout-*.jsonl` (default `~/.codex`) | The newest session log, tailed by byte offset. Seven values out of `payload.rate_limits` — two window percentages, their window lengths, their reset seconds — and nothing else. No prompt, no response, no tool call. |
| `~/.nazar/statusline/<session_id>.json` | The status-line payload the wrapper captured. `rate_limits` is read out of it for `limits.json`; the rest of the payload is left in the file for Nazar. |
| `%APPDATA%\nazar\config.json` | This application's own settings. |
| `%APPDATA%\nazar\alerts.json` | Which notifications have already been shown, so a restart does not repeat a warning. |
| `~/.claude/.credentials.json` | **Only in the opt-in detailed-windows mode, which is off by default.** See below. |

Every path is derived at run time from the environment. There are no absolute paths in the
code, and `crates/nazar-core/tests/hygiene.rs` fails the build on one.

## What it never reads

- **Message content.** Not a prompt, not a response, not a thinking block, not a tool input
  or its result. The Codex reader parses a line, takes seven numeric fields out of one object
  and drops the parse result; there is no code path that could carry a message forward.
- **Transcripts.** The retired C# prototype scanned up to 400 transcript files to estimate
  usage after an HTTP 401. That whole idea is gone: this product reports numbers it was given
  and says "unknown" otherwise.
- **Any credential file, on the default path.** The module that reads one is behind a Cargo
  feature *and* a run-time switch, and CI proves the feature works by building and testing
  with it compiled out (`cargo test -p nazar-core --no-default-features`). The status-line
  wrapper is built exactly that way, so the released `nazar-statusline.exe` contains neither
  an HTTP client nor the code that opens a sign-in file.

## What it writes

| Path | Contents | When |
|---|---|---|
| `~/.nazar/limits.json` | Per window: a percentage, a length in minutes, a reset timestamp, a state. Per provider: whether it is configured, which window binds, where the numbers came from. **No token, no account identifier, no e-mail, no path, no project name.** Safe to paste into a bug report. | Only when a number has actually changed. Written to a temporary file and renamed, by one process holding an advisory lock. |
| `~/.nazar/limits.lock` | The advisory lock. | While the tray runs. |
| `~/.nazar/statusline/<session_id>.json` | The status-line payload, whole. This one **does** hold paths — `cwd`, `transcript_path`, the workspace directories, the repository name — which is why it lives inside your home directory and why nothing uploads it. | By the wrapper, on each status-line refresh, if you installed it. Deleted seven days after a session goes quiet. |
| `%APPDATA%\nazar\config.json`, `alerts.json` | Settings, and which warnings have been given. | On change. |
| `~/.claude/settings.json` | **One key**, `statusLine`, and only if you press the button on the settings page. Every other key and their order survive. A whole-file backup is taken first and never written over. | Never on install, never on update, never by the tray on its own. |

## The network

**The default build makes no network call, and this is checkable rather than promised.** The
only HTTP client in the tree is behind the `detailed-windows` Cargo feature; with the feature
off, no HTTP crate is linked. Turn on detailed windows and one endpoint is contacted, the
official Claude usage endpoint, over HTTPS, with a token read from Claude Code's own sign-in
file, held in memory for that one request and written to no file, log or error message. A
test asserts the token never appears in any of the three.

There is no telemetry, no crash reporting, no update ping, and no analytics — in any build,
with any switch in any position.

## Signing

0.1.0 is unsigned; SmartScreen will warn on a browser download, and `winget install` avoids
that path. Every release carries a `SHA256SUMS` file and a GitHub build attestation that says
which workflow run built the file. The plan for a real certificate, and why the application
comes after the first release, is in [docs/CODE_SIGNING.md](docs/CODE_SIGNING.md).

## Uninstalling

The uninstaller removes the program, the Start Menu entry, the startup entry and the
`StartupApproved` record Windows keeps beside it, and asks the wrapper to put your status line
back before deleting it. It leaves `~/.nazar` alone on purpose — Nazar reads `limits.json`,
and the captures are your data — and keeps `%APPDATA%\nazar` unless you tick the box.
[docs/RELEASE.md](docs/RELEASE.md) lists the by-hand commands for the rest.
