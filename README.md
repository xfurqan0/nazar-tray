# 🧿 nazar-tray

**Your Claude Code and Codex quota, in the Windows system tray.** The tray face of [Nazar](https://github.com/xfurqan0/nazar).

A bead in your tray fills up as you burn through your 5-hour and weekly windows. Click it for the full picture: every window, its percentage, and when it resets. Amber at 60 %, red at 85 %, a toast before you hit the wall.

> Status: **pre-alpha, spec stage.** The underlying tray has run daily on the author's machine since August 2026; it is being migrated into this repo. See [docs/PROJECT.md](docs/PROJECT.md) for the v1 spec.

## Why another quota tray

There are many. This one is built on one rule:

**The tray never touches your credentials or the network.**

Most quota tools read your OAuth token, or even your browser cookies, from inside an unsigned binary. nazar-tray splits the job in two:

- `nazar-limits`, a small fetcher, reads the tokens Claude Code and Codex already keep on disk, holds them in memory only, calls the two official usage endpoints, and writes a `limits.json` that contains **no tokens and no account ids**.
- `nazar-tray`, the thing you see, only reads that file. The compiled tray links no networking stack at all. You can verify that on the binary.

The same `limits.json` feeds the quota strip on the Nazar canvas, so installing the tray gives Nazar its quota for free.

## Roadmap

- v1: Windows, Claude Code + Codex, bead icon, themes (`nazar`, `graphite`), threshold toasts, one-script install
- v2: macOS and Linux (Tauri rewrite), light theme, more providers

## Credits

The original panel layout was inspired by [Win-CodexBar](https://github.com/nesszer/Win-CodexBar). Provider icons from [@lobehub/icons](https://github.com/lobehub/lobe-icons) (MIT).

## License

MIT
