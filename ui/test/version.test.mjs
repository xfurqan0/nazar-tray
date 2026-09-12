// One version number, six files.
//
// The panel prints its version, the installer stamps it, the updater compares it, and the
// three winget manifests have to name the same one or the submission is caught by a reviewer
// rather than by us. Letting them drift is the kind of bug that only shows up in a release,
// so it is a test rather than a release-checklist line.

import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

const read = (relative) => readFileSync(resolve(REPO, relative), "utf8");
const json = (relative) => JSON.parse(read(relative));

test("the workspace, the app, the panel and the winget manifests agree on the version", () => {
  const ui = json("ui/package.json").version;

  const workspace = /\[workspace\.package\][\s\S]*?version = "([^"]+)"/.exec(read("Cargo.toml"));
  assert.ok(workspace, "Cargo.toml has no [workspace.package] version");

  assert.equal(workspace[1], ui, "Cargo.toml and ui/package.json disagree");
  assert.equal(
    json("crates/nazar-tray/tauri.conf.json").version,
    ui,
    "tauri.conf.json and ui/package.json disagree",
  );

  // Read the directory rather than name the three files: a manifest added later and left on
  // the old version would otherwise be the one file this test does not look at.
  const manifests = readdirSync(resolve(REPO, "packaging/winget"))
    .filter((name) => name.endsWith(".yaml"))
    .sort();
  assert.equal(manifests.length, 3, `expected three winget manifests, found ${manifests.join(", ") || "none"}`);

  for (const name of manifests) {
    const declared = /^PackageVersion:[ \t]*(\S+)[ \t]*$/m.exec(read(`packaging/winget/${name}`));
    assert.ok(declared, `packaging/winget/${name} has no PackageVersion`);
    assert.equal(declared[1], ui, `packaging/winget/${name} and ui/package.json disagree`);
  }
});

test("the bundle identifier is the reverse-DNS form of the maintainer's GitHub namespace", () => {
  assert.equal(json("crates/nazar-tray/tauri.conf.json").identifier, "io.github.xfurqan0.nazar-tray");
});

test("the panel window matches the label the Rust side looks up", () => {
  const windows = json("crates/nazar-tray/tauri.conf.json").app.windows;
  assert.equal(windows.length, 1);
  assert.equal(windows[0].label, "panel");
  assert.equal(windows[0].visible, false, "the panel opens from the tray, not on start-up");
  assert.equal(windows[0].skipTaskbar, true);
  assert.equal(windows[0].alwaysOnTop, true);
  assert.equal(windows[0].decorations, false);
});
