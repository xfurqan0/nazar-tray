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

test("the tray menu offers the same three actions", () => {
  const tray = read("crates/nazar-tray/src/tray.rs");
  for (const key of ["tray.menu.open", "tray.menu.refresh", "tray.menu.quit"]) {
    assert.ok(tray.includes(`"${key}"`), `the tray menu no longer names ${key}`);
  }
  assert.match(tray, /state\.shutdown\(\);/, "the menu's Quit must release the lock");
});

test("the panel listens for the event name the Rust side emits", () => {
  const constant = /pub const SNAPSHOT_CHANGED: &str = "([^"]+)"/.exec(rustState);
  assert.ok(constant, "state.rs no longer names the event");
  assert.match(panel, new RegExp(`listen\\("${constant[1]}"`), "the panel listens for something else");
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
const KEY = /"((?:app|panel|tray|time|window)\.[A-Za-z0-9_.]+)"/g;

test("every message key either side names is in the locale files", () => {
  const english = JSON.parse(read("ui/locales/en.json"));

  const sources = [
    ...readdirSync(resolve(REPO, "ui/src"))
      .filter((name) => name.endsWith(".ts"))
      .map((name) => `ui/src/${name}`),
    "ui/src/index.html",
    "crates/nazar-tray/src/tray.rs",
    "crates/nazar-tray/src/i18n.rs",
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
