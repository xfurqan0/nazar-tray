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

/** Every `#[tauri::command]` the Rust side defines. */
const defined = [...rustState.matchAll(/#\[tauri::command\]\s*\npub fn (\w+)/g)].map((m) => m[1]);

/** Every command named in `generate_handler!`. */
const registered = (() => {
  const block = /generate_handler!\[([\s\S]*?)\]/.exec(rustMain);
  assert.ok(block, "main.rs has no generate_handler! block");
  return block[1]
    .split(",")
    .map((name) => name.trim().replace(/^state::/, ""))
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
  const main = read("crates/nazar-tray/src/main.rs");
  assert.match(main, /Event::Refreshed => alerts::on_refresh\(app\)/);
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
const KEY = /"((?:app|panel|tray|time|window|alert|settings)\.[A-Za-z0-9_.]+)"/g;

test("every message key either side names is in the locale files", () => {
  const english = JSON.parse(read("ui/locales/en.json"));

  const sources = [
    ...readdirSync(resolve(REPO, "ui/src"))
      .filter((name) => name.endsWith(".ts"))
      .map((name) => `ui/src/${name}`),
    "ui/src/index.html",
    "crates/nazar-tray/src/tray.rs",
    "crates/nazar-tray/src/i18n.rs",
    "crates/nazar-tray/src/alerts.rs",
  ];

  for (const source of sources) {
    const text = read(source);
    const keys = source.endsWith(".html")
      ? [...text.matchAll(/data-i18n="([^"]+)"/g)].map((match) => match[1])
      : [...text.matchAll(KEY)].map((match) => match[1]);
    for (const key of keys) {
      assert.ok(key in english, `${source} names ${key}, which en.json does not have`);
    }
  }

  // Four keys are built at run time from the provider's name, so no literal appears.
  for (const key of [
    "panel.provider.claude",
    "panel.provider.codex",
    "tray.provider.claude",
    "tray.provider.codex",
  ]) {
    assert.ok(key in english, `${key} is built at run time and must exist`);
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
