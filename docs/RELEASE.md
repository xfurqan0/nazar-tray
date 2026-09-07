# Release checklist

Every command on this page is **for the maintainer to run**. Nothing here is run
by CI or by any assistant: publishing, tagging, submitting a package and changing
a repository's visibility are one-way doors, and they belong to a person.

Read it top to bottom the first time. Steps 0 to 5 are reversible; step 6 onwards
is not.

---

## 0. Preconditions

- The gate in the unified plan (§6.1) is green — every functional and hygiene
  item, each one either ✅ or a ⚠ you have read and accepted.
- `main` is clean and CI is green on it: lint, tests and the panel on Windows,
  `nazar-core` + `nazar-statusline` on Ubuntu and macOS, and the installer job,
  which is a gate rather than a courtesy since WP7.
- You are signed in to GitHub as `xfurqan0`, with two-factor authentication on.

```powershell
git switch main
git pull --ff-only
git status --porcelain          # must print nothing

gh auth status                  # must show xfurqan0
gh run list --branch main --limit 3
```

---

## 1. Set the version

Four files carry it, and a test asserts they agree.

```powershell
# Edit by hand, all in the same commit:
#   Cargo.toml                        -> [workspace.package] version = "0.1.0"
#   crates/nazar-tray/tauri.conf.json -> "version": "0.1.0"
#   ui/package.json                   -> "version": "0.1.0"
#   packaging/winget/*.yaml           -> PackageVersion: 0.1.0  (three files)

Select-String -Path Cargo.toml, crates\nazar-tray\tauri.conf.json, ui\package.json -Pattern 'version'
Select-String -Path packaging\winget\*.yaml -Pattern 'PackageVersion'
```

Move the `## [0.1.0] — unreleased` heading in `CHANGELOG.md` to `## [0.1.0] — YYYY-MM-DD`
with today's date, and set the README status line to the released wording (drop
"release candidate (WP7) — not yet published").

The `Cargo.lock` moves with the version. Let cargo do it rather than editing it:

```powershell
cargo check --workspace --locked   # fails if the lock file is behind
git add -A
git commit -m "Release 0.1.0"
```

---

## 2. Build from clean

A stale `ui/dist` or a stale sidecar is the classic way to ship something that
does not match the source. Start from nothing.

```powershell
Remove-Item -Recurse -Force target, target-baseline, ui\dist, ui\node_modules -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force crates\nazar-tray\binaries -ErrorAction SilentlyContinue

cd ui; npm ci; cd ..
node scripts/build-installer.mjs
```

That one script does the four things whose order matters — the panel, the
third-party notices, the sidecar, then `cargo tauri build` — and prints the
artefacts with their sizes and SHA-256 at the end. Keep that output; step 6 uses
it.

Expected, for 0.1.0 on `x86_64-pc-windows-msvc`:

| Artefact | Size |
|---|---|
| `nazar-tray.exe` | 4.79 MB |
| `nazar-statusline.exe` | 335 KB |
| `nazar-tray_0.1.0_x64-setup.exe` | 1.99 MB |

Measured 2026-09-07 with the release profile in `Cargo.toml` (`opt-level = "s"`,
`lto`, one codegen unit, stripped, `panic = "abort"`). Cargo's stock release
settings give 11.95 MB, 497 KB and 3.06 MB for the same source, which is what
those four lines are worth.

An installer that is suddenly 10 MB means something got into the bundle. Look at
`bundle.resources` in `tauri.conf.json` first.

---

## 3. Inspect the bundle

The installer must carry **five** things and nothing else:

```
nazar-tray.exe
nazar-statusline.exe
LICENSE.txt
THIRD-PARTY-NOTICES.md
uninstall.exe            (written by NSIS at install time, not packed)
```

Plus the panel, which is compiled into `nazar-tray.exe` rather than shipped as
files. No fixtures, no screenshots, no documentation, no `.pdb`.

Read the file list out of the package itself:

```powershell
# 7-Zip reads an NSIS installer's contents without running it.
& "C:\Program Files\7-Zip\7z.exe" l target\release\bundle\nsis\nazar-tray_0.1.0_x64-setup.exe
```

Then install it and look at what landed:

```powershell
Get-ChildItem -Recurse "$env:LOCALAPPDATA\nazar-tray" | Select-Object Name, Length
```

---

## 4. Smoke the installer as a user would

```powershell
$setup = "target\release\bundle\nsis\nazar-tray_0.1.0_x64-setup.exe"

# Silent, per user, no elevation prompt.
Start-Process $setup -ArgumentList "/S" -Wait

# It is running, it is on the Start Menu, and it answers.
Get-Process nazar-tray
Test-Path "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\nazar-tray.lnk"
& "$env:LOCALAPPDATA\nazar-tray\nazar-tray.exe" --print | Select-Object -First 20
& "$env:LOCALAPPDATA\nazar-tray\nazar-statusline.exe" status
```

Click the bead. The panel opens, both providers are drawn, the countdowns tick.
Open the settings page, switch the language, switch the theme. Then the way out:

```powershell
Start-Process "$env:LOCALAPPDATA\nazar-tray\uninstall.exe" -ArgumentList "/S" -Wait

# Nothing left behind that should not be.
Test-Path "$env:LOCALAPPDATA\nazar-tray"
Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -Name nazar-tray -ErrorAction SilentlyContinue
Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" -Name nazar-tray -ErrorAction SilentlyContinue
Test-Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\nazar-tray"
Test-Path "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\nazar-tray.lnk"
```

All five must be absent or `False`.

**Two things are kept on purpose, and both are correct.**

- `%APPDATA%\nazar` — your settings and the record of which warnings you have
  already been given. The GUI uninstaller's *delete application data* checkbox
  removes it; a silent uninstall (which is what `winget uninstall` performs)
  never sees that page, so it stays. Remove it by hand with
  `Remove-Item -Recurse "$env:APPDATA\nazar"`.
- `~/.nazar` — `limits.json`, the status-line captures, and `chain.json`.
  **Never removed by any path.** Nazar reads `limits.json`, and a user may be
  running Nazar without this tray having ever been installed; `chain.json` is
  the only machine-readable record of the status line the wrapper replaced.

---

## 5. Last read-through

```powershell
# No AI attribution anywhere in the history or the tree.
git log --all --format=%B | Select-String -Pattern "generated with|co-authored-by|claude\.ai/" -CaseSensitive:$false
git grep -niE "generated with|co-authored-by" -- . ':!Cargo.lock' ':!ui/package-lock.json'

# No personal data, no absolute paths, no leftover secrets.
git grep -niE "yldz|@gmail|C:\\\\Users|/Users/|/home/" -- . ':!Cargo.lock' ':!ui/package-lock.json'
git grep -nE "(sk-ant-|ghp_|xox[abprs]-|AIza)" -- . ':!Cargo.lock'

# The gates that answer these questions properly, on their own.
cargo test -p nazar-core --test hygiene
cd ui; npm test; cd ..
```

All of it must come back empty or green. The only expected hits are the
maintainer's own name in `LICENSE`, `tauri.conf.json` and the winget manifests,
and the forbidden patterns written into the gate tests themselves.

Also confirm by eye:

- `LICENSE` — MIT, the right year and name, and the same name in
  `tauri.conf.json`'s `copyright` and in `packaging/winget/*.locale.en-US.yaml`.
- `THIRD-PARTY-NOTICES.md` — regenerated by the build in step 2, and current
  (`node scripts/third-party-notices.mjs --check`).
- `ui/assets/LICENSE-lobehub.txt` — present, and credited in the README.
- `.gitattributes` — present, so line endings do not turn a first contributor's
  pull request into a whole-repository diff.
- The README's screenshots render on github.com once the repository is public.

Write the release notes, which the workflow will attach to the draft:

```powershell
# docs/release-notes-0.1.0.md — the CHANGELOG entry, shortened, with the known
# limits kept rather than buried. Template at the bottom of this page.
git add -A
git commit -m "Release notes for 0.1.0"
git push origin main
```

---

## 6. Tag

**From here on, nothing is reversible.**

```powershell
git tag -a v0.1.0 -m "nazar-tray 0.1.0"
git push origin v0.1.0
```

The tag starts `.github/workflows/release.yml`: it re-runs the whole gate, builds
the installer on a GitHub runner, writes `SHA256SUMS`, attests the build
provenance, and creates a **draft** release with both files attached. It does not
publish.

```powershell
gh run watch
gh release view v0.1.0        # a draft, with two assets
```

Take the installer's hash from the release's own `SHA256SUMS` — the one built by
the runner, not the one on your laptop — and put it in the winget manifest:

```powershell
gh release download v0.1.0 --pattern SHA256SUMS --dir .
Get-Content SHA256SUMS
# Paste the hash into packaging/winget/xfurqan0.nazar-tray.installer.yaml
winget validate --manifest packaging\winget
git add packaging/winget && git commit -m "winget: hash for 0.1.0" && git push
```

---

## 7. Flip the repository to public

Do this deliberately, and only after step 5 came back clean: the history becomes
public with it, and a repository that has been public cannot be un-published from
anyone who cloned it.

```powershell
gh repo view xfurqan0/nazar-tray --json visibility
gh repo edit xfurqan0/nazar-tray --visibility public --accept-visibility-change-consequences
```

Then turn on the two things a public repository wants:

```powershell
gh api -X PATCH repos/xfurqan0/nazar-tray -f has_issues=true
gh api -X PUT repos/xfurqan0/nazar-tray/vulnerability-alerts
```

---

## 8. Publish the release

Read the draft on github.com first — the notes, the two assets, the file names.
Then:

```powershell
gh release edit v0.1.0 --draft=false
```

Check what a user gets:

```powershell
gh release view v0.1.0 --web
gh attestation verify (gh release download v0.1.0 --pattern "*-setup.exe" --dir . --clobber; ".\nazar-tray_0.1.0_x64-setup.exe") --repo xfurqan0/nazar-tray
```

---

## 9. Submit to winget

The package is `xfurqan0.nazar-tray`; the manifests are in `packaging/winget`,
already validated. Install from them one last time, which checks the hash against
the file the release actually serves:

```powershell
winget install --manifest packaging\winget
```

Then submit. **`wingetcreate` opens a pull request against
`microsoft/winget-pkgs`, which is a message that leaves this machine**: read the
text it produces before it goes, and let it carry no AI attribution — the same
rule as every commit here.

```powershell
winget install --id Microsoft.WingetCreate --exact
wingetcreate submit --token <GitHub PAT with public_repo> packaging\winget
```

Merging takes a day or two and is done by their automation plus a reviewer. When
it lands:

```powershell
winget show xfurqan0.nazar-tray
winget install xfurqan0.nazar-tray
```

---

## 10. Apply to SignPath Foundation

**After** the release is out, not before —
[docs/CODE_SIGNING.md](CODE_SIGNING.md) has the reasoning and the conditions.
The application asks for a released, actively maintained project; give the
repository a little history first, then apply at
[signpath.org/apply](https://signpath.org/apply) with:

- the repository URL, and `docs/CODE_SIGNING.md` as the published signing policy;
- `.github/workflows/release.yml` as the CI that builds from source;
- the release itself, with its attestation.

If it is approved: add the four repository secrets, delete `if: false` from the
`sign` job, and change the README's "unsigned" paragraph. Three lines, in one
reviewed commit.

---

## 11. After the release

- Leave `main` on the released version until the next change lands, then bump it
  with a `## [Unreleased]` section in `CHANGELOG.md`. A `main` that is already on
  the registry's version with no note is how a mystery starts.
- Watch the first issues. A tool that reads another program's internal files gets
  "it shows nothing" reports; `nazar-tray --print` and `nazar-statusline status`
  are the first two things to ask for.
- The updater plugin is compiled in but **no updater endpoint is configured and
  no minisign key exists** for 0.1.0. Before turning it on: generate the key with
  `cargo tauri signer generate`, keep the private key in a password manager and a
  second offline copy, and never put it in the repository. Losing it means never
  shipping an update to the installed base again; `.gitignore` already refuses
  `*.key`.

---

## Release notes template

`docs/release-notes-0.1.0.md`, which the workflow attaches to the draft:

```markdown
# nazar-tray 0.1.0

**Your Claude Code and Codex quota, in the system tray.** A bead fills up as you
burn through your 5-hour and weekly windows; click it for every window, its
percentage and when it resets.

    winget install xfurqan0.nazar-tray

Windows 10 1809 or newer. Installs per user, no administrator rights.

## What it does

- Both providers, both windows each, with the binding one marked.
- Amber at 60 %, red at 85 %, a notification once per window per reset.
- Reads local files only: no credentials and no network in the default build.
- Writes `~/.nazar/limits.json`, which the Nazar canvas reads.
- Six languages: English, Türkçe, 中文, 한국어, Русский, Español.

## Known limits

- Claude numbers move only while a session refreshes its status line.
- Model-scoped weekly windows need the opt-in detailed-windows mode.
- Codex's log format is not a documented contract; the reader is defensive.
- **Unsigned.** SmartScreen warns on a browser download; `winget install` does
  not go through it. See docs/CODE_SIGNING.md.
- Windows 11 keeps new tray icons in the overflow; the first run says so.

Full list in the [README](https://github.com/xfurqan0/nazar-tray#known-limits).

## Verify

    Get-FileHash .\nazar-tray_0.1.0_x64-setup.exe -Algorithm SHA256
    gh attestation verify .\nazar-tray_0.1.0_x64-setup.exe --repo xfurqan0/nazar-tray

MIT licensed.
```
