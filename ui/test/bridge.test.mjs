// The names that have to match across the two languages.
//
// A command the panel invokes and a command the Rust side registers are the same string
// written in two files that no compiler reads together. Neither `tsc` nor `cargo` can catch
// a typo here; it shows up as a panel that draws nothing, at run time, on somebody else's
// machine. So it is a test.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
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

/** Every command the panel invokes. */
const invoked = [...panel.matchAll(/invoke<[^>]*>\("(\w+)"\)/g)].map((m) => m[1]);

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
});
