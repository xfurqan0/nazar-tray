/**
 * The usage view's arithmetic: the window the panel asks for, the local re-bucketing of the
 * answer, the calendar the heat-map is drawn on, and the numbers the rows are drawn from. No
 * DOM here — `main.ts` draws what this file works out, and `test/usage.test.mjs` imports it
 * without a browser, the same split `format.ts` and `settings.ts` already keep.
 *
 * Five rules from `docs/usage-contract.md` are code in this file rather than notes in a
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
 * * **The headline is all four counters: `input + output + cache_read + cache_create`.**
 *   T-WP16 left `cache_read` out of it, on the argument that cache reads were 98.5 % of the
 *   raw total over six days of real work and a headline with them folded in is a number about
 *   the cache. The argument was right about the arithmetic and wrong about the reader: Claude
 *   Code's own `/usage` calls the four-way sum **total tokens**, and a tray that answers the
 *   same question with a number two orders of magnitude smaller reads as a tray that is
 *   broken rather than as a tray that is being careful. So the headline is the definition
 *   `/usage` uses, and the **four-way breakdown underneath it** is where the cache is told
 *   apart from the work — in the view, every time, rather than in a footnote. T-WP20.
 * * **An absent counter is absent, not zero.** A bucket built from lines that never named an
 *   input count has no `input` key, and the panel prints {@link ABSENT} rather than a
 *   confident `0`. The same rule `snapshot.ts` keeps for a window with no percentage.
 * * **A day is a cell, and a week is a column.** *All* is a calendar heat-map — weeks as
 *   columns, Monday at the top — over the same local days the re-bucketing produces. A day
 *   outside the range being drawn is a *hole*, not a zero: February has no 30th and next
 *   Friday has not happened, and both would be a lie drawn as an empty track.
 * * **A day and a week open the same thing.** T-WP21 made every drawn day and every row of
 *   *Weeks* lead to one detail: that span's four counters, and one row per model under its
 *   provider. {@link buildUsageDetail} is that one function, and it filters the answer
 *   already in hand by the local day or the local Monday an hour **starts** in — never by a
 *   second window asked of the store, which could come back from a different scan.
 *
 * Model ids are never merged and never translated. `claude-opus-5` and a bare `opus` are the
 * same model and appear as two rows, because merging them means a table of aliases that has
 * to be right about names nobody here controls, and a wrong merge cannot be undone once it
 * has been drawn. The chart on the *Models* tab keeps the rule: one line per model as its
 * source spelled it, and two providers that happen to agree on an id are two lines.
 */

import { formatDuration, type Translate } from "./format";
import type { UsageBucket, UsageError, UsageRange, UsageResponse } from "./snapshot";

/**
 * What is printed where a number is not known.
 *
 * An em dash rather than a `0`, and punctuation rather than a message key: it reads the same
 * in all six languages, exactly like the colons in a countdown.
 */
export const ABSENT = "—";

/**
 * What joins two facts on one line: the breakdown's four counters, the footer's two dates,
 * the hover line's three fields.
 *
 * Punctuation rather than a message key, for the reason {@link ABSENT} is one: a middle dot
 * is a middle dot in all six languages, and a separator in a locale file is a separator
 * somebody eventually translates into the comma that is already the decimal mark in
 * `22,3 Mn`.
 */
export const SEPARATOR = " · ";

/**
 * The `from` of the *all* range: early enough to hold every store this build can meet.
 *
 * The panel cannot know where a store begins before it has asked, and asking with a floor is
 * cheaper than asking twice — the query walks the months that exist on disk, not the months
 * between these two instants. The honest boundary of the words *all time* is the `since` the
 * answer carries, which is what the footer line shows.
 */
export const ALL_TIME_FLOOR = "2020-01-01T00:00:00Z";

/**
 * How far back the *all* grid is drawn, in months.
 *
 * Twelve, counting the month being lived in, which is what `/usage` shows and what fits a row
 * of month labels. A store older than that keeps every number it holds — the headline, the
 * rows and the group totals are the whole answer — and only the calendar is cut, because a
 * grid two years wide is a grid nobody scrolls to the end of.
 */
export const ALL_MONTHS = 12;

/** How many week columns a grid may hold. Twelve months is at most fifty-three. */
const MAX_COLUMNS = 54;

/** The smallest height a column with any work in it may be drawn at, in percent. */
const MIN_VISIBLE_BAR = 3;

/** How many shades of the accent a day can be drawn in, on top of the empty one. */
export const HEAT_LEVELS = 4;

/** The three ranges `get_usage` knows, which is not the same list as the tabs. */
export const USAGE_RANGES: readonly UsageRange[] = ["week", "month", "all"];

/**
 * The four tabs, in the order they sit in.
 *
 * Not the same list as {@link USAGE_RANGES}, and that is the point of having two: *week* and
 * *all* are windows the store understands, while *weeks* and *models* are two more readings
 * of the widest of them. {@link tabRange} is the whole mapping, and {@link needsFetch} is why
 * moving between *Weeks*, *All* and *Models* asks the Rust side for nothing at all.
 *
 * *Month* left with T-WP21: a five-column heat-map of the month being lived in answered a
 * narrower question than the calendar on *All*, which draws the same month and eleven more,
 * and the tab it freed is where *Weeks* went.
 */
export const USAGE_TABS = ["week", "weeks", "all", "models"] as const;

/** One of {@link USAGE_TABS}. */
export type UsageTab = (typeof USAGE_TABS)[number];

/**
 * The three spans the *Models* tab offers, in the order they sit in.
 *
 * The same three Claude Code's own Stats screen offers, because that is the screen this tab
 * is read beside. *all* reuses the word on the tab above it rather than asking six languages
 * for a second one.
 */
export const USAGE_SPANS = ["all", "days7", "days30"] as const;

/** One of {@link USAGE_SPANS}. */
export type UsageSpan = (typeof USAGE_SPANS)[number];

/** How many local days a span covers, counting today, or everything the store holds. */
export const SPAN_DAYS: Readonly<Record<UsageSpan, number | undefined>> = {
  all: undefined,
  days7: 7,
  days30: 30,
};

/**
 * How many model lines the chart draws.
 *
 * Six, because six is how many hues {@link import("./theme").seriesPalette} can put on one
 * panel and still have the reader tell them apart. A seventh model is **not** in the chart
 * and **is** in the list underneath it, which holds every model the span named — so nothing
 * is lost, and the legend never says a colour the eye cannot match to a line.
 */
export const CHART_SERIES = 6;

/** How many points one line may hold. A year of days, and a little slack. */
const MAX_POINTS = 372;

/** The error kinds `get_usage` answers with, each of which has a message key. */
export const USAGE_ERROR_KINDS: readonly string[] = [
  "bad_range",
  "bad_window",
  "no_state_dir",
  "scan_failed",
  "store_unreadable",
];

/**
 * The four counters, kept apart.
 *
 * Each is `number | undefined` rather than an optional property: they are computed here
 * rather than parsed off a wire, and a caller asking `parts.input === undefined` should not
 * have to wonder whether the key was left out or set.
 */
export interface UsageParts {
  readonly input: number | undefined;
  readonly output: number | undefined;
  readonly cacheRead: number | undefined;
  readonly cacheCreate: number | undefined;
}

/** One model's totals over the whole window. */
export interface UsageRow {
  /** `claude` or `codex`, as the store spells it. */
  readonly provider: string;
  /** The model id exactly as the source reported it. Never merged, never translated. */
  readonly model: string;
  /** All four counters added, or `undefined` when the window named none of them. */
  readonly total: number | undefined;
  /** The same four, kept apart, for the line under the total. */
  readonly parts: UsageParts;
  /** Deduplicated records that carried usage. Not a count of prompts. */
  readonly requests: number;
  /** How wide the row's bar is drawn, `0`–`100`, against the largest row. */
  readonly share: number;
  /**
   * The row's share of the whole window, as a **floored** percentage, or `undefined` when
   * either number is not known.
   *
   * Floored rather than rounded, the same rule `displayPercent` keeps for a quota: a row
   * worth 4.9 % of the window is not 5 %. So the column does not add up to 100, which is
   * honest arithmetic rather than a bug — the totals beside it are the numbers that do.
   *
   * Drawn only on the *Models* tab, where the question is which model the window went on;
   * the other lists answer that with the bar, which is against the largest row instead.
   */
  readonly percent: number | undefined;
}

/** The rows of one provider, and what they add up to. */
export interface UsageGroup {
  readonly provider: string;
  readonly total: number | undefined;
  readonly rows: readonly UsageRow[];
}

/** One local day in the calendar, whether or not it is inside the range being drawn. */
export interface UsageCell {
  /** The local day, `YYYY-MM-DD`. Empty for a cell outside the range. */
  readonly key: string;
  /** The instant that day starts at locally, for an `Intl` date. `0` outside the range. */
  readonly at: number;
  /** Known tokens on that day. Zero means nothing was recorded, which is a fact. */
  readonly total: number;
  /** `0`–{@link HEAT_LEVELS}. See {@link heatLevel} for what decides it. */
  readonly level: number;
  /**
   * Whether this day is inside the range at all.
   *
   * The first column of a month that starts on a Wednesday has two cells that are not days of
   * that month, and the last column of *Month* holds the days that have not happened yet.
   * Neither is a day with no work on it, so neither is drawn as one.
   */
  readonly present: boolean;
  /** The model that spent most of that day's tokens, for the hover line. */
  readonly topModel: string | undefined;
}

/** One column of the calendar: seven cells, Monday at the top. */
export interface UsageWeek {
  /** The local Monday this column starts on. */
  readonly start: number;
  readonly days: readonly UsageCell[];
}

/** A month name over the column its first drawn day falls in. */
export interface UsageMonthLabel {
  /** Zero-based column index. */
  readonly column: number;
  /** An instant inside that month, for an `Intl` month name. */
  readonly at: number;
}

/** The calendar the *Month* and *All* tabs draw. */
export interface UsageGrid {
  readonly columns: readonly UsageWeek[];
  readonly months: readonly UsageMonthLabel[];
  /** The busiest day in the grid, which is what the shading is measured against. */
  readonly busiest: number;
}

/** One column of the week strip: a local day, from Monday to today. */
export interface UsageBar {
  readonly key: string;
  readonly at: number;
  readonly total: number;
  /** Height against the tallest column, `0`–`100`. */
  readonly height: number;
  readonly level: number;
  readonly topModel: string | undefined;
}

/**
 * One row of the *Weeks* tab: a calendar week, Monday to Sunday, in the reader's own zone.
 *
 * The list is **dense**, for the reason the heat-map is: a week with nothing in it is a week
 * nobody worked, and a list that closed the gap would move a quiet fortnight next to a busy
 * one and call them neighbours. A week with no counters at all has `total: undefined` and
 * prints {@link ABSENT}, which is the same distinction the rows draw between a silence and a
 * measured zero.
 */
export interface UsageWeekRow {
  /** The local Monday the week starts on, `YYYY-MM-DD`. The key a detail is opened with. */
  readonly key: string;
  /** That Monday, local midnight. */
  readonly start: number;
  /** The Sunday, local midnight, which is what the second half of the label is drawn from. */
  readonly end: number;
  readonly total: number | undefined;
  readonly parts: UsageParts;
  readonly requests: number;
  /** How wide the row's bar is drawn, `0`–`100`, against the busiest week in the list. */
  readonly share: number;
  /** The week being lived in, which is a part week and is drawn as one. */
  readonly current: boolean;
}

/** What a detail is opened for: one local day, or one local Monday-start week. */
export interface UsageScope {
  readonly kind: "day" | "week";
  /** The local day, or the week's local Monday, `YYYY-MM-DD`. */
  readonly key: string;
  /** That day's local midnight, for an `Intl` date. */
  readonly at: number;
}

/** A window's numbers, however the window was cut: the whole answer, a week, or a day. */
export interface UsageTotals {
  /** All four counters added. The same definition as `/usage`'s total, at every grain. */
  readonly total: number | undefined;
  readonly parts: UsageParts;
  readonly requests: number;
  /** Every row, biggest first, whatever provider it belongs to. */
  readonly rows: readonly UsageRow[];
  /** The same rows under their providers, biggest group first. */
  readonly groups: readonly UsageGroup[];
  /** Whether more than one provider is in the answer, which is when headings earn a line. */
  readonly grouped: boolean;
}

/** What a day or a week says when it is opened. */
export interface UsageDetail extends UsageTotals {
  readonly scope: UsageScope;
  /** Nothing was recorded in that day or week. Not an error, and not a row of zeroes. */
  readonly empty: boolean;
}

/** One local day of the chart's x axis. */
export interface UsageChartDay {
  readonly key: string;
  readonly at: number;
}

/** One model's line: a total per local day, in the order {@link UsageChart.days} holds. */
export interface UsageSeries {
  readonly provider: string;
  readonly model: string;
  /** What the model spent over the whole span, which is what the lines are sorted by. */
  readonly total: number;
  /** One value per day. A day the model spent nothing on is `0`, which is a measurement. */
  readonly points: readonly number[];
}

/** The *Models* tab's chart, worked out from one answer, one span and one instant. */
export interface UsageChart {
  readonly span: UsageSpan;
  readonly days: readonly UsageChartDay[];
  /** At most {@link CHART_SERIES} lines, biggest first. */
  readonly series: readonly UsageSeries[];
  /** The tallest single point among the lines drawn, which the y axis is measured against. */
  readonly peak: number;
  /** How many models the span held, drawn or not, so a caller can say what is not on it. */
  readonly models: number;
  /**
   * The span's own numbers, for the list under the chart.
   *
   * The list is the span's and not the window's, and it is computed in the same pass as the
   * lines: a chart of the last seven days over a table of the last twelve months would be
   * two answers on one screen to a question the reader asked once.
   */
  readonly totals: UsageTotals;
  readonly empty: boolean;
}

/** Everything the view draws, worked out from one answer and one instant. */
export interface UsageView extends UsageTotals {
  readonly range: UsageRange;
  /** The seven-day strip. Only the *Week* tab draws one; the others leave it empty. */
  readonly bars: readonly UsageBar[];
  /** The calendar. Every range builds one; only *All* draws it. */
  readonly grid: UsageGrid;
  /**
   * Every calendar week the range covers, **newest first**, which is the *Weeks* tab.
   *
   * Built for every range rather than only for the one that draws it, for the reason the
   * grid is: the arithmetic costs one map over the same walk, and a field that exists only
   * sometimes is a field every caller has to ask about.
   */
  readonly weeks: readonly UsageWeekRow[];
  readonly since: string | undefined;
  readonly scannedAt: string | undefined;
  readonly damaged: readonly string[];
  /** Nothing was recorded in this window. Not an error, and not a row of zeroes. */
  readonly empty: boolean;
}

// ------------------------------------------------------------- tabs and state

/**
 * Which window a tab asks the store for.
 *
 * *Week* is this Monday to now. Everything else is the widest window there is, because
 * *Weeks*, *All* and *Models* are three readings of the same history and asking three times
 * for three cuts of it would mean three scans, three `since` lines and three chances for two
 * tabs to disagree about a number the user can put side by side.
 */
export function tabRange(tab: UsageTab): UsageRange {
  return tab === "week" ? "week" : "all";
}

/** Where the view is: which tab, which span under *Models*, and which detail is open. */
export interface UsageState {
  readonly tab: UsageTab;
  readonly span: UsageSpan;
  /** The day or week being looked into, or `undefined` when the list itself is on screen. */
  readonly detail: UsageScope | undefined;
}

/** What the view opens on: this week, no detail, and a span nothing is looking at yet. */
export const FIRST_USAGE_STATE: UsageState = { tab: "week", span: "all", detail: undefined };

/**
 * Move to a tab, which closes any detail that was open.
 *
 * A detail belongs to the list it was opened from — Tuesday came from the strip, a week from
 * the list — so carrying one across a tab change would leave the panel showing a day nothing
 * on screen points at any more.
 */
export function withTab(state: UsageState, tab: UsageTab): UsageState {
  return { ...state, tab, detail: undefined };
}

/** Choose a span under *Models*. Nothing else about the view moves. */
export function withSpan(state: UsageState, span: UsageSpan): UsageState {
  return { ...state, span };
}

/** Open one day or one week. */
export function openDetail(state: UsageState, detail: UsageScope): UsageState {
  return { ...state, detail };
}

/** *Back*: the list the detail was opened from, on the tab it was opened from. */
export function closeDetail(state: UsageState): UsageState {
  return { ...state, detail: undefined };
}

/**
 * Whether a move has to ask `get_usage` again.
 *
 * Only a change of **window** does. Opening a day, going back, choosing a span and moving
 * between *Weeks*, *All* and *Models* are all cuts of an answer already in hand, and asking
 * again for each of them would put a loading line — and a scan — in front of a number the
 * panel is holding.
 */
export function needsFetch(before: UsageState, after: UsageState): boolean {
  return tabRange(before.tab) !== tabRange(after.tab);
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
 * Local rather than UTC on purpose: the two instants are a *local* Monday and a *local* now,
 * and writing them with the offset they were computed in is what makes them readable in an
 * error message. The Rust side parses the offset and echoes the window back in UTC.
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

/**
 * The last day of the month an instant falls in, 00:00 local.
 *
 * Day zero of the next month, which is how the local constructor is asked for "the day before
 * the first" — so February is 28 or 29 without this file owning a leap-year rule.
 */
export function endOfLocalMonth(at: Date): Date {
  return new Date(at.getFullYear(), at.getMonth() + 1, 0);
}

/** Midnight of the local day an instant falls in. */
export function startOfLocalDay(at: Date): Date {
  return new Date(at.getFullYear(), at.getMonth(), at.getDate());
}

/** `count` local days after a local midnight, across months, years and clock changes. */
function addLocalDays(at: Date, count: number): Date {
  return new Date(at.getFullYear(), at.getMonth(), at.getDate() + count);
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

/** The four counters of a breakdown added together, absence surviving the addition. */
export function partsTotal(parts: UsageParts): number | undefined {
  const half = add(add(undefined, parts.input), parts.output);
  return add(add(half, parts.cacheRead), parts.cacheCreate);
}

/**
 * One bucket's total: `input + output + cache_read + cache_create`.
 *
 * The same definition Claude Code's `/usage` prints as *total tokens*. Nothing about the
 * cache is lost by adding it in — it is the line under the headline, in all four parts.
 */
export function bucketTotal(bucket: UsageBucket): number | undefined {
  return partsTotal({
    input: bucket.input,
    output: bucket.output,
    cacheRead: bucket.cache_read,
    cacheCreate: bucket.cache_create,
  });
}

/** A count that has to be a number, from a document that may hold anything. */
function count(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

/**
 * How dark a day is drawn, `0`–{@link HEAT_LEVELS}.
 *
 * **Max-relative, in quarters.** A day with nothing on it is `0`; any other day takes one of
 * four shades by its share of the busiest day in the same grid — under a quarter is 1, then a
 * half, then three quarters, and the busiest day itself is always 4.
 *
 * A **quantile** scale — the other obvious choice, and the one a contribution graph usually
 * uses — was written and rejected here. Quartiles over the *active* days cannot fill four
 * buckets when only three days are active, which is an ordinary first week with this product:
 * the busiest day would come out a shade lighter than the darkest the legend shows, and a
 * scale whose top step never appears reads as a bug rather than as a distribution.
 * Max-relative has the opposite failure — one enormous day flattens the rest to the lightest
 * shade — and that failure is *true*: beside a day that spent thirty times as much, the rest
 * of the month really was quiet. The exact number is one hover away either way, which is what
 * makes the colour a hint rather than a measurement.
 */
export function heatLevel(total: number, busiest: number): number {
  if (!(total > 0) || !(busiest > 0)) return 0;
  return Math.min(HEAT_LEVELS, 1 + Math.floor((total / busiest) * HEAT_LEVELS));
}

// ------------------------------------------------------------------ the view

/** What one model accumulates while the answer is walked. */
interface Sums {
  input: number | undefined;
  output: number | undefined;
  cacheRead: number | undefined;
  cacheCreate: number | undefined;
  requests: number;
}

/** A fresh accumulator. */
function sums(): Sums {
  return {
    input: undefined,
    output: undefined,
    cacheRead: undefined,
    cacheCreate: undefined,
    requests: 0,
  };
}

/** The four counters an accumulator has gathered. */
function partsOf(sum: Sums): UsageParts {
  return {
    input: sum.input,
    output: sum.output,
    cacheRead: sum.cacheRead,
    cacheCreate: sum.cacheCreate,
  };
}

/** Four counters nobody reported, which is what a week or a day with no hours in it holds. */
const EMPTY_PARTS: UsageParts = {
  input: undefined,
  output: undefined,
  cacheRead: undefined,
  cacheCreate: undefined,
};

/** Add one bucket into an accumulator. */
function gather(sum: Sums, bucket: UsageBucket): void {
  sum.input = add(sum.input, bucket.input);
  sum.output = add(sum.output, bucket.output);
  sum.cacheRead = add(sum.cacheRead, bucket.cache_read);
  sum.cacheCreate = add(sum.cacheCreate, bucket.cache_create);
  sum.requests += count(bucket.requests);
}

/** A row's order: the biggest first, and a tie broken by a name rather than by luck. */
function byTotal(left: UsageRow, right: UsageRow): number {
  const difference = (right.total ?? -1) - (left.total ?? -1);
  return difference !== 0 ? difference : left.model.localeCompare(right.model);
}

/**
 * What one window gathers while the answer is walked, at whatever grain the window is.
 *
 * Provider, then model: two levels rather than one key with a separator in it, because a
 * model id is whatever the source wrote and a separator that turned up inside one would
 * merge two models this file is not allowed to merge.
 */
interface Tally {
  readonly models: Map<string, Map<string, Sums>>;
  readonly whole: Sums;
}

/** A fresh window. */
function tally(): Tally {
  return { models: new Map(), whole: sums() };
}

/** Add one bucket to a window, under its provider and its model and to the window itself. */
function into(store: Tally, provider: string, model: string, bucket: UsageBucket): void {
  const own = store.models.get(provider) ?? new Map<string, Sums>();
  store.models.set(provider, own);
  const row = own.get(model) ?? sums();
  gather(row, bucket);
  own.set(model, row);
  gather(store.whole, bucket);
}

/**
 * One window's numbers: the four counters, the rows, and the rows under their providers.
 *
 * The whole answer, one week and one day are the same arithmetic over a different set of
 * hours, so they are the same function. That is what keeps a week's detail from adding up
 * differently from the row of the list it was opened from.
 */
function summarise(store: Tally): UsageTotals {
  const flat: UsageRow[] = [];
  for (const [provider, own] of store.models) {
    for (const [model, sum] of own) {
      const parts = partsOf(sum);
      flat.push({
        provider,
        model,
        total: partsTotal(parts),
        parts,
        requests: sum.requests,
        share: 0,
        percent: undefined,
      });
    }
  }

  const parts = partsOf(store.whole);
  const total = partsTotal(parts);

  // The share is against the largest row rather than against the window's total, so the
  // biggest bar is always full: at 360 px a row worth 3 % of the week would otherwise be a
  // bar too short to see, and the number beside it is what carries the proportion anyway.
  // The *percentage* is the other question — how much of the window went here — and it is
  // floored, never rounded up, which is the rule a quota reading already keeps.
  const largest = flat.reduce((most, row) => Math.max(most, row.total ?? 0), 0);
  const rows: UsageRow[] = flat
    .map((row) => ({
      ...row,
      share: largest > 0 ? Math.min(100, ((row.total ?? 0) / largest) * 100) : 0,
      percent:
        row.total === undefined || total === undefined || !(total > 0)
          ? undefined
          : Math.floor((row.total / total) * 100),
    }))
    .sort(byTotal);

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

  return {
    total,
    parts,
    requests: store.whole.requests,
    rows,
    groups,
    grouped: groups.length > 1,
  };
}

/**
 * The model that spent most of one day, or `undefined` when nothing did.
 *
 * A tie is broken by the id rather than by the order two providers happened to be walked in,
 * so the hover line says the same thing twice in a row.
 */
function busiestModel(models: Map<string, number> | undefined): string | undefined {
  let best: string | undefined;
  let most = 0;
  for (const [model, total] of models ?? []) {
    if (total > most || (total === most && best !== undefined && model.localeCompare(best) < 0)) {
      best = model;
      most = total;
    }
  }
  return best;
}

/** The `since` instant as milliseconds, or `undefined` when the store has none. */
function sinceInstant(since: string | undefined): number | undefined {
  if (!since) return undefined;
  const at = Date.parse(since);
  return Number.isNaN(at) ? undefined : at;
}

/**
 * The first and last local day a range's calendar covers.
 *
 * *week* is this Monday to Sunday and *month* the first to the last of this month — the whole
 * calendar shape, so that a month is a month rather than a stub that grows a cell a day —
 * while *all* starts at the store's own floor, or {@link ALL_MONTHS} months back, whichever is
 * later. The days inside those bounds that have not happened yet are cut by `today` in
 * {@link buildGrid}: "not yet" and "outside the range" are the same fact about a day nobody
 * could have worked on, and they are drawn the same way.
 */
function gridBounds(range: UsageRange, now: Date, floor: number | undefined): [Date, Date] {
  if (range === "month") return [startOfLocalMonth(now), endOfLocalMonth(now)];
  if (range === "week") {
    const monday = startOfLocalWeek(now);
    return [monday, addLocalDays(monday, 6)];
  }
  const window = new Date(now.getFullYear(), now.getMonth() - (ALL_MONTHS - 1), 1);
  const begin = floor === undefined ? window : startOfLocalDay(new Date(floor));
  return [begin.getTime() > window.getTime() ? begin : window, startOfLocalDay(now)];
}

/**
 * The calendar, as columns of seven cells with Monday at the top.
 *
 * Dense rather than only the days that have something in them: a week with Wednesday missing
 * is a week where no work happened on Wednesday, and a grid that closed the gap would say the
 * opposite. The cells outside the range — before the first of the month, after today — are
 * holes rather than empty days, which is the one distinction a heat-map has to draw and the
 * reason {@link UsageCell} carries `present` at all.
 */
function buildGrid(
  range: UsageRange,
  now: Date,
  floor: number | undefined,
  totals: Map<string, number>,
  models: Map<string, Map<string, number>>,
): UsageGrid {
  const [first, last] = gridBounds(range, now, floor);
  const today = startOfLocalDay(now);
  const stop = last.getTime() < today.getTime() ? last : today;

  const columns: UsageWeek[] = [];
  let busiest = 0;
  const cursor = startOfLocalWeek(first);
  const final = startOfLocalWeek(last).getTime();

  while (cursor.getTime() <= final && columns.length < MAX_COLUMNS) {
    const days: UsageCell[] = [];
    for (let index = 0; index < 7; index++) {
      const day = addLocalDays(cursor, index);
      const at = day.getTime();
      const present = at >= first.getTime() && at <= stop.getTime();
      const key = present ? localDate(day) : "";
      const total = present ? (totals.get(key) ?? 0) : 0;
      if (total > busiest) busiest = total;
      days.push({
        key,
        at: present ? at : 0,
        total,
        level: 0,
        present,
        topModel: present ? busiestModel(models.get(key)) : undefined,
      });
    }
    columns.push({ start: cursor.getTime(), days });
    cursor.setDate(cursor.getDate() + 7);
  }

  const shaded: UsageWeek[] = columns.map((column) => ({
    start: column.start,
    days: column.days.map((cell) => ({ ...cell, level: heatLevel(cell.total, busiest) })),
  }));

  // A label goes over the column whose first drawn day opens a new month. Reading the first
  // *present* day rather than the column's Monday is what keeps a September grid from being
  // labelled August because its first column starts on the 31st of the month before.
  const months: UsageMonthLabel[] = [];
  let previous = -1;
  shaded.forEach((column, index) => {
    const opener = column.days.find((cell) => cell.present);
    if (!opener) return;
    const month = new Date(opener.at).getMonth();
    if (month === previous) return;
    previous = month;
    months.push({ column: index, at: opener.at });
  });

  return { columns: shaded, months, busiest };
}

/**
 * The *Weeks* tab: one row per calendar week the range covers, newest first.
 *
 * The same bounds the calendar is drawn on, so the list and the heat-map cover the same
 * history and a week counted in one is a week drawn in the other. Dense, and capped at the
 * width a grid is capped at — a list nobody scrolls to the end of is the same failure as a
 * grid nobody scrolls to the end of.
 */
function buildWeeks(
  range: UsageRange,
  now: Date,
  floor: number | undefined,
  weekly: Map<string, Sums>,
): UsageWeekRow[] {
  const [first, last] = gridBounds(range, now, floor);
  const today = startOfLocalDay(now);
  const stop = last.getTime() < today.getTime() ? last : today;
  const thisWeek = startOfLocalWeek(now).getTime();

  const rows: UsageWeekRow[] = [];
  const cursor = startOfLocalWeek(first);
  const final = startOfLocalWeek(stop).getTime();

  while (cursor.getTime() <= final && rows.length < MAX_COLUMNS) {
    const start = new Date(cursor.getTime());
    const key = localDate(start);
    const sum = weekly.get(key);
    const parts = sum ? partsOf(sum) : EMPTY_PARTS;
    rows.push({
      key,
      start: start.getTime(),
      end: addLocalDays(start, 6).getTime(),
      total: partsTotal(parts),
      parts,
      requests: sum?.requests ?? 0,
      share: 0,
      current: start.getTime() === thisWeek,
    });
    cursor.setDate(cursor.getDate() + 7);
  }

  const busiest = rows.reduce((most, row) => Math.max(most, row.total ?? 0), 0);
  return rows
    .map((row) => ({
      ...row,
      share: busiest > 0 ? Math.min(100, ((row.total ?? 0) / busiest) * 100) : 0,
    }))
    .reverse();
}

/**
 * Everything the view needs, from one answer and the instant it is being drawn at.
 *
 * The answer's hourly UTC buckets are walked once: into per-model sums, into the local day
 * each hour **starts** in — both that day's total and, per model, what it spent there, which
 * is what the hover line names — and into the local week that day belongs to. Nothing is
 * stored — the same rule the quota view keeps — so a panel left open over midnight redraws
 * into the new day the moment it is asked to.
 */
export function buildUsageView(response: UsageResponse, now: Date): UsageView {
  const range: UsageRange = USAGE_RANGES.includes(response.range) ? response.range : "week";
  const window = tally();
  const dayTotals = new Map<string, number>();
  const dayModels = new Map<string, Map<string, number>>();
  const weekly = new Map<string, Sums>();
  let earliest: number | undefined;

  for (const [provider, hours] of Object.entries(response.providers ?? {})) {
    for (const [key, buckets] of Object.entries(hours)) {
      const at = hourStart(key);
      if (at === undefined) continue;
      if (earliest === undefined || at < earliest) earliest = at;
      const day = localDayKey(at);
      const week = localWeekKey(at);

      for (const [model, bucket] of Object.entries(buckets)) {
        into(window, provider, model, bucket);

        const own = weekly.get(week) ?? sums();
        gather(own, bucket);
        weekly.set(week, own);

        const sum = bucketTotal(bucket);
        if (sum === undefined) continue;
        dayTotals.set(day, (dayTotals.get(day) ?? 0) + sum);
        const perModel = dayModels.get(day) ?? new Map<string, number>();
        perModel.set(model, (perModel.get(model) ?? 0) + sum);
        dayModels.set(day, perModel);
      }
    }
  }

  const totals = summarise(window);
  const floor = range === "all" ? (sinceInstant(response.since) ?? earliest) : earliest;
  const grid = buildGrid(range, now, floor, dayTotals, dayModels);
  const weeks = buildWeeks(range, now, floor, weekly);

  // The strip is the week grid's one column, cut at today: seven cells is a calendar, and a
  // strip that drew Saturday before it happened would be a bar chart with a promise in it.
  const bars: UsageBar[] = (range === "week" ? (grid.columns[0]?.days ?? []) : [])
    .filter((cell) => cell.present)
    .map((cell) => ({
      key: cell.key,
      at: cell.at,
      total: cell.total,
      level: cell.level,
      topModel: cell.topModel,
      height:
        grid.busiest > 0 && cell.total > 0
          ? Math.max(MIN_VISIBLE_BAR, (cell.total / grid.busiest) * 100)
          : 0,
    }));

  return {
    ...totals,
    range,
    bars,
    grid,
    weeks,
    since: response.since,
    scannedAt: response.scanned_at,
    damaged: response.damaged ?? [],
    empty: totals.rows.length === 0,
  };
}

// ------------------------------------------------------------------ the detail

/**
 * What one day or one week holds, per model, under its provider.
 *
 * The answer already in hand is filtered rather than asked for again: a second `get_usage`
 * would run against a store that may have been scanned in between, and a detail that did not
 * add up to the row it was opened from is the failure T-WP20b spent a package on. The walk
 * is the same one {@link buildUsageView} does, over the same hours, with one comparison in
 * front of it.
 */
export function buildUsageDetail(response: UsageResponse, scope: UsageScope): UsageDetail {
  const store = tally();

  for (const [provider, hours] of Object.entries(response.providers ?? {})) {
    for (const [key, buckets] of Object.entries(hours)) {
      const at = hourStart(key);
      if (at === undefined) continue;
      // The hour lands in the local day it **starts** in, so a day and the week that holds
      // it are cut from the same instant and a week is exactly its seven days added up.
      const where = scope.kind === "day" ? localDayKey(at) : localWeekKey(at);
      if (where !== scope.key) continue;
      for (const [model, bucket] of Object.entries(buckets)) into(store, provider, model, bucket);
    }
  }

  const totals = summarise(store);
  return { ...totals, scope, empty: totals.rows.length === 0 };
}

// ------------------------------------------------------------------- the chart

/** The first and last local day of a span, as midnights. */
function chartBounds(span: UsageSpan, now: Date, floor: number | undefined): [Date, Date] {
  const today = startOfLocalDay(now);
  const days = SPAN_DAYS[span];
  if (days !== undefined) return [addLocalDays(today, -(days - 1)), today];
  // *All time* is drawn over the history the calendar is drawn over, for the same reason the
  // weeks list is: two tabs that disagree about how far back *all* goes are worse than one
  // that is honest about the floor, which is what the `Since` line under the headline says.
  const [first] = gridBounds("all", now, floor);
  const oldest = addLocalDays(today, -(MAX_POINTS - 1));
  return [first.getTime() > oldest.getTime() ? first : oldest, today];
}

/**
 * The *Models* tab's chart: one line per model, a point per local day.
 *
 * **A day a model spent nothing on is `0`, not a gap.** A line that skipped it would join
 * Monday to Wednesday with a straight segment through the Tuesday it is silent about, which
 * draws work that did not happen. Zero is what happened, and a line at the axis says so.
 *
 * The span decides the days and nothing else: the same answer is cut three ways, so moving
 * between *All*, *Last 7 days* and *Last 30 days* redraws without asking the store anything.
 */
export function buildUsageChart(response: UsageResponse, span: UsageSpan, now: Date): UsageChart {
  // Where *all time* begins needs the store's floor, and the floor needs a look at the keys,
  // so the hours are walked twice: once for the earliest of them and once for the numbers.
  // Both passes are over an object already in memory and neither of them parses anything.
  let earliest: number | undefined;
  for (const hours of Object.values(response.providers ?? {})) {
    for (const key of Object.keys(hours)) {
      const at = hourStart(key);
      if (at !== undefined && (earliest === undefined || at < earliest)) earliest = at;
    }
  }

  const floor = sinceInstant(response.since) ?? earliest;
  const [first, last] = chartBounds(span, now, floor);
  const days: UsageChartDay[] = [];
  const inside = new Set<string>();
  for (
    let cursor = new Date(first.getTime());
    cursor.getTime() <= last.getTime() && days.length < MAX_POINTS;
    cursor = addLocalDays(cursor, 1)
  ) {
    const key = localDate(cursor);
    days.push({ key, at: cursor.getTime() });
    inside.add(key);
  }

  // Provider, then model, then local day. Three levels rather than one key with separators
  // in it, for the reason {@link Tally} gives: a model id is whatever the source wrote, and a
  // separator that turned up inside one would merge two lines this file may not merge.
  const daily = new Map<string, Map<string, Map<string, number>>>();
  const store = tally();

  for (const [provider, hours] of Object.entries(response.providers ?? {})) {
    const own = daily.get(provider) ?? new Map<string, Map<string, number>>();
    daily.set(provider, own);

    for (const [key, buckets] of Object.entries(hours)) {
      const at = hourStart(key);
      if (at === undefined) continue;
      const day = localDayKey(at);
      if (!inside.has(day)) continue;

      for (const [model, bucket] of Object.entries(buckets)) {
        into(store, provider, model, bucket);
        const sum = bucketTotal(bucket);
        if (sum === undefined) continue;
        const line = own.get(model) ?? new Map<string, number>();
        line.set(day, (line.get(day) ?? 0) + sum);
        own.set(model, line);
      }
    }
  }

  const inSpan: UsageSeries[] = [];
  for (const [provider, own] of daily) {
    for (const [model, line] of own) {
      const points = days.map((day) => line.get(day.key) ?? 0);
      const total = points.reduce((sum, value) => sum + value, 0);
      // A model that spent nothing inside the span is not a flat line at the axis, it is a
      // model this span has nothing to say about, and a legend entry for it would be a
      // colour with no line beside it.
      if (total > 0) inSpan.push({ provider, model, total, points });
    }
  }
  inSpan.sort((left, right) => right.total - left.total || left.model.localeCompare(right.model));

  const series = inSpan.slice(0, CHART_SERIES);
  const peak = series.reduce(
    (most, own) => own.points.reduce((high, value) => Math.max(high, value), most),
    0,
  );

  return {
    span,
    days,
    series,
    peak,
    models: inSpan.length,
    totals: summarise(store),
    empty: series.length === 0,
  };
}

/** Where a line is drawn, in the coordinate space of the SVG that holds it. */
export interface ChartBox {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

/** Two decimals, which is finer than a pixel and shorter than an attribute full of digits. */
function place(value: number): number {
  return Math.round(value * 100) / 100;
}

/**
 * One line's `points` attribute: evenly along the x axis, and against the chart's own peak.
 *
 * A span of one day has no width to spread over, so its single point sits in the middle of
 * the box rather than on its left edge, where a line of one point would be invisible against
 * the axis. A chart with a peak of zero draws every line flat on the floor, which is what a
 * span nobody spent anything in looks like.
 */
export function chartPoints(
  points: readonly number[],
  peak: number,
  box: ChartBox,
): string {
  const last = points.length - 1;
  return points
    .map((value, index) => {
      const x = last > 0 ? box.left + (index / last) * box.width : box.left + box.width / 2;
      const y = peak > 0 ? box.top + box.height - (value / peak) * box.height : box.top + box.height;
      return `${place(x)},${place(y)}`;
    })
    .join(" ");
}

/**
 * Which days carry a label under the chart: the first, the middle and the last.
 *
 * Three, because a month of ticks at 10 px in a 330 px panel is a grey smear, and because
 * the three that matter are where the span begins, where it ends and roughly where the
 * middle of it is. Claude Code's own Stats screen labels its axis the same way.
 */
export function chartTicks(count: number): number[] {
  if (count <= 0) return [];
  if (count <= 2) return [...Array(count).keys()];
  return [0, Math.floor((count - 1) / 2), count - 1];
}

/**
 * The values the y axis is labelled with, biggest first: the peak, half of it, and zero.
 *
 * Three lines rather than a computed "nice" scale: the numbers beside them are compact —
 * `1B`, `500M`, `0` — and a scale that rounded the top up to a round number would draw a
 * ceiling above the tallest day rather than at it.
 */
export function chartLevels(peak: number): number[] {
  if (!(peak > 0)) return [0];
  return [peak, peak / 2, 0];
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

/** An instant as a date the reader recognises: the first field of a hover line. */
export function formatDay(at: number | string | undefined, locale: string): string {
  if (at === undefined) return ABSENT;
  const value = typeof at === "string" ? Date.parse(at) : at;
  if (Number.isNaN(value)) return ABSENT;
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium" }).format(new Date(value));
}

/** A day without its year, for the footer's *since*: `Aug 27`. */
export function formatColumn(at: number | string | undefined, locale: string): string {
  if (at === undefined) return ABSENT;
  const value = typeof at === "string" ? Date.parse(at) : at;
  if (Number.isNaN(value)) return ABSENT;
  return new Intl.DateTimeFormat(locale, { month: "short", day: "numeric" }).format(new Date(value));
}

/** A month name over a column of the calendar: `Sep`. */
export function formatMonth(at: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, { month: "short" }).format(new Date(at));
}

/**
 * What joins the two ends of a span: `Sep 14 – Sep 20`.
 *
 * An en dash rather than a message key, for the reason {@link SEPARATOR} is punctuation: a
 * dash between two dates is a dash between two dates in all six languages, and a key would
 * eventually be translated into a word that does not fit a 360 px row.
 */
export const RANGE_DASH = " – ";

/**
 * A week as the *Weeks* tab writes it: Monday to Sunday, without the year.
 *
 * The year is left off for the reason the footer's *since* leaves it off — the list is read,
 * not parsed, and `Dec 28 – Jan 3` says which two years it crosses by saying which two months
 * it crosses. The detail that opens from the row carries the full date.
 */
export function weekLabel(week: { start: number; end: number }, locale: string): string {
  return `${formatColumn(week.start, locale)}${RANGE_DASH}${formatColumn(week.end, locale)}`;
}

/**
 * What the detail calls itself: a date for a day, and a named week for a week.
 *
 * A week gets a word — *Week of 14 Sep 2026* — because `Sep 14 – Sep 20` at the top of a page
 * of model rows reads as a filter somebody typed rather than as the thing being looked at.
 */
export function detailTitle(scope: UsageScope, locale: string, t: Translate): string {
  const date = formatDay(scope.at, locale);
  return scope.kind === "day" ? date : t("usage.detail.week", { date });
}

/**
 * The four counters on one line, as `/usage` writes them under its total.
 *
 * Always all four, always in this order, and an absent one prints {@link ABSENT} rather than
 * disappearing: a breakdown that dropped the counters nobody reported would not add up to the
 * number above it, and the reader would be left to work out which of the four went missing.
 */
export function partsLine(parts: UsageParts, locale: string, t: Translate): string {
  return [
    t("usage.part.input", { tokens: formatTokens(parts.input, locale) }),
    t("usage.part.output", { tokens: formatTokens(parts.output, locale) }),
    t("usage.part.cacheRead", { tokens: formatTokens(parts.cacheRead, locale) }),
    t("usage.part.cacheWrite", { tokens: formatTokens(parts.cacheCreate, locale) }),
  ].join(SEPARATOR);
}

/**
 * What a day says when it is hovered or focused: the date, the total, the model that led it.
 *
 * A day with nothing on it says so in words rather than as `0`, for the reason the quota view
 * never draws an empty bar: a zero and a silence look the same and only one of them is true.
 *
 * This is the panel's own line and not the operating system's tooltip. A tooltip inside a tray
 * popup is a second floating window over a first one that is already `alwaysOnTop`, it arrives
 * after a delay nobody controls, and it cannot be read by anyone navigating the grid with the
 * keyboard — which is three reasons for a line the panel owns and redraws itself.
 */
export function cellLine(
  cell: { at: number; total: number; topModel: string | undefined },
  locale: string,
  t: Translate,
): string {
  const date = formatDay(cell.at, locale);
  if (!(cell.total > 0) || cell.topModel === undefined) return t("usage.cell.none", { date });
  return t("usage.cell", {
    date,
    tokens: formatTokens(cell.total, locale),
    model: cell.topModel,
  });
}

/**
 * The line under the headline: `Since Aug 27 · scanned 2 m ago`.
 *
 * Both halves answer "should I believe this", which is why they are on the page rather than in
 * a log — and why they are now one line at the size of the rest of the view rather than two at
 * the smallest size the stylesheet has. T-WP16 made them footnotes; the maintainer could not
 * read them on a real desktop. *Since* stays on the *all* tab alone: it is the honest boundary
 * of the words *all time*, and under a tab headed **Week** the same date would read as a claim
 * about which week is being shown.
 */
export function footerLine(view: UsageView, locale: string, t: Translate, now: number): string {
  const parts: string[] = [];
  if (view.range === "all" && view.since !== undefined) {
    parts.push(t("usage.since", { date: formatColumn(view.since, locale) }));
  }
  const at = view.scannedAt === undefined ? Number.NaN : Date.parse(view.scannedAt);
  if (Number.isFinite(at)) {
    parts.push(t("usage.scanned", { age: formatDuration(Math.max(0, now - at), t) }));
  }
  return parts.join(SEPARATOR);
}

/**
 * The message key for a row's second number, which is not the same number twice.
 *
 * `requests` counts **records that carried usage**, and what a record is differs by source. On
 * the Claude side it is a deduplicated assistant message, which is close enough to "a reply"
 * to be called one. On the Codex side T-WP14 counts `token_count` **events**, which are not
 * turns at all — a single turn emits several — so a row labelled *requests* there would be a
 * number the label lies about. Two labels, one honest each, rather than one word stretched
 * over two meanings.
 */
export function requestsKey(provider: string): string {
  return provider === "codex" ? "usage.row.events" : "usage.row.requests";
}

/**
 * The message key for an error kind.
 *
 * A kind this build does not know still gets a sentence rather than a blank panel: the Rust
 * side may learn a sixth one before this file does, and `detail` is printed underneath either
 * way.
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
