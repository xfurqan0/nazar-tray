# nazar-tray — project notes and v1 plan

Internal project notes. Kept in English so the repo is readable by everyone.

> **The system-tray face of Nazar.** Shows your Claude Code and Codex usage windows (5-hour and weekly) in the tray, with a bead icon that fills up as you burn quota. Writes the same `limits.json` that the Nazar canvas reads. **By default it reads nothing but local files: no credentials, no network.** An opt-in "detailed windows" mode adds the model-scoped weekly windows from the official usage endpoint for users who need them.

- Folder: `C:\nazar-tray` · Repo: `github.com/xfurqan0/nazar-tray` — public since 2026-09-08; no release cut yet, so the source is ahead of anything installable
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
| A third panel view that answers *how much did I spend, on which model* from numbers the providers already reported (2026-09-13) | **Still not an analytics dashboard.** One view, three ranges, one row per model, no currency in v1 — the non-goal on the right is what keeps the usage packages from growing a cost column, a project breakdown and a session list |
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
| Visual identity | Nazar bead on the 16-cell pixel grid: deep blue rim / white band / **yellow** iris / black pupil *(revised 2026-09-09: the mark was four circles and the iris was Nazar's light blue)*. Icon = **the mark, whole, at every reading** *(revised again the same evening: it used to fill with the binding window)*; the one exception is **grey = unknown** — a grey rim and a hollow ring when nothing could be read, never a reassuring "0". The quota is read in the tooltip and in the panel, where amber ≥ 60 % and red ≥ 85 % still colour it. Panel on navy. Themes as JSON: `nazar` (default), `graphite`. | Drops the Apple-grey look inherited from Win-CodexBar; inspiration credited in README, not in code. |

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
- Transcript scanning for "estimated usage" (the prototype read up to 400 transcript files on 401) is **dropped**; no estimates, only reported numbers. — **Half of this was reversed on 2026-09-13, see §8.** Transcripts *are* read, for usage history; what stays dropped is the estimating. No quota percentage is derived from them, and "only reported numbers" is now the argument **for** reading them rather than against it.

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
| T-WP10 ✅ | **A window whose reset has passed, reported as current** (from live use; the narrow fix T-WP9 wrote down and left). The Codex reader marks a window whose `resets_at` is more than five minutes behind the current instant as `state: "stale"` — the percentage is kept, because it is still the last thing the server said, and the claim that it is current is not. `crate::alerts` gains rule 6: a period that has already ended cannot be crossed, so no toast is fired from one. Codex's `resets_at` goes through the same minute-flooring gate the Claude reader has used since T-WP9. The comparisons both rules rest on get named-point grids, and the readers get **captured** payloads to read: `crates/nazar-core/fixtures/captured/` is a real `~/.nazar` with its identity removed | ~~A machine nobody has opened Codex on for two days stops claiming a live 70 %, and does not warn about it~~ — **landed 2026-09-09**. The maintainer's own `limits.json` was the bug report. 490 tests, from 464 |
| T-WP11 ✅ | **Release binaries that named the machine they were built on** (from a pre-release read-through). Panic locations are compiled in as string literals, so `strip = true` left every compile-time path in place: **310** copies of `C:\Users\<account>\.cargo\registry\src\…` in `nazar-tray.exe`, **7** in `nazar-statusline.exe`, and the NSIS package shipped both. `scripts/build-installer.mjs` passes `--remap-path-prefix` for the cargo registry, the git checkouts and this checkout to **every** cargo call it makes, the wrapper's included, and on every profile rather than release alone — so the debug bundle job can prove the remap on a pull request instead of the rule being discovered broken on release day. `scripts/check-binary-paths.mjs` reads the binaries back, in text and in both UTF-16 alignments, and the build deletes the bundle rather than leave an installer that failed it looking finished | ~~Both binaries count zero, and a run whose binaries carry the runner's path goes red~~ — **landed 2026-09-09**. 310 → 0 and 7 → 0; the check is a gate inside the build script and a named step in `ci.yml` and `release.yml`. No Rust changed, so still 490 tests |

Going public happens after WP7, not before. **WP7 does not include going public**: the tag,
the GitHub Release, the winget pull request, the SignPath application and the repository's
visibility are five one-way doors, and every one of them is a command in `docs/RELEASE.md`
for the maintainer to run. Nothing in this repository runs them.

### The usage-history packages (T-WP12 – T-WP19)

Requested by the maintainer 2026-09-09, planned 2026-09-13, and **this is where the
`docs/FUTURE.md` entry went** — an entry leaves that file when it becomes work packages here.
The decision underneath them, including what was disproven on the way, is §8. None of them
has landed.

| # | Package | Done when |
|---|---|---|
| T-WP12 | **Theme toggle leaves the panel footer**, which is where the Usage button goes; the settings page's two selects become the only way to change a theme | The footer has no theme button, the settings selects still switch theme and locale, UI tests green |
| T-WP13 | **The Claude usage reader**: recursive scan of `~/.claude/projects/**/*.jsonl` including `subagents/`, deduplication by `(message.id, requestId)`, hourly UTC buckets, the monthly store | A fixture with known totals matches to the token; a duplicate fixture inflates by zero; the sentinel leak test passes; 230 MB in under two seconds |
| T-WP14 | **The Codex usage reader**: every `token_count` event, `last_token_usage` summed, the model from the session's newest `turn_context` | The mid-session reset fixture totals correctly; a model that was never named reads `unknown`; sentinel leak test passes |
| T-WP15 | **`get_usage`**: one Tauri command and one view type — range × provider × model, plus `since` | The command returns hourly buckets and the panel does its own local-week arithmetic |
| T-WP16 | **The usage view**: the third view, three ranges, one row per model, headline `input + output + cache_create` with `cache_read` beside it, and a *since {date}* line | Fits 360 × 720; CSS bars, no charting library (decision K2); the three i18n gates pass |
| T-WP17 | **The tray tooltip's second line**: this week's tokens and the model that spent most of them | Under 127 characters in all six languages; the icon still says nothing about usage |
| T-WP19 | *(optional, after v1)* a `pricing.json` data file and cost in currency | A model that is not in the table shows no cost rather than a guess |

T-WP12, T-WP13 and T-WP18 are independent; T-WP14 follows T-WP13; T-WP15 → T-WP16 → T-WP17
is a chain. **T-WP18 is this one** — the written decisions, the contract and the README — and
its first two items belong in the same commit as the readers they describe, because rule 4 of
`pinned-internal-formats.md` says a fixture and its row land together.

## 8. Open decisions
- ~~Does usage history mean reading Claude Code's transcripts, which §5 says are not read?~~ → **decided 2026-09-13 03:35, maintainer approved: yes, metadata only — and the §5 line above is corrected rather than quietly outgrown.** `~/.claude/projects/**` moves from "not read, on purpose" into the inventory in [`pinned-internal-formats.md`](pinned-internal-formats.md), with the nine fields it may read, the version observed and its fixtures.
  - **Why the old rule does not cover this.** The retired prototype read up to 400 transcripts to *estimate a quota percentage* when a fetch failed — it manufactured a number to stand in for one the server would not give. Usage history reads `message.usage`, which is **the server's own reported number**, written to disk by Claude Code rather than computed by it, and it replaces nothing: the quota view keeps coming from the status line and the endpoint, and no percentage is derived from a transcript. "No estimates, only reported numbers" is the rule this obeys, not the rule it breaks.
  - **Why it is not a new privacy stance.** It is the second application of the one this repository has shipped since WP1: the Codex reader already opens files full of prompts and takes seven values out of them, protected by an allow-list construction (a new object built from named fields, never a filtered line) and a sentinel leak test. The usage readers get the same two mechanisms, and the sentinel test is extended to the accumulated state and the file that is written, which is the stricter form Nazar's reader uses. What is read is nine fields; prompt text, response text, reasoning, tool input and output, paths, project names and session ids are not among them, and a test fails the build if any of them appears in the output.
  - **What it costs to say it.** The README's privacy paragraph is rewritten in the same change rather than left to age — "never reads your tokens and never talks to the network" is still true and stays, because "token" there is a sign-in token, and the sentence now says in plain words which files are read and which fields are taken. A promise that has to be read carefully to stay true is a promise that has already broken.
  - **The FUTURE.md plan it replaces is recorded as disproven, not as superseded.** That entry had the panel differencing the status line's own counters. Measured on the maintainer's machine on 2026-09-13: `context_window.total_input_tokens` = **562 431**, and `current_usage.{input + cache_creation + cache_read}` = 32 + 3 891 + 558 508 = **562 431** — the same number to the token, and the same identity holds in both committed fixtures. That field is **the current size of the context window, not a cumulative session total**: it *falls* when the context is compacted, so the plan's "a counter going backwards means a new session" would have fired on every `/compact` and thrown the session's usage away. The only genuinely cumulative counter in the payload is `cost.total_cost_usd`. **So the status line can give a cost history and cannot give a token history**, and the transcripts are not a convenience here, they are the only source.
  - **Scope, so this does not become the dashboard §2 says it is not:** the store is a new file (`docs/usage-contract.md`), `limits.json` does not change, Nazar reads nothing new, and there is no currency in v1.
- ~~Opt-in official-endpoint mode~~ → **decided 2026-09-07 03:20: in v1 as WP2b, off by default** (maintainer approved; the maintainer's own binding window is the Fable weekly, invisible to the passive path).
- ~~Panel technology inside Tauri: plain HTML/CSS vs. a tiny framework~~ → **closed 2026-09-07 in WP0: plain TypeScript, HTML and CSS, bundled by esbuild, no framework** (decision K2). The panel's only runtime dependency is `@tauri-apps/api`; `ui/` has two dev dependencies, esbuild and TypeScript.
- ~~Icon rendering: pre-rendered bead PNG set per fill level vs. runtime drawing~~ → **decided 2026-09-07 in WP4: drawn at run time in Rust, and without `tiny-skia`** (decision K3). A pre-rendered set was one file per fill level per severity per freshness per scale, invalidated all at once by a theme change; the drawing is no dependency and, since 2026-09-09, not even arithmetic — sixteen rows of sixteen cells and a lookup. That evening's second revision cut the states to two, so a pre-rendered set would now be eight small files and the decision is no longer obvious; it stands because the theme hexes then live in one place instead of two, and because the shell asks for a size rather than picking one from a list. `crates/nazar-tray/src/icon.rs`.
- **Does the expired-reset rule belong to Claude too?** T-WP10 gave it to the Codex reader only, because Claude Code's `five_hour` window renews while a session is open and is re-reported seconds later — the captured document in `crates/nazar-core/fixtures/captured/limits.json` has that window fifteen seconds past its own reset on a machine actively in use, and a rule applied there would have blinked it grey. The general form of the question — *is this reading about a period that is over?* — is already answered for the thing that matters, in `alerts` rule 6, which is provider-agnostic. What is left open is only whether the **panel** should dim such a window, and that is a rendering decision with a live example to try it against.
- **`trim-paths` instead of T-WP11's remap flags.** `[profile.release] trim-paths = "all"` is one line in `Cargo.toml` and does the whole job, with no environment variable that every future cargo call has to remember to inherit — which is the one weakness of what landed: a build run some other way than `scripts/build-installer.mjs` gets no remap at all, and only `scripts/check-binary-paths.mjs` would say so. It is not hypothetical — `cargo test --release` rebuilds `nazar-statusline` and leaves an unremapped binary sitting in `target/release`, which is how this weakness was found rather than argued about, and which is why `docs/RELEASE.md` step 5 now says to rebuild if anything has touched that directory since step 2. It is nightly-only on the toolchain `rust-toolchain.toml` pins (Cargo 1.98), so this stays open until it is stable, at which point the change is a deletion. The checker stays either way; it is what would prove the swap. A related loose end with the same answer: there is no prefix for `.rustup`, because the standard library already arrives remapped by the Rust project as `/rustc/<hash>/…` and nothing here builds it from source — `.rustup` is a pattern in the checker instead, so a toolchain that ever did would go red rather than quietly ship.
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
  the same locale files the panel reads, because the tray is UI too. *(Two evenings later the
  circles became the pixel grid and the fill, the severity colours and the fade were taken out
  — decision K25 and its two addenda. The tooltip is unchanged; the icon is the mark, or grey
  when nothing could be read.)*
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
  read, where the clock already is. Written down rather than done — and done the next day, as
  T-WP10, in exactly that shape.

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

- 2026-09-09 — **The pixel bead, with a yellow iris** (decision K25). The tray icon was four
  concentric circles and a 4×4 supersampler; it is now the mark Nazar chose as its direction
  04 — sixteen rows of sixteen cells — and the renderer is a lookup. Three decisions, each
  with its own reason.

  **Why pixel.** At 16 pixels a cell is exactly one device pixel, so what Windows puts on the
  taskbar is the artwork rather than an approximation of it, and there is no antialiased
  fringe to go muddy against a dark taskbar. That argument is Nazar's and it applies here
  unchanged. The one that belongs to this program is the gauge: the chamber is **twelve grid
  rows** tall, the percentage is rounded to a whole number of rows, and 50 % is the bottom six
  of them — the bead's lower half, exactly. A circle cut by a horizontal line has to be
  measured to be believed; a row count can be read off the icon, and `icon/tests.rs` now
  asserts it in cells rather than in pixels. *(Superseded twice the same evening: the fill was
  cut back from the chamber to the iris and from twelve rows to eight, and then removed
  altogether — see the two addenda below. The icon carries no gauge.)*

  **Why yellow.** Rim `#0E2A5A`, band `#FFFFFF` and pupil `#0A0A0F` stay Nazar's, on purpose:
  two programs by the same hand, one mark, and `ui/test/theme.test.mjs` still fails if any of
  the three drifts. The iris is `#F2A93B` where Nazar's is `#3FA9F5`, because the two beads sit
  **side by side in one Windows tray** and 16 pixels is not enough room to tell them apart by
  shape. Sibling applications, not the same application twice. The same test now asserts the
  *difference*, so a well-meaning "sync the palettes" commit fails.

  `#F2A93B` is not a hex invented for this repository. It is **Nazar's amber — the colour its
  bar bead turns past the warning threshold**, `modes.dark.warn` in the theme files both
  programs carry. The one place the two marks differ is still drawn from the family's own
  palette; the tray simply spends on an iris what Nazar spends on a warning.

  **Why the warning fill moved to orange.** It was `modes.light.warn`, `#B87400`, borrowed from
  the panel. Dark amber reads as "not blue" beside a light-blue iris and as "the same colour,
  dimmer" beside a yellow one, which is the one thing a severity colour may not do. The bead's
  three fills are now three hues — yellow `#F2A93B`, orange `#E8720C`, red `#C0392B` — and the
  warning tone got its own key, `bead.warnFill`, in both theme files rather than moving
  `modes.light.warn` underneath the panel. The panel's warn tone is text on a pale card with a
  4.5:1 floor to clear (`ui/test/contrast.test.mjs` measures it); the bead's is a block of
  colour with no text on it. They stopped wanting the same hex, so they stopped sharing one.

  **And why it did not move a second time.** The iris settling on `#F2A93B` — a step darker
  than the first yellow this decision reached for — brings it closer to the warning fill, so
  the obvious follow-up is to darken the warning to something like `#D9530F` and reopen the
  gap. Measured on the regenerated `docs/design/bead-states.png`, that is a losing trade.
  Iris-to-warning and warning-to-red are **ΔE00 16.6 and 22.7** as they stand; with `#D9530F`
  they become **26.0 and 13.0**. The first gap widens by taking the second below where the
  first ever was, and the ordering is the same under simulated protanopia and deuteranopia and
  at all three freshness levels, where the drained fills keep 14.9–16.6 against 13.0–13.5.
  Three fills read as three states when they are *spaced*, not when two of them are far apart
  and the other two are crowded, and `#E8720C` sits nearest the middle of the run. It stays.

  **What else changed, and what did not.** Every rule the audit put into the icon survives
  unchanged: unknown never draws a fill (finding B03) — it draws a grey rim and, in place of
  the old hollow circle, a **six-by-six square ring one cell thick**, which is what a 16-cell
  grid can spell without antialiasing; the fill is the highest binding window across both
  providers (B04); freshness still drains the colour; a spent window still gets the pupil,
  which is now the grid's own 2×2 centre rather than a circle. `size_for_scale` is the one
  behaviour that did move: it returns a **whole multiple of sixteen** (16/32/48/64) instead of
  the exact `SM_CXSMICON` size, because a fill measured in rows needs rows of equal height and
  20 or 24 pixels for 16 cells does not give them. At 125 % and 150 % the shell scales a
  16-pixel bead into a 20- or 24-pixel slot: a slightly soft mark whose *level* is still exact,
  which is the opposite of the trade Nazar took and the right one for an icon that is read
  rather than recognised.

  **And the icon set is no longer `cargo tauri icon`.** The CLI resamples one large PNG down
  with a smooth filter, which turns 8-bit art into a blur of it. `scripts/render-app-icons.mjs`
  renders every size from `ui/assets/bead.svg` nearest-neighbour and packs the same containers
  the CLI produced — PNG-in-ICO at 16/24/32/48/64/256, an `.icns` of the PNG-carrying types,
  the four loose PNGs and the ten MSIX logos. The nine logo boxes that are not multiples of 16
  (30, 44, 71, 89, 107, 142, 150, 284, 310) get the bead at the largest whole cell size that
  fits, centred, with transparent padding. Zero dependencies, no browser, no network. The
  documentation artefacts moved with it: `docs/design/bead-states.png` is the strip that used
  to be `docs/screenshots/wp4-icons.png`, and beside it are the five corners of the contract —
  empty, half, warning, spent, unknown — at 16 and 32 pixels, one file each, all produced by
  `nazar-tray --icons docs/design` from the code that draws the real icon.

  **Addendum, 2026-09-09 01:40 — the fill is confined to the iris.** Written a few hours
  after the rest of K25, with the real icon in the real tray rather than in a strip.

  **What was wrong.** The gauge above fills the whole chamber, band and iris together. On the
  taskbar at 74 % that means the white band is gone, the iris has nothing left to sit inside,
  and the mark is an orange disc with a blue rim — next to Nazar's bead it reads as *a
  different thing*, not as a sibling. The eye disappears at exactly the percentage the icon
  most needs to be understood at a glance, which is the trade backwards: an icon that stops
  being recognisable in order to be precise has spent its identity on a number nobody reads
  off an icon anyway.

  **The rule now.** The `W` cells — the band — are white in every state and at every level,
  and the fill only ever climbs the **iris**: grid rows 4 to 11, eight of them, bottom up.
  Unfilled iris cells are white, so 0 % is a white eye and 100 % is a full coloured one with
  its white ring intact. Severity still picks the colour and freshness still drains it. Spent
  is unchanged: a full iris with the 2×2 pupil black on top of it. Unknown is unchanged in
  every byte — a grey rim and the six-by-six ring — because unknown never had a fill to
  confine, and `PIN_UNKNOWN` in `icon/tests.rs` is the same pair of numbers it was.

  **What the resolution costs, and why it costs nothing.** A step is 12.5 % of the window
  instead of 8.3 %. That is the whole price, and it is not a price: nobody reads a percentage
  off a 16-pixel icon. The icon answers *roughly how far along, and how bad* — eight steps and
  three hues do that comfortably — and the tooltip and the panel answer the rest to the
  decimal. One exception was added so the coarser step cannot lie: **any percentage above zero
  fills at least one row**, since a naive round would draw an empty bead up to 6.25 % and make
  a used window look untouched. Zero is the only reading allowed to look empty.

  `icon/tests.rs` carries the regression as a named test — the band is white at every level
  and in every state, with the 74 % warning that started this written out as its own case —
  and `docs/design/bead-states.png` and the five corner files were regenerated. The
  application icon set did not move: it renders the still mark from `ui/assets/bead.svg` and
  carries no state, so `scripts/render-app-icons.mjs` produces the same bytes it did before.

  **Second addendum, 2026-09-09 01:50 — the tray icon carries no state.** Written ten minutes
  after the addendum above, with the confined fill running in the real tray. The gauge is gone
  entirely: no fill, no severity colour, no freshness fade. The icon is the mark — deep blue
  rim, white band, whole yellow iris, black pupil — at every reading, with exactly one
  exception, below.

  **Why the iris fill did not survive either.** Confining it kept the bead recognisable, which
  was the point, but it moved the whole signal onto eight iris rows, and at 16 pixels a
  part-yellow, part-white iris does not read as a level. It reads as a bead with a piece
  missing. Two sibling icons sit in that tray; one of them is always whole, and the other now
  looked drawn wrong. A signal the user has to rule out as a rendering fault before reading it
  is worse than no signal, and it costs the thing a tray icon is actually for — being found.

  **What the icon is for, restated.** The tray is where the icon is *recognised*; the panel is
  where the quota is *read*. Nobody reads a percentage off sixteen pixels, and both routes to
  the real number — hover for the tooltip, click for the panel — are about a second away and
  are unchanged. **The tooltip still names each provider's binding window and its percentage,
  and the panel still shows every window, its severity colour, its reset and its age.** Nothing
  was removed from either; what was removed is a third, coarse, ambiguous copy of the same
  number, drawn in a space too small to hold it.

  **The one state, and it is the audit's.** Unknown — no provider could be read at all — still
  draws a grey rim `#78879A` and a six-by-six ring one cell thick where the iris and pupil are,
  with no pupil. Finding B03 is exactly this: an icon that has read nothing must not look like
  an icon that has read something reassuring. `IconState` is now that one bit and nothing else,
  and `IconState::from_view` asks one question — did any provider produce a binding window with
  a percentage in it?

  **What went with the fill.** `ORANGE` and `RED`, `bead.warnFill` in both theme files,
  `filled_rows` and the iris-row constants, `Rgb::desaturated` and `fade`, and the severity and
  freshness fields of `IconState`. The panel keeps `modes.light.warn` and `modes.light.danger`
  and every test that measures their contrast; the theme's `bead` block is back to the mark's
  four layers, and `ui/test/theme.test.mjs` fails if a fifth key returns. `size_for_scale`
  keeps returning whole multiples of sixteen, on a new reason: the arithmetic that needed rows
  of equal height is gone, but at 20 or 24 pixels four of the sixteen cells still come out a
  pixel wider than the rest, which lands on a two-cell band and a 2×2 pupil and draws a
  lopsided eye. Soft beats crooked at this size.

  `icon/tests.rs` is thirteen tests where it was twenty: the render **is** `ui/assets/bead.svg`
  cell for cell at 16, 32, 48 and 64 px, the pupil is always there, unknown is a grey rim and a
  hollow ring and nothing else, sixteen pixels is sixteen cells, and the palette is the theme
  file's. `docs/design/` is seven files where it was eleven — `bead-mark-{16,32,64}.png`,
  `bead-unknown-{16,32,64}.png` and the strip — all written by
  `nazar-tray --icons docs/design`. The application icon set is unchanged again, for the same
  reason as last time: it renders the still mark and never carried state.

  **Still to do:** the panel screenshots in `docs/screenshots/` were taken before all of this
  and still show the round bead in the panel header — `wp4-100-*`, `wp4-150-*`, `wp4-200-*`,
  `wp5-100-*`, `wp6-100-*` and `wp7-100-statusline`. Re-shooting them is a separate pass with
  `scripts/screenshot.ps1`; nothing about them is wrong except the mark.

- 2026-09-09 — **T-WP10: a window whose reset has passed, and the readers finally read
  something real** (`crates/nazar-core/src/codex/mod.rs`, `parse.rs`, `alerts.rs`,
  `crates/nazar-core/fixtures/captured/`). The narrow fix T-WP9 wrote down and left, plus the
  two pieces of test debt it exposed.

  **The bug was a sentence in a file that was true yesterday.** On this machine Codex had not
  been opened since the 6th, and `~/.nazar/limits.json` said
  `codex.secondary: percent 70, resetsAt 2026-09-07T12:24:52Z, state: "ok"` — a confident
  number for a week that had ended. Nothing was broken: the rollout log parses perfectly and
  says exactly that, because Codex writes its quota into the session log it is already
  keeping and a source that is not running says nothing at all. The reader had no way to
  notice, because it had no clock.

  **The fix is one comparison, at the point where the clock already is.** `CodexReader::refresh_at(now)`
  is the entry point the refresh loop calls — `Reader::read` has been handed `now` since WP3
  and the Codex source was throwing it away — and a window whose `resets_at` is more than
  **five minutes** behind it is written `state: "stale"`, with the reset named in the reason.
  The percentage stays: it is still the last thing the server said, the panel still draws it,
  and `binding` still sees it. What it loses is the claim that it is current. Five minutes of
  grace, because a window that has *just* turned over is a log that has not caught up — the
  new numbers are one line away — and not a reading from last week. `refresh()` without an
  instant survives for the one-shot `--print` path and is `refresh_at` with the system clock.

  **The rule is the Codex reader's alone, and the captured fixtures are why.** Claude Code's
  `five_hour` window renews while a session is open and is re-reported on the next status-line
  refresh: `crates/nazar-core/fixtures/captured/limits.json` — a real document off this
  machine — has `claude.five_hour` **fifteen seconds past its own reset** while the tray was
  running and the session was live. The same rule there would blink a live window grey once a
  day, which is T-WP9's toast storm arriving through the icon.

  **`crate::alerts` gets the general form of it as rule 6**: a period that has already ended
  cannot be crossed, so a window whose `resetsAt` is behind `now` fires nothing and forgets
  what it saw. Deliberately *not* "silence anything that is not `ok`": the opt-in endpoint
  goes `stale` fifteen minutes after a fetch with its weekly reset three days away, and that
  number is current in the only sense that matters — you were at 86 % twenty minutes ago, so
  you are at 86 % or more now. Silencing it would drop the 85 % warning the whole feature
  exists to deliver. What makes a reading unusable is not that it is old; it is that the
  period it measures is over, and `resetsAt` is where that is written.

  **Codex's `resets_at` now goes through the same minute-flooring gate as Claude's** (T-WP9
  did the Claude half). Codex writes to the second — `2026-09-09T02:31:59Z` was on disk while
  this was being written — and leaving one provider at second precision and the other at
  minute precision means the contract carries two spellings of one kind of value and one
  reader is permanently a bug fix behind the other.

  **The test debt, both halves of it.** *Named-point grids* for the two comparisons these
  rules rest on — `alerts::same_period` and `lock::LockRecord::is_stale` — because every bug
  either has ever had was at a threshold: nothing apart, ±1 s, either side of a minute,
  either side of half a window, a zone offset, the night the clocks go forward, an instant
  before 1970, and text that is not a time. Sixteen rows of table rather than a generator: no
  `proptest` dependency, nothing random, the same answers on every machine. *Captured
  payloads*, which is the direct answer to "these tests test the world their author
  imagined": `crates/nazar-core/fixtures/captured/` is five real status-line captures and the
  real `limits.json` written from them, with identity replaced and **every number and every
  date kept**. They are not decoration — three shapes in them are things no test had:
  `context_window` comes through with four nulls before a session's first API response, a
  payload **drops** `five_hour` once that window has reset instead of reporting zero, and the
  weekly reset really is spelled `02:00:00Z` and `01:59:59Z` one refresh apart, which floors
  to two *different* minutes and is why T-WP9's flooring was never the whole fix.
  `tests/hygiene.rs` greps the directory for identity on every run, and its shared check
  learned the escaped Windows path `C:\\Users\\` — the spelling every path in a status-line
  payload actually arrives in, which the gate had been waving through.

  **Tests: 490, from 464.** `cargo fmt --check`, `cargo clippy --workspace --all-targets -D
  warnings` and `cargo build --release` all clean. No installer built: the tray icon is being
  worked on in a parallel session and a bundle would have picked up a half-finished mark.
- 2026-09-09 — **T-WP11: the installer was shipping the account name of the machine that
  built it** (`scripts/build-installer.mjs`, `scripts/check-binary-paths.mjs`,
  `.github/workflows/ci.yml`, `release.yml`, `docs/RELEASE.md`). Found by a pre-release
  read-through of the artefact rather than of the tree.

  **The greps could not have found it, and that is the whole point.** `docs/RELEASE.md`
  step 5 asks git for `C:\Users`, `/home/` and the maintainer's name, and every one of
  those came back clean — because they read files, and this was only ever inside a compiled
  binary. Every `panic!`, every `unwrap()` and every `#[track_caller]` site carries the
  source path it was compiled from as a **string literal**, which is exactly the kind of
  thing `[profile.release] strip = true` does not remove. `target\release\nazar-tray.exe`
  held **310** copies of `C:\Users\<account>\.cargo\registry\src\index.crates.io-…\` and
  `nazar-statusline.exe` **7**, and the NSIS package carried both to anyone who downloaded
  it. One of the 310 was this checkout's own absolute path, from Tauri's generated context.

  **The fix already existed one repository over.** Nazar's `scripts/build-desktop.mjs` has
  passed `--remap-path-prefix` since its own first release build, for the same reason and
  after the same measurement; this is that approach carried across, with two deliberate
  differences. The registry maps to `cargo` rather than to nazar's `crates`, because this
  workspace *keeps its own code* in `crates/` and a dependency panicking from
  `crates\serde_json-1.0.x\src\…` would read like one of ours. And it applies on **every
  profile**, not release only: nazar's release-only rule keeps `cargo test` and the release
  build on one fingerprint, but it also means nothing can check the rule until release day,
  and CI's bundle job builds `--debug`. Paying one fingerprint split buys a gate that runs
  on every pull request.

  Three prefixes — the registry, git checkouts, this checkout — which is every one that
  appears in practice: the standard library already arrives remapped by the Rust project as
  `/rustc/<hash>/…`. `trim-paths = "all"` would replace all of it with one line in
  `Cargo.toml` and is still nightly-only on the pinned toolchain, so it is written down in
  §8 as a deletion waiting to happen rather than done twice.

  **The check is a script, not a paragraph.** `scripts/check-binary-paths.mjs` reads the
  binaries back and counts `\Users\`, `/Users/`, `/home/`, `.cargo\registry`, `.rustup` and
  this checkout's absolute path — in text and in **both** UTF-16 alignments, because
  resource data is wide and is not aligned to anything in particular. Deliberately not the
  bare string `nazar-tray\`: after the remap this workspace's own panics read
  `crates\nazar-tray\src\icon.rs`, which is relative, identical on every machine, and the
  thing you want to keep. `build-installer.mjs` runs it and **deletes the bundle** if it
  fails, because an installer sitting on disk looking finished is one somebody will upload;
  `ci.yml` and `release.yml` each run it as a named step of their own, where a red cross
  says what broke without anybody opening a log, and where the runner's own
  `C:\Users\runneradmin` is held to exactly the same rule.

  **310 → 0 and 7 → 0**, measured with the same reader either side. The 310 panic locations
  are still there and still useful — they now read
  `cargo\index.crates.io-…\tauri-2.11.5\src\webview\mod.rs`. Installer rebuilt at
  `target\release\bundle\nsis\nazar-tray_0.1.0_x64-setup.exe`, 1.95 MB, **not installed**.
  No Rust changed: still 490 tests, and `cargo fmt --check`, `cargo clippy --workspace
  --all-targets -D warnings` and `cargo test --release` clean.

  **The copy already installed under `%LOCALAPPDATA%\nazar-tray` is still the leaky one.**
  It came from a build made before this, on the maintainer's own machine, where the string
  is the maintainer's own — nothing to fix and nobody to tell. It stops being true the next
  time the installer is run.
- 2026-09-13 — **T-WP12: the footer's theme button is gone; the theme is the settings page's
  and nowhere else** (`ui/src/index.html`, `main.ts`, `format.ts`, `ui/test/format.test.mjs`,
  `ui/locales/*.json`, `crates/nazar-tray/src/state.rs`). Two places to change one setting is
  one too many, and the footer's was the weaker of the two: its visible label was the name of
  the theme it was *already* painting while its `aria-label` said *Switch theme*, so it read
  as a statement to the eye and as an action to a screen reader, and it sat in a row that WP6
  had already taught to wrap because Korean and Russian labels do not fit three buttons. The
  settings page has named both themes in a dropdown since WP5 and writes them with the rest of
  the form, so this was a **deletion and not a move** — nothing was added anywhere, and the
  slot it frees is the one `docs/FUTURE.md` wants for a usage view. Out with the button went
  `nextTheme` in `format.ts` and the test that cycled it, the `[data-theme]` query, the two
  lines that set its label and its `aria-label`, the four-line comment explaining why its
  label came from a message key rather than from the theme file, the click handler, and
  `panel.action.theme` in all six locale files — **120 keys per file, now 119**, which is the
  first key this product has ever removed. No CSS changed: the button was a plain `.button`
  inside `.foot`, and the row still balances itself with *Refresh* and *Settings* at the left
  and the Esc hint pushed right by its own `margin-left: auto`. `set_theme` stays registered,
  tested and uncalled: the panel is no longer its caller, but a released build answers it and
  retiring a command is a decision of its own rather than the side effect of moving a button —
  the doc comment now says so instead of describing a toggle nobody can press. 77 panel tests
  are 76 and green, `npm run typecheck` clean, and the Rust half is untouched by the change
  and was run anyway: 490 tests, `cargo fmt --all --check`, and `cargo clippy --workspace
  --all-targets -D warnings` clean. **Still to do:** the panel screenshots now lag by two
  things rather than one — the round bead noted above, and a footer button that no longer
  exists. Both are the same re-shoot with `scripts/screenshot.ps1`.
- 2026-09-13 — **T-WP18: a written decision is reversed in writing, and the file that comes
  out of it is specified before it exists.** No code in this package; the maintainer asked
  for usage history (tokens per week, per month, per model, per provider) and the honest
  answer turned out to cost a reversal.

  **What was measured first.** The plan in `docs/FUTURE.md` had the panel differencing the
  status line's own token counters. The payload on this machine says
  `context_window.total_input_tokens` = 562 431, and its own
  `current_usage.{input + cache_creation + cache_read}` adds up to 562 431 — the same number,
  and the same identity in both committed fixtures. It is the **current size of the context
  window**, not a session total: it goes *down* on a `/compact`, where the plan's rule
  "backwards means a new session" would have discarded everything counted so far. The one
  cumulative counter in that document is `cost.total_cost_usd`. The plan is therefore recorded
  as **disproven**, not as replaced by something nicer, and the transcripts are the only source
  that can answer the question at all.

  **What was reversed.** `~/.claude/projects/**` was on the "not read, on purpose" list in
  `docs/pinned-internal-formats.md` and in §5 above, because the retired prototype read up to
  400 transcripts to *estimate* a quota percentage. Usage history reads nine fields —
  `type`, `timestamp`, `message.model`, `message.id`, `requestId` and four `message.usage`
  counters — which are the **server's own reported numbers**, and derives no percentage from
  them. The line moves into the inventory with its fields, its observed versions (Claude Code
  2.1.268 – 2.1.269, Codex 0.153.4) and its fixtures; §5's bullet keeps the half that is still
  true. The Codex rows move the same way: `payload.info.last_token_usage` and
  `turn_context.model` become read, `total_token_usage` and `model_context_window` stay unread,
  and the reason is measured — the cumulative counter **falls back down mid-session** in 3 of
  21 logs here, disagreeing with the per-turn sum in 8 of 30 files, once by 43×.

  **What protects it** is what already protects the Codex reader: a record **constructed** from
  named fields rather than a parsed line filtered, an identifier shape check on the one new kind
  of string (a model id), and a sentinel leak test extended to the accumulated state and to the
  file the scan writes. Two ids are read and never kept — `(message.id, requestId)` is the
  deduplication key, and it has to be one: 44.4 % of lines here are repeats and they inflate
  the totals 1.70×, a factor that is not constant (1.04× in one subagent file) and therefore
  cannot be divided out afterwards.

  **The new file is specified before the code.** `docs/usage-contract.md`:
  `%APPDATA%\nazar\usage\YYYY-MM.json`, one file per UTC month, written whole through the same
  temp-and-rename as `limits.json`, `version: 1`, hourly UTC buckets keyed `YYYY-MM-DDTHH`,
  five counters per model per bucket, `since` and `scanned_at`. **No local time anywhere in the
  store** — the hygiene gate forbids it in Rust, and hours rather than days are what let the
  panel derive local Monday-start weeks exactly. The headline number is
  `input + output + cache_create`, with `cache_read` reported beside it and never folded in,
  because the raw total on this machine is 98.5 % cache reads and a chart of it is a chart of
  nothing. No currency in v1. A merge takes the larger of two values per counter, so a pruned
  transcript cannot erase history. **`limits.json` does not change and stays frozen at v1;
  Nazar reads nothing new.**

  **And the README says so.** The privacy paragraph now names the files that are read and the
  fields that are taken from them, states that no prompt or response text is ever read, and
  points at the test that fails if it is. "Never reads your tokens, never talks to the network"
  stays, because it is about sign-in tokens and is still true — but it stops being the whole
  sentence.
- 2026-09-13 — **T-WP15: `get_usage`, the one command that reads the usage store — and the
  three rules that keep it from turning into a second refresh loop**
  (`crates/nazar-tray/src/usage.rs`, `main.rs`, `ui/src/snapshot.ts`,
  `ui/test/bridge.test.mjs`). T-WP13 built a store that nothing read. This is the bridge, and
  it is deliberately thin: it decides **when** a scan may run, hands the store's own answer
  back unchanged, and turns the ways this can fail into something a panel can put on screen.
  No interface — T-WP16 is the view.

  **The window arrives from the panel, already as instants.** A week starts on Monday in the
  reader's own zone, and `nothing_in_the_workspace_asks_the_machine_what_time_zone_it_is_in`
  fails the build if any Rust file tries to find out which zone that is. So the panel does the
  arithmetic and sends `from` and `to`; `range` comes along as a **name** — `week`, `month` or
  `all` — which is validated and echoed back, so the answer says which question it answers and
  a panel that invents a fourth range gets an error rather than a silent empty week. Any legal
  RFC 3339 spelling is accepted, an offset included, and what comes back is always UTC with a
  `Z`, the rule `limits-contract.md` already applies to every timestamp this product writes.
  The two ends are compared as **instants and not as text**, because an offset gives one
  moment two spellings and sorting those spellings puts them in the wrong order.

  **The scan is not on the refresh path.** Quota is why this application exists, it reads two
  small files in milliseconds, and it must never queue behind a scan that walks hundreds of
  megabytes — so the refresh loop does not scan at all, and a bridge test fails if it ever
  starts. The scan runs here, when the usage view is opened, at most once every five minutes;
  `force` is the Refresh button and nothing else. The throttle counts monotonic milliseconds
  rather than wall-clock ones, for the reason `nazar-core::clock` already gives: a clock
  correction, or a laptop waking with its time set by NTP, must not make a scan due a hundred
  times. It is marked **before** the scan rather than after, because a scan that failed is a
  scan that ran — a `cursors.json` that is no longer JSON is an error and not a fresh start —
  and retrying it on every panel open would turn one broken file into a scan on every click.
  Its mutex is held across the whole scan, which makes two panel opens at once cost one scan
  for free.

  **One writer, and the same lock.** The scan writes where `limits.json`'s single-writer rule
  already reaches, so it runs only in the process that took `~/.nazar/limits.lock` —
  `let writes_usage = lock.is_some()`, read off the same acquisition before the lock goes to
  the refresh loop. A second instance, a `--demo` run and an `--autostart` run all still
  *read* the store and would draw the view; none of them adds to it. No second lock and no
  second discipline, which is what that phrase in the contract was promising.

  **The document is snake_case on both sides, and that is the exception being taken on
  purpose.** Everything else on this bridge is camelCase, because `limits.json` is a contract
  with another program and has to give one spelling to an idea two sources spell differently;
  the usage document keeps the shape `message.usage` already has at the source, which is the
  trade `usage-contract.md` argues at length, and renaming it on the way to the panel would
  have this product spelling the same five counters two ways in two files. So
  `ui/src/snapshot.ts` carries `UsageRequest`, `UsageBucket`, `UsageScan`, `UsageResponse` and
  `UsageError` with the store's own names and a comment saying why, and a bridge test fails if
  a `rename_all = "camelCase"` ever appears in the command's module.

  **An error is a key and a diagnostic, never a sentence.** `kind` is one of `bad_range`,
  `bad_window`, `no_state_dir`, `scan_failed` or `store_unreadable`, which the panel looks up
  in its catalogue like every other word on the page — the i18n gate is what catches the first
  English sentence typed into this module, and it caught three of them while this was being
  written. `detail` names the value that was refused, or repeats what a reader said with the
  home directory collapsed to `~`: the settings page already collapses every path it shows,
  and an error line that ends up in a screenshot is no different. A **damaged month is not an
  error**. It rides back in `damaged`, the months beside it still load, and the scan's list is
  merged with the query's so a month found outside the asked-for window is still reported.

  **Two things this package did not do, both written down rather than worked around.**
  `nazar_core::usage::scan_codex` does not exist yet — T-WP14 is writing it — so the single
  line that will call it is marked in place instead of guessed at. And `CLAUDE_CONFIG_DIR` is
  honoured here rather than in the core crate, which takes a *home* and derives
  `<home>/.claude/projects` from it: an override naming a `.claude` directory is expressible,
  because its parent is the home that yields it, and one naming anything else is not. Rather
  than scan `~/.claude` behind the user's back, **no scan runs** and the store answers from
  what it already holds. The fix belongs in the core crate — a `scan_claude_in(projects_dir,
  state_dir)` beside the existing entry point would take the directory directly — and this is
  recorded as a gap rather than patched around, because a scan of the wrong tree reports
  numbers that are wrong without looking wrong.

  **Measured on the maintainer's machine**, by an ignored end-to-end test that scans into a
  throwaway directory so the real store is neither read nor added to: **129 files, 16 040
  usage lines, 7 232 duplicates, 563 ms**, one provider, `since 2026-09-08T12:00:00Z`, no
  damaged month — and the second call inside the window scanned nothing, which is the throttle
  doing the thing it exists for. 91 tray tests where there were 76, 558 in the workspace,
  `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -D warnings` clean; 78
  panel tests where there were 76, and `npm run typecheck` clean.
- 2026-09-13 — **T-WP14 landed: the Codex usage reader.** A second pass over the rollout logs
  `crates/nazar-core::codex` already opens for quota, reading a different part of them: every
  `token_count` event's `payload.info.last_token_usage`, bucketed by the event's own timestamp
  into the same hourly UTC months under the provider key `codex`. Eight values leave a line and
  nothing else does — allow-listed structs, the identifier shape check on the model id, and the
  sentinel leak test extended to a rollout fixture whose every neighbouring string is a marker.
  Measured here: **23 logs, 52.5 MB, 446 events, 88 ms** for a first full pass and **3.7 ms** for
  the second over an unchanged tree; three models, no malformed line, `cache_write_input_tokens`
  `0` on every event.
  **Three things this package exists to get right.** *(1)* The per-turn counter, never the
  cumulative one: `total_token_usage` falls back down mid-session in 3 of the 22 logs here, so
  `rollout-reset.jsonl` reproduces a reset and asserts the sum (4 730 against the 1 430 the last
  cumulative reading would have claimed). *(2)* The model is on another line — the session's last
  `turn_context` — and is carried in the cursor, because the bytes that named it are behind the
  offset; an event before the first one is `unknown`. *(3)* **There is no event id.** Codex writes
  none, so the cursor's `(file identity, byte offset)` is the whole dedupe, plus one rule for the
  shape that defeats it: a log whose *opening run* of events repeats another log's opening run,
  event for event, is a fork of it and that run is skipped (bounded at 32 events, fixture
  `rollout-fork.jsonl`). Nothing on this machine exercises it; it is a rule with a test rather
  than a measurement.
  **`archived_sessions/` stays unread, and now says why**: a cursor is keyed on a path, so a log
  Codex *moves* there would arrive as a file nothing has read and be counted twice. The cost — a
  session archived before it was ever scanned is never counted — is written down rather than
  discovered. **Each reader has its own cursor document** (`cursors.json`, `cursors-codex.json`)
  because a pass replaces the set of files it knows about; they share the month documents, each
  with its own `applied_through` stamp. `docs/usage-contract.md` gains the three sections that
  were wrong or missing — how a scan *adds* rather than recomputes, what the cursor documents are
  and why deleting one alone double counts, and what Codex spells differently — and
  `pinned-internal-formats.md` gains the fixtures and the version observed. 19 new tests, 407 in
  the crate, workspace green.
- 2026-09-13 — **T-WP16 landed: the usage view.** A third view in the same window, opened by
  a button where T-WP12 took the theme toggle out of the footer. Three tabs — *Week*, *Month*,
  *All* — a headline, one row per model, a strip of days underneath, and three lines saying
  how much of it to believe.
  **(1) The local week is the panel's, and it is arithmetic rather than a library.** The view
  computes Monday 00:00 and the first of the month in the reader's own zone, sends the two
  instants as RFC 3339 **with their offset**, and cuts the hourly UTC buckets it gets back into
  local days (week, month) or local Monday-start weeks (all). A bucket lands in the local day
  its hour **starts** in — the rule `docs/usage-contract.md` states — so 21:00Z on a Sunday is
  Monday's for anybody at +03:00, and an offset of `+05:30` never splits an hour by a ratio it
  would have to invent. The dates are built through the local `Date` constructor rather than by
  adding 86 400 000 ms, which is what keeps the week of a daylight-saving change at seven
  differently named days and a week that crosses New Year with its Monday in the old year.
  Both are tests, in `Europe/Berlin` and at the year boundary.
  **(2) The headline is `input + output + cache_create`, and the cache line is beside it.**
  Never inside it: cache reads were 98.5 % of the raw total over six days of real work, so a
  headline that folded them in would be a number about cache behaviour with the work lost in
  the rounding. **An absent counter prints an em dash and a reported zero prints `0`** — the
  same distinction the quota view draws between "nobody read this" and "you have used none of
  it", and it earns its keep here because Codex reports `cache_create: 0` on every event while
  a Claude record can carry no counters at all. Model ids are printed raw, never merged and
  never translated. A row's second number is labelled twice over — *req.* for Claude, *evt.*
  for Codex — because T-WP14 counts `token_count` events, several of which make one turn, and
  one word over both would be a label that lies about one of them.
  **(3) No charting library** (decision K2). The strip is one flex item per day with a height
  in percent, and a model's share is the same `.meter` the quota rows use, so the view themes
  itself and weighs nothing. The rows scroll at 272 px rather than asking for a window taller
  than the 140–720 px clamp `panel.rs` enforces, which is the one thing that would put numbers
  where nobody can see them.
  **(4) The words.** Twenty keys in all six languages, and **no magnitude mark among them**:
  `Intl.NumberFormat`'s compact notation writes `22.3M` in English, `22,3 млн` in Russian and
  `2230만` in Korean, so there is no `K`, `M` or `B` in a locale file to get wrong — and the
  value is **floored to the precision it is shown at** before it is formatted, because 22.39 M
  is not 22.4 M, the same rule as a quota percentage. The frozen counted-key list is untouched:
  what `usage.cacheRead` interpolates is a finished string, like `{time}` and `{age}` before
  it. An error is a key and a diagnostic — the five kinds `get_usage` can return, plus a sixth
  sentence for a kind this build has not met — with `detail` printed verbatim underneath, and a
  damaged month is a notice rather than an error.
  **97 panel tests where there were 78**, `npm run typecheck` clean, and the bridge test now
  watches the `usage.*` namespace too. **Not verified on screen:** this package was written and
  tested without running the tray, so the layout at 360 px, the scroll ceiling and the tab row
  in six languages are argued rather than seen. The numbers, the re-bucketing and the words are
  tested; the pixels are owed a look.
- 2026-09-13 — **T-WP13b landed: the nine findings of an independent review of the usage
  module, fixed.** A read-only audit of `crates/nazar-core/src/usage/` at `ba015f2` found one
  critical, three high, one medium and four low. Three of them were the same bug wearing
  different clothes — *what happens when a log is read a second time* — and the fix is one
  idea rather than three.

  **The cursor carries everything it has credited, not the last sixteen keys.** A transcript
  that is truncated, rotated or rewritten is read from byte zero again, and the first version
  recognised sixteen of the records it found there; the rest were credited a second time,
  into months that already held them, permanently and with nothing able to detect it
  afterwards. So `cursors-claude.json` now holds every `(message.id, requestId)` key a
  transcript has produced and the **largest** reading of each — the largest, never the latest,
  which is the fourth finding: three passes seeing `cache_read` 100, 90 and 100 used to credit
  110. A restart seeds the deduper with that map rather than clearing it, so re-reading the
  same bytes adds nothing. Codex has no event id, so the same guarantee is built out of the
  only evidence a rollout offers: the fingerprint of every credited event — its timestamp to
  the millisecond and its four raw counters — matched with multiplicity, so a log that
  genuinely holds two identical events keeps both. The cost is the document: **382 KB for
  9 457 messages across 136 transcripts here**, against 176 KB before, and the trade is
  written into `usage-contract.md` rather than discovered.

  **The identity says it is the same file; a fingerprint says the offset is still the same
  place in it.** A transcript truncated and rewritten in place keeps its birth time, its inode
  and its first 512 bytes, and if its new length reaches the old offset then every record
  written before that offset is never read. The cursor now carries a hash of the 64 bytes
  immediately before it, checked on resume, and a mismatch takes the restart path above. It
  lives in the byte reader both providers share.

  **A month that cannot be written keeps its totals.** `pending` was one block per pass and
  was cleared whether or not it was filed, so a damaged `2026-09.json` swallowed the records
  that pass had just read — their bytes already behind a cursor that had moved, so repairing
  the month afterwards could not bring them back. It is now a journal of one entry per month,
  and an entry survives until its month accepts it. Two limits are written down: entries for
  the same month merge and carry the newest generation, so the journal stays bounded, and a
  user who "repairs" a month by restoring an older copy of it can still double count.

  **And five smaller ones.** `isApiErrorMessage` is deserialised and such lines are skipped
  and counted — all 11 on this machine were also `<synthetic>`, so no total moved, and the
  flag is read because the flag is what promises the line is an error. A token counter is a
  non-negative integer or the line is malformed: `1.9` is no longer truncated to `1` and `"7"`
  is no longer parsed to `7`. `Record`, `Outcome` and `FileScan` have hand-written `Debug`
  that prints counters and classification and never a `message.id` or a `requestId`, and the
  sentinel leak test now formats all three. `scan_file` errors carry `Error::Log`, a redacted
  label — the same hash the cursor files the log under — instead of a path made of the
  directories somebody works in. And a stored bucket now always writes all five counters,
  `0` where nothing reported one, which is what the contract said and what the writer was not
  doing; the absent-versus-zero distinction is real one record at a time and is kept there.

  Two things that are not fixes. `cursors.json` becomes `cursors-claude.json`, so that two
  documents doing the same job have the same kind of name (`rebuild()` still removes the old
  spelling, so a store written by an earlier build cannot be left behind to double count).
  And `scan_claude_in(projects_dir, state_dir)` joins the core crate, which is the gap T-WP15
  recorded rather than patched around: `CLAUDE_CONFIG_DIR` names a directory that need not be
  called `.claude`, and a scan derived from a home directory could not express it. Calling it
  is the tray crate's, which this package deliberately did not touch.

  **Measured against a frozen copy of this machine's transcripts, old code and new**: 136
  files, 17 132 usage lines, **9 457 unique messages, 7 664 duplicates, inflation 1.701×** —
  byte for byte the same totals, credited and naive, on both sides. The only differences are
  the 11 API-error lines, now named rather than counted under `<synthetic>`, and the cursor
  document's size. 13 new tests, 421 in the crate's own suite where there were 408, and 589
  passing across the workspace; `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -D warnings`, the hygiene gate and
  `--no-default-features` all clean.
