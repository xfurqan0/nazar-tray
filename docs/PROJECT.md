# nazar-tray — project notes and v1 plan

Internal project notes. Kept in English so the repo is readable by everyone.

> **The system-tray face of Nazar.** Shows your Claude Code and Codex usage windows (5-hour and weekly) in the tray, with a bead icon that fills up as you burn quota. Writes the same `limits.json` that the Nazar canvas reads. **By default it reads nothing but local files: no credentials, no network.** An opt-in "detailed windows" mode adds the model-scoped weekly windows from the official usage endpoint for users who need them.

- Folder: `C:\nazar-tray` · Repo: `github.com/xfurqan0/nazar-tray` — private until first release
- Started: 2026-09-07 · License: MIT · Siblings: [Nazar](https://github.com/xfurqan0/nazar) (canvas monitor), Dile (dictation)
- Research phase closed 2026-09-07 03:00: market study (8 competitors, issue clustering, stack and distribution analysis) + code audit of the existing prototype (36 findings). Internal notes hold the full reports.
- Order: **second of the three projects to code** (maintainer decision 2026-09-07). Nazar is first — it is the CV piece and its Node/TS stack needs neither the new machine nor a Tauri/Rust toolchain; Dile is third. WP0 here is also where the shared Tauri v2 / CI skeleton that Dile copies is born.
- **Policy (maintainer):** quality over schedule, no deadline. Work packages are ordered; a package is done before the next starts. Code starts only when the maintainer says go.

---

## 1. Why this exists, and why it is not "the 20th tray"

A tray showing AI-agent quota is a crowded niche: Token Monitor (2.0k★), Win-CodexBar (1.0k★), CodeZeno (447★), aqua5230/usage (309★), and a dozen smaller ones, most active this week. Nothing wins on "tray + progress bars".

What the research found that nobody ships:

1. **Zero credentials, zero network — end to end.** Every competitor reads OAuth tokens or browser cookies from inside an unsigned binary and calls an undocumented endpoint that returns 429 under load. It turns out the data is already on disk:
   - **Codex** writes its server-reported quota into every session log: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` carries `rate_limits.primary/secondary` with `used_percent`, `window_minutes` (300 / 10080), `resets_at`, `plan_type`. Verified on the maintainer's machine (13 writes in one session).
   - **Claude Code** hands the same numbers to the status-line command on every refresh: `rate_limits.five_hour.used_percentage`, `rate_limits.seven_day.used_percentage`, `resets_at` (documented at code.claude.com/docs/en/statusline).
   So nazar-tray needs no fetcher, no token, no HTTP client. This also keeps it clearly outside Anthropic's terms, which forbid tools that "collect, store, or intermediate Claude.ai credentials or session tokens". *(Read as written: this is the **default** path, and it is the whole product for anyone who never changes a setting. The opt-in detailed-windows mode decided in §3 and built in WP2b does have an HTTP client and does read a token — behind a switch, in memory, for one request. `docs/detailed-windows.md` argues that case in full, including against this same clause.)*
2. **Threshold notifications.** The two Windows leaders do not mention notifications at all. Being warned at 85 % before hitting the wall is the reason a quota tray exists.
3. **Multi-account is the most requested feature and nobody has it.** CodexBar's maintainer: token/cookie-based fetching "will not work if we ever wanna support multiple accounts". A file-based design gets it almost for free (v2).
4. **Part of Nazar.** The same `limits.json` drives the quota strip on the Nazar canvas. The tray stays a separately downloadable app (maintainer decision); it is not folded into Nazar.

## 2. Scope and non-goals

| nazar-tray **is** | nazar-tray **is not** |
|---|---|
| Tray icon + popup panel: Claude and Codex windows, percentages, reset countdowns | A launcher, session manager, or cost analytics dashboard |
| A passive reader of local files that writes `~/.nazar/limits.json` | Anything that **stores or forwards** a token, or reads one without being asked to. WP2b's opt-in mode reads one, in memory, for one request, only with the switch on |
| Windows release first; macOS and Linux builds later from the same code | A Windows-only codebase |
| Six UI languages from day one: EN, TR, ZH, KO, RU, ES | — |

## 3. Decisions taken in the research phase (2026-09-07)

| Topic | Decision | Why |
|---|---|---|
| Data source | **Passive by default, hybrid on request.** Codex: tail the newest `rollout-*.jsonl` (both windows present). Claude: a tiny status-line wrapper records the payload's `rate_limits` (5-hour + global 7-day) and then runs the user's existing status-line command unchanged. **Opt-in "detailed windows" mode** (settings, default off): reads the Claude Code OAuth token in memory only and calls the official usage endpoint to get model-scoped weekly windows (e.g. a Fable-only weekly cap). The app suggests the mode once when it detects a Max plan; the user decides. | Verified 2026-09-07 03:05 on the maintainer's machine: status-line `seven_day` = 18 % (global), official endpoint reported Fable weekly = 23 % — the binding window for a Fable-heavy Max user is invisible to the passive path. Passive still removes 429s, the scheduled task, the lock file and ~120 lines of shell glue for everyone; the hybrid keeps the number honest for Max users. |
| Stack | **Tauri v2** (Rust core + small web panel). Ship **Windows only** in v1; macOS build later; Linux via CLI output and the Nazar canvas (no stack supports a tray popup reliably on Linux; even 21k★ CodexBar closed its Linux tray issue as not planned). | Tauri 5 MB vs Electron 105 MB; tray, notification, autostart and updater are first-party plugins. Windows-only → cross-platform is a ~20 % step; WinForms → anything is a rewrite. Same stack as Dile. |
| Existing C# prototype | **Not migrated.** Used as the behavioral spec; its good decisions are listed in §5. | The repo has no code yet, so the cost of not porting is zero today and grows after the first package. |
| Refresh | **Inside the tray process.** A 60 s tick plus a 5 s look at the files that feed it; no OS scheduler. | Kills the Task Scheduler / launchd / systemd triple, the VBS launcher and the lock race the audit proved from logs. *(WP3 built the "file watchers" half as a five-second poll rather than with `notify`: the readers already list those directories, and a watcher would add packages and a second thread for a five-second latency improvement on a display whose slowest input redraws every thirty seconds. Reasoning in `crates/nazar-core/src/refresh/watch.rs`.)* |
| i18n | JSON locale files, system language auto-detected, override in settings. EN and TR by the maintainer; ZH, KO, RU, ES machine-translated first, corrections via PR. | Win-CodexBar ships five languages; table stakes. |
| Distribution | GitHub Releases + **winget** (accepts unsigned installers) + apply to **SignPath Foundation** for a free OSS Windows OV certificate. macOS notarization ($99/yr) only with the macOS build; Homebrew cask needs 225★. | EV certs no longer bypass SmartScreen (2024). Budget ceiling ≈ $126/yr. |
| Visual identity | Nazar bead: deep blue / light blue / white / black dot. Icon = bead filling from the bottom with the binding window; amber ≥ 60 %, red ≥ 85 %, **grey = unknown** (never a reassuring "0"). Panel on navy. Themes as JSON: `nazar` (default), `graphite`. | Drops the Apple-grey look inherited from Win-CodexBar; inspiration credited in README, not in code. |

## 4. Known limits of the passive design (write them in the README)

- Claude numbers update only while a Claude Code session refreshes its status line. Between sessions the tray shows the last value with its age and a countdown computed locally from `resets_at`. Quota does not burn while you are not using it, so this is honest, not stale.
- The status-line payload carries only the 5-hour and the global 7-day window (documented; verified live). Model-scoped weekly windows come only from the official usage endpoint, so they need the opt-in **detailed windows** mode. In that mode the token is read from `~/.claude/.credentials.json`, held in memory for one request, never written or logged; the README states this in plain words, and the mode is off by default. The endpoint is undocumented and rate-limited (429); detailed mode keeps last-good values and backs off.
- Users who already run a custom status line keep it: the wrapper chains to it. Users without one get a minimal default. Contract, files, permissions and the by-hand uninstall: `docs/statusline-wrapper.md`.
- Codex log format is not a documented contract; the reader is defensive (schema-tolerant, keeps last good value, reports "unknown" rather than guessing).
- Unsigned until SignPath approval; SmartScreen will warn. winget install avoids the browser download warning. The application itself waits until after the first release, because SignPath Foundation asks that a project already be released and be actively maintained — `docs/CODE_SIGNING.md` has the whole plan and the CI job that is written and switched off.
- Uninstalling keeps two directories on purpose. `~/.nazar` holds `limits.json`, which Nazar reads and which may outlive this tray, plus the status-line captures and the record of what the wrapper replaced; **no path through the uninstaller touches it.** `%APPDATA%\nazar` holds the settings and the notification history and goes only when the user ticks *delete application data*, which a silent uninstall never asks. Both are named in the README and in `docs/RELEASE.md`.

## 5. What the audit said to keep and to fix

**Keep from the prototype (proven, port the behavior):** split between data producer and display · last-good data with an explicit stale flag · never mix response bodies into error text · one function measures and draws the panel · rotate icon handles to avoid GDI leaks · single-instance guard.

**Fix (all 🔴/🟠 findings become acceptance criteria):**
- On any read error the icon shows **unknown** (grey, "?"), never `0`.
- **Binding window** = the window with the highest percentage across all windows of a provider; never invented from a flag that the source does not provide.
- Atomic writes: temp file + rename. Single writer (the tray process).
- Reset math in the user's local time zone; percent rounding at display time only.
- Windows 11 tray overflow: first-run hint to pin the icon.
- DPI-aware rendering (Tauri handles the window; icon rendered per scale factor).
- Config at `%APPDATA%\nazar\config.json`; data at `~/.nazar/limits.json`. No absolute paths in code.
- Transcript scanning for "estimated usage" (the prototype read up to 400 transcript files on 401) is **dropped**; no estimates, only reported numbers.

## 6. `limits.json` contract (frozen at v1, shared with Nazar)

```json
{
  "schemaVersion": 1,
  "updatedAt": "2026-09-06T21:12:34Z",
  "providers": {
    "claude": {
      "configured": true,
      "plan": "max_20x",
      "sourceAt": "2026-09-06T21:12:30Z",
      "source": "statusline | endpoint",
      "binding": "seven_day_fable",
      "windows": {
        "five_hour":        { "percent": 12, "windowMinutes": 300,   "resetsAt": "2026-09-07T03:10:00Z", "state": "ok" },
        "seven_day":        { "percent": 18, "windowMinutes": 10080, "resetsAt": "2026-09-12T02:00:00Z", "state": "ok" },
        "seven_day_fable":  { "percent": 23, "windowMinutes": 10080, "resetsAt": "2026-09-12T02:00:00Z", "state": "ok", "model": "Fable", "detailed": true }
      }
    },
    "codex": {
      "configured": true,
      "plan": "plus",
      "sourceAt": "…",
      "source": "rollout",
      "binding": "secondary",
      "windows": {
        "primary":   { "percent": 54, "windowMinutes": 300,   "resetsAt": "…", "state": "ok" },
        "secondary": { "percent": 70, "windowMinutes": 10080, "resetsAt": "…", "state": "ok" }
      }
    }
  }
}
```
No tokens, no account ids, no e-mail. `configured:false` when the provider's files are absent. Nazar reads this file and nothing else from nazar-tray.

**Timestamps are RFC 3339 in UTC (`…Z`)** — decided in WP1 and applied to this sample in WP2. The standard library has no time-zone database, a UTC stamp names the same instant a local offset would, and a countdown is offset-independent; consumers render local time. Reasoning in `docs/pinned-internal-formats.md`.

Added after the Nazar data-layer audit (2026-09-07 04:40): every window carries `"state": "ok" | "stale" | "error"` plus an optional `"error"` string (the prototype had `stale`+`lastError`; the first frozen draft lost them); every provider carries `"source"` (`statusline | endpoint` for Claude, `rollout` for Codex); Claude windows carry `windowMinutes` (300 / 10080) like Codex so consumers need no provider-specific logic. The status-line wrapper keys its capture file by `session_id`, never a single fixed path (three concurrent sessions would otherwise overwrite each other).

## 7. Work packages (ordered; no dates)

| # | Package | Done when |
|---|---|---|
| WP0 | Repo skeleton: Tauri v2 project, Rust core crate, CI (GitHub Actions, Windows build + tests), locale scaffolding with EN/TR, `limits.json` contract doc | `cargo test` and a Windows build pass in CI on an empty tray |
| WP1 | **Codex reader**: find newest `rollout-*.jsonl`, tail for `rate_limits`, tolerate schema drift, fixtures from real logs | Unit tests on captured fixtures incl. missing/partial fields; live value matches Codex `/status` on the maintainer's machine |
| WP2 ✅ | **Claude reader**: `nazar-statusline` wrapper (records `rate_limits`, chains to existing command, < 50 ms), installer merges into `~/.claude/settings.json` without clobbering | ~~Works with no status line, with ccstatusline, with the maintainer's custom script; uninstall restores the previous command exactly~~ — **landed 2026-09-07**, all three round-trip byte for byte |
| WP2b ✅ | **Detailed windows mode (opt-in)**: settings toggle, token read in memory only, official usage endpoint client with backoff and last-good cache, model-scoped windows merged into the state with `detailed:true`, Max-plan detection prompt | ~~Off by default; with the toggle on, the Fable weekly window appears and matches `/usage`; token never appears in any file, log, or error text (test asserts it); 429 leaves last value with a stale flag~~ — **landed 2026-09-07**. The UI half of the prompt is WP5's; the decision function and its `detailedSuggested` flag are here. |
| WP3 ✅ | **State model**: binding window across passive + detailed windows, staleness/age, local countdown, unknown state, atomic `limits.json` writer, single-instance | ~~Property tests on window selection and reset math across time zones~~ — **landed 2026-09-07**, and the refresh loop, the write-on-change rule and the advisory lock came with it |
| WP4 ✅ | **Tray icon + panel**: bead icon per scale factor, navy panel near cursor, both providers, countdowns, theme JSON (`nazar`, `graphite`) | ~~Screenshots at 100/150/200 % DPI; panel opens on left/right click, closes on Esc/blur~~ — **landed 2026-09-07**. Screenshots taken at all three scales; the panel opens on a left click and from the menu's `Open` on a right click (see the log below), and closes on Esc and on blur. Quit came with it. |
| WP5 ✅ | **Notifications, autostart, settings**: thresholds 60/85/100 once per window per reset, autostart toggle, language override, theme, per-provider enable | ~~Toast appears exactly once when crossing 85 %; survives sleep/wake~~ — **landed 2026-09-07**. Verified live with `--demo-cross`: one toast at 60 % on arrival, one at 85 % for the crossing, **nothing** for the repeat, one more at 85 % after the reset. Sleep and wake are a test on an injected clock, and a restart on a real machine fired nothing the first run had already said. |
| WP6 ✅ | **i18n**: ZH, KO, RU, ES translations (machine first), pluralization and RTL-safe layout check, language switch without restart | ~~Every UI string comes from locale files; no hard-coded text in code~~ — **landed 2026-09-07**, and the criterion is now two tests that grep the sources rather than a promise |
| WP7 ✅ | **Distribution**: NSIS installer via the Tauri bundler, winget manifests, SignPath preparation, README (EN), CHANGELOG, first release checklist | ~~Fresh Windows VM: `winget install` → tray running in 2 minutes; uninstall leaves no files~~ — **landed 2026-09-07**. Measured on the maintainer's machine rather than a VM (see the log): silent install → **tray up in 4.4 s**, Start Menu entry, no elevation; silent uninstall → nothing left but the two directories that are kept on purpose and named in the README. MSI **builds cleanly** (2.70 MB, WiX downloaded by the bundler) but is **not shipped** — see the log. |
| WP8 | **Nazar handoff**: contract doc, sample files, a `nazar-tray --print` CLI that dumps `limits.json` (also the Linux story) | Nazar canvas reads the file on the maintainer's machine |
| T-WP8 ✅ | **The status line that was installed and never ran** (from live use; the `T-` prefix is this repository's own late-package prefix, as `N-` is Nazar's, and this is not WP8 above). `install` writes the program path with **forward slashes** on Windows, because Claude Code runs `statusLine.command` through Git Bash and an unquoted backslash is bash's escape character; every earlier spelling is recognised as the same installation; a broken one is **repaired** rather than reported as "already installed"; `status` names the shell and the fix; a path out of a build directory is warned about. The data directory against the settings directory is written into `docs/limits-contract.md` as a contract rather than left to be inferred | ~~The wrapper produces a capture on a Windows machine with a fresh install~~ — **landed 2026-09-08**. `status` on the maintainer's machine printed `installed: yes` for a day while nothing was ever spawned; it now prints the warning, and `install --dry-run` shows the rewrite. 459 tests, from 451 |
| T-WP9 ✅ | **The toast storm on a jittering `resetsAt`** (from live use). `crate::alerts` rule 4 asked "is this the same reset period?" by comparing two strings; the usage endpoint answers the same weekly reset one second apart on alternating refreshes, so every flip read as a new week and re-fired the whole ladder. Two resets are now one period when they are **less than half a window apart** (an hour when the window's length is unknown), on both the in-memory and the on-disk half of the rule, and the Claude reader rounds `resets_at` **down to the whole minute** so both sources spell one instant one way | ~~A window whose `resetsAt` wobbles produces one toast per crossing and no more, and `alerts.json` stops being rewritten~~ — **landed 2026-09-08**. 32 toasts in two and a half hours on the maintainer's machine, all of them the same sentence; the sequence that produced them is now four tests. 465 tests, from 459 |

Going public happens after WP7, not before. **WP7 does not include going public**: the tag,
the GitHub Release, the winget pull request, the SignPath application and the repository's
visibility are five one-way doors, and every one of them is a command in `docs/RELEASE.md`
for the maintainer to run. Nothing in this repository runs them.

## 8. Open decisions
- ~~Opt-in official-endpoint mode~~ → **decided 2026-09-07 03:20: in v1 as WP2b, off by default** (maintainer approved; the maintainer's own binding window is the Fable weekly, invisible to the passive path).
- ~~Panel technology inside Tauri: plain HTML/CSS vs. a tiny framework~~ → **closed 2026-09-07 in WP0: plain TypeScript, HTML and CSS, bundled by esbuild, no framework** (decision K2). The panel's only runtime dependency is `@tauri-apps/api`; `ui/` has two dev dependencies, esbuild and TypeScript.
- ~~Icon rendering: pre-rendered bead PNG set per fill level vs. runtime drawing~~ → **decided 2026-09-07 in WP4: drawn at run time in Rust, and without `tiny-skia`** (decision K3). A pre-rendered set is one file per fill level per severity per freshness per scale, invalidated all at once by a theme change; the drawing is four circles and a horizontal cut, which is a hundred lines of arithmetic and no dependency. `crates/nazar-tray/src/icon.rs`.
- Linux: CLI-only in v1 docs, or ship an AppIndicator without popup. Leaning: CLI-only, revisit with Nazar.

## 9. Log
- 2026-09-07 01:55 — Repo opened, first spec (C# migration plan). Decisions: separate downloadable app; nazar identity; English everywhere.
- 2026-09-07 02:30 — Maintainer: Windows first, infrastructure ready for macOS/Linux, six UI languages.
- 2026-09-07 02:35 — Maintainer: quality over schedule; no deadline; research phase before code on every repo.
- 2026-09-07 03:00 — Research phase closed. Plan rewritten: passive data sources (no credentials, no network), Tauri v2 from day one, C# prototype retired as behavioral spec, work packages WP0–WP8. Waiting for the maintainer's go.
- 2026-09-07 05:30 — **Order changed: nazar-tray is second, behind Nazar** (it was first). Nazar is the CV piece and runs on Node/TS, so it can start on the current PC while this repo waits for the Rust/Tauri toolchain on the new machine; Dile stays last. No package here changes: WP2's `nazar-statusline` wrapper is still owned by this repo and still the single implementation. What changes is its consumer's timing — Nazar ships v1 **without** the wrapper (quota strip hidden) and picks it up in its own v1.1, so WP2 no longer blocks anyone's first release, and WP8's handoff lands after both repos already exist.
- 2026-09-07 — **WP0 landed.** Cargo workspace (`nazar-core` + `nazar-tray`), Tauri v2 shell with tray and popup panel, the `limits.json` contract with its atomic writer and `docs/limits-contract.md`, EN/TR locales with four placeholders, theme tokens copied from Nazar, CI on three operating systems, and a licence gate. `cargo test --workspace` and `npm test` green; `cargo tauri build --debug` produces the exe and an NSIS installer.
- 2026-09-07 — **WP1 landed: the Codex reader.** `nazar-core::codex` locates the newest `rollout-*.jsonl` under `$CODEX_HOME` (default `~/.codex`), tails it by byte offset, and takes seven values out of `payload.rate_limits` and nothing else; 96 tests, a leak test with a sentinel in every text field, a credential grep over the workspace and a fixture-hygiene gate. Verified live against the maintainer's real logs: the weekly window read **70 % resetting 2026-09-07T12:24:52Z**, matching the official endpoint to the second, which also confirmed `resets_at` is Unix seconds. Two things the audit had not seen: a second limit family (`limit_id: "premium"`) whose windows are both `null` and which was the **last** quota line in two logs, and three logs with no quota line at all — both are now fixtures. `nazar-tray --print` dumps the document; the writer stays WP3's. Formats pinned in `docs/pinned-internal-formats.md`.
- 2026-09-07 — **WP2 landed: the Claude reader and the shared status-line wrapper.** A third crate, `nazar-statusline`, builds a 334 KB binary with no Tauri in it. As a wrapper it reads the payload, writes it **whole** to `~/.nazar/statusline/<session_id>.json` and runs the status line that was there before, forwarding its output and its exit code — **median 9.6 ms over ten runs** against a 50 ms budget, release build, no chained command. As an installer it edits exactly one key of `settings.json`, keeps every other key and their order, refuses on invalid JSON or a lock file, is idempotent, takes a backup it will never write over, and prints a unified diff whether or not anything is a terminal. All three shapes the acceptance criterion names — no status line, ccstatusline, the maintainer's own `node "…"` script — install, diff and uninstall back to the **byte-identical** original. `nazar-core::claude` maps a capture to `providers.claude` (two windows, `windowMinutes`, UTC `resetsAt`, `state`, computed `binding`), reports `configured:false` with no capture and `state:"error"` with no `rate_limits`, and derives no plan name because the payload does not carry one. 82 new tests (188 in the workspace), a second leak test, a second fixture-hygiene gate, and `nazar-tray --print` now prints both providers. Verified live against the maintainer's own status line: with `chain.json` pointing at `node "…\statusline.js"`, the wrapper captured the payload and the real script drew its usual coloured line unchanged, exit 0. Contract in `docs/statusline-wrapper.md`; formats pinned. **Nothing on the maintainer's machine was written: the real-machine check was `install --dry-run`, verified by a SHA-256 taken either side.**
- 2026-09-07 03:20 — Live check showed the status-line payload lacks model-scoped weekly windows (global 7-day 18 % vs Fable weekly 23 % on the maintainer's machine). Decision: hybrid. Passive by default; opt-in **detailed windows** mode (WP2b) reads the token in memory only and calls the official endpoint. Claim reworded to "by default". Raw status-line payload captured as a WP2 fixture.
- 2026-09-07 — **WP3 landed: the state model, the refresh loop and the writer. The tray
  works.** Three ideas, and the audit is the argument for each.
  **(1) Nothing derived is stored.** `nazar-core::state` computes the binding window, the
  countdown, the age, the freshness and the severity from the document and an instant, at
  read time, every time. `limits.json` keeps only what a source measured. The countdown goes
  **negative** when a reset is already due rather than freezing at "now" — scenario S7 in the
  audit — and `unknown` is a real value that sorts *below* `ok`, so a provider nobody could
  read never colours the icon on its own (B03). One binding rule, one staleness threshold, in
  the settings, read by every display (B04, B14, B16).
  **(2) The loop is inside the process.** One thread owns every reader. Startup, a 60-second
  tick, a five-second look at the capture directory and at the rollout log being followed, an
  explicit request, and a clock jump of more than two ticks — which is how a laptop notices it
  has been asleep. A 250 ms debounce coalesces everything but the tick, so a Codex session
  writing a quota line every few seconds still costs one refresh. That deletes the scheduled
  task, the VBS launcher, the PowerShell wrapper and the launchd/systemd work that was never
  written, and it turns B01's cross-process lock race into a thread that cannot race itself.
  **(3) The writer mostly does not write.** Each refresh is compared with the last, ignoring
  `updatedAt`; an identical document is not written at all. So `updatedAt` moves with the
  content, and "is the tray alive" is answered by the heartbeat in `~/.nazar/limits.lock`
  instead — two questions, two answers, where the prototype had one that was wrong for both.
  The lock is taken with a single `create_new` (B01), reclaimed after five minutes of silence,
  and re-checked immediately **before** each write, so a tray whose lock was taken over while
  its machine slept stops rather than overwrites. The same lock is the single-instance guard:
  a second launch leaves `~/.nazar/tray.request`, the running tray notices it within five
  seconds and opens its panel, and the second process exits.
  **No new dependencies.** 542 packages before, 542 after. `notify` would have added two on
  Windows and more elsewhere for a five-second latency improvement on a display whose slowest
  input refreshes every thirty seconds, and the readers already list those directories;
  `tauri-plugin-single-instance` would have added a D-Bus stack on Linux for a guarantee the
  contract needs anyway; `proptest` would have added ten packages to test rules that fit in a
  page. The reasoning for each is written where the decision lives, not in a commit message.
  **94 new tests (343 in the workspace) and nine more in the panel**, including property tests over ten
  thousand random window sets and three thousand random instants in six time-zone spellings,
  plus a grep proving nothing in the workspace reads `TZ`.
  **Verified live on the maintainer's machine**: the tray wrote `~/.nazar/limits.json` one
  second after launch with the Codex numbers **54 % and 70 %** and Claude `configured:false`
  (the wrapper is not installed here, and the opt-in mode stayed off); over the following two
  minutes the numbers did not move and **the file was not rewritten once** while the lock's
  heartbeat kept advancing; `--print --write` while the tray was running printed the tray's
  document, said so, and changed nothing; and a second launch exited with one process still on
  the tray. `~/.claude/settings.json` was not touched — same SHA-256 before and after.
- 2026-09-07 — **WP4 landed: the bead, the panel and the way out.** Three things, and the
  third is the one that mattered most.
  **(1) The icon is drawn, not shipped** (decision K3, `icon.rs`). A bead rendered in Rust
  for the current scale factor — 16 px at 100 %, 24 at 150 %, 32 at 200 %, which is what
  `SM_CXSMICON` asks for — with a deep-blue rim, a white chamber that *is* what empty looks
  like, and a fill rising from the bottom with the **binding window across both providers**.
  Light blue, amber, red by severity; a black pupil when a window is spent, so the state
  survives a greyscale screenshot; a **grey rim and a hollow ring when nothing could be read
  — never a fill** (finding B03); and the colour drained by freshness, so an icon nobody has
  fed in an hour does not look as confident as one from a second ago. `tiny-skia` was the
  leaning in section 8 and was not needed: the picture is four circles and a cut, the hexes
  come from `ui/theme.nazar.json` (a test parses that file and fails on drift), and the
  tooltip — `Claude Fable week 88 % · Codex week 70 % (resets in 2 h 10 m)` — is built from
  the same locale files the panel reads, because the tray is UI too.
  **(2) The panel is designed.** A card per provider on navy: the lobehub mark (MIT, licence
  in `ui/assets/`), the plan as the source spelled it, and a freshness sentence. A row per
  window: its name taken from `windowMinutes` rather than from the provider, a `detailed`
  mark on the model-scoped weeklies, a bar in the bead's colours, and a countdown the panel
  computes itself every second — a clock below a day, `4 d 2 h` above it. **An unknown window
  gets the word, a hatched empty track and the reader's own sentence, never a bar of zero
  length.** The first-run hint about the Windows 11 overflow appears once and is dismissed
  for good in `config.json`; the footer refreshes, toggles the theme and says that Esc
  closes. The panel measures itself and asks Rust for a window that fits, so the hint can
  come and go without leaving a gap. Every text pair clears **WCAG AA in both themes and
  both modes**, which is a test rather than an intention (`ui/test/contrast.test.mjs`); the
  meter fill moved to the derived accent tone because the raw one is 2.25:1 on a light track.
  **(3) Quit, which is a bug fix.** WP3's note that the tray deliberately had no menu was
  written before it had a lock: a killed process leaves `~/.nazar/limits.lock` behind for its
  five-minute grace period, and the next launch then starts as a reader and looks broken. The
  tray now has the Windows-native menu — `Open`, `Refresh now`, `Quit` — and every way out
  goes through the loop's shutdown first. **The cost, and it is a real deviation from this
  section's WP4 row:** with a menu attached, the right button belongs to the shell, so the
  panel opens on a left click and from the menu's first item on a right click. Both at once
  was tried and is worse than either — the menu takes the focus, the panel blurs behind it,
  and the suppression below then leaves a panel nobody can dismiss.
  **The overflow race, fixed and measured.** A click on a tray icon *inside* the Windows 11
  overflow flyout shows the panel and then closes the flyout, which blurs the panel about
  200 ms later and hid it instantly. The panel now ignores a blur for 300 ms after a
  tray-initiated open **and takes the focus back**, so the next click elsewhere still closes
  it. Verified by driving the real flyout: the panel was still visible and focused at 250 ms,
  1250 ms and 3250 ms.
  **No new dependencies.** `Cargo.lock` and `ui/package-lock.json` are byte for byte what
  they were before this package. `tiny-skia` was pre-approved and not needed; the PNG writer behind `--icons` is a fixed-Huffman deflate in
  eighty lines (the documentation strip is 15 KB rather than the 130 KB stored blocks would
  have cost), and `ui/test/icon-strip.test.mjs` inflates the committed file with Node's own
  zlib, so the encoder is checked by a decoder nobody here wrote.
  **37 new tests in Rust (381 in the workspace) and 25 more in the panel (51).**
  **Verified live on the maintainer's machine**: the bead appeared in the overflow with the
  tooltip `Codex week 70 % (resets in 1 h 56 m)`; clicking it from the flyout opened the
  panel and it stayed; Esc closed it; the right button showed `Open / Refresh now / Quit`;
  **Quit ended the process and removed `~/.nazar/limits.lock`, where a kill left it behind.**
  A second run under a throwaway `NAZAR_HOME` clicked *Got it* and the theme toggle and found
  `firstRunHintDismissed: true` and `theme: "graphite"` in the settings it wrote — so the
  real `%APPDATA%\nazar` still does not exist, and neither the screenshots nor the tests
  touched it.
- 2026-09-07 — **WP2b landed: the detailed-windows mode, off by default.** `nazar-core::claude::detailed` reads four values out of `.credentials.json` — and only while the mode is on — holds the token in a wiping wrapper for one 20-second `GET` to `api.anthropic.com/api/oauth/usage`, and maps `limits[]` into `five_hour`, `seven_day` and `seven_day_<model>`. **61 new tests (249 in the workspace)** against a hand-rolled loopback HTTP server: every status code, the backoff schedule on an injected clock, last-good/stale semantics, both response shapes, plan normalisation, binding across scoped windows, settings round-trip, and four gates — a sentinel token that must appear in no file, no error and no `Debug`; a one-call-site grep on the method that exposes it; no printing macro anywhere in the module; and the credential gate grown into an allow-listed **directory** rather than a dropped needle. The mode is behind the `detailed-windows` cargo feature as well as the runtime flag, so `cargo tree -p nazar-statusline` still shows `serde` and `serde_json` and nothing else. HTTP client chosen by measurement: `ureq` + `rustls` adds **4** packages, `reqwest` with `blocking` adds **17** including `aws-lc-sys` and a `cmake` build. Three findings from the audit became behaviour: `Retry-After` is read (B07), a token whose stored expiry has passed costs no request and a rewritten sign-in file clears the backoff at once (B06), and a plan change drops the remembered numbers (B25). Verified live: **session 2 %, weekly 38 %, Fable weekly 30 %**, `plan: max_20x`, matching an independent reading of the same endpoint seven minutes earlier — and **nothing was written**: `~/.nazar` and `%APPDATA%\nazar` did not exist before or after, and `.credentials.json` kept its size and modification time. The live check also found that the endpoint spells `resets_at` as `2026-09-07T13:10:00.130195+00:00` where the status line writes Unix seconds; both are now rewritten into the contract's `…Z`. Written up in `docs/detailed-windows.md`; precedence in `docs/limits-contract.md`; both new formats pinned.
- 2026-09-07 — **WP5 landed: it warns you, it starts with Windows, and it has settings.**
  Three things, and the first is the reason section 1 says this is not the twentieth tray.
  **(1) The warning, and the file that makes it bearable** (`nazar-core::alerts`,
  `crates/nazar-tray/src/alerts.rs`). A crossing is **edge-triggered** — `previous < T ≤
  current` — so a window sitting above a threshold says nothing for the rest of the week, and
  the key `(provider, window, threshold, resetsAt)` is written to
  `%APPDATA%\nazar\alerts.json` **before the toast is shown**, so a restart does not repeat
  a warning the user has already had. Four rules fell out of writing it down and each is a
  bug that would otherwise have shipped: *the first observation counts* (a tray started at
  91 % has no previous reading, and waiting for a crossing that has already happened would be
  silence when it matters most); *a reset clears the memory as well as the keys* (a window
  that resets from 86 % straight back to 86 % has to warn again, which is exactly what
  `--demo-cross`'s fourth step is); *unknown never notifies and forgets what it saw*, so the
  reading after an unreadable one is a first observation rather than a continuation of a
  number nobody can vouch for (finding B03 through a new door); and **one toast per crossing,
  naming the most severe threshold reached** — a jump from 10 % to 91 % crosses 60 and 85
  together, both are consumed, and one sentence that says 85 % is worth more than two stacked
  on each other. Quiet hours and the notifications switch arrive at the state machine as the
  same flag: the crossing still happens, is still recorded and still colours the icon, only
  the interruption is withheld — which is what makes switching the notifications back on
  quiet rather than a burst of catching up.
  **(2) Autostart, read back from the machine.** `tauri-plugin-autostart` writes
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` with `--hidden` appended, and the
  switch in the settings asks the plugin what the registry says rather than remembering an
  answer of its own — so it agrees with Task Manager's Startup tab, which is where people
  actually look. `--autostart on|off|status` does the same thing from a terminal, for a user
  whose panel will not open and for anybody checking this package.
  **(2b) The offer had to become a question.** §3 says the app "suggests the mode once when
  it detects a Max plan", and WP2b built `should_suggest_detailed` to do exactly that — but
  writing the UI half showed that it can never fire on the path it is for: WP2 derives **no
  plan name at all** from the status-line payload, because the payload does not carry one, so
  on a machine that has never turned the mode on `plan` is simply absent. The rule kept its
  plan branch, for the case where the endpoint has already answered, and gained the branch
  that actually matters: **Claude is set up, no plan is visible, and the numbers did not come
  from the endpoint → ask, once.** It is not offered to somebody who has the mode on, has
  already been asked, or does not use Claude Code.
  **(3) Settings, and the WP4 mismatch they close.** A second view in the same window:
  language, theme, light/dark, per-provider on/off, the three thresholds, quiet hours,
  autostart, the detailed-windows switch with the plain-words explanation and its one-time
  Max-plan offer, where every file lives with `~` collapsed, and a way to bring the first-run
  tip back. **The language is now decided once, in Rust** — the override, then the operating
  system's UI language, then English — and handed to the panel as well as used for the
  tooltip; WP4 had the panel guessing from `navigator.languages` while the tray fell back to
  English, and wrote that down as an open risk. Changing it **rebuilds the tray menu**, since
  a menu item's text is fixed when the item is built. The form is validated before anything
  is written and refused as a whole: `85 / 60 / 100` produces an error and the settings the
  user had, not an error and a tray that has half changed.
  **Two new dependencies, both first-party, both already declared in WP0**:
  `tauri-plugin-notification` 2.4.0 and `tauri-plugin-autostart` 2.5.1 (MIT OR Apache-2.0,
  and the licence gate passes). **The panel is granted neither.** Both are driven from Rust —
  the toast comes from the refresh loop, the switch goes through this application's own
  commands — so the webview never invokes a plugin command, and `capabilities/default.json`
  stays at `core:default` plus `core:window:allow-hide`. The minimal grant for a plugin
  nobody calls from the front end is no grant at all.
  **What could not be done, and it is the plugin's limit rather than a decision:
  clicking a toast cannot open the panel.** `tauri-plugin-notification` 2.4 builds the
  notification, spawns `show()` onto the async runtime and drops the handle, so the
  `on_activated` callback `notify-rust` does offer on Windows never reaches an application;
  `action_type_id` is mobile-only. The tray icon a click away is the workaround, and the toast
  names the window it is about so that click is an informed one.
  **One deliberate deviation from `nazar-core`'s no-dependency rule**, written up in
  `crates/nazar-tray/src/system.rs`: quiet hours are **wall-clock** hours, and the standard
  library has no time-zone database. Rather than spend one of the two allowed dependencies on
  a time crate — and `time::UtcOffset::current_local_offset` refuses to answer in a
  multi-threaded process anyway — two calls into `kernel32` are declared by hand,
  `GetLocalTime` and `GetUserDefaultLocaleName`, against an ABI that has not moved since
  Windows 2000. Everywhere that is not Windows answers `None`, and every caller is written for
  that: an unknown local time suppresses nothing, and an unknown language is English.
  **A settings change reaches the readers.** Switching a provider off has to mean the reader
  is *gone* rather than that its answer is discarded — nobody who does not use Codex should
  have a program listing their session directory every five seconds — so the refresh loop
  gained one command, `Reconfigure`, which replaces the readers and the watch list on the
  thread that owns them and refreshes at once. And it gained one event, `Refreshed`, emitted
  on **every** pass rather than only on a changed one: a tray started when the weekly window
  is already at 91 % changes nothing, and that is the case where the user most needs telling.
  **63 new tests in Rust (444 in the workspace) and 17 more in the panel (68)**, including
  the whole state machine on an injected clock and a temporary `NAZAR_HOME` — edge crossing,
  once per reset, reset clears, quiet hours, startup-above, **sleep-and-wake replay**, unknown
  never fires — the settings validation, the form's round trip, a gate proving the panel's
  validator and Rust's agree, and one proving `ui/src/i18n.ts` and `config.rs` list the same
  languages.
  **Verified live on the maintainer's machine.** `--demo-cross` produced exactly four
  toasts across its four steps: `Codex · weekly window 60 %` on arrival, `Codex · weekly
  window 85 %` for the crossing, **nothing at all** for the repeat, and one more
  `85 %` after the reset with the body moving from *Resets in 11 h 25 m* to *Resets in 7 d
  11 h*. Run again with `--locale tr` it produced the same three toasts in Turkish. A **real**
  run under a throwaway `NAZAR_HOME`, against this machine's own Codex logs, fired one 60 %
  toast and wrote `alerts.json` with `fired: [60.0]` against the real `resetsAt`; **a second
  run fired nothing** and changed the file not at all — and it came out in Turkish with no
  `--locale`, which is `GetUserDefaultLocaleName` working. The autostart round trip was run
  on the real registry and **left off**: the `Run` value appeared as
  `…\nazar-tray.exe --hidden` with the `StartupApproved` flag beside it, and disappeared
  again. Two findings for WP7's uninstaller came out of that: the plugin's `disable` leaves
  the `StartupApproved` value behind (inert, but residue — removed by hand here), and it
  writes the `Run` value **unquoted**, which works only because Windows tries each
  space-delimited prefix. **`%APPDATA%\nazar` still does not exist and `~/.nazar/limits.json`
  has the same SHA it started with**: every live check ran under `--demo` or under a
  throwaway `NAZAR_HOME`.
- 2026-09-07 — **WP6 landed: six languages, and a criterion that is now a test.**
  **(1) The translations.** ZH, KO, RU and ES filled in, **107 keys each**, machine-translated
  first exactly as section 3 says, with the review status and the correction route in
  `ui/locales/README.md` and a paragraph in the README pointing at it. Placeholders are
  identical per key across all six and free to be reordered; product names — `nazar-tray`,
  `Claude Code`, `Codex`, `Nazar` — are untranslated everywhere; and each language's own
  conventions are kept rather than English's, which is most visible on the percent sign
  (`88 %` in English, Russian and Spanish, `%88` in Turkish, `88%` in Chinese and Korean) and
  on the unit abbreviations the countdowns are built from.
  **(2) The `_meta` idea was tried and rejected, with evidence.** A `{"language": …,
  "machineTranslated": true}` object inside each file would have been convenient. It is also
  a **silent catastrophe**: `crates/nazar-tray/src/i18n.rs` parses each file as
  `BTreeMap<String, String>` and turns a parse failure into an *empty* catalogue — the right
  behaviour, since a damaged translation must not stop the tray from starting — so adding it
  to `zh.json` dropped Chinese out of `available()` altogether and everybody who had chosen it
  would have got English with no error anywhere. Watched happen, reverted, and now a test
  fails on a key beginning with `_` and on any value that is not a string. The metadata is a
  table in `ui/locales/README.md`, where nothing has to parse it.
  **(3) No plural forms, by construction rather than by luck.** Every counted string in this
  product renders its number beside a unit **abbreviation** — `4 d 2 h`, `2 sa 10 dk`,
  `2 小时 10 分`, `2시간 10분`, `2 ч 10 мин`, `2 h 10 min` — and an abbreviation is the same
  word after 1 as after 5, which is what carries Russian's three forms (1, then 2–4, then 5 and
  up, with 11–14 in neither of the first two) on a single template. `pluralCategory` and
  `plural` in `ui/src/i18n.ts` implement that rule for the first counted *word* anybody
  writes, and a test **freezes the seven keys that carry a count**, so an eighth fails the
  suite until somebody chooses between an abbreviation and the helper. **RTL is out of scope
  and written down as owed work**: all six languages are left to right, the panel sets
  `<html lang>` and never `dir`, and a test fails if an RTL tag is added to `LOCALES` before
  `styles.css` — still full of physical `margin-left` and `text-align: right` — has been
  audited.
  **(4) One source for the Rust side, which it already was.** WP5's `include_str!` of the same
  six `ui/locales/*.json` needed no refactor: there has never been a Rust translation table.
  What is new is the proof — a test **scans this crate's own sources** for every
  `catalog.text(…)` and `catalog.format(…)`, expands the two keys built from a provider's
  name, and asserts each exists in all six catalogues. The tooltip and the toast are then
  written out per language as assertions, because they are the two pieces of this product's
  text nobody can screenshot: the shell draws the tooltip on hover and Windows owns the toast.
  **(5) The acceptance criterion, as two greps.** *"Every UI string comes from locale files;
  no hard-coded text in code"* is now `ui/test/i18n.test.mjs`, which strips comments, Rust
  `#[cfg(test)]` modules and `*/tests.rs` files, blanks the argument of every `eprintln!`,
  `println!`, `panic!` and `.expect(…)`, and fails on any remaining literal that looks like a
  sentence. **The allow-list is three entries and each has a reason**: `"pill detailed"` and
  `"pill plan"` are CSS class names, and `"no quota line in the newest session log"` is the
  demo's stand-in for a reader's own error sentence — the one thing the panel prints verbatim,
  because it says which file said what and no translation can know that in advance. The
  command line stays English deliberately: `--print` emits JSON a script parses and
  `--autostart` answers a maintainer's question; there is no `--help` in this build, and when
  WP8 adds one it stays English for the same reason.
  **(6) Three layout bugs the six pictures found, all real.** Russian's
  *"устарело · последние данные 1 ч 10 мин назад"* was being **cut off with an ellipsis** in
  the provider card's first line, where English fits; that line now wraps, so the freshness
  drops to a line of its own and the panel — which measures itself after every render — grows
  to match. Korean's footer was breaking `설정` **down the middle**, so the footer now wraps
  whole buttons. And the settings help text was breaking `않습니다` the same way, because CSS's
  default `word-break` treats Hangul as it treats Chinese — correct for Chinese, which has no
  spaces to break at and would otherwise have nowhere to wrap, and wrong for Korean, which
  does; `word-break: keep-all` is now set on `:root[lang="ko"]` and nowhere else. **Not one of
  the three was findable in English or Turkish**, which is the argument for taking the
  pictures rather than trusting the parity tests. `docs/screenshots/wp6-100-{en,tr,zh,ko,ru,es}.png`,
  all 362 px wide and 443–480 px tall, all from `scripts/screenshot.ps1`'s documented set.
  **(7) The WP2b flake, fixed at the cause.** `MockServer` recorded a request **after**
  answering it, so a test that read `requests()` the moment `refresh()` returned could beat
  the worker thread to the push — about one full-suite run in ten on this machine, and it hit
  whichever of the four tests that inspect requests happened to lose. The record is now taken
  **before the response is written**, while the client is still blocked on `read`, so a client
  that has an answer is a client whose request is already visible. Ordering, not a `sleep`: a
  wait would only have widened the window on a fast machine and still lost on a loaded one.
  Measured either side — **4 failures in 40 runs before, 0 in 30 after**.
  **No new dependencies, and not one line of new behaviour in Rust**: WP6 is six JSON files,
  two CSS rules, a plural helper and one localised label in TypeScript, and the tests that
  hold all of it. **Four new tests in Rust (448 in the workspace, and three existing ones grew
  from two languages to six) and 7 more in the panel (75).**
- 2026-09-07 — **WP7 landed: the installer, and the last mile that is not a release.** One
  NSIS package, **2.0 MB**, per user, no elevation, carrying four files and nothing else:
  `nazar-tray.exe`, `nazar-statusline.exe`, `LICENSE.txt`, `THIRD-PARTY-NOTICES.md`. The
  release profile is worth the link time — against Cargo's stock release settings on the same
  source, the tray binary went **11.95 → 4.79 MB**, the wrapper **497 → 335 KB** and the
  installer **3.06 → 1.99 MB**. Four things were decided rather than inherited.

  **The bundle ships the wrapper and installs nothing.** `bundle.externalBin` puts
  `nazar-statusline.exe` beside the tray; Claude Code's `settings.json` is not touched by any
  installer path. What was missing until now is the way a person without a terminal asks for
  it, so the settings page grew a status-line section that **asks twice** — the first button
  runs `install --dry-run` and prints the wrapper's own diff, and only the button that appears
  underneath it writes, on top of the backup the installer takes anyway. The three commands
  behind it run the binary beside them rather than reimplementing the edit: there is one
  implementation of "change `settings.json`", it is the one the command line runs, and it is
  the one the tests cover.

  **The uninstaller takes what Tauri's template cannot know about** (`nsis/hooks.nsh`): it
  asks the wrapper to undo its own installation *before* deleting it, so a status line
  pointing at a file that is about to vanish is restored rather than left dangling; it removes
  the `StartupApproved\Run` value Windows keeps beside the `Run` one, which is the residue
  WP5 found and could only write down; and it removes `HKCU\Software\<publisher>\nazar-tray`
  (`${MANUPRODUCTKEY}`, so the middle segment is whatever `bundle.publisher` holds),
  the key holding the last install location, which the stock template only removes when the
  *delete application data* box is ticked — so a silent uninstall was leaving a registry key
  pointing at a directory that no longer existed. That last one was **found by measurement**:
  it was the one thing still on the machine after the first acceptance run, and the second run
  came back clean.

  **Acceptance, on this machine rather than a VM** (which is the honest caveat — a VM would
  also prove the WebView2 bootstrapper path, and this machine already had the runtime).
  Silent install: **tray running 4.4 s** after the installer started, Start Menu entry
  present, no elevation prompt, five files in `%LOCALAPPDATA%\nazar-tray`. Silent uninstall,
  with the tray running and both startup values planted: install directory, Start Menu
  shortcut, desktop shortcut, `Run`, `StartupApproved\Run`, the Add/Remove entry and the
  manufacturer key **all gone**. What remains is `~/.nazar` and `%APPDATA%\nazar`, both by
  decision and both written down. The maintainer's own `~/.claude/settings.json` was
  SHA-256-identical before and after the whole exercise; the wrapper's install, status and
  uninstall were exercised end to end against a **copy in a temporary directory**, and came
  back byte for byte.

  **MSI was built and then not shipped.** `cargo tauri build --bundles msi` works: the
  bundler downloads WiX itself and produces a 2.70 MB package with no complaint. It is left
  out because Tauri's WiX target has no `installMode`, so an MSI is per-machine and needs
  administrator rights, and because `nsis.installerHooks` has no WiX equivalent — the MSI
  could not restore the status line, remove the `StartupApproved` value or take the
  manufacturer key. A second installer that uninstalls worse than the first is not a choice
  worth offering; NSIS is the winget artefact and the only one.

  Also: `winget` manifests that `winget validate` accepts, with `/S /R` as the silent switch
  because the template only launches the app on `/R` and a tray that is installed but not
  running looks exactly like a broken install; a release workflow on tag `v*` that drafts
  rather than publishes and attests the build provenance; the `bundle` CI job promoted from
  `continue-on-error` to a gate, now that the installer is the product rather than a
  by-product; `THIRD-PARTY-NOTICES.md` generated from the lock file for the 310 packages that
  are actually in the binaries, with a `--check` that fails CI when it goes stale;
  `docs/CODE_SIGNING.md`, `docs/RELEASE.md`, `SECURITY.md` and `CONTRIBUTING.md`. **No tag, no
  release, no winget submission, no SignPath application, no visibility change** — the five
  one-way doors are commands in `docs/RELEASE.md` and nothing here runs them.
- 2026-09-08 — **T-WP8: the status line that was installed, reported installed, and had
  never once run** (`crates/nazar-statusline/src/settings.rs` — `command_for`,
  `has_unquoted_backslash`, `same_command`, `looks_like_a_windows_path`;
  `crates/nazar-statusline/src/install.rs` — `Installed`, `installed_state`, `bash_warning`,
  `is_a_build_artifact`, the repair path through `chain.json`; `docs/statusline-wrapper.md`;
  `docs/limits-contract.md`). Found in a day of live use, and the interesting part is that
  **every diagnostic on the machine said it was fine**. `status` printed `installed: yes`.
  The settings file held the right absolute path. `nazar doctor`, in the other repository,
  said *no status-line tick yet — Claude Code runs it after a turn*, which is what a session
  sitting at a prompt honestly looks like. Nothing anywhere said the command had never been
  spawned.

  **The cause is one character and one sentence of somebody else's documentation.** Claude
  Code on Windows runs `statusLine.command` through **Git Bash** where it can find one, and
  through PowerShell where it cannot. In `sh` a backslash outside quotes is the escape
  character, so the path this installer wrote —
  `C:\Users\…\AppData\Local\nazar-tray\nazar-statusline.exe` — arrived at the shell as
  `C:UsersAppDataLocalnazar-traynazar-statusline.exe`, and a program by that name does not
  exist. The maintainer's *previous* status line had survived the same treatment for months
  because it happened to be written as `node "C:\…\statusline.js"`: the quotes were there for
  the space in an earlier path, and they were the only reason it ever ran. Writing
  `C:/Users/…/nazar-statusline.exe` into the file by hand fixed it in the time Claude Code
  took to reload its settings.

  **The fix is forward slashes, and the reason to prefer them over quotes is that they work
  in both shells.** Quoting makes bash happy; a command line that is nothing but a quoted
  string is, to PowerShell, a string expression rather than a command. Forward slashes are a
  path separator to Windows itself, and neither shell touches them. A space still gets
  quotes on top. A POSIX path is written exactly as it is, because a backslash in a POSIX
  file name is part of the name — the rewrite is gated on the *string* looking like a
  Windows path, not on `cfg!(windows)`, so the case that only happens on Windows is still a
  case the tests reach from anywhere.

  **`install` had to stop being idempotent in the way it was.** It asked "is this command
  ours?", found that it was, and said *Already installed. Nothing to do.* — about a status
  line that had never produced a capture. The question is now asked in the other order: a
  command that the shell cannot start is **unrunnable first and ours second**, and an
  unrunnable one is repaired, with the same diff every other edit is shown as. Two things
  that are *not* repaired, deliberately: a quoted backslash path, which reaches the shell
  intact, and another copy of the wrapper at a different path, which is somebody's choice
  and writes into the same capture directory anyway. Every spelling of one path — slashes
  either way, quoted or bare, either case — normalises to one installation, which is also
  what lets `status` compare the settings file with `chain.json`'s `installedCommand`
  without reporting a difference that is only spelling.

  **The repair path is the one place this could have destroyed something.** A repair replaces
  our own command with our own command, so the status line being displaced *is this program*
  — and writing that into `chain.json` as `previous` would make `uninstall` restore the
  broken command and lose the record of what the user really had, which is the one thing in
  that directory that cannot be reconstructed. So a repair carries the existing record
  forward and brings only `installedCommand` up to date, keeping the **old** backup named,
  because the copy taken a moment ago holds the broken command rather than the original.

  Two smaller things came with it. `status` prints a `warning:` line naming the shell and the
  fix, because `installed: yes` next to an empty capture directory is two facts that look
  unrelated until something says why. And `install` warns when the path it is about to write
  comes out of `target/release` or `target/debug`: that path works until the next
  `cargo clean`, and it is the path a developer installs by accident every day.

  **The home-directory contract is now written down rather than inferred.** Nazar's
  `docs/PROJECT.md` §7 carried an open decision about "two spellings of nazar-tray's home",
  because its TypeScript resolves `<home>/.nazar` and its desktop shell resolves
  `%APPDATA%\nazar`. They were never two spellings of one path: `paths.rs` has a **data**
  directory holding everything a consumer reads (`limits.json`, `limits.lock`,
  `tray.request`, `statusline/`) and a **settings** directory holding the two files that are
  the user's rather than a consumer's (`config.json`, `alerts.json`), with `NAZAR_HOME`
  overriding both. That is a sentence in `docs/limits-contract.md` now, on the side that owns
  it.

  **Not done, and why.** The single-instance lock still proves liveness by heartbeat alone,
  so a tray killed with Task Manager keeps the lock for up to five minutes. Adding a
  process-id probe means either a platform crate or hand-written FFI in the one crate whose
  claim is that it builds and tests everywhere with two dependencies — and `lock.rs` argues
  at length that a heartbeat catches a case a liveness probe cannot (a holder still running
  but wedged). The failure it would fix costs five minutes of a stale reading after a hard
  kill; the failure it could introduce is two writers on a home directory shared between
  machines, which is the exact bug the module exists to prevent. Left alone.

  **Tests: 459, from 451.** `settings.rs` grew the Windows and POSIX rewrites, the unquoted-
  backslash scanner including the escaped-quote case, the three spellings of one path, and
  what actually lands in the document; `install.rs` grew the three states of an existing
  command on and off Windows, the warning's wording, and the build-directory check. No test
  writes to a real settings file: every new one is a pure function, which is the same reason
  the rest of this crate's tests never touch the machine they run on.

- 2026-09-08 — **T-WP9: thirty-two toasts, one second apart** (`crates/nazar-core/src/alerts.rs`
  — `same_period`, `UNKNOWN_WINDOW_TOLERANCE_SECONDS`, rule 4 in the module header;
  `crates/nazar-core/src/claude/mod.rs` — `resets_at_value`; `crates/nazar-core/src/timefmt.rs`
  — `floor_to_minute`, `unix_seconds_auto`). Between 19:16 and 21:43 the maintainer's Windows
  notification database collected **32 toasts**, every one of them *Claude Code · weekly window
  60 % / resets in 3 d 7 h*, one every five minutes — the endpoint refresh cadence — and most of
  them **doubled**. `%APPDATA%\nazar\alerts.json` was rewritten every time.

  **The cause is one second.** The Anthropic usage endpoint reported
  `claude/seven_day.resetsAt` as `2026-09-12T02:00:00Z` on one refresh and
  `2026-09-12T01:59:59Z` on the next, alternating, with `seven_day_fable` swinging the same
  way; two consecutive `--print` runs showed both values for the same window. Rule 4 of
  `alerts.rs` — *a new `resetsAt` is a new week* — compared the two as **strings**, in both
  halves: `record.resets_at.as_deref() == reset` on disk and `seen_reset == reset` in memory.
  So every flip cleared the `fired` set and dropped the remembered percentage, the next
  evaluation was a first observation under rule 3, and 60 % fired again. The doubles are the
  swing landing in both directions inside one pair of evaluations.

  **A wobble is not a renewal, and the difference is measurable.** A period that genuinely
  renews moves its reset forward by a **whole window**; the gap between two spellings of one
  instant is seconds. So `same_period` reads both sides as instants and calls them one period
  when they are less than **half a window** apart — half, because there is nothing legitimate
  between "a rounding wobble" and "one window later" — falling back to a flat **hour** when the
  window carries no `windowMinutes`, and to the old string comparison when either side will not
  parse, because with nothing to subtract there is nothing to be tolerant with. Both halves of
  rule 4 ask through the same closure, so the disk and the memory cannot disagree about where a
  week ends. The record's own `resetsAt` is now written with the **newest** spelling whenever
  something fires and left alone when nothing does: it is allowed to drift with the source,
  because nothing compares it exactly any more, and rewriting it on every wobble is what
  rewrote the file every five minutes.

  **The source is rounded down to the minute as well** — `crate::claude::resets_at_value`, which
  is the one function both Claude paths go through, the status line's Unix seconds and the
  endpoint's `2026-09-12T02:00:00.130216+00:00` alike. Sub-minute precision has no consumer
  anywhere downstream: the panel draws a countdown in minutes and the state machine only asks
  whether two readings name the same period. It costs nothing, it makes the two sources produce
  the same text for the same instant, and it flattens this particular endpoint before the
  tolerance ever has to. Rounding is the belt; the tolerance is the braces, and it is the half
  that survives a source which starts jittering by more than a minute.

  **Not done, and why.** The same session found `codex/secondary` reporting `percent 70,
  state: "ok"` with a `resetsAt` of `2026-09-07T12:24:52Z` — in the past, from a rollout log
  nothing had written to since the 6th. A window whose reset has already passed is not a current
  reading, and the honest rendering is "unknown". It is left alone because it is not the small
  change it looks like: `state::binding` is one function shared by the two file readers and the
  derived view, and **the readers have no clock** — `docs/pinned-internal-formats.md` is why they
  do not — so making an expired window stop binding means threading `now` into both of them, plus
  a severity override in the view and an exception in alerts rule 5. Worse, it would fire on
  every window every period: `five_hour` is a few seconds past its reset on some refresh most
  hours, and a window that flips to unknown and back makes the icon flicker and makes
  `crate::alerts` forget its previous reading — which is a *new* way to produce the toast this
  package exists to stop. The right fix is narrower and belongs to the Codex reader: a rollout
  log that has gone quiet past its own reset should say `state: "stale"` at the point where it is
  read, where the clock already is. Written down rather than done.

  **Tests: 465, from 459.** Five in `alerts/tests.rs` — the live sequence (70 % at `02:00:00Z`,
  the same window at `01:59:59Z`, and back, sixteen more times, with the file compared byte for
  byte before and after), a real seven-day renewal still clearing the keys through the jitter,
  the hour of tolerance a window with no length gets on both sides of the boundary, the
  half-window rule and its fallbacks directly, and a record read **from disk** with the other
  spelling in it, which is what every restart during those two and a half hours looked like. One
  in `timefmt.rs` for the flooring, on both sides of the epoch. The `--demo-cross` sequence is
  untouched and still produces exactly the four answers it always did.

- 2026-09-09 — Maintainer identity: author, publisher and copyright are Furkan Yıldız; bundle
  identifier moved to `io.github.xfurqan0.*` before any release. `LICENSE`, `README.md`, the
  winget locale manifest's `Publisher` and `Copyright`, and `crates/nazar-tray/tauri.conf.json`'s
  `publisher`, `copyright` and `identifier`, which is now `io.github.xfurqan0.nazar-tray` —
  reverse-DNS under a namespace the account controls, where the previous one was under a domain
  it does not; `ui/test/version.test.mjs` pins the new identifier and
  `ui/test/i18n.test.mjs` no longer interpolates the old handle into a greeting.

  **Nothing has shipped, so nothing upgrades.** The bundle identifier and the
  `HKCU\Software\<publisher>\nazar-tray` key that NSIS derives from `bundle.publisher`
  (`${MANUPRODUCTKEY}` = `Software\${MANUFACTURER}\${PRODUCTNAME}`) both change on machines that
  have never had an installed build, so there is no in-place upgrade to break, no orphaned key
  and no second Add/Remove entry: the first release is the first time either name reaches a
  machine. Doing it after v0.1.0 would have meant a new bundle identity, which Windows treats
  as a different product. The winget `PackageIdentifier` is untouched — it was already
  `xfurqan0.nazar-tray`, and the display `Publisher` beside it is the only half that moved.
