# nazar-tray 0.1.0

**Your Claude Code and Codex quota, in the system tray.** A bead fills up as you
burn through your 5-hour and weekly windows; click it for every window, its
percentage and when it resets.

    winget install xfurqan0.nazar-tray

Windows 10 1809 or newer. Installs per user, into `%LOCALAPPDATA%\nazar-tray`,
and asks for no administrator rights.

**The winget package is reviewed a day or two after this release**, so until that
command resolves, take `nazar-tray_0.1.0_x64-setup.exe` from the assets below and
run it.

## What it does

- Both providers, both windows each, with the binding one in the tooltip.
- Amber at 60 %, red at 85 %, and a notification on the crossing — once per
  window per reset, with quiet hours.
- Reads local files only: no credentials and no network in the default build.
- Writes `~/.nazar/limits.json`, which the Nazar canvas reads.
- Six languages: English, Türkçe, 中文, 한국어, Русский, Español.
- `nazar-tray --print` for scripts, and an uninstaller that puts your status line
  back before it removes the wrapper.

## Known limits

- Claude numbers move only while a session refreshes its status line. Between
  sessions the last value is shown with its age.
- Model-scoped weekly windows need the opt-in detailed-windows mode, which reads
  a token — `docs/detailed-windows.md` before you switch it on.
- Codex's log format is not a documented contract; the reader is defensive and
  reports *unknown* rather than guessing.
- **Unsigned.** SmartScreen warns on a browser download — *More info → Run
  anyway*; `winget install` does not go through it. Why, and what to check
  instead: `docs/CODE_SIGNING.md`.
- Windows 11 keeps new tray icons in the `^` overflow; the first run says so.

Full list in the [README](https://github.com/xfurqan0/nazar-tray#known-limits).

## Verify

Both files below were built by the release workflow on a GitHub runner, from this
tag, and `SHA256SUMS` is the hash list it wrote:

    Get-FileHash .\nazar-tray_0.1.0_x64-setup.exe -Algorithm SHA256
    gh attestation verify .\nazar-tray_0.1.0_x64-setup.exe --repo xfurqan0/nazar-tray

The attestation is the stronger of the two: signed by GitHub, it names the
workflow run and the commit that produced that exact file.

MIT licensed.
