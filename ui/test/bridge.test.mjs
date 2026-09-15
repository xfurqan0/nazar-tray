// The names that have to match across the two languages.
//
// A command the panel invokes and a command the Rust side registers are the same string
// written in two files that no compiler reads together. Neither `tsc` nor `cargo` can catch
// a typo here; it shows up as a panel that draws nothing, at run time, on somebody else's
// machine. So it is a test. The same goes for message keys: `t("panel.window.weekly")` in
// TypeScript and `catalog.text("tray.menu.quit")` in Rust both read `locales/en.json`, and
// a key that only one side has is a word that never appears.

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const read = (relative) => readFileSync(resolve(REPO, relative), "utf8");

const rustMain = read("crates/nazar-tray/src/main.rs");
const rustState = read("crates/nazar-tray/src/state.rs");
const panel = read("ui/src/main.ts");

/**
 * Every module that may define a command, and the reason this is a written list.
 *
 * The registered commands are the front end's whole surface: they are what a webview can
 * ask this process to do. Finding them by walking `src/` would let the surface grow without
 * anyone noticing, so a new command module is a line here as well as a line in `main.rs`.
 * WP5 had one, the settings and the snapshot. WP7 added the second: the status-line wrapper,
 * the only part of this product that edits a file belonging to another program. T-WP15 added
 * the third: the usage store, the only command that reads hundreds of megabytes and therefore
 * the only one that is throttled.
 */
const COMMAND_MODULES = ["state.rs", "statusline.rs", "usage.rs"];

/**
 * Every `#[tauri::command]` the Rust side defines.
 *
 * The attribute takes arguments — `#[tauri::command(async)]` is what puts a slow command on
 * a thread of its own — so the pattern allows them. A command the macro accepted and this
 * expression did not would be one the registration check had quietly stopped watching.
 */
const defined = COMMAND_MODULES.flatMap((file) =>
  [
    ...read(`crates/nazar-tray/src/${file}`).matchAll(/#\[tauri::command[^\]]*\]\s*\npub fn (\w+)/g),
  ].map((match) => match[1]),
);

/** Every command named in `generate_handler!`, with the module path taken off. */
const registered = (() => {
  const block = /generate_handler!\[([\s\S]*?)\]/.exec(rustMain);
  assert.ok(block, "main.rs has no generate_handler! block");
  return block[1]
    .split(",")
    .map((name) => name.trim().replace(/^\w+::/, ""))
    .filter(Boolean);
})();

/** Every command the panel invokes, with or without a type parameter or arguments. */
const invoked = [...panel.matchAll(/invoke(?:<[^>]*>)?\("(\w+)"/g)].map((m) => m[1]);

test("every command the Rust side defines is registered", () => {
  assert.ok(defined.length >= 3, `expected the WP3 commands, found ${defined.join(", ")}`);
  assert.deepEqual([...defined].sort(), [...registered].sort());
});

test("every command the panel invokes exists on the Rust side", () => {
  assert.ok(invoked.length > 0, "the panel invokes nothing, which cannot be right");
  for (const command of invoked) {
    assert.ok(registered.includes(command), `the panel invokes ${command}, which is not registered`);
  }
});

test("WP4's commands are all there: the panel's actions and the way out", () => {
  // `quit` is the one that matters. WP3 could only be stopped by killing the process,
  // which leaves ~/.nazar/limits.lock behind for its five-minute grace period.
  for (const command of ["quit", "open_panel", "dismiss_hint", "set_theme", "set_panel_height"]) {
    assert.ok(registered.includes(command), `${command} is not registered`);
  }
  assert.match(
    rustState,
    /pub fn quit\([^)]*\)\s*\{\s*state\.shutdown\(\);\s*app\.exit\(0\)/s,
    "quit must release the advisory lock before the process goes",
  );
});

test("WP5's commands are all there: the settings and the startup entry", () => {
  for (const command of [
    "get_config",
    "set_config",
    "get_autostart",
    "set_autostart",
    "reset_hint",
    "dismiss_detailed_suggestion",
  ]) {
    assert.ok(registered.includes(command), `${command} is not registered`);
    assert.ok(invoked.includes(command), `the panel never calls ${command}`);
  }

  // `open_settings` is the odd one out: it is called from the tray menu, on the Rust side,
  // because the panel that would otherwise call it is the panel it opens. What reaches the
  // webview is the event it emits.
  assert.ok(registered.includes("open_settings"));
  assert.match(
    read("crates/nazar-tray/src/tray.rs"),
    /MENU_SETTINGS => crate::state::open_settings/,
    "the tray menu's Settings entry no longer opens the settings",
  );

  // Validate, then write, then apply. A form that does not validate changes nothing at all.
  assert.match(
    rustState,
    /let problems = next\.validate\(\);\s*if !problems\.is_empty\(\) \{\s*return Err\(problems\);/s,
    "set_config must refuse the whole form rather than saving part of it",
  );
  // The switch reads the machine, not our own settings file.
  assert.match(
    rustState,
    /manager\.is_enabled\(\)\.map_err/,
    "set_autostart must read the state back from the plugin rather than assuming it",
  );
});

test("T-WP15's command is there, and the panel's half of it matches the Rust half", () => {
  const rustUsage = read("crates/nazar-tray/src/usage.rs");
  const types = read("ui/src/snapshot.ts");

  assert.ok(registered.includes("get_usage"), "get_usage is not registered");
  assert.ok(defined.includes("get_usage"), "get_usage is not defined in usage.rs");
  // Off the main thread: the scan reads hundreds of megabytes, and a panel whose window
  // froze while it drew a chart would be a worse bug than a chart that took a second.
  assert.match(rustUsage, /#\[tauri::command\(async\)\]\s*\npub fn get_usage/);

  // The three ranges are written down twice, once per language. A fourth on one side is a
  // range the panel can ask for and the command refuses, at run time, on somebody's machine.
  const fromRust = [...(/pub const RANGES: \[&str; \d+\] = \[([^\]]+)\]/.exec(rustUsage)?.[1] ?? "")
    .matchAll(/"(\w+)"/g)].map((match) => match[1]);
  const fromTs = [...(/export type UsageRange =([^;]+);/.exec(types)?.[1] ?? "").matchAll(
    /"(\w+)"/g,
  )].map((match) => match[1]);
  assert.deepEqual(fromRust, ["week", "month", "all"]);
  assert.deepEqual(fromTs, fromRust, "ui/src/snapshot.ts and usage.rs list different ranges");

  // The one place in this bridge that is snake_case, and the reason is written in both
  // files. The five counters belong to the store, so they are checked against the module
  // that writes them rather than against the command, which hands the buckets through
  // untouched and names none of them.
  const store = read("crates/nazar-core/src/usage/store.rs");
  for (const counter of ["input", "output", "cache_create", "cache_read", "requests"]) {
    assert.ok(types.includes(counter), `ui/src/snapshot.ts does not name ${counter}`);
    assert.ok(store.includes(`pub ${counter}:`), `the store no longer has a ${counter}`);
  }
  for (const key of [
    "scanned_at",
    "files_seen",
    "took_ms",
    // T-WP17: a pass is both readers, so the diagnostic says which ones ran.
    "providers_scanned",
    "skipped_api_errors",
    // T-WP25: a rollout Codex compressed reaches no total, so the pass says how many it
    // walked past. The two sides spell it the same way or this fails.
    "files_compressed",
  ]) {
    assert.ok(types.includes(key), `ui/src/snapshot.ts does not name ${key}`);
    assert.ok(rustUsage.includes(key), `usage.rs does not name ${key}`);
  }
  assert.ok(
    !/#\[serde\(rename_all = "camelCase"\)\]/.test(rustUsage),
    "the usage document is snake_case on both sides; see docs/usage-contract.md",
  );

  // Every kind the Rust side can return is a kind the panel has a word for.
  const kinds = [...rustUsage.matchAll(/UsageError::(?:new|from_core)\(\s*"(\w+)"/g)].map(
    (match) => match[1],
  );
  assert.ok(kinds.length >= 4, `usage.rs returns ${kinds.join(", ")}`);
  for (const kind of kinds) {
    assert.ok(types.includes(`"${kind}"`), `ui/src/snapshot.ts has no case for ${kind}`);
  }
});

test("the usage scan is not on the refresh path, and only the lock holder runs it", () => {
  // The contract's rule: quota is why this application exists, it reads two small files in
  // milliseconds, and it must never queue behind a scan that reads hundreds of megabytes.
  const refresh = read("crates/nazar-core/src/refresh/mod.rs");
  assert.ok(
    !refresh.includes("scan_claude") && !refresh.includes("usage::"),
    "the refresh loop must not scan the transcripts",
  );

  const rustUsage = read("crates/nazar-tray/src/usage.rs");
  assert.match(
    rustUsage,
    /pub const SCAN_INTERVAL_MS: u64 = 5 \* 60 \* 1000;/,
    "docs/usage-contract.md says at most one scan every five minutes",
  );
  // One writer, many readers: the same advisory lock that already guards limits.json.
  const main = read("crates/nazar-tray/src/main.rs");
  assert.match(main, /let writes_usage = lock\.is_some\(\);/);
  assert.match(main, /usage::UsageState::new\(writes_usage\)/);
});

test("a demo run answers from the fixture and has no usage store to open", () => {
  // 0.2.0's privacy rule. `--demo` used to have no usage document of its own, so the usage
  // view and the tray tooltip read the real store — and every picture of the usage view would
  // have carried a month of the maintainer's own model use into a public repository.
  const main = read("crates/nazar-tray/src/main.rs");
  assert.match(
    main,
    /usage::UsageState::demo\(\)/,
    "a --demo run must manage a usage state with no store behind it",
  );

  const rustUsage = read("crates/nazar-tray/src/usage.rs");
  assert.match(
    rustUsage,
    /fn store_dir\(state: &UsageState\) -> Result<Option<PathBuf>, UsageError>/,
    "one gate decides whether this process has a store at all",
  );
  // The Rust side asserts this at run time as well; this is the cross-language half, so that
  // deleting the gate fails both suites rather than one.
  const body = rustUsage.split("#[cfg(test)]")[0] ?? "";
  assert.equal(
    body.match(/paths::settings_dir\(\)/g)?.length,
    1,
    "the settings directory may be named in exactly one place in crates/nazar-tray/src/usage.rs",
  );
});

test("the tray menu offers the four actions, and can be rebuilt in another language", () => {
  const tray = read("crates/nazar-tray/src/tray.rs");
  for (const key of [
    "tray.menu.open",
    "tray.menu.refresh",
    "tray.menu.settings",
    "tray.menu.quit",
  ]) {
    assert.ok(tray.includes(`"${key}"`), `the tray menu no longer names ${key}`);
  }
  assert.match(tray, /state\.shutdown\(\);/, "the menu's Quit must release the lock");
  // WP4's open risk: a menu item's text is set when the item is built, so a language change
  // has to build new ones.
  assert.match(tray, /pub fn rebuild_menu/, "the menu must be rebuildable");
  assert.match(tray, /tray\.set_menu\(Some\(menu\)\)/);
  assert.match(
    rustState,
    /if applied\.locale_changed \{[\s\S]{0,200}?crate::tray::rebuild_menu/,
    "changing the language must rebuild the tray menu",
  );
});

test("the plugins are initialised, and the panel is granted neither", () => {
  const main = read("crates/nazar-tray/src/main.rs");
  assert.match(main, /tauri_plugin_notification::init\(\)/);
  assert.match(main, /tauri_plugin_autostart::init\(/);
  // The autostart entry passes `--hidden`, which is the difference between a tray that
  // starts quietly and one that opens a popup at every login.
  assert.match(main, /Some\(vec!\["--hidden"\]\)/);

  // Both plugins are driven from Rust, so the webview needs no plugin permission at all.
  // A capability the panel does not use is a capability it should not have.
  const capabilities = JSON.parse(read("crates/nazar-tray/capabilities/default.json"));
  for (const permission of capabilities.permissions) {
    assert.ok(
      permission.startsWith("core:"),
      `${permission} grants the panel something outside the core defaults`,
    );
  }
  assert.deepEqual(capabilities.permissions, ["core:default", "core:window:allow-hide"]);
});

test("the notification keys the Rust side names all exist", () => {
  const alerts = read("crates/nazar-tray/src/alerts.rs");
  for (const key of [
    "alert.title",
    "alert.window.fiveHour",
    "alert.window.weekly",
    "alert.window.modelWeekly",
    "alert.body.resets",
    "alert.body.resetDue",
    "alert.body.noReset",
  ]) {
    assert.ok(alerts.includes(`"${key}"`), `alerts.rs no longer names ${key}`);
  }
});

test("the panel and the settings agree on which languages exist", () => {
  // `LOCALES` is written down twice — once in TypeScript, once in Rust — because neither
  // side can import the other's. A language in one list and not the other is a language the
  // settings offer and the panel cannot paint, or the other way round.
  const typescript = read("ui/src/i18n.ts");
  const rust = read("crates/nazar-core/src/config.rs");

  const listed = (source, pattern) =>
    [...(pattern.exec(source)?.[1] ?? "").matchAll(/"(\w+)"/g)].map((match) => match[1]);

  const fromTs = listed(typescript, /export const LOCALES = \[([^\]]+)\]/);
  const fromRust = listed(rust, /pub const LOCALES: \[&str; \d+\] = \[([^\]]+)\]/);

  assert.ok(fromTs.length >= 6, `ui/src/i18n.ts declares ${fromTs.join(", ")}`);
  assert.deepEqual(fromRust, fromTs, "ui/src/i18n.ts and config.rs list different languages");
});

test("the panel listens for the event names the Rust side emits", () => {
  for (const name of ["SNAPSHOT_CHANGED", "OPEN_SETTINGS"]) {
    const constant = new RegExp(`pub const ${name}: &str = "([^"]+)"`).exec(rustState);
    assert.ok(constant, `state.rs no longer names ${name}`);
    assert.match(
      panel,
      new RegExp(`listen\\("${constant[1]}"`),
      `the panel does not listen for ${constant[1]}`,
    );
  }
});

test("the refresh loop announces every pass, not only the ones that moved", () => {
  // The notifications look at every reading: a tray started when the weekly window is
  // already at 91 % changes nothing, and that is the case where the user most needs telling.
  const refresh = read("crates/nazar-core/src/refresh/mod.rs");
  assert.match(
    refresh,
    /self\.emit\(Event::Refreshed\);\s*if changed \{\s*self\.emit\(Event::SnapshotChanged\)/s,
  );
  // Two things hang off every pass, and neither may quietly stop happening. The toast is
  // WP5's; the tooltip's second line is T-WP17's, and it is here rather than on
  // `SnapshotChanged` because a store another process has just scanned moves while the quota
  // numbers stand still.
  const main = read("crates/nazar-tray/src/main.rs");
  assert.match(main, /Event::Refreshed => \{\s*alerts::on_refresh\(app\);/);
  assert.match(main, /Event::Refreshed => \{[^}]*tray::refresh_tooltip\(app\);/s);
});

test("the panel asks for the fields the derived view actually carries", () => {
  const view = read("crates/nazar-core/src/state.rs");
  // camelCase on the wire, snake_case in Rust. Checking the serde attribute is what makes
  // `remainingMs` in the panel and `remaining_ms` in Rust the same field.
  assert.match(view, /#\[serde\(rename_all = "camelCase"\)\]\s*\npub struct SnapshotView/);
  assert.match(view, /#\[serde\(rename_all = "camelCase"\)\]\s*\npub struct WindowView/);
  assert.match(view, /#\[serde\(rename_all = "camelCase"\)\]\s*\npub struct ProviderView/);
  assert.match(rustState, /#\[serde\(rename_all = "camelCase"\)\]\s*\npub struct UiState/);
});

/** Message keys look like `panel.window.weekly`: a namespace and at least one dot. */
const KEY = /"((?:app|panel|tray|time|window|alert|settings|usage)\.[A-Za-z0-9_.]+)"/g;

test("every message key either side names is in all six locale files", () => {
  // Since WP6 the six catalogues are level, so a key is checked against every one of them
  // rather than against English alone: a key the tray draws and Korean lacks would be an
  // English word in an otherwise Korean tray, which is the half-translated state WP6 rules
  // out. The Rust half is checked here *and* in `i18n.rs`, from opposite directions — this
  // reads the keys out of the sources, that one reads them out of the compiled catalogues.
  const catalogs = Object.fromEntries(
    ["en", "tr", "zh", "ko", "ru", "es"].map((locale) => [
      locale,
      JSON.parse(read(`ui/locales/${locale}.json`)),
    ]),
  );

  const sources = [
    ...readdirSync(resolve(REPO, "ui/src"))
      .filter((name) => name.endsWith(".ts"))
      .map((name) => `ui/src/${name}`),
    "ui/src/index.html",
    "crates/nazar-tray/src/tray.rs",
    "crates/nazar-tray/src/i18n.rs",
    "crates/nazar-tray/src/alerts.rs",
  ];

  const named = new Set();
  for (const source of sources) {
    const text = read(source);
    const keys = source.endsWith(".html")
      ? [...text.matchAll(/data-i18n="([^"]+)"/g)].map((match) => match[1])
      : [...text.matchAll(KEY)].map((match) => match[1]);
    for (const key of keys) {
      assert.ok(key in catalogs.en, `${source} names ${key}, which en.json does not have`);
      named.add(key);
    }
  }

  // Four keys are built at run time from the provider's name, so no literal appears. The
  // five usage errors are built from the kind the Rust side sent, which is the same idea:
  // `usage.error.${kind}` in `ui/src/usage.ts`, with `usage.error.unknown` — which *is* a
  // literal — as the fallback for a kind this build has not met.
  for (const key of [
    "panel.provider.claude",
    "panel.provider.codex",
    "tray.provider.claude",
    "tray.provider.codex",
    "usage.error.bad_range",
    "usage.error.bad_window",
    "usage.error.no_state_dir",
    "usage.error.scan_failed",
    "usage.error.store_unreadable",
  ]) {
    assert.ok(key in catalogs.en, `${key} is built at run time and must exist`);
    named.add(key);
  }

  for (const [locale, catalog] of Object.entries(catalogs)) {
    const missing = [...named].filter((key) => !(key in catalog)).sort();
    assert.deepEqual(missing, [], `${locale}.json does not translate keys the code draws`);
  }
});

test("no English text is typed into the panel's markup", () => {
  const html = read("ui/src/index.html");
  const body = /<main[\s\S]*<\/main>/.exec(html);
  assert.ok(body, "index.html has no panel");
  // Anything between two tags that is not whitespace or a comment would be a string no
  // translation could reach — the failure mode WP6 exists to prevent.
  const text = body[0]
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/<[^>]+>/g, " ")
    .split(/\s+/)
    .map((part) => part.trim())
    .filter(Boolean);
  assert.deepEqual(text, [], `hard-coded text in index.html: ${text.join(" | ")}`);
});
