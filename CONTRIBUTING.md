# Contributing

Thanks for looking. nazar-tray is small on purpose, and these are the few rules that keep it
that way.

## Getting set up

Windows, a Rust toolchain, and Node 22 or newer.

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc
cargo install tauri-cli --locked

cd ui
npm ci
npm run build        # the panel; the Rust build embeds ui/dist at compile time
cd ..

node scripts/sidecar.mjs --debug   # the second binary, where the bundler expects it
cargo test --workspace
cd ui && npm test
```

Two of those steps are easy to skip and both fail confusingly:

- **Build the panel first.** `tauri::generate_context!` reads `ui/dist` while the tray crate
  compiles. Without it the macro cannot expand; with a stale one you get a stale panel and no
  warning at all.
- **Prepare the sidecar first.** `bundle.externalBin` names `nazar-statusline`, and
  `tauri-build` checks the file exists — so a missing sidecar fails `cargo clippy` and
  `cargo test`, not only the bundler.

The whole installer, in one command:

```powershell
node scripts/build-installer.mjs
```

## The rules

- **English everywhere.** Code, comments, commit messages, issues, pull requests,
  documentation. The interface is translated; the repository is not.
- **No hard-coded UI text.** Every word a user can see comes from `ui/locales/<lang>.json`,
  in the panel and in the Rust side alike. Two tests grep the sources and fail on a literal
  that reads like a sentence. Command-line output stays English on purpose — `--print` emits
  JSON a script parses.
- **All six locales, or none.** A new message key goes into `en.json` and the other five in
  the same commit; a test asserts exact parity, including placeholders. Machine translation
  is acceptable for the four the maintainer does not speak — corrections by pull request are
  the point of `ui/locales/README.md`.
- **Tests are required.** A change without a test that fails before it and passes after it
  does not go in. Bug fixes need the regression test that pins the bug; new behaviour needs a
  test of the behaviour, not of the implementation.
- **Unknown is a state, never a zero.** If a reader could not get a number, the icon is grey
  with a question mark and the panel says so. A reassuring `0 %` on a window nobody could read
  is the single worst thing this product could do, and it is what the prototype did.
- **No invented numbers.** The binding window is the highest percentage a provider reports.
  Nothing is estimated, extrapolated, or derived from a flag the source does not send.
- **Permissive dependencies only.** `scripts/check-licenses.mjs` fails the build on anything
  that is not, and `deny.toml` holds the same policy for `cargo deny`, down to the one
  `WITH` clause either of them accepts. A copyleft dependency in an MIT product is a rewrite,
  not a paperwork problem. Adding a dependency also means running
  `node scripts/third-party-notices.mjs`; CI checks the result is current.
- **`cargo deny check` is green, and is the gate CI does not run.** It covers what the script
  cannot — advisories, yanked crates, duplicate versions, unexpected registries — and needs
  `cargo install cargo-deny --locked`. Run it when you add or update a dependency. Every
  entry in its `ignore` list carries a RUSTSEC id, the reason, and a review date; adding one
  without all three is how a red gate becomes a green lie.
- **The default build makes no network call.** The one HTTP client is behind the
  `detailed-windows` feature and a run-time switch, and CI builds without the feature to prove
  the switch is real. A dependency that opens a socket outside that feature does not go in.
- **Never write outside our own files.** `~/.nazar` and `%APPDATA%\nazar` are ours.
  `~/.claude/settings.json` is edited by exactly one program, `nazar-statusline`, only when a
  user presses a button, with a backup and a diff shown first. Nothing else in this repository
  may write there.
- **No absolute paths.** Every path is derived at run time. `crates/nazar-core/tests/hygiene.rs`
  fails the build on a literal one, and on a fixture that carries real user data — an account
  identifier, an e-mail, a prompt.
- **No AI attribution in commits or pull requests.** No `Generated with …` trailer, no
  `Co-Authored-By:` for a model or an assistant, no session links. Write the message in your
  own voice; the history is a record of decisions, not of tooling.

## Commits and pull requests

One change per pull request, with the reasoning in the message rather than in a comment
thread. `docs/PROJECT.md` §9 is the project log; a change that closes or reopens a decision
belongs there too.

Run before pushing:

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd ui; npm run typecheck; npm test
```

## Translations

The most useful small contribution. `ui/locales/README.md` has the review table and the rules
a locale file must keep. Four of the six were machine-translated first and say so; a native
speaker fixing a sentence is a welcome pull request and needs no issue first.

## What is out of scope

Cost analytics, session management, a launcher, a chart of anything. This is a tray icon that
tells you how much quota is left, and the shortest way to make it worse is to make it a
dashboard. `docs/PROJECT.md` §2 is the full list of what it is not.
