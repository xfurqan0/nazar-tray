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
- Unsigned until SignPath approval; SmartScreen will warn. winget install avoids the browser download warning.

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
| WP4 | **Tray icon + panel**: bead icon per scale factor, navy panel near cursor, both providers, countdowns, theme JSON (`nazar`, `graphite`) | Screenshots at 100/150/200 % DPI; panel opens on left/right click, closes on Esc/blur |
| WP5 | **Notifications, autostart, settings**: thresholds 60/85/100 once per window per reset, autostart toggle, language override, theme, per-provider enable | Toast appears exactly once when crossing 85 %; survives sleep/wake |
| WP6 | **i18n**: ZH, KO, RU, ES translations (machine first), pluralization and RTL-safe layout check, language switch without restart | Every UI string comes from locale files; no hard-coded text in code |
| WP7 | **Distribution**: NSIS/MSI installer via Tauri bundler, winget manifest, SignPath Foundation application, README (EN) with GIF, CHANGELOG, first release checklist | Fresh Windows VM: `winget install` → tray running in 2 minutes; uninstall leaves no files |
| WP8 | **Nazar handoff**: contract doc, sample files, a `nazar-tray --print` CLI that dumps `limits.json` (also the Linux story) | Nazar canvas reads the file on the maintainer's machine |

Going public happens after WP7, not before.

## 8. Open decisions
- ~~Opt-in official-endpoint mode~~ → **decided 2026-09-07 03:20: in v1 as WP2b, off by default** (maintainer approved; the maintainer's own binding window is the Fable weekly, invisible to the passive path).
- ~~Panel technology inside Tauri: plain HTML/CSS vs. a tiny framework~~ → **closed 2026-09-07 in WP0: plain TypeScript, HTML and CSS, bundled by esbuild, no framework** (decision K2). The panel's only runtime dependency is `@tauri-apps/api`; `ui/` has two dev dependencies, esbuild and TypeScript.
- Icon rendering: pre-rendered bead PNG set per fill level vs. runtime drawing. Leaning: runtime in Rust (`tiny-skia`), one source of truth for themes.
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
- 2026-09-07 — **WP2b landed: the detailed-windows mode, off by default.** `nazar-core::claude::detailed` reads four values out of `.credentials.json` — and only while the mode is on — holds the token in a wiping wrapper for one 20-second `GET` to `api.anthropic.com/api/oauth/usage`, and maps `limits[]` into `five_hour`, `seven_day` and `seven_day_<model>`. **61 new tests (249 in the workspace)** against a hand-rolled loopback HTTP server: every status code, the backoff schedule on an injected clock, last-good/stale semantics, both response shapes, plan normalisation, binding across scoped windows, settings round-trip, and four gates — a sentinel token that must appear in no file, no error and no `Debug`; a one-call-site grep on the method that exposes it; no printing macro anywhere in the module; and the credential gate grown into an allow-listed **directory** rather than a dropped needle. The mode is behind the `detailed-windows` cargo feature as well as the runtime flag, so `cargo tree -p nazar-statusline` still shows `serde` and `serde_json` and nothing else. HTTP client chosen by measurement: `ureq` + `rustls` adds **4** packages, `reqwest` with `blocking` adds **17** including `aws-lc-sys` and a `cmake` build. Three findings from the audit became behaviour: `Retry-After` is read (B07), a token whose stored expiry has passed costs no request and a rewritten sign-in file clears the backoff at once (B06), and a plan change drops the remembered numbers (B25). Verified live: **session 2 %, weekly 38 %, Fable weekly 30 %**, `plan: max_20x`, matching an independent reading of the same endpoint seven minutes earlier — and **nothing was written**: `~/.nazar` and `%APPDATA%\nazar` did not exist before or after, and `.credentials.json` kept its size and modification time. The live check also found that the endpoint spells `resets_at` as `2026-09-07T13:10:00.130195+00:00` where the status line writes Unix seconds; both are now rewritten into the contract's `…Z`. Written up in `docs/detailed-windows.md`; precedence in `docs/limits-contract.md`; both new formats pinned.
