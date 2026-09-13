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

Six files carry it — the three manifests and the three winget files — and
`ui/test/version.test.mjs` asserts all six agree, so a half-done bump fails the
gate rather than the release.

```powershell
# Edit by hand, all in the same commit:
#   Cargo.toml                        -> [workspace.package] version = "0.2.0"
#   crates/nazar-tray/tauri.conf.json -> "version": "0.2.0"
#   ui/package.json                   -> "version": "0.2.0"
#   packaging/winget/*.yaml           -> PackageVersion: 0.2.0  (three files)

Select-String -Path Cargo.toml, crates\nazar-tray\tauri.conf.json, ui\package.json -Pattern 'version'
Select-String -Path packaging\winget\*.yaml -Pattern 'PackageVersion'
```

Move the `## [Unreleased]` heading in `CHANGELOG.md` to `## [X.Y.Z] — YYYY-MM-DD`
with today's date, and read the README's status line and the CHANGELOG's opening
paragraph back: both have to describe a release that exists rather than one that
is coming.

The `Cargo.lock` moves with the version. Let cargo do it rather than editing it:

```powershell
cargo check --workspace --locked   # fails if the lock file is behind
git add -A
git commit -m "Release 0.2.0"
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

Before it prints any of that it reads the two binaries back
(`scripts/check-binary-paths.mjs`) and, if either one carries the path of the
machine it was built on, **deletes the bundle** rather than leave an installer on
disk looking finished. So an installer that exists at this point is one that
passed. Step 5 runs the same check by hand.

Expected, on `x86_64-pc-windows-msvc`:

| Artefact | 0.1.0 | 0.2.0 |
|---|---|---|
| `nazar-tray.exe` | 4.74 MB | 5.03 MB |
| `nazar-statusline.exe` | 340 KB | 340 KB |
| `nazar-tray_<version>_x64-setup.exe` | 1.95 MB | 2.07 MB |

The usage history cost 290 KB of tray binary and 120 KB of installer: a scanner, a
deduplicator, a store and a view, all of it compiled in rather than fetched. The
wrapper did not move, because none of it is in the wrapper.

Measured 2026-09-09 and 2026-09-13 with the release profile in `Cargo.toml` (`opt-level = "s"`,
`lto`, one codegen unit, stripped, `panic = "abort"`) and the path remapping this
script passes. Cargo's stock release settings gave 11.95 MB, 497 KB and 3.06 MB
for the same source on 2026-09-07, which is what those four lines are worth.

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
& "C:\Program Files\7-Zip\7z.exe" l target\release\bundle\nsis\nazar-tray_0.2.0_x64-setup.exe
```

Then install it and look at what landed:

```powershell
Get-ChildItem -Recurse "$env:LOCALAPPDATA\nazar-tray" | Select-Object Name, Length
```

---

## 4. Smoke the installer as a user would

```powershell
$setup = "target\release\bundle\nsis\nazar-tray_0.2.0_x64-setup.exe"

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
the forbidden patterns written into the gate tests themselves, and — since
T-WP11 — the placeholder paths in `scripts/check-binary-paths.mjs`,
`scripts/build-installer.mjs`, the two workflows and the prose describing them.
Read those: `C:\Users\<account>` and `C:\Users\runneradmin` are the shapes being
looked for, and neither names anybody.

**And inside the binaries, which none of the greps above can see.** Panic
locations are compiled in as string literals, so `strip = true` does not remove
them and a release build can carry the path every crate was compiled from — the
account name of whoever built it, shipped to everyone who downloads the
installer. `scripts/build-installer.mjs` passes `--remap-path-prefix` for exactly
this reason; here is the check that it worked:

```powershell
node scripts/check-binary-paths.mjs        # or: cd ui; npm run check-binaries
```

Expected, and the only acceptable answer:

```
target\release\nazar-tray.exe                       4.74 MB  0 match(es)
target\release\nazar-statusline.exe                339.5 KB  0 match(es)
no machine-specific paths in any of them
```

Any non-zero count is a release stopper, and the script prints the first five
hits in context so the escaped prefix names itself.

Two things to know about what it is reading. It reads the binaries **in
`target/`**, not the copy already installed under `%LOCALAPPDATA%\nazar-tray` —
that one came from whichever build put it there. And only
`scripts/build-installer.mjs` passes the remap, so **any release build made
another way replaces these files with unremapped ones** — `cargo test --release`
rebuilds `nazar-statusline.exe` and does exactly that. If anything has touched
`target/release` since step 2, rerun the build before the check means anything.

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
# docs/release-notes-0.2.0.md — the CHANGELOG entry, shortened, with the known
# limits kept rather than buried. Template at the bottom of this page.
git add -A
git commit -m "Release notes for 0.2.0"
git push origin main
```

---

## 6. Tag

**From here on, nothing is reversible.**

```powershell
git tag -a v0.2.0 -m "nazar-tray 0.2.0"
git push origin v0.2.0
```

The tag starts `.github/workflows/release.yml`: it re-runs the whole gate, builds
the installer on a GitHub runner, checks the binaries for build-machine paths
before anything is hashed or attached, writes `SHA256SUMS`, attests the build
provenance, and creates a **draft** release with both files attached. It does not
publish.

```powershell
gh run watch
gh release view v0.2.0        # a draft, with two assets
```

Take the installer's hash from the release's own `SHA256SUMS` — the one built by
the runner, not the one on your laptop — and put it in the winget manifest:

```powershell
gh release download v0.2.0 --pattern SHA256SUMS --dir .
Get-Content SHA256SUMS
# Paste the hash into packaging/winget/xfurqan0.nazar-tray.installer.yaml
winget validate --manifest packaging\winget
git add packaging/winget && git commit -m "winget: hash for 0.2.0" && git push
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
gh release edit v0.2.0 --draft=false
```

Check what a user gets:

```powershell
gh release view v0.2.0 --web
gh attestation verify (gh release download v0.2.0 --pattern "*-setup.exe" --dir . --clobber; ".\nazar-tray_0.2.0_x64-setup.exe") --repo xfurqan0/nazar-tray
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

If it is approved: add the four repository secrets, replace the `sign` job's
`if: false` with `github.event_name == 'workflow_dispatch' && inputs.sign` — the
condition is written out in a comment directly above it, and the `sign` input it
reads is already declared — and change the README's "unsigned" paragraph. Three
lines, in one reviewed commit. Replace rather than delete: a `sign` job with no
condition at all would sign on every tag push, which is the opposite of the
manual approval `docs/CODE_SIGNING.md` promises.

---

## 11. After the release

- Leave `main` on the released version until the next change lands, then bump it
  with a `## [Unreleased]` section in `CHANGELOG.md`. A `main` that is already on
  the registry's version with no note is how a mystery starts. **The section goes
  in as soon as something lands and the six version numbers do not move with it:
  step 1 sets them in the release commit, in the same change that renames the
  heading to `## [X.Y.Z] — YYYY-MM-DD`, so a tree is never carrying a version no
  build was ever made from.**
- Watch the first issues. A tool that reads another program's internal files gets
  "it shows nothing" reports; `nazar-tray --print` and `nazar-statusline status`
  are the first two things to ask for.
- The updater plugin is compiled in but **no updater endpoint is configured and
  no minisign key exists** for any release so far. Before turning it on: generate the key with
  `cargo tauri signer generate`, keep the private key in a password manager and a
  second offline copy, and never put it in the repository. Losing it means never
  shipping an update to the installed base again; `.gitignore` already refuses
  `*.key`.

---

## Release notes template

`docs/release-notes-<version>.md`, which the workflow attaches to the draft. Both releases so
far are in the tree; 0.1.0 is reproduced here because it is the shape rather than the words:

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
