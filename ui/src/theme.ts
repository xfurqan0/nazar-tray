/**
 * Theme tokens, turned into CSS custom properties.
 *
 * `theme.nazar.json` and `theme.graphite.json` are **verbatim copies** of Nazar's files
 * (`nazar/packages/ui/theme.*.json`). The values belong to the brand definition, not to
 * either repo; Nazar happened to write them down first because it ships first. Copies
 * plus an equality test beat a shared package for thirty lines of JSON (decision K23);
 * `test/theme.test.mjs` is that test.
 */

import graphite from "../theme.graphite.json";
import nazar from "../theme.nazar.json";

/** Light or dark surfaces. The bead colours are the same in both. */
export type ThemeMode = "light" | "dark";

/** Shape of a theme file. */
export interface Theme {
  readonly name: string;
  readonly label: string;
  /** The four bead colours. Identical across themes; identical to Nazar's. */
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
