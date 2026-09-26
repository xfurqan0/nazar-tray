# nazar-tray 0.3.0

**Your Claude Code and Codex quota, in the system tray — and now on Linux too.** This is
the first release with a `.deb` and an `.rpm` beside the Windows installer, all three built
by the same release workflow from this tag. `limits.json` is unchanged and stays at
`schemaVersion: 1`, so Nazar and every face read it exactly as before.

    winget install xfurqan0.nazar-tray

Windows 10 1809 or newer. Installs per user, into `%LOCALAPPDATA%\nazar-tray`, and asks
for no administrator rights. **winget follows the release as a pull request that a
reviewer merges**, so until that command resolves to 0.3.0, take
`nazar-tray_0.3.0_x64-setup.exe` from the assets below.

    sudo apt install ./nazar-tray_0.3.0_amd64.deb      # Debian 12+, Ubuntu 22.04+
    sudo dnf install ./nazar-tray-0.3.0-1.x86_64.rpm   # Fedora

Built on `ubuntu-22.04` against glibc 2.35, so it runs on Debian 12 and anything newer.
x86_64 only, and this page is the only place to get it: there is no COPR, AUR or Flatpak
package, and winget is Windows only.

## What's new on Linux

- **The engine on every desktop, the tray where there is a tray.** KDE, XFCE, Cinnamon,
  Budgie and Ubuntu's GNOME draw the bead. A stock GNOME has no StatusNotifier host, so
  nazar-tray asks the session bus first and, when nobody is listening, says so once and
  keeps running as the engine — refresh loop, lock, `limits.json` and notifications.
  `--headless` asks for that on purpose. A host that turns up later, as GNOME's does
  after a lock screen, gets its icon with no restart.
- **The numbers without a tooltip.** The GTK tray shows none, so the menu opens with a
  live row per provider, and the binding percentage sits beside the bead as a label.
- **Faces for the desktops with no tray**: [nazar-gnome](https://github.com/xfurqan0/nazar-gnome)
  in the GNOME top bar, and a Waybar module in `faces/waybar/`.
- **Settings that belong to the desktop**: *Start with the desktop session*, the label
  switch, and an opt-in to open the panel through XWayland, where it can be placed.
  Quiet hours and the system language work on Linux now.
- **The engine comes back after a reboot.** A lock left by the last session is taken over
  at once when its process id now belongs to something else, instead of five minutes of
  "already running" — which the kernel's pid reuse made happen on every reboot.

## What's new everywhere

- **The tray menu** is Open panel · Settings… · Usage history · Refresh now · Quit, and a
  **gear** in the panel header opens the settings.
- **Alt+F4 hides the panel** instead of ending the run. *Quit* is the way out.
- **`nazar-tray --help`** exists, and on Windows it — like `--print` — now prints into
  the terminal it was typed in.
- **A 57 % window reads 57**, not 56, and 99.6 % reads 99 everywhere — the wrapper's own
  status line included. `limits.json` still stores what was reported.
- **"Not set up" says what to do**: Claude Code without the wrapper reads *status line
  wrapper not installed*, with the command that fixes it.
- **Codex's compressed session logs are read**, for the day Codex turns that on.
- **rustls 0.23.45**, which closes RUSTSEC-2026-0285.

## Known limits

- **Removing the Linux package does not put Claude Code's status line back.** If you
  installed the wrapper, run `nazar-statusline uninstall` before `apt remove` or
  `dnf remove`. The Windows uninstaller does this for you.
- **Upgrading a Linux package leaves the old process running.** Afterwards run
  `pkill -x nazar-tray` and start it again, or log out and back in.
- On Wayland the panel opens where the compositor puts it, and with no tray icon it
  opens only by running `nazar-tray` a second time.
- Claude numbers move only while a session refreshes its status line. Between sessions
  the last value is shown with its age.
- Model-scoped weekly windows still need the opt-in detailed-windows mode, which reads a
  token — `docs/detailed-windows.md` before you switch it on.
- Codex's log format is not a documented contract; the reader is defensive and reports
  *unknown* rather than guessing.
- **Unsigned.** SmartScreen warns on a browser download — *More info → Run anyway*;
  `winget install` does not go through it. Why, and what to check instead:
  `docs/CODE_SIGNING.md`.
- Windows 11 keeps new tray icons in the `^` overflow; the first run says so.

Full list in the [README](https://github.com/xfurqan0/nazar-tray#known-limits), and
every change in the [CHANGELOG](https://github.com/xfurqan0/nazar-tray/blob/main/CHANGELOG.md).

## Verify

Every file below was built by the release workflow on a GitHub runner, from this tag —
the installer on Windows, the packages on `ubuntu-22.04` — and `SHA256SUMS` is the one
hash list covering all three.

    Get-FileHash .\nazar-tray_0.3.0_x64-setup.exe -Algorithm SHA256
    gh attestation verify .\nazar-tray_0.3.0_x64-setup.exe --repo xfurqan0/nazar-tray

    sha256sum -c --ignore-missing SHA256SUMS
    gh attestation verify nazar-tray_0.3.0_amd64.deb --repo xfurqan0/nazar-tray
    gh attestation verify nazar-tray-0.3.0-1.x86_64.rpm --repo xfurqan0/nazar-tray

The attestation is the stronger of the two: signed by GitHub, it names the workflow run
and the commit that produced that exact file.

MIT licensed.
