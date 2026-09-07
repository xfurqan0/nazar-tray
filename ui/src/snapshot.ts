/**
 * The shape the Rust side sends, and the formatting the panel does to it.
 *
 * Everything here is derived, not stored: `nazar-core::state` works these values out for
 * the instant the panel asked, and asks nothing of the file but the numbers a source
 * actually reported. The types below mirror `SnapshotView` field for field.
 *
 * Two rules from the contract show up as code in this file:
 *
 * * **Rounding happens at display time, and downwards.** A quota of 99.6 % is not 100 %.
 *   Showing "100" for a window that has not run out is finding B15 of the audit, and
 *   `Math.floor` is the whole fix.
 * * **Unknown is not zero.** A window with no `percent` has none here either — the field is
 *   absent, not `0` — and the panel prints the word rather than a number.
 *
 * WP4 designs this. WP3 shows the raw values so that the loop underneath can be seen to
 * work; that is why there is no bar, no colour beyond a severity class, and no layout.
 */

/** How alarming a window is, from its percentage against the configured thresholds. */
export type Severity = "unknown" | "ok" | "warn" | "critical" | "exhausted";

/** How old a provider's reading is. */
export type Freshness = "unknown" | "fresh" | "aging" | "stale";

/** One usage window, derived. */
export interface WindowView {
  readonly key: string;
  readonly percent?: number;
  readonly windowMinutes?: number;
  readonly resetsAt?: string;
  /** Milliseconds until the reset. Negative when the reset is already due. */
  readonly remainingMs?: number;
  readonly state: string;
  readonly error?: string;
  readonly model?: string;
  readonly detailed: boolean;
  readonly binding: boolean;
  readonly severity: Severity;
}

/** One provider, derived. */
export interface ProviderView {
  readonly name: string;
  readonly configured: boolean;
  readonly plan?: string;
  readonly source?: string;
  readonly sourceAt?: string;
  readonly binding?: string;
  /** Milliseconds between `sourceAt` and now. Negative when the source is ahead. */
  readonly ageMs?: number;
  readonly freshness: Freshness;
  readonly severity: Severity;
  readonly windows: readonly WindowView[];
}

/** The whole derived view, as `get_snapshot` returns it. */
export interface SnapshotView {
  readonly updatedAt: string;
  readonly now: string;
  readonly providers: readonly ProviderView[];
}

/**
 * A duration as a clock: `1:04:12`, `4:12`, `0:07`.
 *
 * Numerals and colons only, so it needs no translation and no plural rules. A negative
 * duration counts as zero: "how long is left" is not a negative number, and whether a past
 * reset says "due" or something else is the caller's decision, not the formatter's.
 */
export function formatClock(milliseconds: number): string {
  const total = Math.max(0, Math.round(milliseconds / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const pad = (value: number) => String(value).padStart(2, "0");
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${minutes}:${pad(seconds)}`;
}

/**
 * The percentage as it should be shown: floored, never rounded up.
 *
 * A window at 99.6 % has not run out, and a display that says `100` is telling the user
 * something that is not true at the exact moment it matters most.
 */
export function displayPercent(percent: number): number {
  return Math.floor(percent);
}

/** The message key for a freshness value. */
export function freshnessKey(freshness: Freshness): string {
  return `panel.freshness.${freshness}`;
}

/** The class the row carries, so the stylesheet can colour it without reading data. */
export function severityClass(severity: Severity): string {
  return `severity-${severity}`;
}

/**
 * The provider whose binding window is the most alarming, if any window is known at all.
 *
 * What the tray icon will show once WP4 draws it, and what the panel's headline says now.
 * `unknown` sorts lowest, so a provider nobody could read never decides the answer on its
 * own — but it is also what comes back when nothing at all could be read, which is exactly
 * the grey state the audit's finding B03 asked for.
 */
export function worstSeverity(view: SnapshotView): Severity {
  const order: readonly Severity[] = ["unknown", "ok", "warn", "critical", "exhausted"];
  let worst: Severity = "unknown";
  for (const provider of view.providers) {
    if (order.indexOf(provider.severity) > order.indexOf(worst)) {
      worst = provider.severity;
    }
  }
  return worst;
}
