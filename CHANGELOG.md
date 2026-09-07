# Changelog

Notable changes to nazar-tray. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

`limits.json` has its own compatibility promise, separate from the app version: see
[docs/limits-contract.md](docs/limits-contract.md).

## [Unreleased]

Nothing released yet. The repository holds the WP0 skeleton, the WP1 Codex reader, the WP2
Claude reader with its status-line wrapper and the WP2b detailed-windows mode: it builds,
it tests, `nazar-tray --print` prints real numbers for both providers, and the tray still
opens an empty panel.

### Added

- **Detailed windows (`nazar-core::claude::detailed`), opt-in and off by default.** With
  `detailedWindows` on, it reads four values out of `<CLAUDE_CONFIG_DIR or
  ~/.claude>/.credentials.json`, holds the access token in a wrapper that wipes itself and
  prints `Secret(<redacted>)`, and sends it as one `Authorization` header on a single
  20-second `GET` to `api.anthropic.com/api/oauth/usage`. `limits[]` becomes `five_hour`,
  `seven_day` and one `seven_day_<model>` per model-scoped weekly cap, all marked
  `detailed: true`, with the plan normalised from `rateLimitTier`
  (`default_claude_max_20x` → `max_20x`). **With the mode off nothing is opened and no
  socket is created**, and a test poisons the credential file to prove it. That file is
  never written, and `refreshToken` is never read: refreshing is Claude Code's job. The
  whole account, including where this sits against Anthropic's terms, is
  [docs/detailed-windows.md](docs/detailed-windows.md).
- **A failure keeps the last good numbers and says they are old.** 401 and 403 report
  "token expired; run Claude Code once to refresh"; 429 honours `Retry-After`; a 200 whose
  body this build cannot read is a failure rather than a blank. Each leaves the previous
  windows in place with `state: "stale"` and a short reason, and pushes the next attempt
  out — 1 s, 2 s, 4 s and so on, capped at 30 minutes, held in memory only. Three audit
  findings became behaviour here: `Retry-After` is read at all (B07), a token whose stored
  `expiresAt` has passed costs no request and a rewritten sign-in file clears the wait
  immediately (B06, the three-hour 401 storm), and a plan that changed between refreshes
  drops the remembered numbers rather than showing one account's percentages under
  another's name (B25).
- **Merge policy** (`nazar-core::claude::merge`): the endpoint lays down the block, and a
  **newer** status-line capture wins on the two windows both paths report — the status line
  is rewritten every few seconds, the endpoint is asked on a timer and backed off from.
  Model-scoped weeklies are never replaced, a passive window with no percentage never
  replaces one that has one, and the binding window is recomputed across everything.
  Written down in
  [docs/limits-contract.md](docs/limits-contract.md).
- **Settings** (`nazar-core::config`): `%APPDATA%\nazar\config.json` with
  `detailedWindows` and `detailedSuggested`, atomic writes, unknown keys preserved, a
  missing file read as the defaults and a damaged one reported rather than replaced.
  `NAZAR_HOME` now moves this file too, so a whole installation really can be pointed at a
  throwaway directory.
- **`should_suggest_detailed(plan_hint, already_asked)`**: offer the mode once, and only on
  Max, because Pro has no model-scoped weekly window for it to reveal. The dialog is WP5's.
- **`nazar-tray --print --detailed`** runs the mode for one run without switching it on for
  the machine. `--print` on its own does exactly what `config.json` says, which on a machine
  nobody has configured is nothing at all.
- **Four gates for the one exception to "no credentials".** A sentinel token goes through
  the entire flow — settings written, sign-in read, request sent, answer mapped, block
  merged, `limits.json` written, then a failure whose body echoes the sentinel back — and
  the test fails if it appears in any file under the temporary `NAZAR_HOME`, in any error's
  `Display` or `Debug`, or in the document. `Secret::expose_for_one_request` must have
  exactly one call site in shipping code. No file in the module may contain a printing
  macro. And the workspace credential grep grew an allow-listed **directory** rather than
  losing a needle, plus a test that fails the day that directory stops needing the
  exception.
- **61 new tests (249 in the workspace)**, every network one against a hand-rolled HTTP
  server on `127.0.0.1:0`. No test in this repository reaches the network, reads the real
  `~/.claude`, or writes outside a directory it made itself.

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

- **`fixtures/limits.sample.json` now says `"source": "endpoint"` for Claude.** The
  document is unchanged otherwise, and it is the same document it always described: three
  windows, two of them replaced by a newer status-line capture (so no `detailed` flag) and
  one that only the endpoint can produce (so it keeps its flag). What changed is the rule
  behind `source`, which WP2b had to pin down: it names the path that **produced the block**,
  and the per-window `detailed` flag says which windows survived from it. The schema did not
  move; consumers that vendored the sample should re-copy it.
- **Timestamps from the usage endpoint are rewritten before they reach `limits.json`.** The
  endpoint answers `2026-09-07T13:10:00.130195+00:00` where the status line writes Unix
  seconds. Both now come out as `2026-09-07T13:10:00Z`, which is what rule 6 of the contract
  always said and what every other timestamp in the file already was.
- **The `limits.json` samples now show UTC.** `fixtures/limits.sample.json`,
  `docs/limits-contract.md` and `docs/PROJECT.md` section 6 carried a `+03:00` offset while
  the writer has produced `…Z` since WP1. The instants are unchanged — the same moments,
  written the way the file actually writes them. Consumers that vendored the sample should
  re-copy it; nothing about the schema moved.

### Notes

- **The detailed-windows mode is behind a cargo feature as well as the runtime flag.**
  `nazar-core`'s `detailed-windows` feature is on by default, because the shipped tray
  offers the toggle; `nazar-statusline` depends on `nazar-core` with
  `default-features = false`, so the binary that runs on every status-line refresh compiles
  neither the HTTP client nor the code that would read a token —
  `cargo tree -p nazar-statusline` still shows `serde` and `serde_json` and nothing else.
  (A `cargo build --workspace` unifies features and builds the shared rlib once with the
  feature on; the released wrapper is built on its own, where it does not.)
- **`ureq` with `rustls` was chosen over `reqwest` by measurement.** Against this
  workspace's lock file: `ureq` adds four packages (`ureq`, `ureq-proto`, `utf8-zero`,
  `webpki-roots`), `reqwest` with `blocking` adds seventeen including `aws-lc-rs`,
  `aws-lc-sys`, `cmake` and `quinn`. `reqwest` is already linked by Tauri, but only in the
  tray crate; this code lives in `nazar-core`, which has no Tauri in it and is linked by a
  binary that has to start in under ten milliseconds. All four are permissively licensed and
  `webpki-roots`' `CDLA-Permissive-2.0` was already on the allow-list. `zeroize` was already
  in the lock file transitively, so the wiping costs nothing new.
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
