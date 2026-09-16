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
 * **The headline number is all four: `input + output + cache_read + cache_create`** — the sum
 * Claude Code's own `/usage` prints as *total tokens*, drawn by the panel since T-WP20 and by
 * the tray tooltip since T-WP20b, so this product answers that question with one definition.
 * This block said `input + output + cache_create` until T-WP21, which is what T-WP20b wrote
 * down as owed: the panel's arithmetic had moved to `usage.ts` and this comment had not.
 *
 * What the old wording was protecting is still true and still shown — cache reads were 98.5 %
 * of the raw total over six days of real work — but it is the argument for the **four-way
 * breakdown** under the headline and under every model row, not for a smaller headline. The
 * store has never held a headline anyway: it holds these four counters per model per hour,
 * and which sum is drawn large is the view's decision rather than this document's.
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

/**
 * What one scan did, for a diagnostic line.
 *
 * The counters are **both readers added together** — a pass runs Claude's and Codex's under
 * one throttle — which is why `providers_scanned` is here: 412 files with no Codex on the
 * machine and 412 files with a Codex nobody could resolve are the same number, and only one
 * of them is complete.
 */
export interface UsageScan {
  readonly files_seen: number;
  readonly lines: number;
  readonly duplicates: number;
  readonly took_ms: number;
  /** Lines the server answered with an error and billed for none of. */
  readonly skipped_api_errors?: number;
  /**
   * Rollout logs Codex had compressed to `.jsonl.zst`, and this pass decoded.
   *
   * `0` on every machine until Codex turns its compression flag on. What it counts is
   * history that is in the numbers beside it, read out of a file nobody can `grep`.
   */
  readonly files_compressed?: number;
  /** The readers that ran, in the order they ran: `claude`, then `codex`. */
  readonly providers_scanned?: readonly string[];
  /** Days copied in from Claude Code's statistics cache, or absent when that setting is off. */
  readonly reported_days?: number;
}

/**
 * Which of the two counts an answer holds.
 *
 * `deduped` is the default and is what this machine actually spent: one message counted once,
 * however many transcript lines Claude Code wrote it on. `per_line` is the sum `/usage` shows,
 * which counts a message once per content block — about 1.7× the real spend, and the reason
 * the two windows disagree (`anthropics/claude-code#91775`). The store holds both; the
 * *Count like Claude Code* setting decides which one arrives, and this says which one did.
 */
export type UsageMode = "deduped" | "per_line";

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
  /** Which count the buckets hold. Absent from an answer written before T-WP22. */
  readonly mode?: UsageMode;
  /**
   * The days older than the transcripts, as Claude Code reported them: date → model → total.
   *
   * Keyed by **date**, not by UTC hour, because that is what they are — a day another program
   * added up, with no hour inside it and no split into the four counters. Empty unless the
   * *Fill history from Claude Code's stats* setting is on. They are drawn apart from the
   * measured days and never added into a total this product promises to have measured.
   */
  readonly reported?: Readonly<Record<string, Readonly<Record<string, number>>>>;
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
