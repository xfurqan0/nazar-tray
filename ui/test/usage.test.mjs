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
  HEAT_LEVELS,
  bucketTotal,
  buildUsageView,
  cellLine,
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
  partsLine,
  requestsKey,
  rfc3339,
  startOfLocalWeek,
  usageErrorKey,
  usageWindow,
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
});
