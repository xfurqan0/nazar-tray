# 🧿 nazar-tray

**Your Claude Code and Codex quota, in the system tray. Zero credentials, zero network.** The tray face of [Nazar](https://github.com/xfurqan0/nazar).

A bead in your tray fills up as you burn through your 5-hour and weekly windows. Click it for the full picture: every window, its percentage, and when it resets. Amber at 60 %, red at 85 %, a notification before you hit the wall.

> Status: **skeleton (WP0).** The workspace builds, the tests pass and the tray opens an
> empty panel that says so. No reader, no state model, nothing written to disk yet.
> Windows first; macOS and Linux builds later from the same codebase.
> See [docs/PROJECT.md](docs/PROJECT.md) for the v1 plan and [CHANGELOG.md](CHANGELOG.md)
> for what has landed.

## Why another quota tray

There are many. This one is built on one rule:

**By default, nazar-tray never reads your tokens and never talks to the network.**

Every other quota tool reads your OAuth token, or even your browser cookies, from inside an unsigned binary, then calls an undocumented endpoint that rate-limits them. nazar-tray does neither by default, because the numbers are already on your disk:

- **Codex** writes its server-reported usage into every session log (`~/.codex/sessions/…/rollout-*.jsonl`).
- **Claude Code** hands the same numbers to your status line on every refresh. nazar-tray installs a tiny status-line wrapper (`nazar-statusline`) that records them and then runs whatever status line you already had, unchanged. It takes a copy of `settings.json` first, shows you the diff before it writes, and `nazar-statusline uninstall` puts your own status line back exactly. What it captures, where, and how to undo it by hand: [docs/statusline-wrapper.md](docs/statusline-wrapper.md).

That is the whole default data path: two local files in, one local `limits.json` out. The same file feeds the quota strip on the Nazar canvas.

**Detailed windows (opt-in).** Claude's status line only reports the 5-hour and the global weekly window. If you are on a Max plan, your real constraint may be a model-specific weekly window that only the official usage endpoint reports. Turn on *Detailed windows* in settings and nazar-tray will read the token Claude Code already stores, keep it in memory for a single request, and never write or log it. Off by default; the app asks once if it detects a Max plan.

With the mode **off** — which is how it ships — nothing in nazar-tray opens a credential file or a socket, and a test poisons that file to prove it. With it **on**, this is the whole of it: `claudeAiOauth.accessToken` is read out of `~/.claude/.credentials.json` (which is never written), held in a wrapper that wipes itself and prints `<redacted>`, and sent as one `Authorization` header on a single 20-second `GET` to `api.anthropic.com/api/oauth/usage` — the endpoint your own `/usage` command calls. The answer becomes your model-scoped weekly windows, marked `detailed` in `limits.json`. The token is never written to a file, never logged and never put in an error message, and no response body reaches one either; a test runs the entire flow with a sentinel in place of the token and fails if it turns up anywhere but that header. When the endpoint says no, the last known numbers stay with a *stale* flag and the next attempt backs off — 1 s, 2 s, 4 s and so on to half an hour, or whatever `Retry-After` asked for. Your `refreshToken` is never read: refreshing is Claude Code's job. `nazar-tray --print --detailed` runs the mode once without switching it on. The full account, including where this sits against Anthropic's terms and why it is your call rather than the default, is in [docs/detailed-windows.md](docs/detailed-windows.md).

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

## Development

Windows, because that is what v1 targets. `nazar-core` builds and tests on Linux and
macOS too, and CI keeps it that way.

**Prerequisites**

| Tool | Version | Why |
|---|---|---|
| Rust | stable, ≥ 1.85 | `rust-toolchain.toml` pins the channel and pulls `clippy` and `rustfmt` |
| MSVC Build Tools 2022 | with the C++ workload | there is no linker without it |
| WebView2 | already part of Windows 10 1803+ and Windows 11 | the panel runs in it |
| Node | 22 | builds the panel and runs its tests |
| `tauri-cli` | 2.x | `cargo install tauri-cli --locked` — only needed for `tauri build` and `tauri icon` |

**Layout**

```
crates/nazar-core    the limits.json contract, its writer and reader. No Tauri.
                     `--no-default-features` drops the opt-in detailed-windows mode
crates/nazar-tray    the Tauri v2 app: tray icon, panel window. Windows first.
ui/                  the panel: plain TypeScript, HTML and CSS, bundled by esbuild
fixtures/            limits.sample.json, the file consumers copy into their tests
scripts/             licence gate and the bead rasteriser
docs/                the plan, and the limits.json contract
```

**Everyday commands**

```powershell
cd ui; npm ci; npm test      # builds ui/dist, then runs the panel tests
cd ..; cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
node scripts/check-licenses.mjs
```

**Running it**

`ui/dist` must exist before the Rust side builds, and nothing builds it implicitly:

```powershell
cd ui; npm run build; cd ..
cd crates/nazar-tray
cargo tauri dev            # or: cargo tauri build --debug
```

**Regenerating the icons** — after editing `ui/assets/bead.svg`:

```powershell
node scripts/render-bead-png.mjs
cargo tauri icon ui/assets/bead-1024.png -o crates/nazar-tray/icons
```

## Credits

The original panel layout was inspired by [Win-CodexBar](https://github.com/nesszer/Win-CodexBar). Provider icons from [@lobehub/icons](https://github.com/lobehub/lobe-icons) (MIT). Built with [Tauri](https://tauri.app).

## License

MIT
