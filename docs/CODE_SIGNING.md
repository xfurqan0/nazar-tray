# Code signing

**nazar-tray 0.1.0 is not signed.** This page says what that means for you, what is being
done about it, and what the signing pipeline will look like when it exists — written now,
while the decisions are fresh, rather than after the certificate arrives.

## What unsigned means today

Windows shows the installer to Microsoft's reputation service, SmartScreen, and an executable
with no publisher and no download history is exactly what SmartScreen is built to interrupt.
Two paths, two experiences:

| How you install | What you see |
|---|---|
| `winget install xfurqan0.nazar-tray` | Nothing. winget does not go through the browser, so the download warning never appears. It checks the SHA-256 in the manifest against the file it fetched. |
| Downloading the `.exe` from GitHub Releases | "Windows protected your PC". **More info → Run anyway.** Some browsers also warn while downloading, because the file is uncommon rather than because anything is known about it. |

This is why the README points at winget first. It is not a workaround: a hash checked against
a manifest in a public repository is a real, if different, guarantee — it says the bytes are
the ones the package was reviewed with.

## What you can check yourself

Every release carries a `SHA256SUMS` file listing the installer's hash, and GitHub's
[build attestation](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations)
for the same artefact. The attestation is the stronger of the two: it is signed by GitHub and
says which workflow run, from which commit, in which repository, produced that exact file.

```powershell
# The hash, against the release's own list.
Get-FileHash .\nazar-tray_0.1.0_x64-setup.exe -Algorithm SHA256

# The provenance, against GitHub's record. Needs the GitHub CLI.
gh attestation verify .\nazar-tray_0.1.0_x64-setup.exe --repo xfurqan0/nazar-tray
```

## The plan: SignPath Foundation

[SignPath Foundation](https://signpath.org/) gives free code-signing certificates to open
source projects, with the signing itself performed on their infrastructure, driven by CI, and
released by a human approval per release. The publisher shown to Windows is "SignPath
Foundation" rather than the maintainer, which is the point: the identity being vouched for is
the project's, verified by them.

### Why the application comes after the first release, not before

SignPath Foundation asks that a project **already be released** and **actively maintained**.
A project applying with no release and no history is asking to be evaluated on a promise. So
the order is: release 0.1.0 unsigned → let the repository accumulate a little history → apply
→ if approved, sign from 0.2.0 onwards. The first release does not wait for a certificate,
and this page is what ships in its place.

*(Recorded as decision K24 in the unified plan, 2026-09-07. The alternative — hold the
release until the certificate arrives — was rejected because the queue is other people's and
the deadline would be theirs.)*

### The conditions, and where this project stands

| Condition | Status |
|---|---|
| OSI-approved licence | MIT, `LICENSE`. |
| No proprietary components | Every dependency is permissive; `scripts/check-licenses.mjs` fails the build otherwise, `THIRD-PARTY-NOTICES.md` is the list. |
| Built from source in CI | `.github/workflows/release.yml` builds the installer on a GitHub-hosted runner from a tagged commit. Nothing is built on a laptop and uploaded. |
| Published signing policy | This page. |
| Two-factor authentication on the source repository account | On for `xfurqan0`. |
| Named author / reviewer / approver roles | One person holds all three today, which SignPath permits for a single-maintainer project and which this page states rather than hides. If the project gains a second regular contributor, author and approver are split — the approver is the one who must not be the one who pushed the tag. |
| Manual approval per release | The signing job is `workflow_dispatch` with an explicit input, never automatic on tag push. A release that nobody approved is a release that does not get signed. |
| "SignPath Foundation" visible as publisher | Accepted. The README will say who signs and why the name is not the maintainer's. |
| Already released, actively maintained | The reason this is step two. |

### The CI job, written and switched off

`.github/workflows/release.yml` carries the signing job with `if: false` on it. It is there so
that the shape of the pipeline is reviewable now — the artefact it would sign, where the
signature would go, which secrets it would need — and so that turning it on is a one-line
change reviewed on its own rather than a new file written under time pressure.

What it will need when it is turned on:

| Secret | What it is |
|---|---|
| `SIGNPATH_API_TOKEN` | The CI user's token. Repository secret, never in the workflow file. |
| `SIGNPATH_ORGANIZATION_ID` | Given at approval. |
| `SIGNPATH_PROJECT_SLUG` | `nazar-tray`. |
| `SIGNPATH_SIGNING_POLICY_SLUG` | `release-signing`, once the policy exists. |

The job uploads the unsigned installer as a workflow artefact, calls SignPath's action to
submit it, waits for the human approval, and downloads the signed file back. **Signing
happens after the tests and after the bundle, and before anything is attached to a release**
— so a release either has the signed installer or has no installer.

### What is signed

The installer, and both binaries inside it: `nazar-tray.exe` and `nazar-statusline.exe`. The
wrapper is the one that Claude Code executes on every status-line refresh, so it is the one
an endpoint-protection product is most likely to notice; leaving it unsigned inside a signed
installer would be the worst of both.

## The alternatives, and why not

| Option | Why not |
|---|---|
| **Certum Open Source** (~€25–69/yr) | The certificate lives on a hardware token, so signing cannot happen in CI — every release would be signed on a laptop, which is the thing SignPath's model exists to avoid. Their terms also allow revocation if the software is distributed commercially, which is a condition this project would rather not carry. |
| **Azure Trusted Signing** | Individual developers are limited to the United States and Canada; the organisation route requires a verifiable legal entity in a list that does not include Türkiye. Checked twice, in two independent reviews. This is the one hard blocker. |
| **A commercial OV/EV certificate** (~$200–600/yr) | Above this project's whole budget ceiling, and since 2024 an EV certificate no longer buys an automatic SmartScreen pass anyway — reputation is built by downloads either way. |
| **Ship unsigned for ever** | What 0.1.0 does, deliberately, with the warning documented rather than hidden. It is a starting point, not an answer. |

## macOS and Linux

Not applicable yet: v1 is Windows only. A macOS build needs an Apple Developer account
($99/yr) for notarisation, which is a separate decision with its own budget line, and it is
not taken. Linux ships no installer — the story there is `nazar-tray --print` and the Nazar
canvas.
