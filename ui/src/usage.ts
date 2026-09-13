/**
 * The usage view's arithmetic: the window the panel asks for, the local re-bucketing of the
 * answer, and the numbers the rows are drawn from. No DOM here — `main.ts` draws what this
 * file works out, and `test/usage.test.mjs` imports it without a browser, the same split
 * `format.ts` and `settings.ts` already keep.
 *
 * Four rules from `docs/usage-contract.md` are code in this file rather than notes in a
 * comment somewhere else:
 *
 * * **The local week is the panel's to compute.** `crates/nazar-core/tests/hygiene.rs` fails
 *   the build on a time-zone conversion anywhere in the workspace, so Rust stores UTC hours
 *   and knows nothing about Monday. This file works out Monday 00:00 in the reader's own
 *   zone, sends the two instants with their offset, and cuts the hours it gets back into
 *   local days and local weeks.
 * * **A bucket lands where its hour starts.** An offset that is not a whole number of hours —
 *   `+05:30`, `+05:45` — puts a local midnight inside a bucket. That bucket counts towards
 *   the day it *starts* in, because splitting it by a ratio would invent numbers the store
 *   never measured.
 * * **The headline is `input + output + cache_create`, and `cache_read` is never inside it.**
 *   Measured over six days of real work, cache reads were 98.5 % of the raw total: a number
 *   that folded them in would be a number about cache behaviour with the work lost in the
 *   rounding. It goes on its own line, and it is still shown, because 1.5 B is a fact about
 *   the machine.
 * * **An absent counter is absent, not zero.** A bucket built from lines that never named an
 *   input count has no `input` key, and the panel prints {@link ABSENT} rather than a
 *   confident `0`. The same rule `snapshot.ts` keeps for a window with no percentage.
 *
 * Model ids are never merged and never translated. `claude-opus-5` and a bare `opus` are the
 * same model and appear as two rows, because merging them means a table of aliases that has
 * to be right about names nobody here controls, and a wrong merge cannot be undone once it
 * has been drawn.
 */

import type { UsageBucket, UsageError, UsageRange, UsageResponse } from "./snapshot";

/**
 * What is printed where a number is not known.
 *
 * An em dash rather than a `0`, and punctuation rather than a message key: it reads the same
 * in all six languages, exactly like the colons in a countdown.
 */
export const ABSENT = "—";

/**
 * The `from` of the *all* range: early enough to hold every store this build can meet.
 *
 * The panel cannot know where a store begins before it has asked, and asking with a floor is
 * cheaper than asking twice — the query walks the months that exist on disk, not the months
 * between these two instants. The honest boundary of the words *all time* is the `since` the
 * answer carries, which is what the footer line shows.
 */
export const ALL_TIME_FLOOR = "2020-01-01T00:00:00Z";

/** How many columns a strip may hold. A store older than this draws its most recent ones. */
const MAX_BARS = 400;

/** The smallest height a column with any work in it may be drawn at, in percent. */
const MIN_VISIBLE_BAR = 3;

/** The three ranges, in the order the tabs sit in. */
export const USAGE_RANGES: readonly UsageRange[] = ["week", "month", "all"];

/** The error kinds `get_usage` answers with, each of which has a message key. */
export const USAGE_ERROR_KINDS: readonly string[] = [
  "bad_range",
  "bad_window",
  "no_state_dir",
  "scan_failed",
  "store_unreadable",
];

/**
 * One model's totals over the whole window.
 *
 * `total` and `cacheRead` are `number | undefined` rather than optional properties: they are
 * computed here rather than parsed off a wire, and a caller asking `row.total === undefined`
 * should not have to wonder whether the key was left out or set.
 */
export interface UsageRow {
  /** `claude` or `codex`, as the store spells it. */
  readonly provider: string;
  /** The model id exactly as the source reported it. Never merged, never translated. */
  readonly model: string;
  /** `input + output + cache_create`, or `undefined` when the window named none of them. */
  readonly total: number | undefined;
  readonly cacheRead: number | undefined;
  /** Deduplicated records that carried usage. Not a count of prompts. */
  readonly requests: number;
  /** How wide the row's bar is drawn, `0`–`100`, against the largest row. */
  readonly share: number;
}

/** The rows of one provider, and what they add up to. */
export interface UsageGroup {
  readonly provider: string;
  readonly total: number | undefined;
  readonly rows: readonly UsageRow[];
}

/** One column of the strip: a local day for *week* and *month*, a local week for *all*. */
export interface UsageBar {
  /** The local day the column stands for, `YYYY-MM-DD`. A week is named by its Monday. */
  readonly key: string;
  /** The instant that day or week starts at locally, for an `Intl` date. */
  readonly at: number;
  /** Known tokens in the column. Zero means nothing was recorded, which is a fact. */
  readonly total: number;
  /** Height against the tallest column, `0`–`100`. */
  readonly height: number;
}

/** Everything the view draws, worked out from one answer and one instant. */
export interface UsageView {
  readonly range: UsageRange;
  readonly total: number | undefined;
  readonly cacheRead: number | undefined;
  readonly requests: number;
  /** Every row, biggest first, whatever provider it belongs to. */
  readonly rows: readonly UsageRow[];
  /** The same rows under their providers, biggest group first. */
  readonly groups: readonly UsageGroup[];
  /** Whether more than one provider is in the answer, which is when headings earn a line. */
  readonly grouped: boolean;
  readonly bars: readonly UsageBar[];
  readonly since: string | undefined;
  readonly scannedAt: string | undefined;
  readonly damaged: readonly string[];
  /** Nothing was recorded in this window. Not an error, and not a row of zeroes. */
  readonly empty: boolean;
}

// ------------------------------------------------------------------ local time

/** Two digits, for a date written by hand. */
function pad(value: number): string {
  return String(value).padStart(2, "0");
}

/** The local calendar date of an instant, `YYYY-MM-DD`. */
export function localDate(at: Date): string {
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
}

/**
 * An instant as RFC 3339 with this machine's offset, which is what `get_usage` is sent.
 *
 * Local rather than UTC on purpose: the two instants are a *local* Monday and a *local*
 * now, and writing them with the offset they were computed in is what makes them readable
 * in an error message. The Rust side parses the offset and echoes the window back in UTC.
 */
export function rfc3339(at: Date): string {
  const offset = -at.getTimezoneOffset();
  const sign = offset < 0 ? "-" : "+";
  const size = Math.abs(offset);
  const clock = `${pad(at.getHours())}:${pad(at.getMinutes())}:${pad(at.getSeconds())}`;
  const zone = `${sign}${pad(Math.floor(size / 60))}:${pad(size % 60)}`;
  return `${localDate(at)}T${clock}${zone}`;
}

/**
 * Monday 00:00 in the reader's own zone, for the week an instant falls in.
 *
 * `getDay()` counts from Sunday, so Monday is `(day + 6) % 7` days back. The date is built
 * through the local constructor rather than by subtracting milliseconds, which is what makes
 * it right across a daylight-saving change — a week that is 23 or 25 hours longer than seven
 * days still starts on its Monday — and across a month or a year boundary, where a negative
 * day number normalises backwards into December.
 *
 * On the two mornings a year when a zone skips midnight, the local constructor answers with
 * the first instant that day actually has. That is the honest start of that day; there is no
 * 00:00 to name.
 */
export function startOfLocalWeek(at: Date): Date {
  return new Date(at.getFullYear(), at.getMonth(), at.getDate() - ((at.getDay() + 6) % 7));
}

/** The first of the month, 00:00, in the reader's own zone. */
export function startOfLocalMonth(at: Date): Date {
  return new Date(at.getFullYear(), at.getMonth(), 1);
}

/** Midnight of the local day an instant falls in. */
export function startOfLocalDay(at: Date): Date {
  return new Date(at.getFullYear(), at.getMonth(), at.getDate());
}

/**
 * The window for a range, in local time, as `get_usage` wants it.
 *
 * *week* is this Monday to now and not last week as well; *month* is the first of this month
 * to now; *all* is {@link ALL_TIME_FLOOR} to now, with the store's own `since` reported back
 * for the footer.
 */
export function usageWindow(range: UsageRange, now: Date): { from: string; to: string } {
  const to = rfc3339(now);
  if (range === "all") return { from: ALL_TIME_FLOOR, to };
  const start = range === "week" ? startOfLocalWeek(now) : startOfLocalMonth(now);
  return { from: rfc3339(start), to };
}

/**
 * The instant a bucket key names, or `undefined` when it is not one.
 *
 * The key is a **UTC hour**, `YYYY-MM-DDTHH`, thirteen characters with no minutes, no offset
 * and no `Z` — it is an hour, not an instant. A key this build does not recognise is skipped
 * rather than guessed at: a store written by a newer version is a thing to survive, not to
 * interpret.
 */
export function hourStart(key: string): number | undefined {
  const parts = /^(\d{4})-(\d{2})-(\d{2})T(\d{2})$/.exec(key);
  if (!parts) return undefined;
  const [year, month, day, hour] = parts.slice(1).map(Number) as [number, number, number, number];
  if (month < 1 || month > 12 || day < 1 || day > 31 || hour > 23) return undefined;
  const at = Date.UTC(year, month - 1, day, hour);
  // A day the month does not have — 31 April — becomes 1 May in the constructor, so the
  // round trip is the validity check, the same one `nazar-core::timefmt` makes.
  const back = new Date(at);
  if (back.getUTCMonth() !== month - 1 || back.getUTCDate() !== day) return undefined;
  return at;
}

/** The local day a UTC hour starts in, `YYYY-MM-DD`. */
export function localDayKey(at: number): string {
  return localDate(new Date(at));
}

/** The local Monday-start week a UTC hour starts in, named by that Monday. */
export function localWeekKey(at: number): string {
  return localDate(startOfLocalWeek(new Date(at)));
}

// ------------------------------------------------------------------ counters

/**
 * Add a counter that may not be there.
 *
 * Absent plus absent is absent; absent plus a number is that number. That is what keeps
 * "nobody reported this" from becoming "this was zero" after one addition.
 */
function add(current: number | undefined, value: number | undefined): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) return current;
  return (current ?? 0) + value;
}

/** The headline counters of one bucket: `input + output + cache_create`. */
export function bucketTotal(bucket: UsageBucket): number | undefined {
  return add(add(add(undefined, bucket.input), bucket.output), bucket.cache_create);
}

/** A count that has to be a number, from a document that may hold anything. */
function count(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

// ------------------------------------------------------------------ the view

/** What one model accumulates while the answer is walked. */
interface Sums {
  total: number | undefined;
  cacheRead: number | undefined;
  requests: number;
}

/** A row's order: the biggest first, and a tie broken by a name rather than by luck. */
function byTotal(left: UsageRow, right: UsageRow): number {
  const difference = (right.total ?? -1) - (left.total ?? -1);
  return difference !== 0 ? difference : left.model.localeCompare(right.model);
}

/**
 * The dense run of columns a strip is drawn from.
 *
 * Dense rather than only the days that have something in them: a week with Wednesday missing
 * is a week where no work happened on Wednesday, and a strip that closed the gap would say
 * the opposite. It stops at today, because the rest of the week has not happened yet.
 */
function series(range: UsageRange, now: Date, first: number | undefined): UsageBar[] {
  const step = range === "all" ? 7 : 1;
  const last = range === "all" ? startOfLocalWeek(now) : startOfLocalDay(now);
  let start: Date;
  if (range === "week") start = startOfLocalWeek(now);
  else if (range === "month") start = startOfLocalMonth(now);
  else start = first === undefined ? last : startOfLocalWeek(new Date(first));

  const bars: UsageBar[] = [];
  const cursor = new Date(start.getFullYear(), start.getMonth(), start.getDate());
  while (cursor.getTime() <= last.getTime() && bars.length < MAX_BARS) {
    bars.push({ key: localDate(cursor), at: cursor.getTime(), total: 0, height: 0 });
    cursor.setDate(cursor.getDate() + step);
  }
  return bars;
}

/** The `since` instant as milliseconds, or `undefined` when the store has none. */
function sinceInstant(since: string | undefined): number | undefined {
  if (!since) return undefined;
  const at = Date.parse(since);
  return Number.isNaN(at) ? undefined : at;
}

/**
 * Everything the view needs, from one answer and the instant it is being drawn at.
 *
 * The answer's hourly UTC buckets are walked once: into per-model sums, and into the local
 * day or local week each hour **starts** in. Nothing is stored — the same rule the quota view
 * keeps — so a panel left open over midnight redraws into the new day the moment it is asked
 * to.
 */
export function buildUsageView(response: UsageResponse, now: Date): UsageView {
  const range: UsageRange = USAGE_RANGES.includes(response.range) ? response.range : "week";
  // Provider, then model: two levels rather than one key with a separator in it, because a
  // model id is whatever the source wrote and a separator that turned up inside one would
  // merge two models this file is not allowed to merge.
  const models = new Map<string, Map<string, Sums>>();
  const columns = new Map<string, number>();
  let total: number | undefined;
  let cacheRead: number | undefined;
  let requests = 0;
  let earliest: number | undefined;

  for (const [provider, hours] of Object.entries(response.providers ?? {})) {
    const own = models.get(provider) ?? new Map<string, Sums>();
    models.set(provider, own);

    for (const [key, buckets] of Object.entries(hours)) {
      const at = hourStart(key);
      if (at === undefined) continue;
      if (earliest === undefined || at < earliest) earliest = at;
      const column = range === "all" ? localWeekKey(at) : localDayKey(at);

      for (const [model, bucket] of Object.entries(buckets)) {
        const sum = bucketTotal(bucket);
        const row = own.get(model) ?? { total: undefined, cacheRead: undefined, requests: 0 };
        row.total = add(row.total, sum);
        row.cacheRead = add(row.cacheRead, bucket.cache_read);
        row.requests += count(bucket.requests);
        own.set(model, row);

        total = add(total, sum);
        cacheRead = add(cacheRead, bucket.cache_read);
        requests += count(bucket.requests);
        if (sum !== undefined) columns.set(column, (columns.get(column) ?? 0) + sum);
      }
    }
  }

  const flat: UsageRow[] = [];
  for (const [provider, own] of models) {
    for (const [model, sum] of own) {
      flat.push({
        provider,
        model,
        total: sum.total,
        cacheRead: sum.cacheRead,
        requests: sum.requests,
        share: 0,
      });
    }
  }

  // The share is against the largest row rather than against the window's total, so the
  // biggest bar is always full: at 360 px a row worth 3 % of the week would otherwise be a
  // bar too short to see, and the number beside it is what carries the proportion anyway.
  const largest = flat.reduce((most, row) => Math.max(most, row.total ?? 0), 0);
  const share = (total: number | undefined): number =>
    largest > 0 ? Math.min(100, ((total ?? 0) / largest) * 100) : 0;
  const rows: UsageRow[] = flat.map((row) => ({ ...row, share: share(row.total) })).sort(byTotal);

  const groups: UsageGroup[] = [...new Set(rows.map((row) => row.provider))]
    .map((provider) => {
      const own = rows.filter((row) => row.provider === provider);
      return {
        provider,
        total: own.reduce<number | undefined>((sum, row) => add(sum, row.total), undefined),
        rows: own,
      };
    })
    .sort((left, right) => (right.total ?? -1) - (left.total ?? -1));

  const floor = range === "all" ? (sinceInstant(response.since) ?? earliest) : earliest;
  const tallest = [...columns.values()].reduce((most, value) => Math.max(most, value), 0);
  const bars = series(range, now, floor).map((bar) => {
    const value = columns.get(bar.key) ?? 0;
    const height =
      tallest > 0 && value > 0 ? Math.max(MIN_VISIBLE_BAR, (value / tallest) * 100) : 0;
    return { ...bar, total: value, height };
  });

  return {
    range,
    total,
    cacheRead,
    requests,
    rows,
    groups,
    grouped: groups.length > 1,
    bars,
    since: response.since,
    scannedAt: response.scanned_at,
    damaged: response.damaged ?? [],
    empty: rows.length === 0,
  };
}

// ------------------------------------------------------------------ formatting

/**
 * A token count, short enough for a 360 px panel and spelled the way the language spells it.
 *
 * `Intl` decides the magnitude mark, not this file: English gets `22.3M` and `1.5B`, Russian
 * `22,3 млн`, Chinese `2230万`, Korean `2230만`, Turkish `22,3 Mn`, Spanish `22,3 M`. That is
 * a magnitude *abbreviation* in every one of them, which is the same word after 1 as after 5
 * — the property `locales/README.md` says this product's counted strings rely on — and it
 * puts no letter of English in a Korean panel.
 *
 * **The value is floored to the precision it will be shown at before it is formatted.** The
 * same rule as a quota percentage, for the same reason: a display that rounds 22.39 M up to
 * 22.4 M has told the reader something the store did not measure. Flooring first also makes
 * the number exact at whatever grain `Intl` picks, including the 万 and 억 that group by four
 * digits rather than three.
 *
 * Anything under a thousand is printed as itself: `843` is not `0.8K`.
 */
export function formatTokens(value: number | undefined, locale: string): string {
  if (value === undefined || !Number.isFinite(value)) return ABSENT;
  const whole = Math.max(0, Math.floor(value));
  const unit = whole >= 1e9 ? 1e9 : whole >= 1e6 ? 1e6 : whole >= 1e3 ? 1e3 : 1;
  if (unit === 1) return new Intl.NumberFormat(locale).format(whole);

  const digits = whole / unit < 100 ? 1 : 0;
  const step = unit / 10 ** digits;
  const floored = Math.floor(whole / step) * step;
  return new Intl.NumberFormat(locale, {
    notation: "compact",
    compactDisplay: "short",
    maximumFractionDigits: digits,
  }).format(floored);
}

/** A plain count — the requests behind a row — in the reader's own digits and grouping. */
export function formatNumber(value: number, locale: string): string {
  return new Intl.NumberFormat(locale).format(Math.max(0, Math.floor(value)));
}

/** An instant as a date the reader recognises: the *since* line, and a column's hover. */
export function formatDay(at: number | string | undefined, locale: string): string {
  if (at === undefined) return ABSENT;
  const value = typeof at === "string" ? Date.parse(at) : at;
  if (Number.isNaN(value)) return ABSENT;
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium" }).format(new Date(value));
}

/** A column's date, short enough to sit in a hover: `13 Sep`. */
export function formatColumn(at: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, { month: "short", day: "numeric" }).format(new Date(at));
}

/**
 * The message key for a row's second number, which is not the same number twice.
 *
 * `requests` counts **records that carried usage**, and what a record is differs by source.
 * On the Claude side it is a deduplicated assistant message, which is close enough to "a
 * reply" to be called one. On the Codex side T-WP14 counts `token_count` **events**, which
 * are not turns at all — a single turn emits several — so a row labelled *requests* there
 * would be a number the label lies about. Two labels, one honest each, rather than one word
 * stretched over two meanings.
 */
export function requestsKey(provider: string): string {
  return provider === "codex" ? "usage.row.events" : "usage.row.requests";
}

/**
 * The message key for an error kind.
 *
 * A kind this build does not know still gets a sentence rather than a blank panel: the Rust
 * side may learn a sixth one before this file does, and `detail` is printed underneath
 * either way.
 */
export function usageErrorKey(kind: string): string {
  return USAGE_ERROR_KINDS.includes(kind) ? `usage.error.${kind}` : "usage.error.unknown";
}

/**
 * Whether a rejected `invoke` handed back the error shape rather than something else.
 *
 * A command that failed inside Tauri — a window being torn down, a serialisation problem —
 * rejects with a string, and treating that as a `kind` would look up a message key that does
 * not exist and print it at the user.
 */
export function isUsageError(value: unknown): value is UsageError {
  if (typeof value !== "object" || value === null) return false;
  const shape = value as { kind?: unknown; detail?: unknown };
  return typeof shape.kind === "string" && typeof shape.detail === "string";
}
