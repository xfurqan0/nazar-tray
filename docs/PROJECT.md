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
   So nazar-tray needs no fetcher, no token, no HTTP client. This also keeps it clearly outside Anthropic's terms, which forbid tools that "collect, store, or intermediate Claude.ai credentials or session tokens".
2. **Threshold notifications.** The two Windows leaders do not mention notifications at all. Being warned at 85 % before hitting the wall is the reason a quota tray exists.
3. **Multi-account is the most requested feature and nobody has it.** CodexBar's maintainer: token/cookie-based fetching "will not work if we ever wanna support multiple accounts". A file-based design gets it almost for free (v2).
4. **Part of Nazar.** The same `limits.json` drives the quota strip on the Nazar canvas. The tray stays a separately downloadable app (maintainer decision); it is not folded into Nazar.

## 2. Scope and non-goals

| nazar-tray **is** | nazar-tray **is not** |
|---|---|
| Tray icon + popup panel: Claude and Codex windows, percentages, reset countdowns | A launcher, session manager, or cost analytics dashboard |
| A passive reader of local files that writes `~/.nazar/limits.json` | Anything that reads, stores, or forwards tokens |
| Windows release first; macOS and Linux builds later from the same code | A Windows-only codebase |
| Six UI languages from day one: EN, TR, ZH, KO, RU, ES | — |

## 3. Decisions taken in the research phase (2026-09-07)

| Topic | Decision | Why |
|---|---|---|
| Data source | **Passive by default, hybrid on request.** Codex: tail the newest `rollout-*.jsonl` (both windows present). Claude: a tiny status-line wrapper records the payload's `rate_limits` (5-hour + global 7-day) and then runs the user's existing status-line command unchanged. **Opt-in "detailed windows" mode** (settings, default off): reads the Claude Code OAuth token in memory only and calls the official usage endpoint to get model-scoped weekly windows (e.g. a Fable-only weekly cap). The app suggests the mode once when it detects a Max plan; the user decides. | Verified 2026-09-07 03:05 on the maintainer's machine: status-line `seven_day` = 18 % (global), official endpoint reported Fable weekly = 23 % — the binding window for a Fable-heavy Max user is invisible to the passive path. Passive still removes 429s, the scheduled task, the lock file and ~120 lines of shell glue for everyone; the hybrid keeps the number honest for Max users. |
| Stack | **Tauri v2** (Rust core + small web panel). Ship **Windows only** in v1; macOS build later; Linux via CLI output and the Nazar canvas (no stack supports a tray popup reliably on Linux; even 21k★ CodexBar closed its Linux tray issue as not planned). | Tauri 5 MB vs Electron 105 MB; tray, notification, autostart and updater are first-party plugins. Windows-only → cross-platform is a ~20 % step; WinForms → anything is a rewrite. Same stack as Dile. |
| Existing C# prototype | **Not migrated.** Used as the behavioral spec; its good decisions are listed in §5. | The repo has no code yet, so the cost of not porting is zero today and grows after the first package. |
| Refresh | **Inside the tray process.** File watchers + a 60 s poll; no OS scheduler. | Kills the Task Scheduler / launchd / systemd triple, the VBS launcher and the lock race the audit proved from logs. |
| i18n | JSON locale files, system language auto-detected, override in settings. EN and TR by the maintainer; ZH, KO, RU, ES machine-translated first, corrections via PR. | Win-CodexBar ships five languages; table stakes. |
| Distribution | GitHub Releases + **winget** (accepts unsigned installers) + apply to **SignPath Foundation** for a free OSS Windows OV certificate. macOS notarization ($99/yr) only with the macOS build; Homebrew cask needs 225★. | EV certs no longer bypass SmartScreen (2024). Budget ceiling ≈ $126/yr. |
| Visual identity | Nazar bead: deep blue / light blue / white / black dot. Icon = bead filling from the bottom with the binding window; amber ≥ 60 %, red ≥ 85 %, **grey = unknown** (never a reassuring "0"). Panel on navy. Themes as JSON: `nazar` (default), `graphite`. | Drops the Apple-grey look inherited from Win-CodexBar; inspiration credited in README, not in code. |

## 4. Known limits of the passive design (write them in the README)

- Claude numbers update only while a Claude Code session refreshes its status line. Between sessions the tray shows the last value with its age and a countdown computed locally from `resets_at`. Quota does not burn while you are not using it, so this is honest, not stale.
- The status-line payload carries only the 5-hour and the global 7-day window (documented; verified live). Model-scoped weekly windows come only from the official usage endpoint, so they need the opt-in **detailed windows** mode. In that mode the token is read from `~/.claude/.credentials.json`, held in memory for one request, never written or logged; the README states this in plain words, and the mode is off by default. The endpoint is undocumented and rate-limited (429); detailed mode keeps last-good values and backs off.
- Users who already run a custom status line keep it: the wrapper chains to it. Users without one get a minimal default.
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
  "updatedAt": "2026-09-07T00:12:34+03:00",
  "providers": {
    "claude": {
      "configured": true,
      "plan": "max_20x",
      "sourceAt": "2026-09-07T00:12:30+03:00",
      "source": "statusline | endpoint",
      "binding": "seven_day_fable",
      "windows": {
        "five_hour":        { "percent": 12, "resetsAt": "2026-09-07T06:10:00+03:00" },
        "seven_day":        { "percent": 18, "resetsAt": "2026-09-12T05:00:00+03:00" },
        "seven_day_fable":  { "percent": 23, "resetsAt": "2026-09-12T05:00:00+03:00", "model": "Fable", "detailed": true }
      }
    },
    "codex": {
      "configured": true,
      "plan": "plus",
      "sourceAt": "…",
      "binding": "secondary",
      "windows": {
        "primary":   { "percent": 54, "windowMinutes": 300,   "resetsAt": "…" },
        "secondary": { "percent": 70, "windowMinutes": 10080, "resetsAt": "…" }
      }
    }
  }
}
```
No tokens, no account ids, no e-mail. `configured:false` when the provider's files are absent. Nazar reads this file and nothing else from nazar-tray.

Added after the Nazar data-layer audit (2026-09-07 04:40): every window carries `"state": "ok" | "stale" | "error"` plus an optional `"error"` string (the prototype had `stale`+`lastError`; the first frozen draft lost them); every provider carries `"source"` (`statusline | endpoint` for Claude, `rollout` for Codex); Claude windows carry `windowMinutes` (300 / 10080) like Codex so consumers need no provider-specific logic. The status-line wrapper keys its capture file by `session_id`, never a single fixed path (three concurrent sessions would otherwise overwrite each other).

## 7. Work packages (ordered; no dates)

| # | Package | Done when |
|---|---|---|
| WP0 | Repo skeleton: Tauri v2 project, Rust core crate, CI (GitHub Actions, Windows build + tests), locale scaffolding with EN/TR, `limits.json` contract doc | `cargo test` and a Windows build pass in CI on an empty tray |
| WP1 | **Codex reader**: find newest `rollout-*.jsonl`, tail for `rate_limits`, tolerate schema drift, fixtures from real logs | Unit tests on captured fixtures incl. missing/partial fields; live value matches Codex `/status` on the maintainer's machine |
| WP2 | **Claude reader**: `nazar-statusline` wrapper (records `rate_limits`, chains to existing command, < 50 ms), installer merges into `~/.claude/settings.json` without clobbering | Works with no status line, with ccstatusline, with the maintainer's custom script; uninstall restores the previous command exactly |
| WP2b | **Detailed windows mode (opt-in)**: settings toggle, token read in memory only, official usage endpoint client with backoff and last-good cache, model-scoped windows merged into the state with `detailed:true`, Max-plan detection prompt | Off by default; with the toggle on, the Fable weekly window appears and matches `/usage`; token never appears in any file, log, or error text (test asserts it); 429 leaves last value with a stale flag |
| WP3 | **State model**: binding window across passive + detailed windows, staleness/age, local countdown, unknown state, atomic `limits.json` writer, single-instance | Property tests on window selection and reset math across time zones |
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
- 2026-09-07 03:20 — Live check showed the status-line payload lacks model-scoped weekly windows (global 7-day 18 % vs Fable weekly 23 % on the maintainer's machine). Decision: hybrid. Passive by default; opt-in **detailed windows** mode (WP2b) reads the token in memory only and calls the official endpoint. Claim reworded to "by default". Raw status-line payload captured as a WP2 fixture.
