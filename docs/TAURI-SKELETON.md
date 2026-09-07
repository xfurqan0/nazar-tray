# Tauri v2 skeleton — checklist

What WP0 set up here, as a list to **apply** rather than copy. Dile is the second
consumer and Nazar v2 the third; a shared crate gets extracted when the third one proves
the repetition is real, not before (unified plan, section 1.5). Early abstraction is
debt, not quality.

Each line says what to do and why, so a repository with different needs can disagree on
purpose instead of by accident.

## Layout

- [ ] Cargo workspace at the repository root; `[workspace.package]` carries `version`,
      `edition`, `rust-version`, `license`, `repository`. Members inherit with
      `version.workspace = true`.
- [ ] Split the pure logic into its own crate with **no Tauri dependency**. It is what
      builds on Linux and macOS from day one, and it is what makes a later port a step
      rather than a rewrite.
- [ ] The Tauri crate plays the role of `src-tauri`: `tauri.conf.json`, `build.rs`,
      `capabilities/`, `icons/` live inside it. `frontendDist` is a relative path back to
      the frontend directory.
- [ ] Frontend at the repository root, not nested in the Tauri crate. It is a peer of the
      Rust code, not an asset of it.

## Configuration

- [ ] `identifier` on a domain the maintainer actually owns. It ends up in the registry,
      the installer and the update feed, and changing it later orphans installed copies.
- [ ] `webviewInstallMode: downloadBootstrapper`. Adds 0 MB to the installer;
      `offlineInstaller` adds 127 MB and `fixedVersion` 180 MB, for a component that is
      already part of Windows 10 1803+ and Windows 11.
- [ ] Bundle target `nsis`. It is 86 % of what Tauri projects ship, and MSI cannot do
      per-user installs without an administrator.
- [ ] An explicit `security.csp`. The default is permissive; a panel that renders local
      state needs `default-src 'self'` and little else.
- [ ] Windows declared in config, created hidden, shown from Rust. A window that flashes
      on start-up is the first thing users complain about in a tray app.
- [ ] `beforeBuildCommand` empty, and the frontend built as its own step. The Tauri CLI
      infers the "app directory" from the layout, and an unconventional layout makes that
      inference fragile; an explicit build step never guesses wrong.

## Capabilities

- [ ] One capability file, scoped to the windows that need it, with a `description` that
      says what the window is allowed to do and why.
- [ ] Start from `core:default` and add permissions one at a time, each with a reason.
      Anything granted here is reachable from the webview.
- [ ] A plugin that is declared but not initialised needs **no** capability entry. Adding
      one grants reach that nothing uses.

## Frontend

- [ ] No framework unless the interface actually needs one. Resource use is part of what
      these products claim.
- [ ] esbuild, invoked from a small `build.mjs`. It emits the browser bundle and a plain
      ESM copy of the modules, so tests exercise the shipped code rather than a
      re-implementation of it.
- [ ] `@tauri-apps/api` as a dependency and `withGlobalTauri: false`. The global injects
      the whole API surface into the page for the sake of skipping one import.
- [ ] Type-check with `tsc --noEmit` in CI. esbuild strips types; it does not check them.

## i18n

- [ ] `locales/<lang>.json`, flat keys, `{placeholder}` interpolation, no ICU.
- [ ] **No string typed into code or markup.** A hard-coded word is a string no
      translation can reach.
- [ ] Parity between the hand-written languages is a failing test. Machine-translated
      languages are reported as warnings until they land, so the suite stays green and
      therefore stays read.

## Icons

- [ ] One SVG as the source, rasterised by a script, then `tauri icon`. Both the artwork
      in the interface and the application icon come from the same file, so they cannot
      drift.
- [ ] Delete the iOS and Android output unless mobile is on the roadmap. It is a quarter
      of a megabyte of files nothing builds.

## CI

- [ ] Test job and bundle job **separate**. The bundler is the fragile part of a Windows
      pipeline (WebView2, NSIS, signing); a bundler failure must never make a correct
      change look broken.
- [ ] The bundle job is `continue-on-error` until the installer is a deliverable rather
      than a by-product. It uploads an artifact; it does not gate.
- [ ] The pure crate is tested on Ubuntu and macOS from the first commit, with a comment
      in the workflow saying why the app itself is not.
- [ ] `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`,
      `npm ci && npm test`, and a licence gate. Pin `runs-on` to a named runner image.
- [ ] Cache the Cargo build. A Tauri dependency tree is roughly 550 crates.

## Hygiene

- [ ] `.gitattributes` with `* text=auto eol=lf` **before** the first contribution. Mixed
      line endings put every file in the first pull request's diff.
- [ ] `.gitignore` covering `target/`, `node_modules/`, `dist/`, the Tauri `gen/`
      directory, and any private key. Losing an updater key means never shipping an
      update to the installed base again.
- [ ] `rust-toolchain.toml` pinning the channel and pulling `clippy` and `rustfmt`, so a
      contributor's toolchain matches CI's without being told.
- [ ] A licence gate that runs with nothing installed beyond the toolchain the repository
      already needs, plus a `deny.toml` for a maintainer who has `cargo-deny`. Copyleft in
      an MIT product is a rewrite, not paperwork.
- [ ] One version number, checked by a test. It appears in `Cargo.toml`, the frontend
      `package.json` and `tauri.conf.json`, and drift only shows up in a release.
