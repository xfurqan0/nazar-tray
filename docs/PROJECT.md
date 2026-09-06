# nazar-tray — project notes and v1 spec

Internal project notes. Kept in English so the repo is readable by everyone.

> **The system-tray face of Nazar.** Shows your Claude Code and Codex usage windows (5-hour and weekly) in the tray, with an icon that fills up as you burn quota. Feeds the same `limits.json` that the Nazar canvas reads.

- Folder: `C:\nazar-tray` · Repo: `github.com/xfurqan0/nazar-tray` — private until first release
- Started: 2026-09-07 (maintainer decision; the underlying tray app has existed since 2026-08-27 inside a private vault)
- License: MIT · Sibling projects: [Nazar](https://github.com/xfurqan0/nazar) (canvas monitor), Dile (dictation)
- Competitive scan: internal notes, 2026-09-06 (19+ Windows tray tools found; summary in §1)

---

## 1. Why this exists, and why it is not "the 20th tray"

A Windows tray showing AI-agent quota is a crowded niche: Win-CodexBar (1.0k★), Token Monitor (2.0k★), CodeZeno's Usage Monitor, and a dozen smaller ones, most committed to within the last week. Nothing here wins on "tray + progress bars".

nazar-tray is different in two ways, and only these two are worth writing in the README:

1. **The tray process never touches credentials or the network.** Most tools in this niche read `~/.claude/.credentials.json` or even browser cookies from inside an unsigned binary. nazar-tray splits the job: a small fetcher (`nazar-limits`) reads the local OAuth tokens in memory, calls the two official usage endpoints, and writes a token-free `limits.json`. The tray only reads that file. The compiled tray links no networking stack at all, which can be verified from the binary.
2. **It is part of Nazar.** The same `limits.json` drives the quota strip on the Nazar canvas. Install the tray, and the canvas gets quota for free. The tray stays a separately downloadable app (maintainer decision, 2026-09-07); it does not get folded into Nazar.

## 2. Scope and non-goals

| nazar-tray **is** | nazar-tray **is not** |
|---|---|
| Tray icon + popup panel with Claude and Codex windows, reset countdowns | A launcher, session manager, or cost analytics dashboard |
| A tiny fetcher that writes `~/.nazar/limits.json` on a schedule | A tool that stores, logs, or forwards tokens anywhere |
| Windows first (v1), macOS and Linux later (v2, Tauri rewrite) | A cross-platform app in v1 |

## 3. What already exists (source of v1)

Built 2026-08-27 as a personal tool, audited, running daily since. To be migrated into this repo at M1:

| Piece | Current form | Notes |
|---|---|---|
| Tray app | `Tray.cs`, ~690 lines, C# 5, WinForms, compiled with the `csc.exe` that ships with .NET Framework 4.x — **no SDK needed**, ~24 KB exe | Hard-coded vault paths; Turkish comments; visual language borrowed from Win-CodexBar (Apple system greys) |
| Fetcher | `limit-widget.js`, Node stdlib only | Reads `~/.claude/.credentials.json` and `~/.codex/auth.json` in memory, calls `api.anthropic.com/api/oauth/usage` and `chatgpt.com/backend-api/codex/usage`, writes `limits.json` + a Markdown note; keeps last-good data on failure and marks it stale |
| Scheduler | Windows Scheduled Task every 30 min via a hidden `.vbs` launcher | `install-limit-widget-task.ps1` |
| Autostart | Startup-folder shortcut (added 2026-09-06) | |

Security profile of the tray, verified on source and binary: no `System.Net`, `wininet`, `ws2_32`, `winhttp`, `crypt32`; no credential file paths; exactly two `Process.Start` uses (hidden refresh, note open) — the note-open action was removed on 2026-09-06, leaving one.

## 4. v1 scope (one working day)

**In:**
1. Migrate `Tray.cs` → `src/tray/`, `limit-widget.js` → `src/limits/`. **All comments and strings in English.** Paths parameterized: config at `%APPDATA%\nazar\config.json`, data at `~/.nazar/limits.json` (shared with Nazar).
2. **New visual identity.** Drop the Apple-grey / Win-CodexBar look. Palette from the nazar bead: deep blue, light blue, white, black dot. Tray icon = the bead, filling from the bottom as the binding window fills; amber at 60 %, red at 85 %. Panel on a navy ground. Remove the "Win-CodexBar panel" comment from the source; acknowledge inspiration in README instead.
3. **Themes as a JSON file.** Two shipped: `nazar` (default) and `graphite` (today's dark grey). Palette + thresholds only. More themes later, shared with Nazar's theme system.
4. Threshold notifications (Windows toast) at 85 % and 100 %, once per window per reset.
5. Stale-data state: if `limits.json` is older than 45 min, icon turns grey.
6. Show the Fable weekly window separately (present in the data, not yet in the tray).
7. `install.ps1`: compile with `csc.exe`, place exe, register the scheduled task, add autostart. `uninstall.ps1` reverses all of it.

**Out (v2+):** macOS/Linux (Tauri rewrite, same stack as Dile) · light theme · per-provider enable/disable · history graph · other providers (Gemini, Copilot).

**Known limits (README):** the Codex usage endpoint is undocumented and may change. The tray is unsigned; SmartScreen will warn (same as every tool in this niche; code signing is a later decision). Windows 11 hides new tray icons in the `^` overflow by default.

## 5. Architecture

```
nazar-limits (Node, every 30 min)        nazar-tray (C#, reads only)
  reads OAuth tokens in memory   ──▶   ~/.nazar/limits.json   ──▶   icon + panel
  calls 2 official endpoints                  │
  writes token-free JSON                      └──▶   Nazar canvas quota strip
```

`limits.json` schema (v1, stable, shared with Nazar): `{ updatedAt, providers: { claude: { plan, ok, stale, windows: [{ label, percent, resetsAt, active }] }, codex: {...} } }`. No tokens, no account ids.

## 6. Milestones

| # | What | Acceptance | Time |
|---|---|---|---|
| M1 | Migrate sources, English comments, parameterized paths, `limits.json` at `~/.nazar/` | Builds with `build.ps1` on a clean Windows; tray shows today's data | ½ day |
| M2 | Nazar identity: icon, palette, theme JSON, remove borrowed look | Icon fills as bead; `graphite` theme switch works | ½ day |
| M3 | Toasts, stale-grey, Fable window, install/uninstall scripts, README + GIF | Fresh install → tray in 2 minutes; uninstall leaves nothing | ½ day |

Runs in parallel with Nazar, triggered when the maintainer says go.

## 7. Open decisions
- Code signing: none for v1. Revisit if downloads grow.
- Whether `nazar-limits` should live here or in the Nazar repo. v1: here (it ships with the tray). Nazar consumes the file.
- Icon rendering: GDI+ at runtime (current) vs. pre-rendered PNG set. v1: runtime, it already works.

## 8. Log
- 2026-09-07 01:55 — Repo opened, spec written. Decisions (maintainer): stays a separate downloadable app; new nazar identity replaces the Win-CodexBar-derived look; cross-platform via Tauri in v2; all code, comments, commits and docs in English. No code migrated yet.
