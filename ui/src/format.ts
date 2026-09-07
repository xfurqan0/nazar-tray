/**
 * Everything the panel turns a number into a word with, and nothing that touches the DOM.
 *
 * Kept apart from `main.ts` so it can be tested without a browser: `test/format.test.mjs`
 * imports this file and nothing else. Three rules from the contract are enforced here
 * rather than in the markup:
 *
 * * **Unknown is a word.** A window with no percentage gets the word and no bar (finding
 *   B03 of the audit). {@link meterWidth} returns `0` *and* the caller hides the fill.
 * * **Rounding happens at display time, downwards.** 99.6 % is not 100 % (finding B15).
 * * **A window is named by its length, not by its provider.** `windowMinutes` is in the
 *   contract precisely so that a display needs no `if (provider === "codex")`.
 *
 * The units in a duration — `h`, `m`, `s` — come from the locale files, because "h" is not
 * "sa". The colons in a countdown do not: they are punctuation, and a clock reads the same
 * in every language this product ships.
 */

import type { Freshness, Severity, WindowView } from "./snapshot";

/** A bound translator, the shape `i18n.createTranslator` returns. */
export type Translate = (key: string, params?: Record<string, string | number>) => string;

/** Five-hour windows, in minutes, as both providers report them. */
export const FIVE_HOUR = 300;
/** Weekly windows, in minutes. */
export const WEEKLY = 10080;

/**
 * A duration in words: `4 d 2 h`, `2 h 10 m`, `10 m`, `45 s`.
 *
 * Used for ages, and for any countdown longer than a day. The same four keys the Rust side
 * uses, so the tray tooltip and the panel say the same thing in the same words — and days
 * are a unit because `98:59:59` is not a number anybody reads as "four days".
 */
export function formatDuration(milliseconds: number, t: Translate): string {
  const total = Math.max(0, Math.floor(milliseconds / 1000));
  const days = Math.floor(total / 86400);
  const hours = Math.floor((total % 86400) / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  if (days > 0) return t("time.daysHours", { days, hours });
  if (total >= 3600) return t("time.hoursMinutes", { hours, minutes });
  if (minutes > 0) return t("time.minutes", { minutes });
  return t("time.seconds", { seconds: total % 60 });
}

/** Longer than this and a countdown stops being a clock and becomes a duration. */
export const CLOCK_LIMIT = 24 * 3600 * 1000;

/** The message key and parameters for a window's name. */
export interface Label {
  readonly key: string;
  readonly params?: Record<string, string>;
}

/**
 * What a window is called.
 *
 * A model-scoped weekly says which model — that is the whole reason the detailed-windows
 * mode exists, and "Weekly" twice in one card would be a lie. Everything else is named by
 * its length. A length this build does not know falls back to the raw key, which is data
 * rather than a word and needs no translation.
 */
export function windowLabel(window: WindowView): Label {
  if (window.model) return { key: "panel.window.modelWeekly", params: { model: window.model } };
  if (window.windowMinutes === FIVE_HOUR) return { key: "panel.window.fiveHour" };
  if (window.windowMinutes === WEEKLY) return { key: "panel.window.weekly" };
  return { key: window.key };
}

/** Render a {@link Label} through a translator. A raw key comes back as itself. */
export function labelText(label: Label, t: Translate): string {
  return t(label.key, label.params);
}

/**
 * How full the bar is drawn, `0`–`100`.
 *
 * An unknown window is `0`, and the caller must draw an empty track rather than a bar of
 * zero length — a 0 % bar and an unknown window look identical, and one of them is a lie.
 */
export function meterWidth(percent: number | null | undefined): number {
  if (percent == null || !Number.isFinite(percent)) return 0;
  return Math.max(0, Math.min(100, percent));
}

/** The freshness line: `updated 12 s ago`, `stale · last data 2 h ago`, `age unknown`. */
export function ageText(
  freshness: Freshness,
  ageMs: number | null | undefined,
  t: Translate,
): string {
  // `== null` on purpose: the Rust side leaves an empty field out, but a stray `null` from
  // a newer build must read as "unknown" rather than as an age of zero.
  if (freshness === "unknown" || ageMs == null) return t("panel.age.unknown");
  const age = formatDuration(Math.max(0, ageMs), t);
  return freshness === "stale" ? t("panel.age.stale", { age }) : t("panel.age.updated", { age });
}

/** The class the row carries, so the stylesheet colours it without reading data. */
export function severityClass(severity: Severity): string {
  return `severity-${severity}`;
}

/** The class the freshness line carries. */
export function freshnessClass(freshness: Freshness): string {
  return `age-${freshness}`;
}

/**
 * When a reset lands, in the reader's own time zone.
 *
 * Contract timestamps are UTC (`docs/pinned-internal-formats.md`); local time is a
 * rendering decision and this is where it is made. A reset within a day is a time, one
 * further out gets its weekday too — "02:00" four days from now would be a riddle.
 */
export function resetClock(resetsAt: Date, remainingMs: number, locale: string): string {
  const sameDay = remainingMs < 24 * 3600 * 1000;
  const options: Intl.DateTimeFormatOptions = sameDay
    ? { hour: "2-digit", minute: "2-digit" }
    : { weekday: "short", hour: "2-digit", minute: "2-digit" };
  return new Intl.DateTimeFormat(locale, options).format(resetsAt);
}

/** The same instant in full, for the row's `title`. */
export function resetTitle(resetsAt: Date, locale: string): string {
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" }).format(
    resetsAt,
  );
}

/** The next theme the toggle should switch to. Two themes, so it is a toggle. */
export function nextTheme(current: string, available: readonly string[]): string {
  const at = available.indexOf(current);
  return available[(at + 1) % available.length] ?? available[0] ?? current;
}

/**
 * Light or dark, given the setting and what the operating system says.
 *
 * `system` is the default and the only value a fresh machine has: the panel follows
 * `prefers-color-scheme`, which is what the rest of the desktop does.
 */
export function resolveMode(mode: string, prefersDark: boolean): "light" | "dark" {
  if (mode === "light" || mode === "dark") return mode;
  return prefersDark ? "dark" : "light";
}
