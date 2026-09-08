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

/** Apply a theme to an element, usually `<html>`. */
export function applyTheme(root: HTMLElement, theme: Theme, mode: ThemeMode): void {
  for (const [name, value] of Object.entries(cssVariables(theme, mode))) {
    root.style.setProperty(name, value);
  }
  root.dataset["theme"] = theme.name;
  root.dataset["mode"] = mode;
}
