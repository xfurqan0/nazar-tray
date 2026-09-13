# Changelog

Notable changes to nazar-tray. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

`limits.json` has its own compatibility promise, separate from the app version: see
[docs/limits-contract.md](docs/limits-contract.md).

## [Unreleased]

**Usage history.** 0.1.0 answered *how much of my window is gone?* The tray now also answers
*how many tokens did I actually spend, and on which model?* — from numbers that were already
on the disk, for both providers, without a network call and without an estimate anywhere in
it. Nothing about the quota path moved: `limits.json` is unchanged and stays at
`schemaVersion: 1`, and Nazar reads nothing new.

### Added

- **A Usage view, on a button in the panel footer.** Four tabs — **Week**, **Weeks**, **All**,
  **Models** — one row per model, both providers in the same list, and a `Since 27 Aug ·
  scanned 2 m ago` line saying how far back the history goes and how old it is. The headline is
  **`input + output + cache_read + cache_create`**, which is the same definition Claude Code's
  own `/usage` prints as *total tokens*, so the two can be read side by side. Underneath it,
  and underneath every model row, the **same total taken apart in four** — `In 802K · Out
  354K · Cache read 1B · Cache write 253K` — because cache reads were 98.5 % of the raw total
  over six days of real work and that is a fact the reader should be able to see rather than
  one the headline quietly decides for them. An absent counter prints an em dash and a
  reported zero prints `0`, in the breakdown as well as in the total — the same distinction
  the quota view draws between *nobody read this* and *you have used none of it*. Model ids
  are printed exactly as the provider spelled them, never merged and never translated.
  Thirty-one new keys in all six languages, and **no magnitude mark among them** — `1B` in
  English, `1 млрд` in Russian and `10억` in Korean are `Intl.NumberFormat`'s, floored to the
  precision they are shown at, because 22.39 M is not 22.4 M.
- **All is a calendar heat-map.** Weeks as columns, Monday to Sunday down each one, one cell
  per local day, shaded in five steps from the theme's own accent, up to twelve months with the
  month names above them, scrolling sideways when it is wider than the panel. Hovering or
  focusing a day says its date, its tokens and the model that spent most of them, on a line
  inside the panel rather than in an operating-system tooltip — and the grid is **one tab
  stop**, walked with the arrow keys, a day up and down and a week across. The shading is
  quarters of the busiest day in the grid, so the busiest day is always the darkest and a
  legend marks which end is which. Days the range does not contain — after today, before the
  store began — are drawn as nothing at all, because an empty track would say a day nobody
  could have worked on was a day nobody worked on. Week keeps its strip of seven columns.
  **Still no charting library** (decision K2): seven CSS grid rows and one custom property, in
  the same tokens as the rest of the panel, so the view themes itself and weighs nothing.
- **Every day and every week opens.** Click a day — in the Week strip or anywhere in the
  heat-map — or a row of **Weeks**, and the view shows what that day or that week went on: the
  four counters over it, then one row per model with its own four, under a provider heading
  when both providers worked in it. *Back* returns to the list it was opened from, and so does
  Esc. Nothing is asked of the store to do it: a detail is the answer already on screen cut by
  the local day, or the local Monday, each hour **starts** in — which is also why a week's
  detail adds up to exactly the row that opened it.
- **Weeks is one row per calendar week, newest first**, Monday to Sunday where you are: the
  week's total, its four counters and a bar against the busiest week in the list. The list is
  dense, so a week nobody worked is a row saying so rather than a gap between two busy ones,
  and a week with no counters at all prints an em dash rather than a zero. The week being lived
  in is marked as the part week it is.
- **Models is a chart of tokens per day, one line per model**, over **All**, **Last 7 days** or
  **Last 30 days** — with a legend that names every line, and under it the same model rows with
  the share of the span each took. A day a model spent nothing on is drawn as zero rather than
  skipped, because a line that jumped the gap would draw work that did not happen. **Still no
  charting library**: it is hand-written SVG, six polylines and nine bits of text, in six
  colours **derived from the theme's own accent** — its hue rotated six ways and its lightness
  moved only as far as 3:1 against the panel needs, so there is no second palette in either
  theme file and both themes and both modes are checked by the same contrast test the meter
  bars are. The six steps are uneven on purpose: evenly spaced hues put two lines in the green
  band, and one green, one amber, one red, one magenta, one violet and one blue are six colours
  a legend swatch can actually tell apart. Six lines is what a 360 px legend can name; a seventh model has no line and
  is still in the list under the chart.
- **The numbers come off files that were already there.** Claude Code's transcripts —
  `~/.claude/projects/**/*.jsonl`, **sub-agent transcripts included**, which are 78 % of the
  bytes — and the `token_count` events in the Codex session logs the quota reader already
  opens. Nine fields are read from a record and a **new record is built out of them**, so
  everything else is gone with the parse. Nothing is derived, inferred or estimated: these are
  the counters the provider itself reported, added up.
- **Duplicates are dropped exactly, because they cannot be divided out afterwards.** Claude
  Code writes one line per content block and every copy carries the whole `usage` object —
  **44.4 % of lines on this machine are repeats, inflating the totals 1.70×**, and the factor
  is not a constant that could be corrected for later (2.49×, 2.38× and 1.04× over three
  slices of the same machine's logs). A Claude record is keyed `(message.id, requestId)` and
  the **largest** reading of each key wins; Codex writes no event id, so the same guarantee is
  built out of file identity, the byte offset and a fingerprint of every event already
  credited. A transcript that is truncated, rotated or pruned is re-read from the top and adds
  **nothing**.
- **Hourly UTC totals, one file per month, in `%APPDATA%\nazar\usage\`.** Written whole
  through the same temp-file-and-rename that writes `limits.json`, by the one process holding
  the writer's lock, and kept there so the history survives the transcripts being pruned — a
  merge takes the larger of two values per counter, so a vanished transcript cannot erase what
  it once contributed. The file is specified in [docs/usage-contract.md](docs/usage-contract.md)
  and every field either reader may touch, including everything deliberately not read, is in
  [docs/pinned-internal-formats.md](docs/pinned-internal-formats.md).
- **A second line on the tray tooltip**: `This week 1.5B · claude-sonnet-5` under the quota
  line — the week's tokens and the model that spent most of them. The number is the four-way
  sum the panel draws, so the tooltip and the view behind it cannot disagree; on six days of
  the maintainer's real work that is 1 514 068 891, which is why the example here is a
  billions-class one rather than the `22.3M` it said while the two were being counted
  differently. Windows shows 127 characters
  of a tooltip and silently drops the rest, so the tooltip is built in three steps that give
  up the least valuable thing left, and the week line is given up before any part of the quota
  line is. **The icon still says nothing about usage**: it is the mark at every reading, and
  grey only for unknown.
- **The scan is never on the quota path.** Quota is why this application exists, it reads two
  small files in milliseconds, and it must not queue behind a walk of hundreds of megabytes —
  so nothing scans on the refresh loop. It runs when the Usage view is opened, **at most once
  every five minutes**, and *Refresh* is the only thing that overrides that. Measured on the
  maintainer's machine: **126 transcripts, 259 MB, 234 ms** for a first full pass and **15 ms**
  for the next one; 23 Codex logs, 52.5 MB, **88 ms** and then 3.7 ms.

### Changed

- **The theme toggle left the panel footer**, and the settings page's picker is the only way
  to change a theme now. The footer button's visible label was the name of the theme it was
  *already* painting while its `aria-label` said *Switch theme* — a statement to the eye and
  an action to a screen reader — and the settings page has named both themes in a dropdown
  since 0.1.0. It is a **deletion rather than a move**: nothing was added anywhere, and the
  slot it frees is where the Usage button went. `panel.action.theme` is gone from all six
  locale files, the first key this product has ever removed.
- **Every button in the panel looks like a button under the pointer now, and under the
  keyboard.** Hover used to change a shade of text and put a hairline in the raw accent, which
  is 2.25:1 against a white panel — the *Usage* button was reported as dead on hover, and it
  was, along with every other one in the product. Hover and `:focus-visible` share one rule
  now and fill the control with the same ground a settings field sits on, a pair
  `ui/test/contrast.test.mjs` already measures in both themes and both modes.
- **The README's privacy statement says which files are read and which fields are taken.**
  *"Never reads your tokens and never talks to the network"* is still true and stays — "token"
  there is a sign-in token — but it is no longer the whole sentence, because reading
  transcripts at all reverses a line this repository had written down. So the reversal is
  written down too: what is read, and what is **never** read — not message content, not
  replies, not reasoning, not tool input or output, not the paths, project names, branch names
  or session ids sitting beside them in the same file. **A test is what makes that a fact
  rather than a promise**: a transcript whose every text field carries a sentinel goes through
  the whole scan, and the build fails if that string appears in the result, in the totals, or
  in the file they are written to. A promise that has to be read carefully to stay true has
  already broken.

### Notes

- **The counters are the provider's own reported numbers, and there is no cost anywhere.** No
  currency, no price table, no guess at what a model charges — a number this product cannot
  source is a number it does not print.
- **A week starts on Monday where you are.** Nothing on the Rust side ever asks the machine
  which zone that is; the panel does the local arithmetic and sends instants, so the store
  holds hourly UTC buckets and every local view is cut out of them. A bucket lands in the
  local day its hour **starts** in, which is what keeps an offset like `+05:30` from splitting
  an hour by a ratio it would have to invent.
- **A Codex event before its session's first turn context is counted under the model id
  `unknown`**, rather than dropped or attributed to whichever model was named next. Those
  tokens were spent; which model spent them is a thing nobody knows.
- **The usage store lives with your settings, not in `~/.nazar`.** `limits.json` is safe to
  paste into a bug report — two percentages and two reset times say nothing about what anybody
  was doing — and a month of hourly token counts is a usage profile. It breaks no rule about
  credentials and it is still not a thing to hand over by reflex, so it sits with the user's
  own files rather than in the directory this project tells other programs to read. No other
  program reads it, this one included, until that is its own work package on both sides.

## [0.1.0] — 2026-09-13

The first release, and everything in it: the WP0 skeleton, the WP1 Codex reader, the WP2
Claude reader with its status-line wrapper, the WP2b detailed-windows mode, the WP3 state
model, refresh loop and writer, the WP4 icon and panel, the WP5 notifications, autostart and
settings, the WP6 translations, and now the WP7 installer. It builds, it tests, **it is
something you can leave running**, and it is now something you can hand to somebody else: a
per-user NSIS package with no administrator prompt, winget manifests, a release workflow that
drafts rather than publishes, and an uninstaller that takes the residue with it.

**Published unsigned, which is a decision rather than an omission.** SignPath Foundation asks
that a project already be released and actively maintained before it applies, so 0.1.0 ships
with a `SHA256SUMS` file and a GitHub build attestation where a signature would go; the whole
reasoning, and what you can check instead, is in [docs/CODE_SIGNING.md](docs/CODE_SIGNING.md).
The winget package is a pull request against `microsoft/winget-pkgs` and is reviewed a day or
two after the release, so `winget install xfurqan0.nazar-tray` starts answering then rather
than on the day the tag is pushed — until it does, the installer is on the
[Releases](https://github.com/xfurqan0/nazar-tray/releases) page. Every step of a release, in
the order it happens and with the one-way doors marked, is in
[docs/RELEASE.md](docs/RELEASE.md).

### Added

- **The installer** (`node scripts/build-installer.mjs`). One NSIS package, **2.0 MB**, that
  installs **per user** into `%LOCALAPPDATA%\nazar-tray` and asks for **no elevation** — a
  tray application that watches one account's files has no business writing to Program Files
  or to HKLM, and a package that needs a UAC prompt is a package many people never install.
  It carries exactly four files: `nazar-tray.exe`, `nazar-statusline.exe`, `LICENSE.txt` and
  `THIRD-PARTY-NOTICES.md`. No fixtures, no screenshots, no documentation, no debug symbols.
  WebView2 is **not** bundled: `downloadBootstrapper` adds 0 MB and fetches Microsoft's own
  installer only on a machine that does not already have the runtime, which is every Windows
  10 1803 or newer.
- **No MSI, and it was tried rather than assumed.** `cargo tauri build --bundles msi` works —
  the bundler downloads WiX itself and produces a 2.70 MB package with no complaint. It is
  left out because Tauri's WiX target has no `installMode`, so an MSI installs per machine
  and needs administrator rights, and because `nsis.installerHooks` has no WiX equivalent:
  the MSI could not restore the status line, remove the `StartupApproved` value, or take the
  manufacturer key with it. A second installer that uninstalls worse than the first is not a
  choice worth offering.
- **The release profile is worth its build time.** `opt-level = "s"`, `lto = true`,
  `codegen-units = 1`, `strip = true`, `panic = "abort"`, measured against Cargo's stock
  release settings on the same source: the tray binary **11.95 MB → 4.79 MB**, the wrapper
  **497 KB → 335 KB**, the installer **3.06 MB → 1.99 MB**. A third off the download for four
  minutes of link time.
- **The build is one script, because the order of four things matters** and three of them are
  easy to forget: the panel (`generate_context!` reads `ui/dist` at compile time),
  `THIRD-PARTY-NOTICES.md` (regenerated from the lock file, so the attribution cannot describe
  a different dependency set from the one built), the sidecar, then the bundler. It prints the
  artefacts with their sizes and SHA-256 at the end, because those are the numbers the release
  checklist asks for and computing them by hand is how they end up wrong.
- **The status-line wrapper ships beside the tray and installs nothing.**
  `bundle.externalBin` puts `nazar-statusline.exe` next to `nazar-tray.exe`, and that is where
  the installer stops. Claude Code's `settings.json` belongs to Claude Code and to its user;
  an installer that edited it would be editing a file the user never mentioned, on a machine
  where a broken `statusLine` is a broken prompt. Instead the settings page grew a section
  that **asks twice**: the first button runs `install --dry-run` and prints the diff the
  wrapper would make, and only the button that appears underneath it writes — on top of the
  whole-file backup the installer takes anyway. Three commands (`statusline_status`,
  `statusline_preview`, `statusline_apply`) which do nothing but run the binary beside them,
  because there is one implementation of that edit, it is the one the command line runs, and
  it is the one the tests cover.
- **The uninstaller takes the residue with it.** Three things Tauri's own template cannot know
  about, in `crates/nazar-tray/nsis/hooks.nsh`:
  - it asks the wrapper to undo its own installation **before** deleting it, so a status line
    pointing at a file that is about to vanish is restored rather than left dangling;
  - it removes the `StartupApproved\Run` value Windows keeps beside the `Run` one — the
    residue WP5 found and could only write down. It is inert, but it is a row carrying this
    application's name in a list a person reads, left behind by an uninstall that claims to
    leave nothing;
  - it removes `%APPDATA%\nazar` when *delete application data* is ticked, which the stock
    template could not do: it only knows the bundle identifier, and this application's
    settings are not under it.

  `~/.nazar` is never touched by any path through that file. Nazar reads `limits.json`, a user
  may be running Nazar without ever having had this tray, and `chain.json` is the only
  machine-readable record of the status line the wrapper replaced.
- **`THIRD-PARTY-NOTICES.md`, generated** (`scripts/third-party-notices.mjs`) and shipped
  inside the installer. 310 packages: the **normal** dependencies of the two binaries that
  ship, resolved for the build target — build and dev dependencies are excluded because none
  of their code is distributed. Where a crate offers a choice it names the one taken, and it
  carries each licence's text once plus every crate's own copyright line, which is the part a
  permissive licence actually asks to be kept and the part a generated notice most often
  loses. `--check` fails CI when a dependency was added and the notices were not regenerated.
  Written rather than installing `cargo about`, for the same reason
  `scripts/check-licenses.mjs` exists: the file has to be regeneratable by anyone who can
  build the product, with nothing beyond the toolchain the repository already needs.
- **winget manifests** (`packaging/winget/`), which `winget validate` accepts:
  `xfurqan0.nazar-tray`, installer type `nullsoft`, `Scope: user`, silent switches `/S` and
  `/P`, `MinimumOSVersion` 10.0.17763.0, MIT, twelve tags and a release-notes URL.
  `InstallerSha256` is sixty-four zeros until the release exists — it is the hash of a file
  that gets uploaded in the step before, and `docs/RELEASE.md` step 6 is where the real one
  goes in. winget is the install path this project points people at while it is unsigned: it
  does not go through the browser, so it does not raise the warning a browser download does.
- **`.github/workflows/release.yml`**, on tag `v*`. It re-runs the whole gate — a tag gets no
  shortcut — builds the installer on a GitHub runner, checks the tag against the version in
  the tree, writes `SHA256SUMS`, **attests the build provenance** with GitHub's own signature,
  and creates a **draft** release. It does not publish and it does not tag: both are one-way
  doors and both belong to a person. `workflow_dispatch` runs everything except the draft, for
  rehearsing the pipeline before a tag exists.
- **`docs/CODE_SIGNING.md`**, and the signing job that is written and switched off. SignPath
  Foundation asks that a project already be released and be actively maintained, so the order
  is release first, apply second: the first version ships unsigned and this page ships in
  place of a certificate — what SmartScreen will say, what `winget install` avoids, and the
  two things a downloader can check today. The job sits in the release workflow behind
  `if: false` so the shape of the pipeline is reviewable now and turning it on is one reviewed
  line rather than a new file written on release day. Azure Trusted Signing is recorded as the
  one hard blocker: individuals are United States and Canada only, and the organisation list
  does not include the maintainer's country.
- **`docs/RELEASE.md`, `SECURITY.md`, `CONTRIBUTING.md`.** The first is the checklist, every
  command in it for the maintainer and none of it run by CI or by anything else; it marks
  where reversible stops. `SECURITY.md` is the file-by-file inventory — what is read, what is
  never read, what is written, and the one key in another program's configuration this product
  will ever touch, only when asked.
- **A README that can be read by somebody who has never seen this repository**: how to install
  it, what the status-line wrapper does and how to undo it by hand, and a **Known limits**
  section that writes down the nine things this design cannot do, each one a consequence of a
  decision explained somewhere in the repository rather than a surprise.

- **Six languages: English, Türkçe, 中文, 한국어, Русский, Español** (`ui/locales/*.json`).
  Chinese, Korean, Russian and Spanish were **machine-translated first**, as planned; English
  and Turkish were written by hand. **107 keys in every file**, and the panel, the tray
  tooltip, the context menu and the Windows notifications all read the same six files — the
  panel bundles them, the Rust side compiles the same paths in with `include_str!`, and there
  is no translation table anywhere in the Rust sources. Changing the language applies
  **without a restart**. Each language keeps its own conventions rather than English's: the
  percent sign sits where the language puts it (`88 %`, `%88`, `88%`) and the countdowns are
  built from each language's own unit abbreviations. **Corrections are welcome as pull
  requests** — the review table and the rules a locale file has to keep are in
  [`ui/locales/README.md`](ui/locales/README.md).
- **No plural machinery, and that is deliberate.** Every counted string renders its number
  beside a unit *abbreviation*, which is the same word after 1 as after 5 in all six — the
  reason Russian's three forms (1, then 2–4, then 5 and up) never come up. `pluralCategory`
  and `plural` in `ui/src/i18n.ts` implement that rule for the first counted *word* anybody
  writes, and a test freezes the seven keys that carry a count so an eighth is a decision
  rather than an accident. No right-to-left language is in scope; a test fails if one is added
  to `LOCALES` before `styles.css` has been audited for physical `left`/`right` properties.
- **"No hard-coded text" is now two tests rather than a promise** (`ui/test/i18n.test.mjs`).
  They read `ui/src` and `crates/nazar-tray/src`, throw away comments, Rust test modules and
  the argument of every `eprintln!`/`println!`/`panic!`/`.expect(…)`, and fail on any
  remaining literal that reads like a sentence. The allow-list is three entries: two CSS class
  names, and the demo's stand-in for a reader's own error sentence — the one string the panel
  prints verbatim, because it says which file said what. Command-line output stays English on
  purpose.
- **A picture of the panel in each of the six languages**, `docs/screenshots/wp6-100-*.png`,
  produced by `scripts/screenshot.ps1`'s documented set like every other picture in the
  repository.
- **Threshold notifications** (`nazar-core::alerts`, `crates/nazar-tray/src/alerts.rs`). A
  toast when a window crosses 60, 85 or 100 % — whatever the settings say — **once per
  threshold per reset period**, keyed `(provider, window, threshold, resetsAt)` and written to
  `%APPDATA%\nazar\alerts.json` **before** it is shown, so a restart does not repeat a
  warning the user has already had. Crossing is **edge-triggered** (`previous < T ≤ current`),
  so a window sitting above a threshold says nothing for the rest of the week; the **first**
  observation counts, so a tray started at 91 % says so once rather than waiting for a
  crossing that already happened; a **reset clears the memory as well as the keys**, so a
  window that resets from 86 % straight back to 86 % warns again; and **a window nobody could
  read never notifies and forgets what it last saw**, so the reading after it is a first
  observation rather than a continuation of a number nobody can vouch for. A jump that crosses
  two thresholds at once is **one** toast naming the more severe of them, with both consumed.
  `Claude Code · weekly window 85 %` over `Resets in 2 h 10 m`, in the user's language.
- **Quiet hours**, and a switch above them. `quietHours: {from, to}` in **local wall-clock**
  time, wrapping midnight when `to` is earlier than `from`. A suppressed crossing still
  happens, is still recorded and still colours the tray icon — only the interruption is
  withheld, and it is not delivered late either. The same is true of turning the
  notifications off, so turning them back on is quiet rather than a burst of catching up.
- **Start with Windows** (`tauri-plugin-autostart`). Adds
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` → `nazar-tray` = the executable
  plus `--hidden`. The switch **reads its state back from the registry** rather than
  remembering an answer of its own, so it agrees with Task Manager's Startup tab. The same
  thing from a terminal, for a user whose panel will not open:
  `nazar-tray --autostart on|off|status`.
- **A settings page**, as a second view in the same window and reachable from the tray menu.
  Language, theme, light/dark, **per-provider on and off**, the three thresholds, quiet hours,
  autostart, the detailed-windows switch with the plain-words explanation of what it reads,
  where every file lives with the home directory collapsed to `~`, a way to bring the
  first-run tip back, and the version. The form is **validated before anything is written and
  refused as a whole**: `85 / 60 / 100` gets an error and the settings the user had, not an
  error and a tray that has half changed.
- **The one-time Max-plan offer** that WP2b decided and left to be asked. *On a Max plan?
  Detailed windows shows model-specific weekly limits.* Shown once, in the panel, and answered
  for good either way. It is phrased as a **question** because it has to be: the status-line
  payload carries no plan name, so on the passive path there is nothing to detect — a rule
  that only fired for a *known* Max plan would never fire at all. It is not offered to
  somebody who has the mode on, has already been asked, or does not have Claude Code set up
  on this machine.
- **A provider you switch off is not read at all.** No reader is built for it, so its files
  are never opened and its card is not drawn — which is a stronger promise than hiding it,
  and it needed one new command in the refresh loop (`Reconfigure`) so the readers can be
  replaced on the thread that owns them, and a refresh at once rather than at the next minute.
- **One answer about the language, for the panel and the tray alike.** The override, then the
  operating system's UI language, then English — decided in Rust, handed to the panel, and
  used for the tooltip, the menu and the toasts. **This closes WP4's open risk**, where the
  panel guessed from `navigator.languages` while the tray fell back to English and the two
  could disagree. Changing it **rebuilds the tray menu**, because a menu item's text is fixed
  when the item is built.
- **`--hidden`** (what the startup entry passes: open no window, whatever else was asked for)
  and **`--demo-cross`**, the acceptance run — `80 → 86 → 86 → reset → 86` on one window,
  four seconds apart, which should produce one 60 % toast, one 85 % toast, **silence**, and
  one more 85 % toast. It implies `--demo`, so it takes no lock and its notification log is in
  memory: a demo run cannot consume the keys of a real crossing nobody has been shown yet.
- **63 new tests in Rust (444 in the workspace) and 17 more in the panel (68).** The state
  machine on an injected clock and a temporary `NAZAR_HOME` — edge crossing, once per reset,
  reset clears, quiet hours, startup-above, **sleep-and-wake replay**, unknown never fires,
  the log's round trip and its damaged-file behaviour — the settings validation, the form's
  round trip, a gate proving the panel's validator and Rust's agree, and one proving
  `ui/src/i18n.ts` and `nazar-core::config` list the same languages.

### Changed

- **The bead is pixel art now, its iris is yellow, and it carries no state** (decision K25,
  `crates/nazar-tray/src/icon.rs`). The tray icon was four concentric circles and a 4×4
  supersampler; it is the mark Nazar adopted as its direction 04 — **sixteen rows of sixteen
  cells** — and the renderer is a lookup with no arithmetic and no blending in it. At 16 px a
  cell is exactly one device pixel, so what Windows draws is the artwork rather than an
  approximation of it, and no pixel is ever partly transparent at any size.
  Rim `#0E2A5A`, band `#FFFFFF` and pupil `#0A0A0F` stay Nazar's; the **iris is `#F2A93B`**
  where Nazar's is `#3FA9F5`, because the two beads sit side by side in one Windows tray and
  16 pixels is not enough room to tell sibling applications apart by shape. `theme.test.mjs`
  asserts the three shared hexes *and* the one deliberate difference. `#F2A93B` is itself a
  family colour — **Nazar's amber, the colour its bar bead turns past the warning
  threshold**, `modes.dark.warn` in both theme files — so the marks differ by a hex the brand
  already owns rather than by a new one.
- **The icon does not fill with the quota.** It is the mark, whole, at every reading: no fill
  level, no severity colour, no fading with age. Two attempts at a gauge were built and taken
  back the same evening — one filling the whole chamber, which lost the white band past about
  70 % and turned the bead into an orange disc, and one confined to the iris, which kept the
  mark but made a part-coloured iris look like a bead with a piece missing rather than like a
  measurement. Sixteen pixels is not a place to read a percentage off, and both places that
  *are* were always a second away and are unchanged: **the tooltip still names each provider's
  binding window and its percentage, and the panel still shows every window, its severity
  colour, its reset and its age.**
- **Unknown is the icon's one state, and it is the audit's** (finding B03). When no provider
  could be read the rim goes grey `#78879A` and a six-by-six ring one cell thick stands where
  the iris and pupil are — never a confident bead that could pass for a healthy reading. Every
  other reading draws the same mark.
- **`size_for_scale` returns a whole multiple of sixteen** — 16, 32, 48, 64 — instead of the
  exact `SM_CXSMICON` size. At 20 or 24 pixels four of the sixteen cells come out a pixel wider
  than the rest, which lands on a two-cell band and a 2×2 pupil and draws a lopsided eye. At
  125 % and 150 % the shell scales a 16-pixel bead into a 20- or 24-pixel slot instead: soft
  beats crooked at this size.
- **The application icons are rendered from the grid, not by `cargo tauri icon`**
  (`scripts/render-app-icons.mjs`). The CLI resamples one large PNG down with a smooth filter,
  which turns 8-bit art into a blur of it. The new script renders every size from
  `ui/assets/bead.svg` nearest-neighbour and packs the same containers the CLI produced —
  PNG-in-ICO at 16/24/32/48/64/256, an `.icns` of the PNG-carrying types, the loose PNGs and
  the MSIX logos. The nine logo boxes that are not multiples of 16 get the bead at the largest
  whole cell size that fits, centred, with transparent padding. Zero dependencies, no browser,
  no network.
- **The icon strip moved to `docs/design/bead-states.png`** and brought company: both states —
  the mark, and unknown — at 16, 32 and 64 pixels, one file each. `nazar-tray --icons
  docs/design` writes all seven from the code that draws the real icon. The panel screenshots
  in `docs/screenshots/` still show the round bead in the header and are due a re-shoot.
- **Three layout fixes for text that only a translation could produce.** A provider card's
  first line **wraps**, because Russian's *"устарело · последние данные 1 ч 10 мин назад"* was
  being cut off with an ellipsis where the English fits. The footer **wraps whole buttons**
  rather than squeezing them. And Korean gets `word-break: keep-all`, scoped to `lang="ko"`,
  because CSS's default treats Hangul the way it treats Chinese and was splitting `설정` and
  `않습니다` down the middle — right for Chinese, which has no spaces to break at, and wrong
  for Korean, which does. The panel measures itself after every render, so a line that moves
  makes the window grow rather than the text vanish.
- **The theme toggle names the theme through a message key** rather than through the theme
  file's own English `label`, so the footer no longer says "Graphite" beneath a dropdown that
  says "Grafit".
- **The metadata that was going to live in each locale file lives in
  `ui/locales/README.md` instead.** A `_meta` object in the JSON is not a harmless extra key:
  the Rust side parses these files as `BTreeMap<String, String>`, a nested value makes the
  whole file fail to parse, and a file that fails to parse becomes an *empty* catalogue —
  correct, because a damaged translation must not stop the tray from starting, but silent.
  Adding one to `zh.json` removed Chinese from the settings with no error anywhere. A test now
  fails on a key beginning with `_` and on any value that is not a string.
- **The refresh loop announces every pass**, not only the ones that moved the numbers
  (`Event::Refreshed`). A tray started when the weekly window is already at 91 % changes
  nothing, and that is exactly the case where the user most needs telling.
- **The whole settings document is the tray's state**, rather than the four panel keys WP4
  kept beside it: the readers themselves now depend on the settings, and there is one place a
  change is written and acted on.
- `paths::settings_dir` joins `config.json` and `alerts.json`, which share a lifetime:
  removing the settings should take the notification bookkeeping with it.

### Fixed

- **Release binaries no longer carry the build machine's user profile path.** Every panic
  location and every `#[track_caller]` site is compiled in as a **string literal** naming the
  source file it came from, which is why `[profile.release] strip = true` never touched them:
  `nazar-tray.exe` carried **310** copies of `C:\Users\<account>\.cargo\registry\src\…` and
  `nazar-statusline.exe` **7**, and the NSIS package shipped both. No grep in
  `docs/RELEASE.md` could have found it — those read the working tree, and this existed only
  inside the artefact. `scripts/build-installer.mjs` now passes `--remap-path-prefix` to
  every cargo invocation it makes, the wrapper's build included, so a dependency's panic
  reads `cargo\serde_json-1.0.x\src\…` and one of this workspace's own reads
  `crates\nazar-tray\src\…` wherever it was built. Both counts are now **0**, and they stay
  there: `scripts/check-binary-paths.mjs` reads the binaries back in text and in UTF-16, the
  build **deletes the bundle** rather than hand over an installer that fails the check, and
  both workflows run it as a step of their own — a GitHub runner's `C:\Users\runneradmin` is
  held to the same rule, which is what makes this checkable on a pull request instead of on
  release day. `[profile.release] trim-paths` would be the tidy version of all of it and is
  still nightly-only on the pinned toolchain.
- **A Codex window whose reset had passed was reported as a current reading.** Codex writes
  its quota into the session log it is already keeping, so it only ever says anything while it
  is running — and on a machine nobody had opened it on for two days, `limits.json` said
  `codex.secondary: percent 70, resetsAt 2026-09-07T12:24:52Z, state: "ok"`. A confident
  number for a week that had ended the day before. Such a window is now `state: "stale"`: it
  **keeps its percentage**, because that is still the last thing the server said and the panel
  should still show it, and it stops counting as current. The threshold is five minutes past
  the reset, which is grace for a window that has just turned over and whose new numbers are
  one log line away. The rule is the **Codex** reader's alone — Claude Code's five-hour window
  renews while a session is open and is re-reported seconds later, so the same rule there
  would grey out a live window once a day.
- **A threshold notification is no longer fired for a period that has already ended.** Rule 6
  of the notification state machine: a window whose `resetsAt` is behind the current instant
  produces nothing and forgets what it last saw, so the first reading of the next period
  starts from nothing. Written against `resetsAt` rather than against `state`, because a
  window can be `stale` and still be about the week you are in — the opt-in endpoint goes
  stale fifteen minutes after a fetch with its weekly reset three days away, and silencing
  *that* would drop the 85 % warning the feature exists for.
- **Codex's `resetsAt` is rounded down to the whole minute, like Claude's.** The Claude reader
  has done this since the toast-storm fix; Codex was still writing seconds
  (`2026-09-09T02:31:59Z`), so `limits.json` carried two spellings of one kind of value and
  one reader was permanently a fix behind the other. Nothing downstream consumes sub-minute
  precision: the panel counts down in minutes, and the notification state machine only asks
  whether two readings name the same period.
- **The toast storm on ±1 s `resetsAt` jitter.** On 2026-09-08 the maintainer's machine
  showed **32 identical toasts** in two and a half hours — *Claude Code · weekly window 60 %* —
  one every five minutes, most of them doubled, and rewrote `%APPDATA%\nazar\alerts.json` every
  time. The cause was one second: the usage endpoint reported the same weekly reset as
  `2026-09-12T02:00:00Z` and `2026-09-12T01:59:59Z` on alternating refreshes, and rule 4 of the
  notification state machine — *a new `resetsAt` is a new week* — compared the two as **strings**
  in both of its halves. Every flip cleared the fired thresholds and the remembered percentage,
  so the next reading was a "first observation" and fired 60 % again. Two `resetsAt` values are
  now one period when they are **less than half a window apart** — half, because a period that
  genuinely renews moves its reset forward by a whole window, and nothing legitimate lands in
  between — or, for a window with no `windowMinutes`, less than an hour. Text that will not parse
  as an instant still falls back to comparing the strings. The record's own `resetsAt` is written
  with the newest spelling when something fires and left alone when nothing does, so the file
  stops being rewritten on every wobble.
- **Both Claude sources now spell one instant one way.** `resets_at` is rounded **down to the
  whole minute** as it is read, for the status-line payload's Unix seconds and for the usage
  endpoint's `2026-09-12T02:00:00.130216+00:00` alike. Nothing downstream has a consumer for
  sub-minute precision — the panel counts down in minutes — and flooring makes the endpoint
  produce the same text twice running for a reset that has not moved. It is the cheap half of
  the fix; the tolerance above is the half that survives a source jittering by more than a
  minute.
- **`nazar-statusline install` wrote a command Windows shells never ran.** Claude Code
  hands `statusLine.command` to **Git Bash** on Windows where it can find one, and in `sh` a
  backslash outside quotes is the escape character — so the absolute path this installer
  wrote arrived at the shell with its separators eaten, no program was spawned, and no
  capture was ever written. Nothing said so: `status` printed `installed: yes`, the settings
  file held the right path, and the only symptom was a status line that showed nothing,
  which is also what a session that has not refreshed yet looks like. The path is now
  written with **forward slashes** — a separator to Windows itself, untouched by Git Bash
  and by PowerShell — and still quoted when it contains a space. A POSIX path is written
  exactly as it is.
- **`install` repairs a broken command instead of reporting "already installed".** It used
  to ask "is this command ours?" first, and a command can be this very executable and still
  be one the shell cannot start. An unrunnable command is now detected before ownership and
  rewritten, with the same diff every other edit is shown as; a quoted backslash path and
  another copy of the wrapper elsewhere are both left alone, because both work. Every
  spelling of one path — slashes either way, quoted or bare, either case — counts as one
  installation, so `status` and `uninstall` recognise an installation made by any earlier
  build. A repair keeps `chain.json`'s record of the status line the user really had and
  updates only `installedCommand`: recording our own command as `previous` would have made
  `uninstall` restore the broken one.
- **`status` says why an installed status line produces nothing.** A command with an
  unquoted backslash on Windows now prints a `warning:` line naming the shell that will not
  run it and the command that rewrites it, and a settings file running a different copy of
  the wrapper from the one `chain.json` records is named rather than silently ignored.
- **`install` warns when it is about to write a build-directory path** (`target/release`,
  `target/debug`). It still installs — that is what a developer running it from `target`
  asked for — but the path stops working at the next `cargo clean`.
- **A flaky test in the detailed-windows suite**, at the cause rather than with a wait.
  `MockServer` recorded a request **after** answering it, so a test that read `requests()` the
  moment `refresh()` returned could beat the worker thread to the push — about one full-suite
  run in ten, hitting whichever of the four request-inspecting tests happened to lose. The
  record is now taken **before the response is written**, while the client is still blocked on
  `read`, so a client that has an answer is a client whose request is already visible.
  Measured either side: 4 failures in 40 runs before, 0 in 30 after. A `sleep` would have
  widened the window on a fast machine and still lost on a loaded one.

### Known limits

- **Clicking a toast cannot open the panel**, and it is the plugin rather than a decision:
  `tauri-plugin-notification` 2.4 builds the notification, spawns `show()` onto the async
  runtime and drops the handle, so the `on_activated` callback that `notify-rust` does offer
  on Windows never reaches an application, and `action_type_id` is mobile-only. The tray icon
  a click away is the workaround, and the toast names the window it is about.
- ~~**Disabling autostart leaves one registry value behind.**~~ **The uninstaller takes it.**
  `auto-launch` removes the `Run` value but not the matching `StartupApproved\Run` one; the
  NSIS uninstall hook now removes it. Turning the switch off *inside a running tray* still
  leaves it — that is `auto-launch`'s behaviour, and it is inert, since Task Manager only
  lists entries that exist in `Run`.
- **The `Run` value is written unquoted.** `C:\Program Files\nazar-tray\nazar-tray.exe
  --hidden` works because Windows tries each space-delimited prefix, but it does depend on
  that.

- **The tray bead, drawn at run time** (`crates/nazar-tray/src/icon.rs`, decision K3). One
  rasteriser, one drawing, four sizes: 16 px at 100 %, 32 at 200 %, 48 and 64 above that —
  handed to the shell as raw RGBA with no PNG anywhere in the path. A deep-blue rim, a
  **white chamber that is what "empty" looks like**, and a fill rising from the bottom row by
  row with the **binding window across both providers**: yellow below 60 %, orange at 60, red
  at 85, and a black pupil at 100 so a spent window survives a greyscale screenshot.
  **Unknown is a grey rim and a hollow ring and never a fill** (finding B03), and freshness
  drains the colour, so an icon nobody has fed in an hour does not look as confident as one
  from a second ago. The seven hexes are copied from `ui/theme.nazar.json` and a test parses
  that file and fails if they drift. *(Written in WP4 as four concentric circles; redrawn on
  the pixel grid on 2026-09-09, and the fill, the severity colours and the fade taken out the
  same evening — the shipped icon is the mark, or grey when nothing was read. See Changed.)*
- **A tooltip that says something**: `Claude Fable week 88 % · Codex week 70 % (resets in
  2 h 10 m)`, or `nazar-tray: no data`. Each provider's binding window, named by its length
  rather than by its provider, with the model on a model-scoped weekly; the reset belongs to
  the worst window; a provider nobody could read contributes nothing rather than a `0 %`.
  Built from the same `ui/locales/*.json` the panel reads — the tray is UI too, and
  `crates/nazar-tray/src/i18n.rs` is forty lines that make `no hard-coded text` true outside
  the webview as well.
- **The panel, designed** (`ui/src`). A card per provider on navy: the provider mark (from
  `@lobehub/icons`, MIT, licence in `ui/assets/LICENSE-lobehub.txt`), the plan exactly as the
  source spelled it, and a freshness sentence — *updated 12 s ago*, *stale · last data 2 h
  ago*, *age unknown*. A row per window: its name from `windowMinutes` so no display needs to
  know which provider it is looking at, a `detailed` mark on the windows only the opt-in mode
  can produce, a bar in the bead's colours, the percentage floored, and a countdown the panel
  computes itself every second — a clock below a day, `4 d 2 h` above it, plus the reset in
  the reader's own time zone. **A window nobody could read gets the word, a hatched empty
  track and the reader's own sentence — never a bar of zero length.**
- **The first-run hint.** Windows 11 hides every new tray icon behind the `^` button, so the
  panel says so once — *drag the bead onto the taskbar to pin it* — and remembers being
  dismissed in `config.json`. The panel measures its own content after every render and asks
  for a window that fits, which is what lets the hint appear and disappear without leaving a
  gap.
- **Themes, and light and dark.** The footer toggles `nazar` and `graphite` and the choice is
  remembered; light and dark follow `prefers-color-scheme` unless the settings override it.
  Four new settings keys — `theme`, `themeMode`, `firstRunHintDismissed`, `locale` — with the
  same forward-compatible rules as the rest of the file.
- **Contrast as a test, not an intention** (`ui/test/contrast.test.mjs`). Every foreground
  the stylesheet puts on every background, in both themes and both modes, against WCAG AA
  (4.5:1 for text, 3:1 for a graphic that carries meaning). Twenty-two pairs, all passing;
  the meter fill moved from the raw accent to the derived `accentText` because `#3FA9F5` is
  2.25:1 on a light track.
- **A menu, and with it Quit** — `Open`, `Refresh now`, `Quit`. The reason it exists is the
  lock: a tray that can only be killed leaves `~/.nazar/limits.lock` behind for its
  five-minute grace period, and the next launch then starts as a reader and looks broken.
  Every route out of the application now stops the refresh loop first, which releases the
  lock. Verified by clicking it: the file is gone afterwards, where a kill leaves it.
- **The overflow-flyout race, fixed.** Clicking a tray icon *inside* the Windows 11 overflow
  shows the panel and then closes the flyout, which blurs the panel about 200 ms later and
  hid it instantly. A blur within 300 ms of a tray-initiated open is now ignored **and the
  focus is taken back**, so the next click elsewhere still closes the panel. Verified against
  the real flyout: visible and focused at 250 ms, 1250 ms and 3250 ms.
- **Screenshot flags, documented rather than hidden**: `--demo` (synthetic numbers, panel
  open, **no lock and nothing written**), `--scale`, `--theme`, `--mode`, `--hint`,
  `--locale`, and `--icons <dir>`. `scripts/screenshot.ps1` produces every picture in
  `docs/screenshots` from them, so "how was this made" has an answer that is not a memory.
  150 % and 200 % are genuinely rendered at those scales — WebView2 is given
  `--force-device-scale-factor` and the window is multiplied to match — rather than being
  upscaled screenshots.
- **37 new tests in Rust (381 in the workspace) and 25 more in the panel (51).** The
  rasteriser's are pixels — since 2026-09-09, cells: the render is the shipped artwork cell
  for cell at every size, the pupil is always there, unknown is a grey rim and a hollow ring
  and nothing else, and the PNG bytes are pinned for three cases. The panel's are pure
  functions — window names, durations, freshness sentences, the
  theme toggle — plus the cross-language gate grown to cover the new commands and **every
  message key either language names**, in either direction.

- **State model** (`nazar-core::state`). `limits.json` stores what a source measured;
  everything a display wants is derived at read time and stored nowhere, because all of it
  changes with the clock rather than with the data: the **binding** window (highest
  percentage, ties to the shorter window, and a window with no percentage never binds), the
  **countdown** (`resetsAt` − now, **negative when the reset is already due** rather than
  frozen at "now", which is audit scenario S7), the **age** (now − `sourceAt`), **freshness**
  (`fresh` ≤ 5 min, `aging` ≤ 45 min, `stale` beyond) and **severity** (`ok` < 60, `warn`
  ≥ 60, `critical` ≥ 85, `exhausted` ≥ 100). `unknown` is a real value in both of the last
  two, and it sorts *below* `ok`, so a provider nobody could read never decides the icon's
  colour on its own.
- **One definition of "stale", in the settings.** The thresholds live in
  `%APPDATA%\nazar\config.json` (`thresholds`, `freshness`) and every display reads them from
  there. The retired prototype had three displays with three different answers — 45 minutes
  in the tray, 30 in the status line, none in the fetcher — which is audit finding B14. The
  binding rule is likewise one function now, shared by both readers and the view (B16).
- **Refresh loop inside the tray process** (`nazar-core::refresh`), which is what
  `docs/PROJECT.md` section 3 decided and what deletes the Task Scheduler / launchd / systemd
  triple, the VBS launcher and the lock race the audit proved from logs. One thread owns
  every reader. Five triggers: startup, a 60-second tick, a **five-second look** at the
  status-line captures and at the rollout log being followed, an explicit request from the
  panel or a second launch, and a **clock jump of more than two ticks** — which is how a
  machine notices it has been asleep (audit scenario S7: gaps of 480, 575 and 867 minutes in
  one week of logs, each followed by something going wrong). Everything but the tick is
  coalesced by a 250 ms debounce, so a busy Codex session that writes a quota line every few
  seconds still costs one refresh. The opt-in endpoint is asked at most **once every five
  minutes**, on top of its own backoff.
- **The writer, which mostly does not write** (`nazar-core::writer`). Each refresh is
  compared with the last one — the serialised document, minus `updatedAt` — and an identical
  one is not written at all: no temporary file, no rename, no modification time, nothing for
  a watching consumer to wake up for. So **`updatedAt` moves with the content**, which is the
  honest reading of "when the tray last wrote this file", and a consumer that wants to know
  whether the *tray* is alive reads the lock's heartbeat instead. A tray restarting over an
  unchanged file writes nothing, and its panel shows the last known numbers before its first
  read finishes.
- **One writer, enforced** (`nazar-core::lock`, `~/.nazar/limits.lock`). Acquisition is a
  single `create_new`, so of two processes racing for it exactly one wins — finding B01 was a
  lock whose check and whose write were separate operations, and the logs caught two
  refreshers passing that check four seconds apart, which is what produced the observed
  HTTP 429. Liveness is a **heartbeat** rewritten on every refresh, not a process id: there is
  no portable way to ask whether a process is running, and a heartbeat also catches a holder
  that is alive but wedged. A record silent for five minutes is reclaimed; an unreadable one
  is respected until it ages out, so a competitor cannot evict a holder that is mid-write.
  The holder re-checks the file **before** it writes, so a tray whose lock was taken over
  while its machine slept stops writing rather than overwriting its replacement.
- **Single instance.** The same lock, rather than a named mutex or a plugin: it is the
  guarantee `limits.json` needs anyway, it works the same on all three target platforms, and
  it added nothing to the dependency tree. A second launch leaves a marker at
  `~/.nazar/tray.request`, which the running tray notices within five seconds, deletes, and
  answers by opening its panel — then the second process exits. A marker older than a minute
  is swept up without being obeyed.
- **`nazar-tray --print --write`**: one refresh, written to `~/.nazar/limits.json`, for
  scripts and for Linux — where v1 ships no tray at all and the CLI plus the Nazar canvas
  *is* the story. It takes the same lock, and if the tray already holds it this **reads
  instead of writing**: it prints what the tray wrote, says so on standard error, and changes
  nothing.
- **The panel reads the state.** `get_snapshot` derives the view for the instant the panel
  asked, `get_warnings` exposes the loop's per-reader counters, `refresh_now` asks for a pass
  (audit finding B13 was a Refresh button that appeared to do nothing), and a
  `snapshot-changed` event fires **only when the document changed**. The panel counts the
  seconds between refreshes itself. It shows raw values — provider, each window's percentage
  or the word "unknown", the countdown, how old the reading is — because designing it is WP4
  and a placeholder would only have to be deleted. Percentages are floored, never rounded up:
  99.6 % is not 100 % (finding B15).
- **Property tests**, with a hand-rolled generator rather than `proptest`: ten thousand
  random window sets for the binding rule, three thousand random instants × six time-zone
  spellings for the countdown (plus fixed cases at both American daylight-saving boundaries
  and at Riyadh's, which has none), and monotonicity for severity and freshness across
  their whole ranges. The severity boundaries are pinned at 59.9 / 60 / 84.9 / 85 / 100 / 101.
  A hygiene test backs the time-zone property with a grep: nothing in the workspace reads
  `TZ` or converts to local time, because a property test cannot see a dependency that has
  not been written yet.
- **94 new tests (343 in the workspace) and nine more in the panel (26)**, every loop test
  driving `Engine::tick` with a clock it moves by hand rather than by waiting — a debounce, a
  sixty-second tick and an eight-hour sleep are one line each. Three of the panel's new tests
  are a cross-language gate: a command the panel invokes and a command the Rust side
  registers are the same string in two files no compiler reads together, and a typo there
  shows up as a panel that draws nothing, at run time, on somebody else's machine.

- **Detailed windows (`nazar-core::claude::detailed`), opt-in and off by default.** With
  `detailedWindows` on, it reads four values out of `<CLAUDE_CONFIG_DIR or
  ~/.claude>/.credentials.json`, holds the access token in a wrapper that wipes itself and
  prints `Secret(<redacted>)`, and sends it as one `Authorization` header on a single
  20-second `GET` to `api.anthropic.com/api/oauth/usage`. `limits[]` becomes `five_hour`,
  `seven_day` and one `seven_day_<model>` per model-scoped weekly cap, all marked
  `detailed: true`, with the plan normalised from `rateLimitTier`
  (`default_claude_max_20x` → `max_20x`). **With the mode off nothing is opened and no
  socket is created**, and a test poisons the credential file to prove it. That file is
  never written, and `refreshToken` is never read: refreshing is Claude Code's job. The
  whole account, including where this sits against Anthropic's terms, is
  [docs/detailed-windows.md](docs/detailed-windows.md).
- **A failure keeps the last good numbers and says they are old.** 401 and 403 report
  "token expired; run Claude Code once to refresh"; 429 honours `Retry-After`; a 200 whose
  body this build cannot read is a failure rather than a blank. Each leaves the previous
  windows in place with `state: "stale"` and a short reason, and pushes the next attempt
  out — 1 s, 2 s, 4 s and so on, capped at 30 minutes, held in memory only. Three audit
  findings became behaviour here: `Retry-After` is read at all (B07), a token whose stored
  `expiresAt` has passed costs no request and a rewritten sign-in file clears the wait
  immediately (B06, the three-hour 401 storm), and a plan that changed between refreshes
  drops the remembered numbers rather than showing one account's percentages under
  another's name (B25).
- **Merge policy** (`nazar-core::claude::merge`): the endpoint lays down the block, and a
  **newer** status-line capture wins on the two windows both paths report — the status line
  is rewritten every few seconds, the endpoint is asked on a timer and backed off from.
  Model-scoped weeklies are never replaced, a passive window with no percentage never
  replaces one that has one, and the binding window is recomputed across everything.
  Written down in
  [docs/limits-contract.md](docs/limits-contract.md).
- **Settings** (`nazar-core::config`): `%APPDATA%\nazar\config.json` with
  `detailedWindows` and `detailedSuggested`, atomic writes, unknown keys preserved, a
  missing file read as the defaults and a damaged one reported rather than replaced.
  `NAZAR_HOME` now moves this file too, so a whole installation really can be pointed at a
  throwaway directory.
- **`should_suggest_detailed(plan_hint, already_asked)`**: offer the mode once, and only on
  Max, because Pro has no model-scoped weekly window for it to reveal. The dialog is WP5's.
- **`nazar-tray --print --detailed`** runs the mode for one run without switching it on for
  the machine. `--print` on its own does exactly what `config.json` says, which on a machine
  nobody has configured is nothing at all.
- **Four gates for the one exception to "no credentials".** A sentinel token goes through
  the entire flow — settings written, sign-in read, request sent, answer mapped, block
  merged, `limits.json` written, then a failure whose body echoes the sentinel back — and
  the test fails if it appears in any file under the temporary `NAZAR_HOME`, in any error's
  `Display` or `Debug`, or in the document. `Secret::expose_for_one_request` must have
  exactly one call site in shipping code. No file in the module may contain a printing
  macro. And the workspace credential grep grew an allow-listed **directory** rather than
  losing a needle, plus a test that fails the day that directory stops needing the
  exception.
- **61 new tests (249 in the workspace)**, every network one against a hand-rolled HTTP
  server on `127.0.0.1:0`. No test in this repository reaches the network, reads the real
  `~/.claude`, or writes outside a directory it made itself.

- **`nazar-statusline`**, the shared status-line wrapper, in a third crate with no Tauri in
  it (334 KB release binary, three dependencies). Installed as Claude Code's
  `statusLine.command`, it writes the payload **whole** to
  `~/.nazar/statusline/<session_id>.json` and then runs the status line that was already
  there, with the same bytes on standard input and its stdout, stderr and exit code
  forwarded. Keyed by session id, so three concurrent sessions do not overwrite each
  other. Measured at a **median of 9.6 ms** over ten runs against a 50 ms budget, release
  build, no chained command. Empty or unparseable input still chains and still exits `0`:
  nothing this program does can break the status line.
- **`nazar-statusline install | uninstall | status`**. The install touches exactly one key
  of `settings.json`, keeps every other key and their order, preserves `padding`,
  `refreshInterval` and anything else the existing `statusLine` carried, prints a unified
  diff whether or not anything is a terminal, and takes a backup it will never write over.
  It refuses — changing nothing — on invalid JSON, on a lock file, and when it is already
  installed. `uninstall` restores the exact object recorded in `chain.json`, checks it
  against the backup, and leaves the backup in place. `--dry-run` prints the same diff and
  writes nothing at all.
- **Claude reader** (`nazar-core::claude`): reads the captures, takes the newest, and maps
  `rate_limits.five_hour` and `.seven_day` to the `claude` provider block — percentage,
  `windowMinutes`, `resetsAt` in UTC, `state`, and a binding window computed as the highest
  percentage. `configured:false` when there is no capture; both windows `state:"error"`
  with no percentage when the payload carries no `rate_limits`, which is what a
  non-subscriber and a session before its first API response both look like. No plan name
  is derived: the payload does not carry one, and `model.id` is not a subscription.
- Claude fixtures sanitised from a real payload and a real settings file — a payload with
  one live window, one with both, one with no `rate_limits`, and the three settings shapes
  the acceptance criterion names (no status line, ccstatusline, a custom `node` script) —
  plus a second leak test, a second fixture-hygiene gate, and rows in
  `docs/pinned-internal-formats.md` for the payload and for `settings.json`.
- `docs/statusline-wrapper.md`: what is captured, where it lives, why captures hold paths
  and therefore never leave `~/.nazar`, and how to uninstall by hand from `chain.json` if
  the binary is gone.
- `NAZAR_HOME` moves `~/.nazar`. It exists so that the wrapper's tests run against a
  throwaway directory; **no test in this repository reads or writes the real
  `~/.claude` or the real `~/.nazar`**, which is risk R10's mitigation made mechanical.

- **Codex reader** (`nazar-core::codex`): finds the newest `rollout-*.jsonl` under
  `$CODEX_HOME` (default `~/.codex`), follows it by byte offset, and maps
  `payload.rate_limits` to the `codex` provider block of `limits.json` — `primary` and
  `secondary` with their percentage, window length and reset time, the plan name, and a
  binding window computed as the highest percentage rather than taken from a flag the
  source does not provide. No credential file is opened and nothing leaves the machine.
- `nazar-tray --print`: writes the `limits.json` document to standard output and exits
  without starting the tray. It does **not** write `~/.nazar/limits.json`; that file keeps
  its single writer, and WP3 gave it one — plus `--print --write` for the callers that want
  a refresh without a tray.
- Codex fixtures sanitised from real logs — a session's quota lines, the `limit_id`
  `premium` variant whose windows are both `null`, a log with no quota line, and a damaged
  log — plus `docs/pinned-internal-formats.md`, which records every field the reader
  depends on, the version it was observed under, and the files it must never open.
- Three gates that fail the build rather than a review: a leak test that puts a sentinel in
  every text field of a rollout line and fails if it reaches the output, a grep over the
  whole workspace for the name of any credential file, and a fixture scan for home paths,
  e-mail addresses, long hexadecimal ids and the current user's name.
- Cargo workspace: `nazar-core` (pure Rust, no Tauri, builds everywhere) and `nazar-tray`
  (the Tauri v2 application, Windows first).
- `limits.json` contract at `schemaVersion: 1` — serde types, an atomic writer
  (temporary file plus rename) and a reader, with the contract written down in
  `docs/limits-contract.md` and a sample in `fixtures/limits.sample.json`.
- Tray icon with a placeholder bead, and a frameless popup panel that opens near the
  cursor on either mouse button and closes on Esc or on losing focus.
- Panel built from plain TypeScript, HTML and CSS with esbuild — no framework
  (decision K2).
- i18n scaffolding: `locales/{en,tr}.json` written by hand, `{zh,ko,ru,es}.json` as empty
  placeholders, a `t()` helper with `{placeholder}` interpolation, and a language guess.
  EN/TR parity is a test; the four pending languages are reported, not failed (WP6).
- Theme tokens copied verbatim from Nazar (`theme.nazar.json`, `theme.graphite.json`)
  with a test asserting the four bead hexes (decision K23).
- CI: Windows lint, test and build as the gate; Ubuntu and macOS running `nazar-core`
  only; the Windows debug bundle as a non-blocking artifact job (risk R1).
- Repository hygiene: `.gitattributes`, `rust-toolchain.toml`, `rustfmt.toml`, `deny.toml`
  and `scripts/check-licenses.mjs`, which fails on any dependency that is not permissively
  licensed.

### Changed

- **`fixtures/limits.sample.json` now says `"source": "endpoint"` for Claude.** The
  document is unchanged otherwise, and it is the same document it always described: three
  windows, two of them replaced by a newer status-line capture (so no `detailed` flag) and
  one that only the endpoint can produce (so it keeps its flag). What changed is the rule
  behind `source`, which WP2b had to pin down: it names the path that **produced the block**,
  and the per-window `detailed` flag says which windows survived from it. The schema did not
  move; consumers that vendored the sample should re-copy it.
- **Timestamps from the usage endpoint are rewritten before they reach `limits.json`.** The
  endpoint answers `2026-09-07T13:10:00.130195+00:00` where the status line writes Unix
  seconds. Both now come out as `2026-09-07T13:10:00Z`, which is what rule 6 of the contract
  always said and what every other timestamp in the file already was.
- **The `limits.json` samples now show UTC.** `fixtures/limits.sample.json`,
  `docs/limits-contract.md` and `docs/PROJECT.md` section 6 carried a `+03:00` offset while
  the writer has produced `…Z` since WP1. The instants are unchanged — the same moments,
  written the way the file actually writes them. Consumers that vendored the sample should
  re-copy it; nothing about the schema moved.

### Notes

- **WP3 added no dependencies.** 542 packages in the lock file before, 542 after. Three
  crates were considered and each was measured rather than argued about. **`notify`** would
  add two packages on Windows (`notify`, `notify-types`; `filetime`, `walkdir`, `same-file`,
  `crossbeam-channel` and `log` are already there through Tauri) and more on Linux and macOS,
  for a five-second latency improvement on a display whose slowest input redraws every thirty
  seconds — and the readers already list those directories on every refresh, so the poll is
  one extra `read_dir` of a handful of entries rather than a second thread with a platform
  backend. **`tauri-plugin-single-instance`** would solve half of a problem the advisory lock
  has to solve anyway, and brings a D-Bus stack on Linux. **`proptest`** would add about ten
  packages to test rules that fit on one page; the generator here is a four-instruction
  xorshift that prints a seed a failure can be replayed from. Each decision is written down
  where the code is, not in a commit message: `refresh/watch.rs`, `main.rs` and
  `state/tests.rs`.
- **`updatedAt` now means "when the content last changed".** It always said "when the tray
  last wrote the file", and that is still exactly what it is — the writer only writes on a
  change. A consumer that was using it as a liveness signal for the tray should read
  `~/.nazar/limits.lock`'s `heartbeatAt` instead, which advances every minute regardless.
- **A tray that is killed rather than quit still keeps its lock for five minutes.** WP4's
  menu means it need not be killed: `Quit` stops the loop, which releases the lock. Deleting
  `~/.nazar/limits.lock` by hand remains the escape after a crash, and is safe when no tray
  is running.
- **The right button opens the menu, not the panel.** `docs/PROJECT.md` section 7 asks for
  "panel opens on left/right click"; with a native menu attached, the right button belongs to
  the shell. The panel opens on a left click and from the menu's first item, `Open`. Showing
  both at once was tried: the menu takes the focus, the panel blurs behind it, and the
  blur-suppression that keeps it alive then leaves a panel nobody can dismiss.
- **WP4 added no dependencies either.** `Cargo.lock` and `ui/package-lock.json` are byte
  for byte what they were before it. `tiny-skia` was
  pre-approved for the icon and was not needed — the picture is four circles and a horizontal
  cut — and the PNG writer behind `--icons` is a fixed-Huffman deflate in eighty lines, which
  puts the documentation strip at 15 KB rather than the 130 KB uncompressed blocks would have
  cost. `ui/test/icon-strip.test.mjs` inflates the committed file with Node's own zlib, so
  that encoder is checked by a decoder nobody in this repository wrote.
- **The derived view now leaves empty fields out rather than sending `null`.** `SnapshotView`
  is the panel's wire shape and nothing else reads it; `"percent": null` becomes `0` after
  one careless `Math.floor`, which is finding B03 arriving through the back door. The panel
  is defensive about it as well.
- **The detailed-windows mode is behind a cargo feature as well as the runtime flag.**
  `nazar-core`'s `detailed-windows` feature is on by default, because the shipped tray
  offers the toggle; `nazar-statusline` depends on `nazar-core` with
  `default-features = false`, so the binary that runs on every status-line refresh compiles
  neither the HTTP client nor the code that would read a token —
  `cargo tree -p nazar-statusline` still shows `serde` and `serde_json` and nothing else.
  (A `cargo build --workspace` unifies features and builds the shared rlib once with the
  feature on; the released wrapper is built on its own, where it does not.)
- **`ureq` with `rustls` was chosen over `reqwest` by measurement.** Against this
  workspace's lock file: `ureq` adds four packages (`ureq`, `ureq-proto`, `utf8-zero`,
  `webpki-roots`), `reqwest` with `blocking` adds seventeen including `aws-lc-rs`,
  `aws-lc-sys`, `cmake` and `quinn`. `reqwest` is already linked by Tauri, but only in the
  tray crate; this code lives in `nazar-core`, which has no Tauri in it and is linked by a
  binary that has to start in under ten milliseconds. All four are permissively licensed and
  `webpki-roots`' `CDLA-Permissive-2.0` was already on the allow-list. `zeroize` was already
  in the lock file transitively, so the wiping costs nothing new.
- Times in `limits.json` are RFC 3339 in UTC (`…Z`). They name the same instant a local
  offset would; consumers render local time, and countdowns do not depend on the offset.
  Reasoning in `docs/pinned-internal-formats.md`.
- A capture file holds the whole status-line payload, and the payload holds paths — `cwd`,
  `transcript_path`, the workspace. There is no token and no account identifier in it, but
  the paths are why captures live under `~/.nazar` and why `limits.json` gets four numbers
  out of one and nothing else. Cost and context usage stay in the capture, for Nazar.
- The wrapper runs a chained command through `cmd.exe /C` on Windows and `sh -c`
  elsewhere, because `statusLine.command` is a command line rather than a program and its
  arguments. On Windows the line is passed raw and wrapped in one more pair of quotes;
  Rust's own escaping follows the C runtime's rules and `cmd.exe` does not, which would
  break every command with a quoted path in it.
- The Codex reader reports what it read and how old it is (`sourceAt`); it does not decide
  when old becomes stale. That threshold belongs to the state model, which WP3 put in the
  settings so that all three displays answer it the same way — the retired prototype had
  three different ones.
- The tray still shows a static bead. The bead that fills from the bottom with the binding
  window is WP4 (decision K3, drawn in Rust per scale factor); WP3 computes the fill level
  and the colour but draws neither.
- `tauri-plugin-notification`, `tauri-plugin-autostart` and `tauri-plugin-updater` are
  declared but not initialised. They are wired in WP5 and WP7.
