// Every colour the panel puts text in, against the colour it puts it on.
//
// WCAG 2.1 AA is 4.5:1 for text of this size — the panel's smallest type is 10 px, which
// is nowhere near the 18 pt that would let it drop to 3:1 — and 3:1 for a graphic that
// carries meaning. The pairs below are read off `src/styles.css`: if a rule there starts
// using a different token, this file has to say so too, which is the point of it.
//
// Both themes and both modes, because "it looks fine on my machine" is exactly the failure
// this catches: the panel follows `prefers-color-scheme`, so half of these combinations
// are ones the maintainer never sees.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { SERIES_CONTRAST, SERIES_HUES, seriesPalette } from "../dist/lib/theme.mjs";

const UI = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const THEMES = ["theme.nazar.json", "theme.graphite.json"].map((name) =>
  JSON.parse(readFileSync(resolve(UI, name), "utf8")),
);

/** Relative luminance, WCAG 2.1 definition. */
function luminance(hex) {
  const channels = [1, 3, 5]
    .map((at) => parseInt(hex.slice(at, at + 2), 16) / 255)
    .map((value) => (value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4));
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

/** Contrast ratio between two `#RRGGBB` colours, 1:1 to 21:1. */
function contrast(foreground, background) {
  const [a, b] = [luminance(foreground), luminance(background)].sort((x, y) => y - x);
  return (a + 0.05) / (b + 0.05);
}

/** Where a `#RRGGBB` sits on the colour wheel, in degrees. */
function hue(hex) {
  const [r, g, b] = [1, 3, 5].map((at) => parseInt(hex.slice(at, at + 2), 16) / 255);
  const high = Math.max(r, g, b);
  const low = Math.min(r, g, b);
  const span = high - low;
  if (span === 0) return 0;
  const sixth =
    high === r ? ((g - b) / span) % 6 : high === g ? (b - r) / span + 2 : (r - g) / span + 4;
  return (((sixth * 60) % 360) + 360) % 360;
}

/** The shorter way round the wheel between two hues, 0°–180°. */
function apart(one, other) {
  const gap = Math.abs(one - other) % 360;
  return Math.min(gap, 360 - gap);
}

/** Foreground, background, and what the panel uses the pair for. */
const TEXT = [
  ["text", "panel", "the title in the header"],
  ["text", "node", "a provider's name and the binding window's numbers"],
  ["textMuted", "node", "a window's label"],
  ["textMuted", "panel", "the footer buttons and the empty-state line"],
  ["textFaint", "node", "the freshness line and the countdown"],
  ["textFaint", "panel", "the version and the Esc hint"],
  ["warnText", "node", "a window over the amber threshold"],
  ["warnText", "panel", "the demo badge"],
  ["dangerText", "node", "a window over the red threshold"],
  ["unknownGreyText", "node", "a window nobody could read, and its error line"],
  ["bannerText", "bannerBg", "the first-run overflow hint, and a refused settings form"],
  ["text", "nodeHeader", "what you type into a settings control"],
  ["accentText", "node", "the Saved confirmation on the settings page"],
  ["text", "nodeHeader", "the label on the settings page's Save button, hovered"],
];

/** The meter is decorative — the percentage is text beside it — but it is the thing the
 *  eye goes to first, so it is held to the 3:1 a meaningful graphic would need. */
const GRAPHICS = [
  ["accentText", "nodeHeader", "the bar of a window inside its quota"],
  ["warn", "nodeHeader", "the bar of a window over the amber threshold"],
  ["danger", "nodeHeader", "the bar of a window over the red threshold"],
  ["unknownGrey", "nodeHeader", "the hatched track of an unknown window"],
  ["focusRing", "panel", "the keyboard focus ring"],
  ["accentText", "panel", "the outline of the settings page's Save button"],
];

for (const theme of THEMES) {
  for (const mode of ["light", "dark"]) {
    const colours = theme.modes[mode];

    test(`${theme.name}/${mode}: every piece of text clears WCAG AA`, () => {
      for (const [foreground, background, what] of TEXT) {
        const ratio = contrast(colours[foreground], colours[background]);
        assert.ok(
          ratio >= 4.5,
          `${foreground} on ${background} (${what}) is ${ratio.toFixed(2)}:1, under 4.5:1`,
        );
      }
    });

    test(`${theme.name}/${mode}: every graphic that carries meaning clears 3:1`, () => {
      for (const [foreground, background, what] of GRAPHICS) {
        const ratio = contrast(colours[foreground], colours[background]);
        assert.ok(
          ratio >= 3,
          `${foreground} on ${background} (${what}) is ${ratio.toFixed(2)}:1, under 3:1`,
        );
      }
    });

    // The usage chart's lines. Not in GRAPHICS above because they are not tokens: they are
    // derived from `accentText` at run time by `seriesPalette`, precisely so that neither
    // theme file has to carry a second palette nobody would remember to keep legible. A line
    // carries meaning — it says *which model* — so it owes the same 3:1 a meter bar owes, and
    // it owes it on a ground half of which the maintainer never sees.
    test(`${theme.name}/${mode}: every chart line clears 3:1 on the panel`, () => {
      const palette = seriesPalette(theme, mode);
      assert.equal(palette.length, SERIES_HUES);
      palette.forEach((line, index) => {
        const ratio = contrast(line, colours.panel);
        assert.ok(
          ratio >= SERIES_CONTRAST,
          `line ${index} (${line}) on panel is ${ratio.toFixed(2)}:1, under ${SERIES_CONTRAST}:1`,
        );
      });
    });

    // And they are told apart by hue rather than by shade, which is the whole reason the
    // heat-map's four opacities of one accent could not be reused here.
    test(`${theme.name}/${mode}: no two chart lines are the same colour`, () => {
      const hues = seriesPalette(theme, mode).map(hue);
      for (let one = 0; one < hues.length; one++) {
        for (let other = one + 1; other < hues.length; other++) {
          const gap = apart(hues[one], hues[other]);
          assert.ok(gap >= 40, `lines ${one} and ${other} are ${gap.toFixed(0)}° apart, under 40°`);
        }
      }
    });

    test(`${theme.name}/${mode}: the palette is stable, and it opens on the theme's accent`, () => {
      // A model keeps its colour across a redraw, a language change and a restart, because
      // this is arithmetic on the theme rather than a list somebody shuffled.
      assert.deepEqual(seriesPalette(theme, mode), seriesPalette(theme, mode));
      assert.equal(
        hue(seriesPalette(theme, mode)[0]).toFixed(0),
        hue(colours.accentText).toFixed(0),
        "the busiest model is drawn in the colour the rest of the panel speaks in",
      );
    });
  }
}

test("the stylesheet uses the tokens this file measured, not other ones", () => {
  const css = readFileSync(resolve(UI, "src/styles.css"), "utf8");
  // The bar is the pair that had to change for contrast: the raw accent is 2.25:1 on a
  // light track. If someone puts it back, the measurement above stops describing the panel.
  assert.match(css, /\.meter-fill \{[^}]*--color-accent-text/s);
  assert.ok(
    !/\.meter-fill \{[^}]*var\(--color-accent,/s.test(css),
    "the meter fill must use the derived accent tone, not the raw one",
  );

  // The settings page's own pairs, so that a rule quietly changing background is a failing
  // test rather than a page nobody can read in light mode.
  assert.match(css, /\.control \{[^}]*--color-node-header/s, "a control sits on nodeHeader");
  assert.match(css, /\.control \{[^}]*--color-text,/s, "and what you type into it is `text`");
  // The raw accent as a *fill* is 2.56:1 under the panel's own text colour in light mode.
  // Save is an outline in the derived tone instead, which is the same trade the meter made.
  assert.match(
    css,
    /\.button\.primary \{[^}]*--color-accent-text/s,
    "Save is outlined in the derived accent tone",
  );
  assert.ok(
    !/\.button\.primary \{[^}]*background:\s*var\(--color-accent,/s.test(css),
    "a filled Save button would put white on a mid blue in light mode",
  );
  assert.ok(
    !/\.note-error \{[^}]*color:\s*var\(--color-danger-text/s.test(css),
    "a refused form says what is wrong in words; the red is the border, and `dangerText` " +
      "on the banner background is not a pair this file has measured",
  );
});
