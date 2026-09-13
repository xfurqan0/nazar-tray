/**
 * Theme tokens, turned into CSS custom properties.
 *
 * `theme.nazar.json` and `theme.graphite.json` started as **verbatim copies** of Nazar's
 * files (`nazar/packages/ui/theme.*.json`). Most of the values still belong to the brand
 * definition rather than to either repo; Nazar happened to write them down first because
 * it ships first. Copies plus an equality test beat a shared package for thirty lines of
 * JSON (decision K23); `test/theme.test.mjs` is that test.
 *
 * One key under `bead` is this repository's own and is asserted to *differ* from Nazar's:
 * `iris` is `#F2A93B` rather than light blue, so the two beads are told apart in one tray.
 * The hex is not new — it is Nazar's amber, the colour its bar bead turns past the warning
 * threshold, and it is sitting in `modes.dark.warn` of these very files.
 *
 * The block is four layers and nothing else. It briefly carried a fifth, `warnFill`, for a
 * tray icon that filled with the quota; the icon carries no state any more, so the key went
 * with the gauge rather than staying behind as a colour nothing draws. `docs/PROJECT.md`,
 * 2026-09-09.
 */

import graphite from "../theme.graphite.json";
import nazar from "../theme.nazar.json";

/** Light or dark surfaces. The bead colours are the same in both. */
export type ThemeMode = "light" | "dark";

/** Shape of a theme file. */
export interface Theme {
  readonly name: string;
  readonly label: string;
  /** The mark's four colours. Identical across themes. */
  readonly bead: Readonly<Record<string, string>>;
  /** Amber and red thresholds, in percent. */
  readonly thresholds: Readonly<Record<string, number>>;
  readonly typography: Readonly<Record<string, string>>;
  readonly modes: Readonly<Record<ThemeMode, Readonly<Record<string, string>>>>;
}

/** Both themes v1 ships. `nazar` is the default; `graphite` drops the navy. */
export const THEMES: Readonly<Record<string, Theme>> = { nazar, graphite };

function kebab(name: string): string {
  return name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
}

/** Flatten a theme into the CSS custom properties `styles.css` reads. */
export function cssVariables(theme: Theme, mode: ThemeMode): Record<string, string> {
  const variables: Record<string, string> = {};
  for (const [name, value] of Object.entries(theme.bead)) {
    variables[`--bead-${kebab(name)}`] = value;
  }
  for (const [name, value] of Object.entries(theme.typography)) {
    variables[`--type-${kebab(name)}`] = value;
  }
  for (const [name, value] of Object.entries(theme.modes[mode])) {
    variables[`--color-${kebab(name)}`] = value;
  }
  return variables;
}

/*
 * ---------------------------------------------------------------------------------------
 * The chart palette, and why it is derived rather than written down.
 *
 * T-WP21's *Models* tab draws one line per model, and a line chart needs colours told apart
 * by **hue** — the heat-map's four opacities of one accent say *more* and *less*, which is
 * the wrong axis for *which model*. Neither theme file holds six hues: `nazar` is a blue, an
 * amber, a red and three greys, and `graphite` is the same shape with the blue muted.
 * Writing six hexes into both files would be **a second palette in this product**, in two
 * places, that no theme change reaches — which is exactly what `styles.css` says the heat-map
 * exists in order not to invent.
 *
 * So the palette is computed from the token a chart line is most like: `accentText`, the
 * derived accent tone the meter fill and the heat-map already use, picked in the first place
 * because it survives both grounds. Its hue is rotated in six even steps, its saturation is
 * kept (with a floor, so a muted theme gives colour rather than six greys), and its lightness
 * moves — down on a light panel, up on a dark one — only as far as it must for
 * {@link SERIES_CONTRAST} against `panel`.
 *
 * Two properties fall out of that and `test/contrast.test.mjs` asserts both, in both themes
 * and both modes: every line clears the 3:1 a meaningful graphic owes, and no two lines are
 * within 40° of each other. A third is why this is a function rather than a constant — the
 * first colour **is** the theme's accent, so the busiest model is drawn in the colour the
 * rest of the panel is already speaking in.
 *
 * Colour is still never the only channel. The legend names every model beside its swatch, and
 * the list under the chart repeats every number the lines are drawn from.
 * ---------------------------------------------------------------------------------------
 */

/**
 * Where the six hues sit, in degrees from the accent's own.
 *
 * **Not six even steps, and the reason was seen rather than argued.** Sixty degrees apart off
 * a blue accent puts two of them in the green band — a yellow-green at 85° and a spring green
 * at 145° — and at the size of a 10 px legend swatch those read as the same colour. The eye
 * tells hues apart far better between red and yellow than it does between yellow and green,
 * so the steps are tighter through the blues and magentas and wider through the greens, which
 * gives one of each family: blue, violet, magenta, red, amber, green. The closest pair is 50°.
 */
const SERIES_OFFSETS: readonly number[] = [0, 50, 100, 150, 205, 265];

/** How many hues the chart palette holds. Six is what 330 px of legend can name. */
export const SERIES_HUES = SERIES_OFFSETS.length;

/** The smallest contrast a chart line may have against the panel it is drawn on. */
export const SERIES_CONTRAST = 3;

/** How saturated a line is at its least, so a muted theme still gives hues. */
const SERIES_SATURATION = 0.5;

/** The three channels of a `#RRGGBB`, `0`–`1`. */
function channels(hex: string): [number, number, number] {
  const [red, green, blue] = [1, 3, 5].map((at) => parseInt(hex.slice(at, at + 2), 16) / 255);
  return [red ?? 0, green ?? 0, blue ?? 0];
}

/** Relative luminance, WCAG 2.1 definition — the arithmetic `contrast.test.mjs` also uses. */
function luminance(hex: string): number {
  const [red, green, blue] = channels(hex).map((value) =>
    value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4,
  ) as [number, number, number];
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

/** Contrast ratio between two `#RRGGBB` colours, 1:1 to 21:1. */
function contrast(one: string, other: string): number {
  const [high, low] = [luminance(one), luminance(other)].sort((left, right) => right - left) as [
    number,
    number,
  ];
  return (high + 0.05) / (low + 0.05);
}

/** A `#RRGGBB` as a hue in degrees, plus saturation and lightness from `0` to `1`. */
function toHsl(hex: string): [number, number, number] {
  const [red, green, blue] = channels(hex);
  const high = Math.max(red, green, blue);
  const low = Math.min(red, green, blue);
  const light = (high + low) / 2;
  const span = high - low;
  if (span === 0) return [0, 0, light];
  const saturation = span / (1 - Math.abs(2 * light - 1));
  const sixth =
    high === red
      ? ((green - blue) / span) % 6
      : high === green
        ? (blue - red) / span + 2
        : (red - green) / span + 4;
  return [(((sixth * 60) % 360) + 360) % 360, saturation, light];
}

/** Hue, saturation and lightness back to a `#RRGGBB`. */
function toHex(hue: number, saturation: number, light: number): string {
  const chroma = (1 - Math.abs(2 * light - 1)) * saturation;
  const second = chroma * (1 - Math.abs(((hue / 60) % 2) - 1));
  const base = light - chroma / 2;
  const [red, green, blue] =
    hue < 60
      ? [chroma, second, 0]
      : hue < 120
        ? [second, chroma, 0]
        : hue < 180
          ? [0, chroma, second]
          : hue < 240
            ? [0, second, chroma]
            : hue < 300
              ? [second, 0, chroma]
              : [chroma, 0, second];
  const pair = (value: number): string =>
    Math.round((value + base) * 255)
      .toString(16)
      .padStart(2, "0")
      .toUpperCase();
  return `#${pair(red ?? 0)}${pair(green ?? 0)}${pair(blue ?? 0)}`;
}

/**
 * Six line colours for one theme and one mode, the first of them the theme's own accent.
 *
 * Deterministic: the same theme and mode always give the same six hexes in the same order, so
 * a model keeps its colour across a redraw, a language change and a restart. The order is the
 * chart's, and the chart sorts its models by total, so the busiest line is the accent.
 */
export function seriesPalette(theme: Theme, mode: ThemeMode): string[] {
  const colours = theme.modes[mode];
  const ground = colours["panel"] ?? "#FFFFFF";
  const accent = colours["accentText"] ?? "#3FA9F5";
  const [hue, saturation, light] = toHsl(accent);
  const strength = Math.min(1, Math.max(SERIES_SATURATION, saturation));
  // A dark panel wants a lighter line and a light panel a darker one. Either way the step is
  // away from the ground, so the loop always ends on a colour that clears the ratio.
  const step = luminance(ground) < 0.5 ? 0.02 : -0.02;

  const palette: string[] = [];
  for (const offset of SERIES_OFFSETS) {
    const own = (hue + offset) % 360;
    let level = light;
    let hex = toHex(own, strength, level);
    for (let tries = 0; tries < 60 && contrast(hex, ground) < SERIES_CONTRAST; tries++) {
      level = Math.min(0.97, Math.max(0.03, level + step));
      hex = toHex(own, strength, level);
    }
    palette.push(hex);
  }
  return palette;
}

/** Apply a theme to an element, usually `<html>`. */
export function applyTheme(root: HTMLElement, theme: Theme, mode: ThemeMode): void {
  for (const [name, value] of Object.entries(cssVariables(theme, mode))) {
    root.style.setProperty(name, value);
  }
  root.dataset["theme"] = theme.name;
  root.dataset["mode"] = mode;
}
