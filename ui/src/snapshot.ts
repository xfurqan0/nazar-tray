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
 * This file is the wire shape and the two rules above. Everything else the panel says out
 * loud — a window's name, a freshness line, when a reset lands in local time — is in
 * `format.ts`, which is where the words are.
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

/*
 * ---------------------------------------------------------------------------------------
 * The usage store, as `get_usage` answers it.
 *
 * **These keys are snake_case, and every other name on this bridge is camelCase.** That is a
 * deliberate exception, taken once, and `docs/usage-contract.md` argues it: `limits.json`
 * renames what it reads because it is a contract with another program and has to give one
 * spelling to an idea two sources spell differently, while the usage document keeps the shape
 * `message.usage` already has at the source — so a reader comparing a bucket with a raw
 * transcript line does not have to hold a rename in their head. Renaming it here would mean
 * this product spelled the same five counters two ways in two files, which is the thing that
 * contract was written to avoid. So the panel reads the document's own names.
 *
 * Two rules from the same page show up as types below:
 *
 * * **An absent counter is absent, not zero.** A bucket built from lines that never named an
 *   input count says so by having no `input` key, and the panel prints nothing rather than a
 *   confident `0`. The same rule as `percent` above, on a different document.
 * * **The grain is a UTC hour, and the local week is the panel's to compute.** Rust may not
 *   ask this machine which zone it is in — a workspace-wide test fails the build on it — so
 *   the panel works out the Monday-start week itself, sends `from` and `to` as instants, and
 *   gets hours back to re-bucket. An offset of `+05:30` puts a local day boundary inside a
 *   bucket; that bucket counts towards the local day it *starts* in, because splitting it by
 *   a ratio would invent numbers.
 *
 * No drawing here: T-WP16 is the view.
 * ---------------------------------------------------------------------------------------
 */

/** The three ranges `get_usage` knows. A fourth is an error, not an empty week. */
export type UsageRange = "week" | "month" | "all";

/**
 * What the panel asks for.
 *
 * `from` and `to` are RFC 3339 instants and the window is half-open: an hour is in the answer
 * when **its start** falls inside `[from, to)`. `force` skips the five-minute scan throttle,
 * and belongs to the Refresh button alone.
 */
export interface UsageRequest {
  readonly range: UsageRange;
  readonly from: string;
  readonly to: string;
  readonly force?: boolean;
}

/**
 * One model's totals for one UTC hour.
 *
 * The headline number is `input + output + cache_create`. `cache_read` goes beside it and is
 * never folded into it: measured over six days of real work it was 98.5 % of the raw total,
 * so a chart of raw totals is a chart of cache behaviour with the work lost in the rounding.
 *
 * A future version of the store may add counters. Unknown keys survive a rewrite there and
 * are simply not named here.
 */
export interface UsageBucket {
  readonly input?: number;
  readonly output?: number;
  readonly cache_create?: number;
  readonly cache_read?: number;
  /** Distinct messages that carried usage, after dedupe. Never absent. */
  readonly requests: number;
}

/** What one scan of the transcripts did, for a diagnostic line. */
export interface UsageScan {
  readonly files_seen: number;
  readonly lines: number;
  readonly duplicates: number;
  readonly took_ms: number;
}

/** The answer: provider → UTC hour `YYYY-MM-DDTHH` → model → counters. */
export interface UsageResponse {
  readonly range: UsageRange;
  readonly from: string;
  readonly to: string;
  /** The earliest instant the store holds anything for. The *since {date}* line. */
  readonly since?: string;
  /** When a scan last wrote to the store. */
  readonly scanned_at?: string;
  readonly providers: Readonly<
    Record<string, Readonly<Record<string, Readonly<Record<string, UsageBucket>>>>>
  >;
  /**
   * `YYYY-MM` months whose documents no longer parse, and so are missing from the answer.
   *
   * Not an error: the months beside a damaged one still load. It is reported and left alone
   * rather than repaired, because a lost month may not exist anywhere else.
   */
  readonly damaged: readonly string[];
  /**
   * What the scan this call ran did, or absent when it ran none.
   *
   * Absent is not a failure: the throttle said not yet, another instance holds the advisory
   * lock, or the transcripts could not be located. The store answers from what it holds
   * either way.
   */
  readonly scan?: UsageScan;
}

/** Why there is no answer. `kind` is a message key; `detail` is the diagnostic line. */
export interface UsageError {
  readonly kind: "bad_range" | "bad_window" | "no_state_dir" | "scan_failed" | "store_unreadable";
  readonly detail: string;
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

/**
 * The provider whose binding window is the most alarming, if any window is known at all.
 *
 * This used to be the severity the tray icon took, mirrored here so the panel and the icon
 * could not disagree about the worst thing on the machine. Since 2026-09-09 the icon takes
 * no severity at all — it is the mark, or grey when nothing was read — so what is left is
 * one place that answers "how bad is this machine" for the panel to draw on.
 *
 * `unknown` sorts lowest, so a provider nobody could read never decides the answer on its
 * own — but it is also what comes back when nothing at all could be read, which is still
 * the grey state the audit's finding B03 asked for, and still what the icon shows then.
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
