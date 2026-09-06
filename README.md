# 🧿 nazar-tray

**Your Claude Code and Codex quota, in the system tray. Zero credentials, zero network.** The tray face of [Nazar](https://github.com/xfurqan0/nazar).

A bead in your tray fills up as you burn through your 5-hour and weekly windows. Click it for the full picture: every window, its percentage, and when it resets. Amber at 60 %, red at 85 %, a notification before you hit the wall.

> Status: **pre-alpha, planning complete, no code yet.** Windows first; macOS and Linux builds later from the same codebase. See [docs/PROJECT.md](docs/PROJECT.md) for the v1 plan.

## Why another quota tray

There are many. This one is built on one rule:

**nazar-tray never reads your tokens and never talks to the network.**

Every other quota tool reads your OAuth token, or even your browser cookies, from inside an unsigned binary, then calls an undocumented endpoint that rate-limits them. nazar-tray does neither, because the numbers are already on your disk:

- **Codex** writes its server-reported usage into every session log (`~/.codex/sessions/…/rollout-*.jsonl`).
- **Claude Code** hands the same numbers to your status line on every refresh. nazar-tray installs a tiny status-line wrapper that records them and then runs whatever status line you already had.

That is the whole data path: two local files in, one local `limits.json` out. No token is ever read, stored, or sent. The same file feeds the quota strip on the Nazar canvas.

## What you get

- Tray bead icon showing your most-constrained window; grey means "unknown", never a false zero
- Popup panel with both providers, all windows, reset countdowns
- Notifications at 60 / 85 / 100 %, once per window per reset
- Themes (`nazar`, `graphite`), autostart, six UI languages (English, Türkçe, 中文, 한국어, Русский, Español)
- `nazar-tray --print` for scripts and for Linux

## Roadmap

- v1: Windows (winget + GitHub Releases), Claude Code + Codex
- v2: macOS build, multiple accounts, more providers
- Linux: CLI output and the Nazar canvas; a tray popup is not reliably possible on Linux today

## Credits

The original panel layout was inspired by [Win-CodexBar](https://github.com/nesszer/Win-CodexBar). Provider icons from [@lobehub/icons](https://github.com/lobehub/lobe-icons) (MIT). Built with [Tauri](https://tauri.app).

## License

MIT
