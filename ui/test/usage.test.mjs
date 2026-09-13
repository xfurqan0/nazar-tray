// The usage view's arithmetic, which is the half of it that can be wrong without looking
// wrong.
//
// Four of these are contract rules rather than presentation taste, and the reason each is a
// test rather than a screenshot:
//
//   * **The local week is the panel's to compute.** Rust may not ask this machine which zone
//     it is in, so `ui/src/usage.ts` works out Monday 00:00 locally and re-buckets the hours
//     it gets back. A bucket lands in the local day its hour **starts** in — the rule
//     `docs/usage-contract.md` states — and the case that proves it is 21:00Z on a Sunday,
//     which is Monday for anybody at +03:00.
//   * **The headline is all four counters**, `input + output + cache_read + cache_create`,
//     which is the definition Claude Code's own `/usage` prints as *total tokens*. T-WP16 left
//     the cache reads out and the number came out two orders of magnitude short of the one
//     the maintainer was comparing it with; what tells the cache from the work is the
//     **breakdown line**, which is asserted here counter by counter.
//   * **An absent counter is absent, not zero.** A bucket that never named a counter prints
//     an em dash. A counter that really is zero — Codex reports `cache_create: 0` on every
//     event — prints `0`, because that is a measurement.
//   * **A day outside the range is a hole, not an empty day.** The first column of a month
//     that opens on a Wednesday, the cells after today, the week a store began mid-way
//     through: all of them are `present: false`, and the tests below are the calendar
//     arithmetic that decides it — a month on five columns, a month on six, and a year
//     boundary inside one column.
//
// The zone is changed inside the tests rather than at the top of the file: several of these
// only mean something in a particular one, and the tests in a file run one after another.
//
// The last tests are as close to a rendering as a suite with no DOM gets: they compose the
// lines the panel writes, in order, from a fixture answer, and then read `src/styles.css` for
// the handful of rules that carry meaning rather than taste. They cannot prove `main.ts`
// calls these functions — `ui/test/i18n.test.mjs` and the bridge test guard the words and the
// command — but they do prove that what they compose reads the way it is meant to.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { formatDuration } from "../dist/lib/format.mjs";
import { createTranslator } from "../dist/lib/i18n.mjs";
import { catalogs } from "../dist/lib/locales.mjs";
import {
  ABSENT,
  ALL_TIME_FLOOR,
  REPORTED_PROVIDER,
  CHART_SERIES,
  FIRST_USAGE_STATE,
  HEAT_LEVELS,
  SEPARATOR,
  USAGE_SPANS,
  USAGE_TABS,
  bucketTotal,
  buildUsageChart,
  buildUsageDetail,
  buildUsageView,
  cellLine,
  chartLevels,
  chartPoints,
  chartTicks,
  closeDetail,
  detailTitle,
  footerLine,
  formatColumn,
  formatDay,
  formatMonth,
  formatNumber,
  formatTokens,
  heatLevel,
  hourStart,
  isUsageError,
  localDayKey,
  localWeekKey,
  needsFetch,
  openDetail,
  partsLine,
  requestsKey,
  rfc3339,
  startOfLocalWeek,
  modeTagKey,
  tabRange,
  usageErrorKey,
  usageMode,
  usageWindow,
  weekLabel,
  weekTotal,
  withSpan,
  withTab,
} from "../dist/lib/usage.mjs";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const english = JSON.parse(readFileSync(resolve(REPO, "ui/locales/en.json"), "utf8"));
const en = createTranslator(catalogs, "en");

/** Run a body in one time zone, whatever the machine running the suite is set to. */
function inZone(zone, body) {
  const before = process.env.TZ;
  process.env.TZ = zone;
  try {
    return body();
  } finally {
    if (before === undefined) delete process.env.TZ;
    else process.env.TZ = before;
  }
}

// ------------------------------------------------------------ re-bucketing

test("a bucket lands in the local day its hour starts in", () => {
  // 21:00Z on Sunday 13 September 2026. In UTC that is Sunday; at +03:00 it is Monday
  // midnight, and it belongs to the week that starts the next morning.
  const at = hourStart("2026-09-13T21");
  assert.equal(at, Date.UTC(2026, 8, 13, 21));

  inZone("UTC", () => {
    assert.equal(localDayKey(at), "2026-09-13");
    assert.equal(localWeekKey(at), "2026-09-07", "in UTC it is still last week");
  });

  inZone("Asia/Istanbul", () => {
    assert.equal(localDayKey(at), "2026-09-14", "+03:00 puts 21:00Z into the next day");
    assert.equal(localWeekKey(at), "2026-09-14", "and that day is a Monday, so it opens a week");
  });
});

test("an offset that is not a whole hour keeps the bucket in the day it starts in", () => {
  // +05:30: the hour beginning 18:00Z runs from 23:30 to 00:30 local, so half of it happened
  // on the next day. It counts towards the day it starts in rather than being cut by a ratio,
  // which is the one honest limit `docs/usage-contract.md` names.
  const at = hourStart("2026-09-13T18");
  inZone("Asia/Kolkata", () => {
    assert.equal(localDayKey(at), "2026-09-13");
  });
});

test("a bucket key that is not an hour is skipped rather than guessed at", () => {
  assert.equal(hourStart("2026-09-13"), undefined, "a day is not an hour");
  assert.equal(hourStart("2026-09-13T21:00"), undefined, "an hour carries no minutes");
  assert.equal(hourStart("2026-09-13T21Z"), undefined, "an hour is not an instant");
  assert.equal(hourStart("2026-13-01T00"), undefined, "there is no thirteenth month");
  assert.equal(hourStart("2026-04-31T00"), undefined, "April has thirty days");
  assert.equal(hourStart("2026-09-13T24"), undefined, "a day has hours 00 to 23");
});

// ------------------------------------------------------------- the local week

test("a week starts on the local Monday, whatever day it is asked on", () => {
  inZone("UTC", () => {
    const monday = "2026-09-14";
    for (const day of ["14", "15", "16", "17", "18", "19", "20"]) {
      const at = new Date(`2026-09-${day}T13:00:00Z`);
      assert.equal(
        localDayKey(startOfLocalWeek(at).getTime()),
        monday,
        `${day} September belongs to the week of ${monday}`,
      );
    }
    // Sunday is the end of the week, not the start of the next one.
    const next = startOfLocalWeek(new Date("2026-09-21T13:00:00Z"));
    assert.equal(localDayKey(next.getTime()), "2026-09-21");
  });
});

test("a week that crosses a year boundary keeps its Monday in the old year", () => {
  inZone("UTC", () => {
    // 1 January 2027 is a Friday; its week began on 28 December 2026.
    const start = startOfLocalWeek(new Date("2027-01-01T10:00:00Z"));
    assert.equal(localDayKey(start.getTime()), "2026-12-28");
    assert.equal(start.getFullYear(), 2026);
  });
});

test("the days of a week survive a daylight-saving change", () => {
  // Europe/Berlin springs forward on Sunday 29 March 2026: that week is 167 hours long, so a
  // strip built by adding 86 400 000 ms seven times would name Sunday twice and lose Monday.
  inZone("Europe/Berlin", () => {
    const now = new Date("2026-03-29T10:00:00Z");
    assert.equal(localDayKey(startOfLocalWeek(now).getTime()), "2026-03-23");

    const view = buildUsageView(answer({ range: "week", providers: {} }), now);
    assert.deepEqual(
      view.bars.map((bar) => bar.key),
      [
        "2026-03-23",
        "2026-03-24",
        "2026-03-25",
        "2026-03-26",
        "2026-03-27",
        "2026-03-28",
        "2026-03-29",
      ],
      "seven days, each named once",
    );
  });
});

test("the window the panel asks for is local, and carries its own offset", () => {
  inZone("Asia/Istanbul", () => {
    const now = new Date("2026-09-16T09:00:00Z"); // Wednesday, 12:00 local
    assert.equal(rfc3339(now), "2026-09-16T12:00:00+03:00");

    const week = usageWindow("week", now);
    assert.equal(week.from, "2026-09-14T00:00:00+03:00", "Monday midnight, locally");
    assert.equal(week.to, "2026-09-16T12:00:00+03:00");

    const month = usageWindow("month", now);
    assert.equal(month.from, "2026-09-01T00:00:00+03:00");

    // All time asks from a floor rather than guessing where the store begins; the answer's
    // own `since` is what the footer then says.
    assert.equal(usageWindow("all", now).from, ALL_TIME_FLOOR);
  });
});

// ------------------------------------------------------------------ counters

test("the headline is all four counters, the way /usage counts a total", () => {
  // T-WP16 answered 25 173 here, leaving the 1 988 416 cache reads out. Next to `/usage`'s
  // own *total tokens* that is a tray reporting 1.2 % of the number it is being compared
  // with, which reads as a broken tray rather than as a careful one.
  assert.equal(
    bucketTotal({ input: 2, output: 328, cache_create: 24843, cache_read: 1988416, requests: 14 }),
    2013589,
  );
  assert.equal(
    bucketTotal({ cache_read: 1988416, requests: 14 }),
    1988416,
    "a bucket that is nothing but cache reads still has a total",
  );
  assert.equal(bucketTotal({ input: 1, requests: 1 }), 1, "one counter is enough to know one");
  assert.equal(bucketTotal({ requests: 14 }), undefined, "and none of them is not a zero");
});

test("the breakdown names all four, in order, and prints an absence as one", () => {
  // The numbers are `formatTokens`' — three significant digits and no decimal past 100, the
  // rule the headline has always used — so this is `/usage`'s line in this product's spelling
  // rather than a second number format invented for one row.
  const line = partsLine(
    { input: 511_100, output: 34_700_000, cacheRead: 13_300_000_000, cacheCreate: 379_100_000 },
    "en",
    en,
  );
  assert.equal(line, "In 511K · Out 34.7M · Cache read 13.3B · Cache write 379M");

  // Never dropped, even when nobody reported it: a breakdown missing a counter would not add
  // up to the headline above it, and the reader would have to work out which one went.
  assert.equal(
    partsLine({ input: 40, output: 60, cacheRead: 0, cacheCreate: undefined }, "en", en),
    `In 40 · Out 60 · Cache read 0 · Cache write ${ABSENT}`,
  );

  // The words are the language's and the numbers are `Intl`'s, on one line either way.
  const tr = createTranslator(catalogs, "tr");
  assert.equal(
    partsLine({ input: 1000, output: 2000, cacheRead: 3000, cacheCreate: 4000 }, "tr", tr).replace(
      /\s/g,
      " ",
    ),
    "Giriş 1 B · Çıkış 2 B · Önbellek okuma 3 B · Önbellek yazma 4 B",
  );
});

test("a counter that is absent stays absent; a counter that is zero is a measurement", () => {
  // Codex reports `cache_create: 0` on every event, which is a number somebody measured.
  assert.equal(bucketTotal({ input: 10, output: 5, cache_create: 0, requests: 1 }), 15);
  assert.equal(formatTokens(0, "en"), "0", "a measured zero is a zero");
  assert.equal(formatTokens(undefined, "en"), ABSENT, "an absent counter is a word, not a zero");

  const view = buildUsageView(
    answer({
      providers: {
        claude: { "2026-09-14T09": { "claude-sonnet-4-5": { requests: 3 } } },
        codex: {
          "2026-09-14T10": {
            "gpt-5.6-sol": { input: 40, output: 60, cache_create: 0, cache_read: 0, requests: 4 },
          },
        },
      },
    }),
    new Date("2026-09-16T09:00:00Z"),
  );
  const [codex, claude] = view.rows;
  assert.equal(claude.total, undefined, "a bucket of nothing but requests totals nothing");
  assert.equal(formatTokens(claude.total, "en"), ABSENT);
  assert.equal(claude.share, 0, "and it draws an empty track rather than a bar");
  assert.deepEqual(claude.parts, {
    input: undefined,
    output: undefined,
    cacheRead: undefined,
    cacheCreate: undefined,
  });
  assert.equal(codex.total, 100);
  assert.equal(codex.parts.cacheCreate, 0, "a zero that was reported is kept");
  assert.equal(codex.parts.cacheRead, 0);
  assert.equal(formatTokens(codex.parts.cacheCreate, "en"), "0");
});

test("a token count is floored to the precision it is shown at, never rounded up", () => {
  assert.equal(formatTokens(22_399_999, "en"), "22.3M", "22.39 M has not reached 22.4 M");
  assert.equal(formatTokens(1_599_999_999, "en"), "1.5B");
  assert.equal(formatTokens(999, "en"), "999", "under a thousand is printed as itself");
  assert.equal(formatTokens(4_899, "en"), "4.8K");
  assert.equal(formatTokens(220_400_000, "en"), "220M", "three digits need no decimal");
  assert.equal(formatTokens(-5, "en"), "0", "a count cannot be negative");
});

test("the magnitude mark is the language's own, and never an English letter", () => {
  // `Intl` picks the mark, which is why no locale file has to carry K, M or B — and why a
  // Korean panel says 만 rather than M. Every one of them is an abbreviation, which is the
  // property that lets this product ship without plural forms.
  //
  // The spaces are normalised before comparing: several of these languages put a
  // *non-breaking* one between the number and the mark, which is right on screen and
  // invisible in a diff.
  const spelled = (value, locale) => formatTokens(value, locale).replace(/\s+/g, " ");
  assert.equal(spelled(22_300_000, "en"), "22.3M");
  assert.equal(spelled(22_300_000, "tr"), "22,3 Mn");
  assert.equal(spelled(22_300_000, "ru"), "22,3 млн");
  assert.equal(spelled(22_300_000, "zh"), "2230万");
  assert.equal(spelled(22_300_000, "ko"), "2230만");
  assert.equal(spelled(22_300_000, "es"), "22,3 M");
  assert.equal(formatNumber(1204, "en"), "1,204");
  assert.equal(formatNumber(1204, "tr"), "1.204", "the grouping follows the language too");
});

// ------------------------------------------------------------------- errors

test("every error kind the command can send has a sentence, and an unknown one still does", () => {
  for (const kind of [
    "bad_range",
    "bad_window",
    "no_state_dir",
    "scan_failed",
    "store_unreadable",
  ]) {
    const key = usageErrorKey(kind);
    assert.equal(key, `usage.error.${kind}`);
    assert.ok(key in english, `en.json has no ${key}`);
  }
  // A kind this build has not met — a sixth one added on the Rust side — gets the general
  // sentence rather than a message key printed at the user.
  assert.equal(usageErrorKey("cursor_ate_it"), "usage.error.unknown");
  assert.equal(usageErrorKey(""), "usage.error.unknown");
  assert.ok("usage.error.unknown" in english);
});

test("only the command's own error shape is read as an error", () => {
  assert.ok(isUsageError({ kind: "scan_failed", detail: "~/.claude/projects: permission denied" }));
  assert.ok(!isUsageError("the window went away"), "a string rejection is not a kind");
  assert.ok(!isUsageError(null));
  assert.ok(!isUsageError({ kind: "scan_failed" }), "a kind with no detail is not the shape");
});

// -------------------------------------------------------------------- labels

test("a Codex row counts events and a Claude row counts requests", () => {
  // `requests` is records that carried usage, and a record is not the same thing on both
  // sides: T-WP14 counts `token_count` events, several of which make one turn.
  assert.equal(requestsKey("claude"), "usage.row.requests");
  assert.equal(requestsKey("codex"), "usage.row.events");
  assert.equal(requestsKey("something-new"), "usage.row.requests");
  for (const key of ["usage.row.requests", "usage.row.events"]) assert.ok(key in english);
});

// -------------------------------------------------------------- the whole view

/** A `get_usage` answer with the fields a test does not care about filled in. */
function answer(fields) {
  return {
    range: "week",
    from: "2026-09-14T00:00:00Z",
    to: "2026-09-16T09:00:00Z",
    damaged: [],
    providers: {},
    ...fields,
  };
}

/** One week of work on this machine, in the shape `get_usage` sends it. */
const FIXTURE = answer({
  range: "week",
  since: "2026-09-08T12:00:00Z",
  scanned_at: "2026-09-16T08:00:00Z",
  damaged: ["2026-07"],
  providers: {
    claude: {
      // Sunday 21:00Z — Monday 00:00 in Istanbul, and the first hour of the local week.
      "2026-09-13T21": {
        "claude-opus-5": {
          input: 1000,
          output: 2000,
          cache_create: 3000,
          cache_read: 4000,
          requests: 10,
        },
      },
      "2026-09-14T09": {
        "claude-opus-5": {
          input: 500_000,
          output: 250_000,
          cache_create: 250_000,
          cache_read: 1_000_000_000,
          requests: 40,
        },
        "claude-haiku-4-5": {
          input: 1_000,
          output: 2_000,
          cache_create: 0,
          cache_read: 5_000,
          requests: 7,
        },
      },
      // A record that carried no counters at all: three requests and nothing to add up.
      "2026-09-15T07": { "claude-sonnet-4-5": { requests: 3 } },
    },
    codex: {
      "2026-09-16T05": {
        "gpt-5.6-sol": {
          input: 300_000,
          output: 100_000,
          cache_create: 0,
          cache_read: 2_000_000,
          requests: 25,
        },
      },
    },
  },
});

test("the week view adds up, sorts and groups the way the panel draws it", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView(FIXTURE, new Date("2026-09-16T09:00:00Z"));

    assert.equal(view.total, 1_003_418_000, "all four counters over both providers");
    assert.deepEqual(view.parts, {
      input: 802_000,
      output: 354_000,
      cacheRead: 1_002_009_000,
      cacheCreate: 253_000,
    });
    assert.equal(
      view.parts.input + view.parts.output + view.parts.cacheRead + view.parts.cacheCreate,
      view.total,
      "the breakdown is the headline taken apart, not four numbers beside it",
    );
    assert.equal(view.requests, 85);
    assert.equal(view.empty, false);
    assert.equal(view.grouped, true, "two providers, so the headings earn their line");

    assert.deepEqual(
      view.rows.map((row) => [row.provider, row.model, row.total]),
      [
        ["claude", "claude-opus-5", 1_001_010_000],
        ["codex", "gpt-5.6-sol", 2_400_000],
        ["claude", "claude-haiku-4-5", 8_000],
        ["claude", "claude-sonnet-4-5", undefined],
      ],
      "biggest first, and a model nobody could total sorts last rather than as a zero",
    );

    const opus = view.rows[0];
    assert.equal(opus.share, 100, "the largest row fills its track");
    assert.equal(opus.requests, 50, "one model's hours add up across the window");
    assert.deepEqual(opus.parts, {
      input: 501_000,
      output: 252_000,
      cacheRead: 1_000_004_000,
      cacheCreate: 253_000,
    });

    assert.deepEqual(
      view.groups.map((group) => [group.provider, group.total]),
      [
        ["claude", 1_001_018_000],
        ["codex", 2_400_000],
      ],
    );
    assert.equal(view.groups[0].rows.length, 3, "a provider's rows stay under its heading");
  });
});

test("the strip is one column per local day, dense, stopping at today", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView(FIXTURE, new Date("2026-09-16T09:00:00Z"));
    assert.deepEqual(
      view.bars.map((bar) => [bar.key, bar.total, bar.topModel]),
      [
        // The Sunday 21:00Z bucket is in here, not in last week.
        ["2026-09-14", 1_001_018_000, "claude-opus-5"],
        ["2026-09-15", 0, undefined],
        ["2026-09-16", 2_400_000, "gpt-5.6-sol"],
      ],
      "Tuesday recorded requests but no counters, which is not tokens",
    );
    assert.equal(view.bars[0].height, 100, "the tallest column is the scale");
    assert.equal(view.bars[1].height, 0, "an empty day is an empty track, not a stub");
    assert.ok(view.bars[2].height > 0 && view.bars[2].height < 100);
    assert.deepEqual(
      view.bars.map((bar) => bar.level),
      [4, 0, 1],
      "and the same three days shade the same way as they would in the calendar",
    );
  });
});

test("all time is a calendar of local days, and says where the store begins", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView({ ...FIXTURE, range: "all" }, new Date("2026-09-16T09:00:00Z"));
    assert.equal(view.since, "2026-09-08T12:00:00Z");
    assert.deepEqual(view.bars, [], "the strip belongs to the week tab alone");

    // `since` is a Tuesday, so the first column opens with two holes; today is a Wednesday,
    // so the last one closes with four.
    const [first, second] = view.grid.columns;
    assert.equal(view.grid.columns.length, 2);
    assert.deepEqual(
      first.days.map((cell) => [cell.key, cell.present]),
      [
        ["", false],
        ["2026-09-08", true],
        ["2026-09-09", true],
        ["2026-09-10", true],
        ["2026-09-11", true],
        ["2026-09-12", true],
        ["2026-09-13", true],
      ],
      "the store began on the Tuesday, so its Monday is not a day with no work on it",
    );
    assert.deepEqual(
      second.days.map((cell) => [cell.key, cell.total]),
      [
        // The Sunday 21:00Z bucket is on this Monday, because that is where its hour starts.
        ["2026-09-14", 1_001_018_000],
        ["2026-09-15", 0],
        ["2026-09-16", 2_400_000],
        ["", 0],
        ["", 0],
        ["", 0],
        ["", 0],
      ],
    );
    assert.deepEqual(second.days.slice(3).map((cell) => cell.present), [false, false, false, false]);
    assert.equal(view.grid.busiest, 1_001_018_000);

    // One month in the grid, one label, over the column its first drawn day falls in.
    assert.deepEqual(view.grid.months, [{ column: 0, at: first.days[1].at }]);
    assert.equal(formatMonth(view.grid.months[0].at, "en"), "Sep");

    assert.equal(formatColumn(view.since, "en"), "Sep 8");
    assert.equal(formatDay(view.since, "en"), "Sep 8, 2026");
    assert.equal(formatDay(undefined, "en"), ABSENT, "a store with no floor says so");
  });
});

// ---------------------------------------------------------------- the calendar

test("a month that opens on a Wednesday opens with two holes", () => {
  inZone("Asia/Istanbul", () => {
    // 1 July 2026 is a Wednesday and the month is 31 days, so it is five columns wide and the
    // last one ends on a Friday.
    const view = buildUsageView(
      answer({ range: "month", providers: {} }),
      new Date("2026-07-31T10:00:00Z"),
    );
    const { columns } = view.grid;
    assert.equal(columns.length, 5, "five Mondays touch July 2026");

    assert.deepEqual(
      columns[0].days.map((cell) => cell.present),
      [false, false, true, true, true, true, true],
      "Monday 29 and Tuesday 30 June are not days of this month",
    );
    assert.equal(columns[0].days[2].key, "2026-07-01");
    assert.equal(columns[0].days[0].key, "", "a hole has no day to name");
    assert.equal(columns[0].days[0].at, 0, "and no instant to hover");
    assert.equal(columns[0].days[0].level, 0);

    // The month is drawn whole, so the last column closes with the two days of August it
    // shares a week with.
    assert.deepEqual(
      columns[4].days.map((cell) => cell.present),
      [true, true, true, true, true, false, false],
    );
    assert.equal(columns[4].days[4].key, "2026-07-31");
    assert.deepEqual(view.grid.months, [{ column: 0, at: columns[0].days[2].at }]);
  });
});

test("a month that needs six columns gets six", () => {
  inZone("Asia/Istanbul", () => {
    // 1 August 2026 is a Saturday and August is 31 days, so the 31st is a Monday of its own
    // column: the six-week shape a five-column grid would silently truncate.
    const view = buildUsageView(
      answer({ range: "month", providers: {} }),
      new Date("2026-08-31T10:00:00Z"),
    );
    const { columns } = view.grid;
    assert.equal(columns.length, 6);
    assert.deepEqual(
      columns[0].days.map((cell) => cell.present),
      [false, false, false, false, false, true, true],
      "only the Saturday and the Sunday of that week are in August",
    );
    assert.equal(columns[0].days[5].key, "2026-08-01");
    assert.deepEqual(
      columns[5].days.map((cell) => cell.present),
      [true, false, false, false, false, false, false],
      "and the last column is the 31st alone",
    );
    assert.equal(columns[5].days[0].key, "2026-08-31");
  });
});

test("a month still being lived in draws the days that have not happened as holes", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView(FIXTURE, new Date("2026-09-16T09:00:00Z"));
    const month = buildUsageView({ ...FIXTURE, range: "month" }, new Date("2026-09-16T09:00:00Z"));
    assert.equal(view.grid.columns.length, 1, "the week tab's calendar is one column");

    const { columns } = month.grid;
    // September 2026 opens on a Tuesday and runs to the 30th, a Wednesday: five columns.
    assert.equal(columns.length, 5);
    assert.equal(columns[0].days[1].key, "2026-09-01");
    const days = columns.flatMap((column) => column.days).filter((cell) => cell.present);
    assert.equal(days.length, 16, "the 1st to the 16th, and not a day further");
    assert.equal(days[days.length - 1].key, "2026-09-16");
  });
});

test("a year boundary inside one column is two months and two years", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView(
      answer({
        range: "all",
        since: "2026-12-30T09:00:00Z",
        providers: {
          claude: {
            "2026-12-31T20": { "claude-opus-5": { input: 10, output: 10, requests: 1 } },
            "2027-01-04T08": { "claude-opus-5": { input: 100, output: 100, requests: 1 } },
          },
        },
      }),
      new Date("2027-01-05T10:00:00Z"),
    );
    const { columns, months } = view.grid;
    assert.equal(columns.length, 2, "28 December 2026 and 4 January 2027");
    assert.deepEqual(
      columns[0].days.map((cell) => cell.key),
      ["", "", "2026-12-30", "2026-12-31", "2027-01-01", "2027-01-02", "2027-01-03"],
      "one column holds the last two days of one year and the first three of the next",
    );
    // 20:00Z on 31 December is 23:00 in Istanbul, which is still the 31st.
    assert.equal(columns[0].days[3].total, 20);
    assert.equal(columns[1].days[0].key, "2027-01-04");
    assert.equal(columns[1].days[0].total, 200);

    assert.deepEqual(
      months.map((month) => [month.column, formatMonth(month.at, "en")]),
      [
        [0, "Dec"],
        [1, "Jan"],
      ],
      "a label goes over the column whose first drawn day opens a new month",
    );
  });
});

test("the shading is quarters of the busiest day, and the busiest day is always the darkest", () => {
  // Max-relative rather than quantile, and `ui/src/usage.ts` says why at length: quartiles
  // over three active days cannot fill four buckets, so the darkest shade in the legend would
  // never be drawn — which reads as a bug rather than as a distribution.
  assert.equal(HEAT_LEVELS, 4);
  assert.equal(heatLevel(0, 100), 0, "a day with nothing on it is not a shade of anything");
  assert.equal(heatLevel(1, 100), 1);
  assert.equal(heatLevel(24, 100), 1);
  assert.equal(heatLevel(25, 100), 2, "a quarter is where the second shade begins");
  assert.equal(heatLevel(49, 100), 2);
  assert.equal(heatLevel(50, 100), 3);
  assert.equal(heatLevel(74, 100), 3);
  assert.equal(heatLevel(75, 100), 4);
  assert.equal(heatLevel(100, 100), HEAT_LEVELS, "the busiest day fills the scale");
  assert.equal(heatLevel(5, 0), 0, "a grid with nothing in it shades nothing");

  inZone("Asia/Istanbul", () => {
    // Three active days, which is the case a quantile scale cannot shade: 1, 40 and 100 parts
    // of the same week.
    const view = buildUsageView(
      answer({
        providers: {
          claude: {
            "2026-09-14T09": { opus: { input: 100, requests: 1 } },
            "2026-09-15T09": { opus: { input: 40, requests: 1 } },
            "2026-09-16T09": { opus: { input: 1, requests: 1 } },
          },
        },
      }),
      new Date("2026-09-16T15:00:00Z"),
    );
    assert.deepEqual(
      view.bars.map((bar) => bar.level),
      [4, 2, 1],
    );
  });
});

test("an answer with nothing in it is empty rather than a page of zeroes", () => {
  inZone("Asia/Istanbul", () => {
    const empty = answer({ providers: { claude: {} } });
    const view = buildUsageView(empty, new Date("2026-09-16T09:00:00Z"));
    assert.equal(view.empty, true);
    assert.equal(view.total, undefined);
    assert.deepEqual(view.rows, []);
    assert.equal(view.grouped, false);
    assert.equal(formatTokens(view.total, "en"), ABSENT);
  });
});

test("a day says its date, its total and the model that led it", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView(FIXTURE, new Date("2026-09-16T09:00:00Z"));
    const [monday, tuesday, wednesday] = view.bars;

    assert.equal(cellLine(monday, "en", en), "Sep 14, 2026 · 1B · claude-opus-5");
    assert.equal(
      cellLine(wednesday, "en", en),
      "Sep 16, 2026 · 2.4M · gpt-5.6-sol",
      "the model that spent most of the day, not the one the rows begin with",
    );
    // Not `0`: a zero and a silence look the same on a coloured square, and only one of them
    // is what happened.
    assert.equal(cellLine(tuesday, "en", en), "Sep 15, 2026 · nothing recorded");

    // A cell outside the range never reaches this function in the panel — it is not
    // inspectable — but if one did it would say the same thing as an empty day rather than
    // printing an instant of zero.
    assert.equal(
      cellLine({ at: 0, total: 0, topModel: undefined }, "en", en),
      `${formatDay(0, "en")} · nothing recorded`,
    );

    const tr = createTranslator(catalogs, "tr");
    assert.equal(cellLine(tuesday, "tr", tr), "15 Eyl 2026 · kayıt yok");
  });
});

test("the footer says where the history begins and how old it is, on one line", () => {
  inZone("Asia/Istanbul", () => {
    const now = Date.parse("2026-09-16T09:00:00Z");
    const week = buildUsageView(FIXTURE, new Date(now));
    const all = buildUsageView({ ...FIXTURE, range: "all" }, new Date(now));

    assert.equal(
      footerLine(all, "en", en, now),
      "Since Sep 8 · scanned 1 h 0 m ago",
      "the date is short because the line is read, not parsed",
    );
    assert.equal(
      footerLine(week, "en", en, now),
      "scanned 1 h 0 m ago",
      "over a tab headed Week, a store's own floor would read as a claim about the week",
    );
    assert.equal(
      footerLine({ ...week, scannedAt: undefined }, "en", en, now),
      "",
      "a store nobody has scanned says nothing rather than saying NaN",
    );

    const tr = createTranslator(catalogs, "tr");
    assert.equal(footerLine(all, "tr", tr, now), "8 Eyl tarihinden beri · 1 sa 0 dk önce tarandı");
  });
});

test("the view reads the way it is meant to, line by line, in English", () => {
  inZone("Asia/Istanbul", () => {
    const now = new Date("2026-09-16T09:00:00Z");
    const view = buildUsageView(FIXTURE, now);
    const lines = [];

    lines.push(`${formatTokens(view.total, "en")} ${en("usage.tokens")}`);
    lines.push(partsLine(view.parts, "en", en));
    lines.push(footerLine(view, "en", en, now.getTime()));
    for (const group of view.groups) {
      if (view.grouped) {
        lines.push(`${en(`panel.provider.${group.provider}`)} ${formatTokens(group.total, "en")}`);
      }
      for (const row of group.rows) {
        lines.push(`${row.model} ${formatTokens(row.total, "en")}`);
        lines.push(
          `${partsLine(row.parts, "en", en)} ${en(requestsKey(row.provider), {
            requests: formatNumber(row.requests, "en"),
          })}`,
        );
      }
    }
    lines.push(en("usage.damaged", { months: view.damaged.join(", ") }));

    assert.deepEqual(lines, [
      "1B tokens",
      "In 802K · Out 354K · Cache read 1B · Cache write 253K",
      "scanned 1 h 0 m ago",
      "Claude Code 1B",
      "claude-opus-5 1B",
      "In 501K · Out 252K · Cache read 1B · Cache write 253K 50 req.",
      "claude-haiku-4-5 8K",
      "In 1K · Out 2K · Cache read 5K · Cache write 0 7 req.",
      "claude-sonnet-4-5 —",
      "In — · Out — · Cache read — · Cache write — 3 req.",
      "Codex 2.4M",
      "gpt-5.6-sol 2.4M",
      "In 300K · Out 100K · Cache read 2M · Cache write 0 25 evt.",
      "Unreadable, left out of these numbers: 2026-07",
    ]);
  });
});

// --------------------------------------------------------------- T-WP21: weeks

test("the weeks list is dense, newest first, and says which week is still being lived in", () => {
  inZone("Asia/Istanbul", () => {
    // Over *all time* the store's own floor is 8 September, a Tuesday, so the list opens on
    // the week that holds it — and the week between that Monday and the work is a week with
    // nothing in it rather than a week missing from the list.
    const view = buildUsageView({ ...FIXTURE, range: "all" }, new Date("2026-09-16T09:00:00Z"));

    assert.deepEqual(
      view.weeks.map((week) => [week.key, week.total]),
      [
        ["2026-09-14", 1_003_418_000],
        ["2026-09-07", undefined],
      ],
      "newest first, and a week nobody recorded anything in is absent rather than zero",
    );

    const [current, quiet] = view.weeks;
    assert.equal(current.current, true, "16 September is a Wednesday in the week of the 14th");
    assert.equal(quiet.current, false);
    assert.equal(formatTokens(quiet.total, "en"), ABSENT, "a silent week prints an em dash");
    assert.equal(quiet.requests, 0);

    // A part week is still a whole calendar week: the row runs to its Sunday even though
    // three of its days have not happened, because that is what the row is a row of.
    assert.equal(weekLabel(current, "en"), "Sep 14 – Sep 20");
    assert.equal(new Date(current.end).getDay(), 0, "the second end of the label is a Sunday");
    assert.deepEqual(current.parts, {
      input: 802_000,
      output: 354_000,
      cacheRead: 1_002_009_000,
      cacheCreate: 253_000,
    });
    assert.equal(current.requests, 85);
    assert.equal(current.share, 100, "the busiest week fills its track");
    assert.equal(quiet.share, 0);

    // The list and the heat-map are cut from the same bounds, so a week counted in one is a
    // column drawn in the other.
    assert.equal(view.weeks.length, view.grid.columns.length);
  });
});

test("a weeks list across a year boundary is two weeks, and neither loses its Monday", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView(
      answer({
        range: "all",
        since: "2026-12-30T09:00:00Z",
        providers: {
          claude: {
            "2026-12-31T20": { "claude-opus-5": { input: 10, output: 10, requests: 1 } },
            "2027-01-04T08": { "claude-opus-5": { input: 100, output: 100, requests: 1 } },
          },
        },
      }),
      new Date("2027-01-05T10:00:00Z"),
    );

    assert.deepEqual(
      view.weeks.map((week) => [week.key, week.total]),
      [
        ["2027-01-04", 200],
        ["2026-12-28", 20],
      ],
    );
    // The week that opens the new year is the one being lived in; the one before it ends on
    // 3 January, so its label crosses two months and says so without naming a year.
    assert.equal(view.weeks[0].current, true);
    assert.equal(weekLabel(view.weeks[1], "en"), "Dec 28 – Jan 3");
    assert.equal(new Date(view.weeks[1].start).getFullYear(), 2026);
    assert.equal(new Date(view.weeks[1].end).getFullYear(), 2027);
  });
});

test("the weeks list only ever holds the weeks the calendar is drawn over", () => {
  inZone("Asia/Istanbul", () => {
    // The *Week* tab's own answer covers one week, so the list is that one week: the same
    // bounds, at a different width.
    const view = buildUsageView(FIXTURE, new Date("2026-09-16T09:00:00Z"));
    assert.equal(view.weeks.length, 1);
    assert.equal(view.weeks[0].key, "2026-09-14");
    assert.equal(view.weeks[0].total, view.total, "one week is the whole of a week's answer");
  });
});

// -------------------------------------------------------------- T-WP21: detail

test("a week opens into its models, and they add up to the row that opened them", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView(FIXTURE, new Date("2026-09-16T09:00:00Z"));
    const detail = buildUsageDetail(FIXTURE, {
      kind: "week",
      key: "2026-09-14",
      at: Date.parse("2026-09-14T00:00:00+03:00"),
    });

    assert.equal(detail.empty, false);
    assert.equal(detail.total, view.weeks[0].total, "the detail is the row taken apart");
    assert.deepEqual(detail.parts, view.weeks[0].parts);
    assert.equal(detail.requests, 85);
    assert.equal(detail.grouped, true, "both providers worked that week");
    assert.deepEqual(
      detail.rows.map((row) => [row.provider, row.model, row.total]),
      [
        ["claude", "claude-opus-5", 1_001_010_000],
        ["codex", "gpt-5.6-sol", 2_400_000],
        ["claude", "claude-haiku-4-5", 8_000],
        ["claude", "claude-sonnet-4-5", undefined],
      ],
    );
    assert.deepEqual(
      detail.groups.map((group) => [group.provider, group.total]),
      [
        ["claude", 1_001_018_000],
        ["codex", 2_400_000],
      ],
    );
  });
});

test("a day opens into the models of that day alone", () => {
  inZone("Asia/Istanbul", () => {
    const monday = buildUsageDetail(FIXTURE, { kind: "day", key: "2026-09-14", at: 0 });
    // 21:00Z on Sunday is Monday 00:00 in Istanbul, so that hour is counted here — the same
    // rule the strip and the heat-map are cut with.
    assert.equal(monday.total, 1_001_018_000);
    assert.equal(monday.requests, 57);
    assert.equal(monday.grouped, false, "no Codex work on the Monday, so no headings");
    assert.deepEqual(
      monday.rows.map((row) => [row.model, row.total, row.requests]),
      [
        ["claude-opus-5", 1_001_010_000, 50],
        ["claude-haiku-4-5", 8_000, 7],
      ],
    );
    // Floored, never rounded up, which is why these two do not add to 100.
    assert.deepEqual(
      monday.rows.map((row) => row.percent),
      [99, 0],
    );

    const wednesday = buildUsageDetail(FIXTURE, { kind: "day", key: "2026-09-16", at: 0 });
    assert.equal(wednesday.total, 2_400_000);
    assert.deepEqual(
      wednesday.rows.map((row) => [row.provider, row.model]),
      [["codex", "gpt-5.6-sol"]],
    );

    // A day whose only record carried no counters: one row, one request count, and an
    // absence rather than a zero at every grain of it.
    const tuesday = buildUsageDetail(FIXTURE, { kind: "day", key: "2026-09-15", at: 0 });
    assert.equal(tuesday.empty, false, "three requests happened; nobody said what they cost");
    assert.equal(tuesday.total, undefined);
    assert.equal(tuesday.rows[0].percent, undefined);
    assert.equal(formatTokens(tuesday.total, "en"), ABSENT);

    const nothing = buildUsageDetail(FIXTURE, { kind: "day", key: "2026-09-11", at: 0 });
    assert.equal(nothing.empty, true);
    assert.deepEqual(nothing.rows, []);
  });
});

test("a detail says what it is a detail of", () => {
  inZone("Asia/Istanbul", () => {
    const at = Date.parse("2026-09-14T00:00:00+03:00");
    assert.equal(detailTitle({ kind: "day", key: "2026-09-14", at }, "en", en), "Sep 14, 2026");
    assert.equal(
      detailTitle({ kind: "week", key: "2026-09-14", at }, "en", en),
      "Week of Sep 14, 2026",
    );
    const tr = createTranslator(catalogs, "tr");
    assert.equal(
      detailTitle({ kind: "week", key: "2026-09-14", at }, "tr", tr),
      "14 Eyl 2026 haftası",
    );
  });
});

// -------------------------------------------------------------- T-WP21: models

test("the chart is one line per model per local day, with a silent day drawn as zero", () => {
  inZone("Asia/Istanbul", () => {
    const chart = buildUsageChart(FIXTURE, "days7", new Date("2026-09-16T09:00:00Z"));

    assert.equal(chart.days.length, 7, "seven days, counting today");
    assert.deepEqual(
      chart.days.map((day) => day.key),
      [
        "2026-09-10",
        "2026-09-11",
        "2026-09-12",
        "2026-09-13",
        "2026-09-14",
        "2026-09-15",
        "2026-09-16",
      ],
    );

    assert.deepEqual(
      chart.series.map((series) => [series.provider, series.model, series.total]),
      [
        ["claude", "claude-opus-5", 1_001_010_000],
        ["codex", "gpt-5.6-sol", 2_400_000],
        ["claude", "claude-haiku-4-5", 8_000],
      ],
      "biggest first, and a model that spent nothing in the span is not a line",
    );

    // A gap is a zero, not a break: a line that skipped the Tuesday would draw a segment
    // straight through a day it is silent about.
    assert.deepEqual(chart.series[0].points, [0, 0, 0, 0, 1_001_010_000, 0, 0]);
    assert.deepEqual(chart.series[1].points, [0, 0, 0, 0, 0, 0, 2_400_000]);
    assert.equal(
      chart.series.every((series) => series.points.length === chart.days.length),
      true,
      "every line has a point for every day",
    );

    assert.equal(chart.peak, 1_001_010_000);
    assert.equal(chart.models, 3);
    assert.equal(chart.empty, false);

    // The list under the chart is the span's, not the window's — and it still has a row for
    // the model nobody could total, which has no line.
    assert.equal(chart.totals.total, 1_003_418_000);
    assert.deepEqual(
      chart.totals.rows.map((row) => row.model),
      ["claude-opus-5", "gpt-5.6-sol", "claude-haiku-4-5", "claude-sonnet-4-5"],
    );
    assert.equal(chart.totals.grouped, true);
  });
});

test("the span selector changes the days and nothing else about the answer", () => {
  inZone("Asia/Istanbul", () => {
    const now = new Date("2026-09-16T09:00:00Z");
    const week = buildUsageChart(FIXTURE, "days7", now);
    const month = buildUsageChart(FIXTURE, "days30", now);
    const all = buildUsageChart(FIXTURE, "all", now);

    assert.equal(month.days.length, 30);
    assert.equal(month.days[0].key, "2026-08-18");
    assert.equal(month.days[29].key, "2026-09-16");

    // *All time* is the store's own floor to today, which is what the `Since` line says and
    // what the calendar on *All* is drawn over.
    assert.equal(all.days.length, 9, "8 September to 16 September");
    assert.equal(all.days[0].key, "2026-09-08");

    // The same work, cut three ways: every span sees all of it here, so the totals agree.
    for (const chart of [week, month, all]) {
      assert.equal(chart.totals.total, 1_003_418_000);
      assert.equal(chart.peak, 1_001_010_000);
    }

    // A span that misses the work says so rather than drawing last week under this week's
    // heading: the days are drawn, the lines are not.
    const quiet = buildUsageChart(FIXTURE, "days7", new Date("2026-10-16T09:00:00Z"));
    assert.equal(quiet.days.length, 7);
    assert.deepEqual(quiet.series, []);
    assert.equal(quiet.peak, 0);
    assert.equal(quiet.empty, true);
    assert.equal(quiet.totals.total, undefined);
  });
});

test("the chart draws at most six lines, and the list under it draws all of them", () => {
  inZone("Asia/Istanbul", () => {
    // Eight models on one day. Six get a colour the reader can tell apart; the other two are
    // in the list underneath, which is the whole answer.
    const many = {};
    for (let index = 0; index < 8; index++) {
      many[`model-${index}`] = { input: (index + 1) * 1000, requests: 1 };
    }
    const chart = buildUsageChart(
      answer({ range: "all", providers: { claude: { "2026-09-14T09": many } } }),
      "days7",
      new Date("2026-09-16T09:00:00Z"),
    );

    assert.equal(CHART_SERIES, 6);
    assert.equal(chart.series.length, 6);
    assert.equal(chart.models, 8, "how many there were, drawn or not");
    assert.equal(chart.totals.rows.length, 8);
    assert.deepEqual(
      chart.series.map((series) => series.model),
      ["model-7", "model-6", "model-5", "model-4", "model-3", "model-2"],
      "the six biggest, biggest first",
    );
  });
});

test("a line is placed against the chart's own peak, and one day is a dot rather than a line", () => {
  const box = { left: 0, top: 0, width: 100, height: 100 };
  assert.equal(chartPoints([0, 50, 100], 100, box), "0,100 50,50 100,0");
  assert.equal(chartPoints([25, 25], 100, box), "0,75 100,75", "a flat line is flat");
  assert.equal(chartPoints([10], 100, box), "50,90", "one point sits in the middle of the box");
  assert.equal(
    chartPoints([0, 0, 0], 0, box),
    "0,100 50,100 100,100",
    "a span nobody spent anything in lies on the floor rather than dividing by zero",
  );
  assert.equal(chartPoints([], 100, box), "");

  // The axis: three dates along the bottom and three numbers up the side, whatever the span.
  assert.deepEqual(chartTicks(7), [0, 3, 6]);
  assert.deepEqual(chartTicks(30), [0, 14, 29]);
  assert.deepEqual(chartTicks(2), [0, 1]);
  assert.deepEqual(chartTicks(1), [0]);
  assert.deepEqual(chartTicks(0), []);
  assert.deepEqual(chartLevels(1_000_000), [1_000_000, 500_000, 0]);
  assert.deepEqual(chartLevels(0), [0], "a chart with nothing in it has one label, and it is 0");
});

// --------------------------------------------------------------- T-WP21: state

test("a tab is a window, a span and a detail are not", () => {
  assert.deepEqual([...USAGE_TABS], ["week", "weeks", "all", "models"]);
  assert.deepEqual([...USAGE_SPANS], ["all", "days7", "days30"]);
  assert.deepEqual(FIRST_USAGE_STATE, { tab: "week", span: "all", detail: undefined });

  assert.equal(tabRange("week"), "week");
  for (const tab of ["weeks", "all", "models"]) {
    assert.equal(tabRange(tab), "all", `${tab} reads the widest window there is`);
  }

  const first = FIRST_USAGE_STATE;
  const weeks = withTab(first, "weeks");
  assert.equal(needsFetch(first, weeks), true, "this week to all time is a different window");
  assert.equal(needsFetch(weeks, withTab(weeks, "all")), false);
  assert.equal(needsFetch(weeks, withTab(weeks, "models")), false);
  assert.equal(needsFetch(weeks, withSpan(weeks, "days30")), false, "a span asks for nothing");
  assert.equal(withSpan(weeks, "days30").span, "days30");
  assert.equal(withSpan(weeks, "days30").tab, "weeks", "and it moves nothing else");
});

test("clicking a day opens it, Back closes it, and a tab change closes it too", () => {
  const scope = { kind: "day", key: "2026-09-14", at: 1_757_800_000_000 };

  const list = withTab(FIRST_USAGE_STATE, "all");
  const open = openDetail(list, scope);
  assert.deepEqual(open.detail, scope);
  assert.equal(open.tab, "all", "a detail is opened on the tab it was opened from");
  assert.equal(needsFetch(list, open), false, "a day is a cut of the answer already in hand");

  const back = closeDetail(open);
  assert.equal(back.detail, undefined);
  assert.deepEqual(back, list, "Back is the list it was opened from, exactly");
  assert.equal(needsFetch(open, back), false);

  // A detail belongs to the list it came from, so a tab change puts it away rather than
  // leaving a day on screen that nothing on screen points at any more.
  assert.equal(withTab(open, "models").detail, undefined);
  assert.equal(withTab(open, "all").detail, undefined, "even the tab it was opened from");

  // A week opens the same way, and the two cannot be confused: the kind decides whether the
  // key is read as a local day or as a local Monday.
  const week = openDetail(list, { kind: "week", key: "2026-09-14", at: scope.at });
  assert.equal(week.detail.kind, "week");
  assert.equal(closeDetail(week).detail, undefined);

  // T-WP23 gave the view one *Back* that reads this field to decide what it means, so the
  // state has to answer for a list as safely as for a detail: closing what is not open is
  // the list it was already on, not a fifth state.
  assert.deepEqual(closeDetail(list), list, "closing nothing moves nothing");
  assert.equal(needsFetch(list, closeDetail(list)), false);
});

// ------------------------------------------------------- T-WP21: reading the view

test("the weeks list reads the way it is meant to, line by line, in English", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView({ ...FIXTURE, range: "all" }, new Date("2026-09-16T09:00:00Z"));
    const lines = view.weeks.flatMap((week) => [
      `${weekLabel(week, "en")} ${formatTokens(week.total, "en")}`,
      partsLine(week.parts, "en", en),
    ]);

    assert.deepEqual(lines, [
      "Sep 14 – Sep 20 1B",
      "In 802K · Out 354K · Cache read 1B · Cache write 253K",
      "Sep 7 – Sep 13 —",
      "In — · Out — · Cache read — · Cache write —",
    ]);
  });
});

test("a day's detail reads the way it is meant to, line by line, in English", () => {
  inZone("Asia/Istanbul", () => {
    const now = Date.parse("2026-09-16T09:00:00Z");
    const at = Date.parse("2026-09-14T00:00:00+03:00");
    const detail = buildUsageDetail(FIXTURE, { kind: "day", key: "2026-09-14", at });
    const lines = [detailTitle(detail.scope, "en", en)];

    // A detail keeps the freshness and loses the floor: `Since Sep 8` under a page headed
    // *Sep 14, 2026* would read as a claim about the day rather than about the store, which
    // is the same reason *since* has never been drawn under the Week tab. The panel drops it
    // by handing `footerLine` a view with no `since`, which is what this asserts.
    const all = buildUsageView({ ...FIXTURE, range: "all" }, new Date(now));
    assert.equal(footerLine(all, "en", en, now), "Since Sep 8 · scanned 1 h 0 m ago");
    assert.equal(footerLine({ ...all, since: undefined }, "en", en, now), "scanned 1 h 0 m ago");

    lines.push(`${formatTokens(detail.total, "en")} ${en("usage.tokens")}`);
    lines.push(partsLine(detail.parts, "en", en));
    for (const group of detail.groups) {
      if (detail.grouped) {
        lines.push(`${en(`panel.provider.${group.provider}`)} ${formatTokens(group.total, "en")}`);
      }
      for (const row of group.rows) {
        lines.push(`${row.model} ${formatTokens(row.total, "en")}`);
        lines.push(
          `${partsLine(row.parts, "en", en)} ${en(requestsKey(row.provider), {
            requests: formatNumber(row.requests, "en"),
          })}`,
        );
      }
    }

    assert.deepEqual(lines, [
      "Sep 14, 2026",
      "1B tokens",
      "In 502K · Out 254K · Cache read 1B · Cache write 253K",
      "claude-opus-5 1B",
      "In 501K · Out 252K · Cache read 1B · Cache write 253K 50 req.",
      "claude-haiku-4-5 8K",
      "In 1K · Out 2K · Cache read 5K · Cache write 0 7 req.",
    ]);
  });
});

test("the models list reads the way it is meant to, share and all, in English", () => {
  inZone("Asia/Istanbul", () => {
    const chart = buildUsageChart(FIXTURE, "all", new Date("2026-09-16T09:00:00Z"));
    const lines = [en("usage.chart.daily")];

    lines.push(chart.series.map((series) => series.model).join(SEPARATOR));
    for (const row of chart.totals.rows) {
      const share =
        row.percent === undefined
          ? ABSENT
          : en("panel.window.percent", { percent: formatNumber(row.percent, "en") });
      lines.push(`${row.model} ${formatTokens(row.total, "en")} ${share}`);
    }

    assert.deepEqual(lines, [
      "Tokens per day",
      "claude-opus-5 · gpt-5.6-sol · claude-haiku-4-5",
      "claude-opus-5 1B 99 %",
      "gpt-5.6-sol 2.4M 0 %",
      "claude-haiku-4-5 8K 0 %",
      "claude-sonnet-4-5 — —",
    ]);
  });
});

test("the markup offers exactly the tabs and spans this file knows", () => {
  // Two lists in two languages: a tab in the markup that this file has never heard of is a
  // button whose click does nothing, and one here that the markup lacks is a tab nobody can
  // reach. `ui/test/i18n.test.mjs` already checks that each of them names a real key.
  const html = readFileSync(resolve(REPO, "ui/src/index.html"), "utf8");
  const named = (attribute) =>
    [...html.matchAll(new RegExp(`${attribute}="([^"]+)"`, "g"))].map((match) => match[1]);

  assert.deepEqual(named("data-usage-tab"), [...USAGE_TABS]);
  assert.deepEqual(named("data-usage-span"), [...USAGE_SPANS]);
});

// ------------------------------------------------------- T-WP23: one way back

test("the usage view has one header, one Back and one heading", () => {
  // T-WP21 gave a detail a header of its own, and it stood under the view's: a day opened
  // from the calendar put two *Back* buttons in the top-left corner, one above the other,
  // with the same word on both and no way to tell which left the day and which left the
  // view. There is one header now, and the heading under the button is what changed instead.
  const html = readFileSync(resolve(REPO, "ui/src/index.html"), "utf8");
  const view = html.slice(
    html.indexOf('<div class="view usage"'),
    html.indexOf('<form class="view settings"'),
  );
  assert.ok(view.length > 0, "index.html has no usage view");

  assert.equal(
    (view.match(/data-usage-back/g) ?? []).length,
    1,
    "the usage view offers exactly one way back",
  );
  assert.equal(
    (view.match(/data-i18n="settings\.back"/g) ?? []).length,
    1,
    "and exactly one button says the word",
  );
  assert.ok(!html.includes("data-usage-scope"), "the second header is gone from the markup");

  // The heading is written at run time — `Usage` on the tabs, a date on a detail — so it
  // must not carry a `data-i18n` attribute, which would put the word `Usage` back over a
  // day the moment the language changed.
  assert.match(view, /<h2 class="settings-title" data-usage-title><\/h2>/);
  assert.ok(
    !/data-usage-title[^>]*data-i18n|data-i18n[^>]*data-usage-title/.test(view),
    "the heading is filled by main.ts, not by the markup",
  );
});

test("Back goes exactly one level up, Esc goes with it, and the focus follows", () => {
  // This is the one rule of the view that lives entirely in `main.ts`, and a suite with no
  // DOM can still hold it: one function decides what *Back* means, and everything that can
  // mean *back* calls that function rather than deciding again.
  const panel = readFileSync(resolve(REPO, "ui/src/main.ts"), "utf8");

  const back = /function usageBack\(\): void \{([\s\S]*?)\n\}/.exec(panel);
  assert.ok(back, "main.ts has no usageBack()");
  assert.match(back[1], /usage\.detail !== undefined\) closeScope\(\)/, "a detail closes first");
  assert.match(back[1], /else showView\("quota"\)/, "and a list leaves for the quota view");

  assert.match(
    panel,
    /usageBackButton\?\.addEventListener\("click", usageBack\)/,
    "the button is the function, not a second copy of the rule",
  );
  assert.match(
    panel,
    /if \(shown === "usage"\) \{\s*usageBack\(\);/,
    "and so is Esc, so the key and the button cannot come to mean different things",
  );

  // The focus moves with the view, both ways: onto *Back* when a day opens — the element
  // that was clicked is about to be replaced — and onto the tab that opened it when it
  // closes.
  assert.match(
    /function openScope\([\s\S]*?\n\}/.exec(panel)?.[0] ?? "",
    /usageBackButton\?\.focus\(\)/,
  );
  assert.match(
    /function closeScope\([\s\S]*?\n\}/.exec(panel)?.[0] ?? "",
    /usageTabs\.find\([\s\S]*?\)\?\.focus\(\)/,
  );

  // While a detail is open the tabs are put away: four ways out of a page with one way back
  // is three ways to lose the day that was just opened.
  assert.match(panel, /usageTabsRow\.hidden = opened/);

  // And the heading is the detail's own title or the word the tabs are headed by — one
  // function, called from the redraw and from a language change alike.
  const title = /function paintUsageTitle\(\): void \{([\s\S]*?)\n\}/.exec(panel);
  assert.ok(title, "main.ts has no paintUsageTitle()");
  assert.match(title[1], /usage\.detail\s*\?\s*detailTitle\(usage\.detail, locale, t\)/s);
  assert.match(title[1], /t\("usage\.title"\)/);
  assert.equal(
    (panel.match(/paintUsageTitle\(\);/g) ?? []).length,
    2,
    "called from paintUsage and from applyLanguage, and nowhere else",
  );

  assert.ok(!panel.includes("usageScope"), "nothing is left of the second header");
});

// -------------------------------------------------------------- the stylesheet

test("the stylesheet draws what this file computes", () => {
  const css = readFileSync(resolve(REPO, "ui/src/styles.css"), "utf8");

  // T-WP20's third complaint: the footer's Usage button did not look like a button under the
  // pointer. It never was a rule of its own — every `.button` shared one that changed a shade
  // of text and a hairline in the raw accent, which is 2.25:1 on a white panel. A ground is
  // what makes a control look pressable, and the same ground answers the keyboard.
  assert.match(
    css,
    /\.button:hover,\s*\.button:focus-visible \{[^}]*background:\s*var\(--color-node-header/s,
    "hover and focus must fill, not only tint",
  );
  assert.match(
    css,
    /\.button:hover,\s*\.button:focus-visible \{[^}]*--color-accent-text/s,
    "the outline is the derived accent tone, which survives a light ground",
  );

  // Five steps, and the four that are not empty are the theme's accent rather than a palette
  // this view invented for itself.
  for (let level = 0; level <= HEAT_LEVELS; level++) {
    assert.match(css, new RegExp(`\\.usage-level-${level}[\\s,{]`), `no rule for level ${level}`);
  }
  assert.match(css, /\.usage-level-0 \{[^}]*var\(--color-node-header/s, "an empty day is a track");
  assert.match(
    css,
    /\.usage-level-1,\s*\.usage-level-2,\s*\.usage-level-3,\s*\.usage-level-4 \{[^}]*var\(--color-accent-text/s,
    "the four shades are one token at four opacities, not four colours",
  );
  assert.match(css, /\.usage-cell\.blank \{[^}]*background:\s*transparent/s, "a hole is nothing");

  // A year is fifty-three columns and cannot be squeezed into 330 px, so it scrolls.
  assert.match(css, /\.usage-grid \{[^}]*overflow-x:\s*auto/s);
  assert.match(css, /\.usage-weeks \{[^}]*grid-template-rows:\s*repeat\(7,/s, "Monday to Sunday");

  // The fourth complaint: *since* and *scanned* were 10 px footnotes. They are a line now, at
  // the size of the rows.
  assert.match(css, /\.usage-info \{[^}]*var\(--type-size-sm/s);
  assert.ok(
    !/\.usage-info \{[^}]*var\(--type-size-xs/s.test(css),
    "the footer line is not a footnote any more",
  );
  assert.ok(!css.includes(".usage-line"), "and the two it replaced are gone");

  // T-WP21. A day is a control now, so it is a real `<button>` with the styling of a button
  // taken off it — which is what hands Enter, Space and focus to the platform.
  assert.match(
    css,
    /button\.usage-cell,\s*\nbutton\.usage-bar,\s*\n\.usage-week-row \{[^}]*appearance:\s*none/s,
    "a day and a week are buttons wearing the square they replaced",
  );
  assert.ok(
    !/\.usage-cell \{[^}]*role/s.test(css),
    "nothing in the stylesheet should be asserting a role",
  );

  // The weeks list scrolls rather than growing a window that is clamped at 720 px, the same
  // ceiling and the same reason as the model rows beside it.
  assert.match(css, /\.usage-week-rows \{[^}]*max-height:\s*236px/s);
  assert.match(css, /\.usage-week-rows \{[^}]*overflow-y:\s*auto/s);
  assert.match(
    css,
    /\.usage-week-row\.current \{[^}]*var\(--color-accent-text/s,
    "the part week is marked in the theme's own accent, not a colour this view invented",
  );

  // The chart is an `<svg>` that scales to the panel, and every colour in it is a token or a
  // stroke `theme.ts` derived — there is no hex in the stylesheet for it.
  assert.match(css, /\.usage-chart svg \{[^}]*width:\s*100%/s);
  assert.match(css, /\.usage-chart-rule \{[^}]*var\(--color-edge/s);
  assert.match(css, /\.usage-chart-label \{[^}]*var\(--color-text-faint/s);
  assert.match(css, /\.usage-chart-line \{[^}]*fill:\s*none/s, "a line chart fills nothing");
  assert.ok(
    !/\.usage-chart-line \{[^}]*stroke:/s.test(css),
    "the stroke is the palette's, set per line, and must not be frozen in the stylesheet",
  );

  // T-WP23. The second header is gone, and the one heading that is left writes a detail's
  // own title into it — `Week of Sep 7, 2026`, and longer in Russian — so it truncates
  // rather than pushing *Back* off a 330 px panel.
  assert.ok(!css.includes(".usage-scope"), "the second header's rules went with it");
  assert.match(css, /\.settings-title \{[^}]*text-overflow:\s*ellipsis/s);
  assert.match(css, /\.settings-title \{[^}]*white-space:\s*nowrap/s);
});

// ------------------------------------------------- the two counts, and the tag

test("an answer says which of the two counts it holds, and only one of them is tagged", () => {
  // The deduplicated spend is this product's own answer and wears no label: a tag on the
  // default would make the default read as the exception.
  assert.equal(usageMode(FIXTURE), "deduped");
  assert.equal(modeTagKey("deduped"), undefined);

  // The per-line count is the one `/usage` prints, about 1.7× the real spend, and a view
  // that swapped one for the other without saying so is a view nobody can trust twice.
  const counted = { ...FIXTURE, mode: "per_line" };
  assert.equal(usageMode(counted), "per_line");
  assert.equal(modeTagKey("per_line"), "usage.mode.perLine");
  assert.equal(en("usage.mode.perLine"), "as /usage counts");

  // The buckets arrive in the same shape either way, which is why one set of functions draws
  // both: only the four numbers move.
  const view = inZone("Asia/Istanbul", () =>
    buildUsageView(counted, new Date("2026-09-16T09:00:00+03:00")),
  );
  assert.equal(view.mode, "per_line");
  assert.equal(view.total, 1003418000, "the same arithmetic over the numbers it was sent");
});

// ------------------------------------------- the days before the transcripts

/** The same week, plus the two days Claude Code reported and this store never measured. */
const REPORTED = {
  ...FIXTURE,
  reported: {
    "2026-09-07": { "claude-opus-5": 900_000, "claude-fable-5-1": 100_000 },
    "2026-09-08": { "claude-opus-5": 500_000 },
    // A day the store measured as well: the boundary is a UTC day in Rust and a local day
    // here, so one day can be both, and the side that knows the offset drops it.
    "2026-09-14": { "claude-opus-5": 12_345 },
  },
};

test("a reported day is kept, and one the store measured is dropped", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView(
      { ...REPORTED, range: "all" },
      new Date("2026-09-16T09:00:00+03:00"),
    );
    assert.deepEqual(
      view.reportedDays.map((day) => day.key),
      ["2026-09-07", "2026-09-08"],
      "14 September is a day this store has numbers for, so its reported twin is dropped",
    );
    const [first] = view.reportedDays;
    assert.equal(first.total, 1_000_000);
    assert.equal(first.topModel, "claude-opus-5");
    assert.deepEqual(
      first.models.map((row) => row.model),
      ["claude-opus-5", "claude-fable-5-1"],
      "biggest first, so the hover line names the model that led the day",
    );

    // And none of it reaches a measured total.
    assert.equal(view.total, 1003418000);
    assert.ok(
      !view.rows.some((row) => row.provider === REPORTED_PROVIDER),
      "a reported day is never a row of the window it sits beside",
    );
  });
});

test("a reported day is an outline rather than a shade, and says so when it is hovered", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView(
      { ...REPORTED, range: "all" },
      new Date("2026-09-16T09:00:00+03:00"),
    );
    const cells = view.grid.columns.flatMap((column) => column.days);
    const said = cells.find((cell) => cell.key === "2026-09-07");
    assert.ok(said, "the calendar reaches back to the oldest reported day");
    assert.equal(said.reported, true);
    assert.equal(said.total, 1_000_000);
    assert.equal(said.level, 0, "a reported day takes no shade: the scale is not its scale");

    // Nor does it move the scale the measured days are ranked on.
    const measured = cells.find((cell) => cell.key === "2026-09-14");
    assert.equal(measured.reported, false);
    assert.equal(measured.level, HEAT_LEVELS, "the busiest measured day is still the darkest");

    // The colour is never the only channel: the line under the grid says it in words.
    const line = cellLine(said, "en", en);
    assert.ok(line.includes("reported by Claude Code"), line);
    assert.ok(line.includes(formatTokens(1_000_000, "en")), line);
    assert.ok(
      !cellLine(measured, "en", en).includes("reported by Claude Code"),
      "and a measured day says nothing of the kind",
    );
  });
});

test("a week with nothing but reported days shows the reported number and no breakdown", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView(
      { ...REPORTED, range: "all" },
      new Date("2026-09-16T09:00:00+03:00"),
    );
    const week = view.weeks.find((row) => row.key === "2026-09-07");
    assert.ok(week, "the week of 7 September is on the list");
    assert.equal(week.reported, true);
    assert.equal(week.total, undefined, "nothing was measured in it");
    assert.equal(week.reportedTotal, 1_500_000, "both reported days of that week");
    assert.equal(weekTotal(week), 1_500_000);
    assert.equal(week.share, 0, "and it is not drawn against a scale it does not share");

    // The week that was measured is untouched by any of it.
    const measured = view.weeks.find((row) => row.key === "2026-09-14");
    assert.equal(measured.reported, false);
    assert.equal(weekTotal(measured), measured.total);
    assert.equal(measured.share, 100, "the only measured week is the busiest one");
  });
});

test("a reported day opens into models with a total each and an em dash everywhere else", () => {
  inZone("Asia/Istanbul", () => {
    const detail = buildUsageDetail(
      { ...REPORTED, range: "all" },
      { kind: "day", key: "2026-09-07", at: Date.parse("2026-09-07T00:00:00+03:00") },
    );
    assert.equal(detail.reported, true);
    assert.equal(detail.empty, false);
    assert.equal(detail.total, 1_000_000);
    assert.deepEqual(
      detail.rows.map((row) => [row.model, row.total]),
      [
        ["claude-opus-5", 900_000],
        ["claude-fable-5-1", 100_000],
      ],
    );
    assert.equal(detail.rows[0].reported, true);
    assert.equal(detail.grouped, false, "one provider is not a heading");

    // `stats-cache.json` holds one number per model per day. The split it does not hold is
    // not going to be invented here, so all four parts print the em dash.
    const parts = partsLine(detail.parts, "en", en);
    assert.equal(parts, `In ${ABSENT}${SEPARATOR}Out ${ABSENT}${SEPARATOR}Cache read ${ABSENT}${SEPARATOR}Cache write ${ABSENT}`);
    assert.equal(detail.requests, 0, "and there is no count of replies to show");

    // A measured day opens the way it always has.
    const measured = buildUsageDetail(REPORTED, {
      kind: "day",
      key: "2026-09-14",
      at: Date.parse("2026-09-14T00:00:00+03:00"),
    });
    assert.equal(measured.reported, false);
    assert.ok(measured.total > 0);
  });
});

test("the setting being off leaves the view exactly as it was", () => {
  inZone("Asia/Istanbul", () => {
    const now = new Date("2026-09-16T09:00:00+03:00");
    const without = buildUsageView({ ...FIXTURE, range: "all" }, now);
    assert.deepEqual(without.reportedDays, []);
    assert.ok(
      without.grid.columns.flatMap((column) => column.days).every((cell) => !cell.reported),
    );
    assert.ok(without.weeks.every((week) => !week.reported));
  });
});

test("the stylesheet draws a reported day as an outline and the tag as a chip", () => {
  const css = readFileSync(resolve(REPO, "ui/src/styles.css"), "utf8");
  // Dashed rather than shaded, and the same dash in all three places, so one glance covers
  // the calendar, the weeks list and the headline.
  assert.match(css, /\.usage-cell\.reported \{[^}]*border:\s*1px dashed/s);
  assert.match(css, /\.usage-cell\.reported \{[^}]*background:\s*transparent/s);
  assert.match(css, /\.usage-week-row\.reported \{[^}]*border-style:\s*dashed/s);
  assert.match(css, /\.usage-tag \{[^}]*border:\s*1px dashed/s);
  // And every colour in them is the theme's accent token rather than one this view invented.
  for (const rule of [/\.usage-cell\.reported \{[^}]*\}/s, /\.usage-tag \{[^}]*\}/s]) {
    const block = rule.exec(css);
    assert.ok(block, "the rule is missing");
    assert.ok(!/#[0-9a-f]{6}(?![^;]*var\()/i.test(block[0].replace(/var\([^)]*\)/g, "")), block[0]);
  }
});
