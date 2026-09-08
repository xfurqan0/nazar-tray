// Three of the bead's colours are shared with Nazar and must not drift. One is this
// repository's own and must not drift *into* Nazar's.
//
// Rim, band and pupil are the brand definition from the unified execution plan, section
// 1.3. Nazar wrote them into a file first only because it ships first; both repos carry a
// verbatim copy and both run this equality test (decision K23 — a copy plus a test, not a
// package, for thirty lines of JSON). The values are hard-coded here rather than read from
// a sibling checkout so the test passes on a CI runner that has only this repo.
//
// The iris is the deliberate exception (docs/PROJECT.md, 2026-09-09). Nazar's is `#3FA9F5`;
// this one is `#F2A93B`, because two beads by the same hand sit in the same Windows tray and
// 16 pixels is not enough room to tell them apart by shape. A test that asserted the iris
// matched Nazar's would now be asserting the bug, so it asserts the difference instead.
//
// `#F2A93B` is still a family colour: it is Nazar's amber, the colour its bar bead turns
// past the warning threshold, and it is `modes.dark.warn` in both theme files below.
//
// The block is four layers and nothing else. A fifth key, `warnFill`, existed for the few
// hours the tray icon filled with the quota; the icon carries no state now, so the key left
// with the gauge. A colour nothing draws is a colour that starts lying, so its absence is
// asserted rather than assumed.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { THEMES, cssVariables } from "../dist/lib/theme.mjs";

const UI = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/** nazar/packages/ui/theme.nazar.json, `bead`: the three hexes the two programs share. */
const SHARED = {
  deepBlue: "#0E2A5A",
  white: "#FFFFFF",
  blackDot: "#0A0A0F",
};

/** Nazar's iris, which must appear nowhere in this repository's bead. */
const NAZAR_IRIS = "#3FA9F5";

/** This repository's own: the yellow iris. */
const OWN = {
  iris: "#F2A93B",
};

/** The whole block, in the order the files write it. */
const BEAD = {
  deepBlue: SHARED.deepBlue,
  white: SHARED.white,
  iris: OWN.iris,
  blackDot: SHARED.blackDot,
};

/** The four colours the still mark is drawn in — which is the whole block. */
const MARK = [SHARED.deepBlue, SHARED.white, OWN.iris, SHARED.blackDot];

/** docs/PROJECT.md section 3: amber at 60 %, red at 85 %, grey for unknown. */
const THRESHOLDS = { amberAtPercent: 60, redAtPercent: 85 };

const read = (name) => JSON.parse(readFileSync(resolve(UI, name), "utf8"));

test("rim, band and pupil are Nazar's hexes exactly", () => {
  for (const name of ["theme.nazar.json", "theme.graphite.json"]) {
    const bead = read(name).bead;
    for (const [key, hex] of Object.entries(SHARED)) {
      assert.equal(bead[key], hex, `${name}: bead.${key} drifted from the shared definition`);
    }
  }
});

test("the iris is this repository's yellow and never Nazar's blue", () => {
  for (const name of ["theme.nazar.json", "theme.graphite.json"]) {
    const bead = read(name).bead;
    assert.equal(bead.iris, OWN.iris, `${name}: the iris is not the tray's yellow`);
    assert.notEqual(bead.iris, NAZAR_IRIS, `${name}: the iris went back to Nazar's blue`);
  }
});

test("the bead block is the four layers and carries no fill colour", () => {
  for (const name of ["theme.nazar.json", "theme.graphite.json"]) {
    const bead = read(name).bead;
    assert.deepEqual(Object.keys(bead), Object.keys(BEAD), `${name}: the bead block grew a key`);
    // `warnFill` was the tray icon's warning colour while the icon filled with the quota.
    // Nothing draws it now, and a colour nothing draws drifts without anyone noticing.
    assert.equal(bead.warnFill, undefined, `${name}: bead.warnFill is back and nothing draws it`);
  }
});

test("graphite drops the navy but keeps the same bead", () => {
  const graphite = read("theme.graphite.json");
  assert.deepEqual(graphite.bead, BEAD);
  assert.deepEqual(read("theme.nazar.json").bead, BEAD);
  assert.notDeepEqual(graphite.modes.dark.canvas, read("theme.nazar.json").modes.dark.canvas);
});

test("both themes agree on the alert thresholds", () => {
  for (const name of ["theme.nazar.json", "theme.graphite.json"]) {
    assert.deepEqual(read(name).thresholds, THRESHOLDS);
  }
});

test("the bead artwork is on the grid and uses the mark's four colours", () => {
  const svg = readFileSync(resolve(UI, "assets/bead.svg"), "utf8");
  assert.match(svg, /viewBox="0 0 16 16"/, "the artwork is authored on the 16x16 grid");
  assert.match(svg, /shape-rendering="crispEdges"/, "the cells must never be antialiased");
  assert.ok(!svg.includes("<circle"), "the round bead is gone; the mark is rects on a grid");

  const used = new Set([...svg.matchAll(/fill="(#[0-9A-Fa-f]{6})"/g)].map((match) => match[1]));
  assert.deepEqual([...used].sort(), [...MARK].sort());
  assert.ok(!used.has(NAZAR_IRIS), "Nazar's iris must not appear in this repository's mark");
});

test("every theme exposes both modes and a full set of bead variables", () => {
  for (const [name, theme] of Object.entries(THEMES)) {
    for (const mode of ["light", "dark"]) {
      const variables = cssVariables(theme, mode);
      assert.equal(variables["--bead-deep-blue"], BEAD.deepBlue, `${name}/${mode}`);
      assert.equal(variables["--bead-white"], BEAD.white, `${name}/${mode}`);
      assert.equal(variables["--bead-iris"], BEAD.iris, `${name}/${mode}`);
      assert.equal(variables["--bead-black-dot"], BEAD.blackDot, `${name}/${mode}`);
      assert.equal(variables["--bead-warn-fill"], undefined, `${name}/${mode} still has a fill`);
      assert.ok(variables["--color-panel"], `${name}/${mode} has no panel colour`);
      assert.ok(variables["--type-font-family"], `${name}/${mode} has no font family`);
    }
  }
});
