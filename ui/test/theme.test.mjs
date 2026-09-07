// The bead palette is shared with Nazar and must not drift.
//
// The hexes below are the brand definition from the unified execution plan, section 1.3.
// Nazar wrote them into a file first only because it ships first; both repos carry a
// verbatim copy and both run this equality test (decision K23 — a copy plus a test, not
// a package, for thirty lines of JSON). The values are hard-coded here rather than read
// from a sibling checkout so the test passes on a CI runner that has only this repo.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { THEMES, cssVariables } from "../dist/lib/theme.mjs";

const UI = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/** nazar/packages/ui/theme.nazar.json, `bead`. */
const BEAD = {
  deepBlue: "#0E2A5A",
  lightBlue: "#3FA9F5",
  white: "#FFFFFF",
  blackDot: "#0A0A0F",
};

/** docs/PROJECT.md section 3: amber at 60 %, red at 85 %, grey for unknown. */
const THRESHOLDS = { amberAtPercent: 60, redAtPercent: 85 };

const read = (name) => JSON.parse(readFileSync(resolve(UI, name), "utf8"));

test("theme.nazar.json carries Nazar's bead hexes exactly", () => {
  assert.deepEqual(read("theme.nazar.json").bead, BEAD);
});

test("graphite drops the navy but keeps the same bead", () => {
  const graphite = read("theme.graphite.json");
  assert.deepEqual(graphite.bead, BEAD);
  assert.notDeepEqual(graphite.modes.dark.canvas, read("theme.nazar.json").modes.dark.canvas);
});

test("both themes agree on the alert thresholds", () => {
  for (const name of ["theme.nazar.json", "theme.graphite.json"]) {
    assert.deepEqual(read(name).thresholds, THRESHOLDS);
  }
});

test("the bead artwork uses the palette and nothing else", () => {
  const svg = readFileSync(resolve(UI, "assets/bead.svg"), "utf8");
  const used = new Set([...svg.matchAll(/fill="(#[0-9A-Fa-f]{6})"/g)].map((match) => match[1]));
  assert.deepEqual([...used].sort(), Object.values(BEAD).sort());
});

test("every theme exposes both modes and a full set of bead variables", () => {
  for (const [name, theme] of Object.entries(THEMES)) {
    for (const mode of ["light", "dark"]) {
      const variables = cssVariables(theme, mode);
      assert.equal(variables["--bead-deep-blue"], BEAD.deepBlue, `${name}/${mode}`);
      assert.equal(variables["--bead-light-blue"], BEAD.lightBlue, `${name}/${mode}`);
      assert.equal(variables["--bead-white"], BEAD.white, `${name}/${mode}`);
      assert.equal(variables["--bead-black-dot"], BEAD.blackDot, `${name}/${mode}`);
      assert.ok(variables["--color-panel"], `${name}/${mode} has no panel colour`);
      assert.ok(variables["--type-font-family"], `${name}/${mode} has no font family`);
    }
  }
});
