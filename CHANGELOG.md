# Changelog

Notable changes to nazar-tray. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

`limits.json` has its own compatibility promise, separate from the app version: see
[docs/limits-contract.md](docs/limits-contract.md).

## [Unreleased]

Nothing released yet. The repository holds the WP0 skeleton, the WP1 Codex reader and the
WP2 Claude reader with its status-line wrapper: it builds, it tests, `nazar-tray --print`
prints real numbers for both providers, and the tray still opens an empty panel.

### Added

- **`nazar-statusline`**, the shared status-line wrapper, in a third crate with no Tauri in
  it (334 KB release binary, three dependencies). Installed as Claude Code's
  `statusLine.command`, it writes the payload **whole** to
  `~/.nazar/statusline/<session_id>.json` and then runs the status line that was already
  there, with the same bytes on standard input and its stdout, stderr and exit code
  forwarded. Keyed by session id, so three concurrent sessions do not overwrite each
  other. Measured at a **median of 9.6 ms** over ten runs against a 50 ms budget, release
  build, no chained command. Empty or unparseable input still chains and still exits `0`:
  nothing this program does can break the status line.
- **`nazar-statusline install | uninstall | status`**. The install touches exactly one key
  of `settings.json`, keeps every other key and their order, preserves `padding`,
  `refreshInterval` and anything else the existing `statusLine` carried, prints a unified
  diff whether or not anything is a terminal, and takes a backup it will never write over.
  It refuses — changing nothing — on invalid JSON, on a lock file, and when it is already
  installed. `uninstall` restores the exact object recorded in `chain.json`, checks it
  against the backup, and leaves the backup in place. `--dry-run` prints the same diff and
  writes nothing at all.
- **Claude reader** (`nazar-core::claude`): reads the captures, takes the newest, and maps
  `rate_limits.five_hour` and `.seven_day` to the `claude` provider block — percentage,
  `windowMinutes`, `resetsAt` in UTC, `state`, and a binding window computed as the highest
  percentage. `configured:false` when there is no capture; both windows `state:"error"`
  with no percentage when the payload carries no `rate_limits`, which is what a
  non-subscriber and a session before its first API response both look like. No plan name
  is derived: the payload does not carry one, and `model.id` is not a subscription.
- Claude fixtures sanitised from a real payload and a real settings file — a payload with
  one live window, one with both, one with no `rate_limits`, and the three settings shapes
  the acceptance criterion names (no status line, ccstatusline, a custom `node` script) —
  plus a second leak test, a second fixture-hygiene gate, and rows in
  `docs/pinned-internal-formats.md` for the payload and for `settings.json`.
- `docs/statusline-wrapper.md`: what is captured, where it lives, why captures hold paths
  and therefore never leave `~/.nazar`, and how to uninstall by hand from `chain.json` if
  the binary is gone.
- `NAZAR_HOME` moves `~/.nazar`. It exists so that the wrapper's tests run against a
  throwaway directory; **no test in this repository reads or writes the real
  `~/.claude` or the real `~/.nazar`**, which is risk R10's mitigation made mechanical.

- **Codex reader** (`nazar-core::codex`): finds the newest `rollout-*.jsonl` under
  `$CODEX_HOME` (default `~/.codex`), follows it by byte offset, and maps
  `payload.rate_limits` to the `codex` provider block of `limits.json` — `primary` and
  `secondary` with their percentage, window length and reset time, the plan name, and a
  binding window computed as the highest percentage rather than taken from a flag the
  source does not provide. No credential file is opened and nothing leaves the machine.
- `nazar-tray --print`: writes the `limits.json` document to standard output and exits
  without starting the tray. It does **not** write `~/.nazar/limits.json`; that file keeps
  its single writer and gets one in WP3.
- Codex fixtures sanitised from real logs — a session's quota lines, the `limit_id`
  `premium` variant whose windows are both `null`, a log with no quota line, and a damaged
  log — plus `docs/pinned-internal-formats.md`, which records every field the reader
  depends on, the version it was observed under, and the files it must never open.
- Three gates that fail the build rather than a review: a leak test that puts a sentinel in
  every text field of a rollout line and fails if it reaches the output, a grep over the
  whole workspace for the name of any credential file, and a fixture scan for home paths,
  e-mail addresses, long hexadecimal ids and the current user's name.
- Cargo workspace: `nazar-core` (pure Rust, no Tauri, builds everywhere) and `nazar-tray`
  (the Tauri v2 application, Windows first).
- `limits.json` contract at `schemaVersion: 1` — serde types, an atomic writer
  (temporary file plus rename) and a reader, with the contract written down in
  `docs/limits-contract.md` and a sample in `fixtures/limits.sample.json`.
- Tray icon with a placeholder bead, and a frameless popup panel that opens near the
  cursor on either mouse button and closes on Esc or on losing focus.
- Panel built from plain TypeScript, HTML and CSS with esbuild — no framework
  (decision K2).
- i18n scaffolding: `locales/{en,tr}.json` written by hand, `{zh,ko,ru,es}.json` as empty
  placeholders, a `t()` helper with `{placeholder}` interpolation, and a language guess.
  EN/TR parity is a test; the four pending languages are reported, not failed (WP6).
- Theme tokens copied verbatim from Nazar (`theme.nazar.json`, `theme.graphite.json`)
  with a test asserting the four bead hexes (decision K23).
- CI: Windows lint, test and build as the gate; Ubuntu and macOS running `nazar-core`
  only; the Windows debug bundle as a non-blocking artifact job (risk R1).
- Repository hygiene: `.gitattributes`, `rust-toolchain.toml`, `rustfmt.toml`, `deny.toml`
  and `scripts/check-licenses.mjs`, which fails on any dependency that is not permissively
  licensed.

### Changed

- **The `limits.json` samples now show UTC.** `fixtures/limits.sample.json`,
  `docs/limits-contract.md` and `docs/PROJECT.md` section 6 carried a `+03:00` offset while
  the writer has produced `…Z` since WP1. The instants are unchanged — the same moments,
  written the way the file actually writes them. Consumers that vendored the sample should
  re-copy it; nothing about the schema moved.

### Notes

- Times in `limits.json` are RFC 3339 in UTC (`…Z`). They name the same instant a local
  offset would; consumers render local time, and countdowns do not depend on the offset.
  Reasoning in `docs/pinned-internal-formats.md`.
- A capture file holds the whole status-line payload, and the payload holds paths — `cwd`,
  `transcript_path`, the workspace. There is no token and no account identifier in it, but
  the paths are why captures live under `~/.nazar` and why `limits.json` gets four numbers
  out of one and nothing else. Cost and context usage stay in the capture, for Nazar.
- The wrapper runs a chained command through `cmd.exe /C` on Windows and `sh -c`
  elsewhere, because `statusLine.command` is a command line rather than a program and its
  arguments. On Windows the line is passed raw and wrapped in one more pair of quotes;
  Rust's own escaping follows the C runtime's rules and `cmd.exe` does not, which would
  break every command with a quoted path in it.
- The Codex reader reports what it read and how old it is (`sourceAt`); it does not decide
  when old becomes stale. That threshold belongs to the state model in WP3, so that all
  three displays answer it the same way — the retired prototype had three different ones.
- The tray shows a static bead. The bead that fills from the bottom with the binding
  window is WP4 (decision K3, drawn in Rust per scale factor).
- `tauri-plugin-notification`, `tauri-plugin-autostart` and `tauri-plugin-updater` are
  declared but not initialised. They are wired in WP5 and WP7.
