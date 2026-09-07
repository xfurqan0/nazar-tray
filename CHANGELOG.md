# Changelog

Notable changes to nazar-tray. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

`limits.json` has its own compatibility promise, separate from the app version: see
[docs/limits-contract.md](docs/limits-contract.md).

## [Unreleased]

Nothing released yet. The repository holds the WP0 skeleton, the WP1 Codex reader, the WP2
Claude reader with its status-line wrapper, the WP2b detailed-windows mode, the WP3 state
model, refresh loop and writer, the WP4 icon and panel, and the WP5 notifications, autostart
and settings: it builds, it tests, and **it is now something you can leave running** — it
warns you before you hit the wall, it can start with Windows, and everything about it can be
changed from the panel. What is left before a release is the four machine-translated
languages (WP6) and the installer (WP7).

### Added

- **Threshold notifications** (`nazar-core::alerts`, `crates/nazar-tray/src/alerts.rs`). A
  toast when a window crosses 60, 85 or 100 % — whatever the settings say — **once per
  threshold per reset period**, keyed `(provider, window, threshold, resetsAt)` and written to
  `%APPDATA%\nazar\alerts.json` **before** it is shown, so a restart does not repeat a
  warning the user has already had. Crossing is **edge-triggered** (`previous < T ≤ current`),
  so a window sitting above a threshold says nothing for the rest of the week; the **first**
  observation counts, so a tray started at 91 % says so once rather than waiting for a
  crossing that already happened; a **reset clears the memory as well as the keys**, so a
  window that resets from 86 % straight back to 86 % warns again; and **a window nobody could
  read never notifies and forgets what it last saw**, so the reading after it is a first
  observation rather than a continuation of a number nobody can vouch for. A jump that crosses
  two thresholds at once is **one** toast naming the more severe of them, with both consumed.
  `Claude Code · weekly window 85 %` over `Resets in 2 h 10 m`, in the user's language.
- **Quiet hours**, and a switch above them. `quietHours: {from, to}` in **local wall-clock**
  time, wrapping midnight when `to` is earlier than `from`. A suppressed crossing still
  happens, is still recorded and still colours the tray icon — only the interruption is
  withheld, and it is not delivered late either. The same is true of turning the
  notifications off, so turning them back on is quiet rather than a burst of catching up.
- **Start with Windows** (`tauri-plugin-autostart`). Adds
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` → `nazar-tray` = the executable
  plus `--hidden`. The switch **reads its state back from the registry** rather than
  remembering an answer of its own, so it agrees with Task Manager's Startup tab. The same
  thing from a terminal, for a user whose panel will not open:
  `nazar-tray --autostart on|off|status`.
- **A settings page**, as a second view in the same window and reachable from the tray menu.
  Language, theme, light/dark, **per-provider on and off**, the three thresholds, quiet hours,
  autostart, the detailed-windows switch with the plain-words explanation of what it reads,
  where every file lives with the home directory collapsed to `~`, a way to bring the
  first-run tip back, and the version. The form is **validated before anything is written and
  refused as a whole**: `85 / 60 / 100` gets an error and the settings the user had, not an
  error and a tray that has half changed.
- **The one-time Max-plan offer** that WP2b decided and left to be asked. *On a Max plan?
  Detailed windows shows model-specific weekly limits.* Shown once, in the panel, and answered
  for good either way. It is phrased as a **question** because it has to be: the status-line
  payload carries no plan name, so on the passive path there is nothing to detect — a rule
  that only fired for a *known* Max plan would never fire at all. It is not offered to
  somebody who has the mode on, has already been asked, or does not have Claude Code set up
  on this machine.
- **A provider you switch off is not read at all.** No reader is built for it, so its files
  are never opened and its card is not drawn — which is a stronger promise than hiding it,
  and it needed one new command in the refresh loop (`Reconfigure`) so the readers can be
  replaced on the thread that owns them, and a refresh at once rather than at the next minute.
- **One answer about the language, for the panel and the tray alike.** The override, then the
  operating system's UI language, then English — decided in Rust, handed to the panel, and
  used for the tooltip, the menu and the toasts. **This closes WP4's open risk**, where the
  panel guessed from `navigator.languages` while the tray fell back to English and the two
  could disagree. Changing it **rebuilds the tray menu**, because a menu item's text is fixed
  when the item is built.
- **`--hidden`** (what the startup entry passes: open no window, whatever else was asked for)
  and **`--demo-cross`**, the acceptance run — `80 → 86 → 86 → reset → 86` on one window,
  four seconds apart, which should produce one 60 % toast, one 85 % toast, **silence**, and
  one more 85 % toast. It implies `--demo`, so it takes no lock and its notification log is in
  memory: a demo run cannot consume the keys of a real crossing nobody has been shown yet.
- **63 new tests in Rust (444 in the workspace) and 17 more in the panel (68).** The state
  machine on an injected clock and a temporary `NAZAR_HOME` — edge crossing, once per reset,
  reset clears, quiet hours, startup-above, **sleep-and-wake replay**, unknown never fires,
  the log's round trip and its damaged-file behaviour — the settings validation, the form's
  round trip, a gate proving the panel's validator and Rust's agree, and one proving
  `ui/src/i18n.ts` and `nazar-core::config` list the same languages.

### Changed

- **The refresh loop announces every pass**, not only the ones that moved the numbers
  (`Event::Refreshed`). A tray started when the weekly window is already at 91 % changes
  nothing, and that is exactly the case where the user most needs telling.
- **The whole settings document is the tray's state**, rather than the four panel keys WP4
  kept beside it: the readers themselves now depend on the settings, and there is one place a
  change is written and acted on.
- `paths::settings_dir` joins `config.json` and `alerts.json`, which share a lifetime:
  removing the settings should take the notification bookkeeping with it.

### Known limits

- **Clicking a toast cannot open the panel**, and it is the plugin rather than a decision:
  `tauri-plugin-notification` 2.4 builds the notification, spawns `show()` onto the async
  runtime and drops the handle, so the `on_activated` callback that `notify-rust` does offer
  on Windows never reaches an application, and `action_type_id` is mobile-only. The tray icon
  a click away is the workaround, and the toast names the window it is about.
- **Disabling autostart leaves one registry value behind.** `auto-launch` removes the `Run`
  value but not the matching `StartupApproved\Run` one. It is inert — Task Manager only lists
  entries that exist in `Run` — but it is residue, and WP7's uninstaller should take it.
- **The `Run` value is written unquoted.** `C:\Program Files\nazar-tray\nazar-tray.exe
  --hidden` works because Windows tries each space-delimited prefix, but it does depend on
  that.

- **The tray bead, drawn at run time** (`crates/nazar-tray/src/icon.rs`, decision K3). One
  rasteriser, one drawing, three scales: 16 px at 100 %, 20 at 125 %, 24 at 150 %, 32 at
  200 % — what `SM_CXSMICON` actually asks for, handed to the shell as raw RGBA with no PNG
  anywhere in the path. A deep-blue rim, a **white chamber that is what "empty" looks like**,
  and a fill rising from the bottom with the **binding window across both providers**: light
  blue below 60 %, amber at 60, red at 85, and a black pupil at 100 so a spent window
  survives a greyscale screenshot. **Unknown is a grey rim and a hollow ring and never a
  fill** (finding B03), and freshness drains the colour, so an icon nobody has fed in an hour
  does not look as confident as one from a second ago. The seven hexes are copied from
  `ui/theme.nazar.json` and a test parses that file and fails if they drift.
- **A tooltip that says something**: `Claude Fable week 88 % · Codex week 70 % (resets in
  2 h 10 m)`, or `nazar-tray: no data`. Each provider's binding window, named by its length
  rather than by its provider, with the model on a model-scoped weekly; the reset belongs to
  the worst window; a provider nobody could read contributes nothing rather than a `0 %`.
  Built from the same `ui/locales/*.json` the panel reads — the tray is UI too, and
  `crates/nazar-tray/src/i18n.rs` is forty lines that make `no hard-coded text` true outside
  the webview as well.
- **The panel, designed** (`ui/src`). A card per provider on navy: the provider mark (from
  `@lobehub/icons`, MIT, licence in `ui/assets/LICENSE-lobehub.txt`), the plan exactly as the
  source spelled it, and a freshness sentence — *updated 12 s ago*, *stale · last data 2 h
  ago*, *age unknown*. A row per window: its name from `windowMinutes` so no display needs to
  know which provider it is looking at, a `detailed` mark on the windows only the opt-in mode
  can produce, a bar in the bead's colours, the percentage floored, and a countdown the panel
  computes itself every second — a clock below a day, `4 d 2 h` above it, plus the reset in
  the reader's own time zone. **A window nobody could read gets the word, a hatched empty
  track and the reader's own sentence — never a bar of zero length.**
- **The first-run hint.** Windows 11 hides every new tray icon behind the `^` button, so the
  panel says so once — *drag the bead onto the taskbar to pin it* — and remembers being
  dismissed in `config.json`. The panel measures its own content after every render and asks
  for a window that fits, which is what lets the hint appear and disappear without leaving a
  gap.
- **Themes, and light and dark.** The footer toggles `nazar` and `graphite` and the choice is
  remembered; light and dark follow `prefers-color-scheme` unless the settings override it.
  Four new settings keys — `theme`, `themeMode`, `firstRunHintDismissed`, `locale` — with the
  same forward-compatible rules as the rest of the file.
- **Contrast as a test, not an intention** (`ui/test/contrast.test.mjs`). Every foreground
  the stylesheet puts on every background, in both themes and both modes, against WCAG AA
  (4.5:1 for text, 3:1 for a graphic that carries meaning). Twenty-two pairs, all passing;
  the meter fill moved from the raw accent to the derived `accentText` because `#3FA9F5` is
  2.25:1 on a light track.
- **A menu, and with it Quit** — `Open`, `Refresh now`, `Quit`. The reason it exists is the
  lock: a tray that can only be killed leaves `~/.nazar/limits.lock` behind for its
  five-minute grace period, and the next launch then starts as a reader and looks broken.
  Every route out of the application now stops the refresh loop first, which releases the
  lock. Verified by clicking it: the file is gone afterwards, where a kill leaves it.
- **The overflow-flyout race, fixed.** Clicking a tray icon *inside* the Windows 11 overflow
  shows the panel and then closes the flyout, which blurs the panel about 200 ms later and
  hid it instantly. A blur within 300 ms of a tray-initiated open is now ignored **and the
  focus is taken back**, so the next click elsewhere still closes the panel. Verified against
  the real flyout: visible and focused at 250 ms, 1250 ms and 3250 ms.
- **Screenshot flags, documented rather than hidden**: `--demo` (synthetic numbers, panel
  open, **no lock and nothing written**), `--scale`, `--theme`, `--mode`, `--hint`,
  `--locale`, and `--icons <dir>`. `scripts/screenshot.ps1` produces every picture in
  `docs/screenshots` from them, so "how was this made" has an answer that is not a memory.
  150 % and 200 % are genuinely rendered at those scales — WebView2 is given
  `--force-device-scale-factor` and the window is multiplied to match — rather than being
  upscaled screenshots.
- **37 new tests in Rust (381 in the workspace) and 25 more in the panel (51).** The
  rasteriser's are pixels: fill height per percentage, a colour per severity, no fill of any
  kind for unknown, a pupil only when a window is spent, and pinned PNG bytes for three
  cases. The panel's are pure functions — window names, durations, freshness sentences, the
  theme toggle — plus the cross-language gate grown to cover the new commands and **every
  message key either language names**, in either direction.

- **State model** (`nazar-core::state`). `limits.json` stores what a source measured;
  everything a display wants is derived at read time and stored nowhere, because all of it
  changes with the clock rather than with the data: the **binding** window (highest
  percentage, ties to the shorter window, and a window with no percentage never binds), the
  **countdown** (`resetsAt` − now, **negative when the reset is already due** rather than
  frozen at "now", which is audit scenario S7), the **age** (now − `sourceAt`), **freshness**
  (`fresh` ≤ 5 min, `aging` ≤ 45 min, `stale` beyond) and **severity** (`ok` < 60, `warn`
  ≥ 60, `critical` ≥ 85, `exhausted` ≥ 100). `unknown` is a real value in both of the last
  two, and it sorts *below* `ok`, so a provider nobody could read never decides the icon's
  colour on its own.
- **One definition of "stale", in the settings.** The thresholds live in
  `%APPDATA%\nazar\config.json` (`thresholds`, `freshness`) and every display reads them from
  there. The retired prototype had three displays with three different answers — 45 minutes
  in the tray, 30 in the status line, none in the fetcher — which is audit finding B14. The
  binding rule is likewise one function now, shared by both readers and the view (B16).
- **Refresh loop inside the tray process** (`nazar-core::refresh`), which is what
  `docs/PROJECT.md` section 3 decided and what deletes the Task Scheduler / launchd / systemd
  triple, the VBS launcher and the lock race the audit proved from logs. One thread owns
  every reader. Five triggers: startup, a 60-second tick, a **five-second look** at the
  status-line captures and at the rollout log being followed, an explicit request from the
  panel or a second launch, and a **clock jump of more than two ticks** — which is how a
  machine notices it has been asleep (audit scenario S7: gaps of 480, 575 and 867 minutes in
  one week of logs, each followed by something going wrong). Everything but the tick is
  coalesced by a 250 ms debounce, so a busy Codex session that writes a quota line every few
  seconds still costs one refresh. The opt-in endpoint is asked at most **once every five
  minutes**, on top of its own backoff.
- **The writer, which mostly does not write** (`nazar-core::writer`). Each refresh is
  compared with the last one — the serialised document, minus `updatedAt` — and an identical
  one is not written at all: no temporary file, no rename, no modification time, nothing for
  a watching consumer to wake up for. So **`updatedAt` moves with the content**, which is the
  honest reading of "when the tray last wrote this file", and a consumer that wants to know
  whether the *tray* is alive reads the lock's heartbeat instead. A tray restarting over an
  unchanged file writes nothing, and its panel shows the last known numbers before its first
  read finishes.
- **One writer, enforced** (`nazar-core::lock`, `~/.nazar/limits.lock`). Acquisition is a
  single `create_new`, so of two processes racing for it exactly one wins — finding B01 was a
  lock whose check and whose write were separate operations, and the logs caught two
  refreshers passing that check four seconds apart, which is what produced the observed
  HTTP 429. Liveness is a **heartbeat** rewritten on every refresh, not a process id: there is
  no portable way to ask whether a process is running, and a heartbeat also catches a holder
  that is alive but wedged. A record silent for five minutes is reclaimed; an unreadable one
  is respected until it ages out, so a competitor cannot evict a holder that is mid-write.
  The holder re-checks the file **before** it writes, so a tray whose lock was taken over
  while its machine slept stops writing rather than overwriting its replacement.
- **Single instance.** The same lock, rather than a named mutex or a plugin: it is the
  guarantee `limits.json` needs anyway, it works the same on all three target platforms, and
  it added nothing to the dependency tree. A second launch leaves a marker at
  `~/.nazar/tray.request`, which the running tray notices within five seconds, deletes, and
  answers by opening its panel — then the second process exits. A marker older than a minute
  is swept up without being obeyed.
- **`nazar-tray --print --write`**: one refresh, written to `~/.nazar/limits.json`, for
  scripts and for Linux — where v1 ships no tray at all and the CLI plus the Nazar canvas
  *is* the story. It takes the same lock, and if the tray already holds it this **reads
  instead of writing**: it prints what the tray wrote, says so on standard error, and changes
  nothing.
- **The panel reads the state.** `get_snapshot` derives the view for the instant the panel
  asked, `get_warnings` exposes the loop's per-reader counters, `refresh_now` asks for a pass
  (audit finding B13 was a Refresh button that appeared to do nothing), and a
  `snapshot-changed` event fires **only when the document changed**. The panel counts the
  seconds between refreshes itself. It shows raw values — provider, each window's percentage
  or the word "unknown", the countdown, how old the reading is — because designing it is WP4
  and a placeholder would only have to be deleted. Percentages are floored, never rounded up:
  99.6 % is not 100 % (finding B15).
- **Property tests**, with a hand-rolled generator rather than `proptest`: ten thousand
  random window sets for the binding rule, three thousand random instants × six time-zone
  spellings for the countdown (plus fixed cases at both American daylight-saving boundaries
  and at Istanbul's, which has none), and monotonicity for severity and freshness across
  their whole ranges. The severity boundaries are pinned at 59.9 / 60 / 84.9 / 85 / 100 / 101.
  A hygiene test backs the time-zone property with a grep: nothing in the workspace reads
  `TZ` or converts to local time, because a property test cannot see a dependency that has
  not been written yet.
- **94 new tests (343 in the workspace) and nine more in the panel (26)**, every loop test
  driving `Engine::tick` with a clock it moves by hand rather than by waiting — a debounce, a
  sixty-second tick and an eight-hour sleep are one line each. Three of the panel's new tests
  are a cross-language gate: a command the panel invokes and a command the Rust side
  registers are the same string in two files no compiler reads together, and a typo there
  shows up as a panel that draws nothing, at run time, on somebody else's machine.

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
  its single writer, and WP3 gave it one — plus `--print --write` for the callers that want
  a refresh without a tray.
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

- **WP3 added no dependencies.** 542 packages in the lock file before, 542 after. Three
  crates were considered and each was measured rather than argued about. **`notify`** would
  add two packages on Windows (`notify`, `notify-types`; `filetime`, `walkdir`, `same-file`,
  `crossbeam-channel` and `log` are already there through Tauri) and more on Linux and macOS,
  for a five-second latency improvement on a display whose slowest input redraws every thirty
  seconds — and the readers already list those directories on every refresh, so the poll is
  one extra `read_dir` of a handful of entries rather than a second thread with a platform
  backend. **`tauri-plugin-single-instance`** would solve half of a problem the advisory lock
  has to solve anyway, and brings a D-Bus stack on Linux. **`proptest`** would add about ten
  packages to test rules that fit on one page; the generator here is a four-instruction
  xorshift that prints a seed a failure can be replayed from. Each decision is written down
  where the code is, not in a commit message: `refresh/watch.rs`, `main.rs` and
  `state/tests.rs`.
- **`updatedAt` now means "when the content last changed".** It always said "when the tray
  last wrote the file", and that is still exactly what it is — the writer only writes on a
  change. A consumer that was using it as a liveness signal for the tray should read
  `~/.nazar/limits.lock`'s `heartbeatAt` instead, which advances every minute regardless.
- **A tray that is killed rather than quit still keeps its lock for five minutes.** WP4's
  menu means it need not be killed: `Quit` stops the loop, which releases the lock. Deleting
  `~/.nazar/limits.lock` by hand remains the escape after a crash, and is safe when no tray
  is running.
- **The right button opens the menu, not the panel.** `docs/PROJECT.md` section 7 asks for
  "panel opens on left/right click"; with a native menu attached, the right button belongs to
  the shell. The panel opens on a left click and from the menu's first item, `Open`. Showing
  both at once was tried: the menu takes the focus, the panel blurs behind it, and the
  blur-suppression that keeps it alive then leaves a panel nobody can dismiss.
- **WP4 added no dependencies either.** `Cargo.lock` and `ui/package-lock.json` are byte
  for byte what they were before it. `tiny-skia` was
  pre-approved for the icon and was not needed — the picture is four circles and a horizontal
  cut — and the PNG writer behind `--icons` is a fixed-Huffman deflate in eighty lines, which
  puts the documentation strip at 15 KB rather than the 130 KB uncompressed blocks would have
  cost. `ui/test/icon-strip.test.mjs` inflates the committed file with Node's own zlib, so
  that encoder is checked by a decoder nobody in this repository wrote.
- **The derived view now leaves empty fields out rather than sending `null`.** `SnapshotView`
  is the panel's wire shape and nothing else reads it; `"percent": null` becomes `0` after
  one careless `Math.floor`, which is finding B03 arriving through the back door. The panel
  is defensive about it as well.
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
  when old becomes stale. That threshold belongs to the state model, which WP3 put in the
  settings so that all three displays answer it the same way — the retired prototype had
  three different ones.
- The tray still shows a static bead. The bead that fills from the bottom with the binding
  window is WP4 (decision K3, drawn in Rust per scale factor); WP3 computes the fill level
  and the colour but draws neither.
- `tauri-plugin-notification`, `tauri-plugin-autostart` and `tauri-plugin-updater` are
  declared but not initialised. They are wired in WP5 and WP7.
