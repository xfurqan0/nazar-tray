// The usage view's arithmetic, which is the half of it that can be wrong without looking
// wrong.
//
// Three of these are contract rules rather than presentation taste, and the reason each is a
// test rather than a screenshot:
//
//   * **The local week is the panel's to compute.** Rust may not ask this machine which zone
//     it is in, so `ui/src/usage.ts` works out Monday 00:00 locally and re-buckets the hours
//     it gets back. A bucket lands in the local day its hour **starts** in — the rule
//     `docs/usage-contract.md` states — and the case that proves it is 21:00Z on a Sunday,
//     which is Monday for anybody at +03:00.
//   * **The headline is `input + output + cache_create`.** Cache reads were 98.5 % of the
//     raw total over six days of real work; a headline with them folded in is a number about
//     the cache with the work lost in the rounding.
//   * **An absent counter is absent, not zero.** A bucket that never named a counter prints
//     an em dash. A counter that really is zero — Codex reports `cache_create: 0` on every
//     event — prints `0`, because that is a measurement.
//
// The zone is changed inside the tests rather than at the top of the file: several of these
// only mean something in a particular one, and the tests in a file run one after another.
//
// The last test is as close to a rendering as a suite with no DOM gets: it composes the
// lines the panel writes, in order, from a fixture answer. It cannot prove `main.ts` calls
// these functions — `ui/test/i18n.test.mjs` and the bridge test guard the words and the
// command — but it does prove that what they compose reads the way it is meant to.

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
  bucketTotal,
  buildUsageView,
  formatColumn,
  formatDay,
  formatNumber,
  formatTokens,
  hourStart,
  isUsageError,
  localDayKey,
  localWeekKey,
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

test("the headline is input plus output plus cache_create, and never cache_read", () => {
  assert.equal(
    bucketTotal({ input: 2, output: 328, cache_create: 24843, cache_read: 1988416, requests: 14 }),
    25173,
  );
  assert.equal(bucketTotal({ cache_read: 1988416, requests: 14 }), undefined, "reads are not work");
  assert.equal(bucketTotal({ input: 1, requests: 1 }), 1, "one counter is enough to know one");
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
  assert.equal(codex.total, 100);
  assert.equal(codex.cacheRead, 0, "a zero that was reported is kept");
  assert.equal(formatTokens(codex.cacheRead, "en"), "0");
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

    assert.equal(view.total, 1_409_000, "input + output + cache_create over both providers");
    assert.equal(view.cacheRead, 1_002_009_000, "beside the headline, never inside it");
    assert.equal(view.requests, 85);
    assert.equal(view.empty, false);
    assert.equal(view.grouped, true, "two providers, so the headings earn their line");

    assert.deepEqual(
      view.rows.map((row) => [row.provider, row.model, row.total]),
      [
        ["claude", "claude-opus-5", 1_006_000],
        ["codex", "gpt-5.6-sol", 400_000],
        ["claude", "claude-haiku-4-5", 3_000],
        ["claude", "claude-sonnet-4-5", undefined],
      ],
      "biggest first, and a model nobody could total sorts last rather than as a zero",
    );

    const opus = view.rows[0];
    assert.equal(opus.share, 100, "the largest row fills its track");
    assert.equal(opus.requests, 50, "one model's hours add up across the window");
    assert.equal(opus.cacheRead, 1_000_004_000);

    assert.deepEqual(
      view.groups.map((group) => [group.provider, group.total]),
      [
        ["claude", 1_009_000],
        ["codex", 400_000],
      ],
    );
    assert.equal(view.groups[0].rows.length, 3, "a provider's rows stay under its heading");
  });
});

test("the strip is one column per local day, dense, stopping at today", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView(FIXTURE, new Date("2026-09-16T09:00:00Z"));
    assert.deepEqual(
      view.bars.map((bar) => [bar.key, bar.total]),
      [
        // The Sunday 21:00Z bucket is in here, not in last week.
        ["2026-09-14", 1_009_000],
        ["2026-09-15", 0],
        ["2026-09-16", 400_000],
      ],
      "Tuesday recorded requests but no counters, which is not tokens",
    );
    assert.equal(view.bars[0].height, 100, "the tallest column is the scale");
    assert.equal(view.bars[1].height, 0, "an empty day is an empty track, not a stub");
    assert.ok(view.bars[2].height > 0 && view.bars[2].height < 100);
  });
});

test("all time is cut into Monday-start weeks and says where the store begins", () => {
  inZone("Asia/Istanbul", () => {
    const view = buildUsageView(
      { ...FIXTURE, range: "all" },
      new Date("2026-09-16T09:00:00Z"),
    );
    assert.equal(view.since, "2026-09-08T12:00:00Z");
    assert.deepEqual(
      view.bars.map((bar) => [bar.key, bar.total]),
      [
        // `since` is inside the week of 7 September, and the Sunday bucket at 21:00Z belongs
        // to the next one because it starts on Monday locally.
        ["2026-09-07", 0],
        ["2026-09-14", 1_409_000],
      ],
    );
    assert.equal(formatDay(view.since, "en"), "Sep 8, 2026");
    assert.equal(formatDay(undefined, "en"), ABSENT, "a store with no floor says so");
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

test("the view reads the way it is meant to, line by line, in English", () => {
  inZone("Asia/Istanbul", () => {
    const now = new Date("2026-09-16T09:00:00Z");
    const view = buildUsageView(FIXTURE, now);
    const lines = [];

    lines.push(`${formatTokens(view.total, "en")} ${en("usage.tokens")}`);
    lines.push(en("usage.cacheRead", { tokens: formatTokens(view.cacheRead, "en") }));
    for (const group of view.groups) {
      if (view.grouped) {
        lines.push(
          `${en(`panel.provider.${group.provider}`)} ${formatTokens(group.total, "en")}`,
        );
      }
      for (const row of group.rows) {
        lines.push(
          `${row.model} ${formatTokens(row.total, "en")} ${en(requestsKey(row.provider), {
            requests: formatNumber(row.requests, "en"),
          })}`,
        );
      }
    }
    lines.push(
      en("usage.scanned", { age: formatDuration(now.getTime() - Date.parse(view.scannedAt), en) }),
    );
    lines.push(en("usage.damaged", { months: view.damaged.join(", ") }));

    assert.deepEqual(lines, [
      "1.4M tokens",
      "cache read 1B",
      "Claude Code 1M",
      "claude-opus-5 1M 50 req.",
      "claude-haiku-4-5 3K 7 req.",
      "claude-sonnet-4-5 — 3 req.",
      "Codex 400K",
      "gpt-5.6-sol 400K 25 evt.",
      "scanned 1 h 0 m ago",
      "Unreadable, left out of these numbers: 2026-07",
    ]);

    assert.equal(
      en("usage.bar", {
        date: formatColumn(view.bars[0].at, "en"),
        tokens: formatTokens(view.bars[0].total, "en"),
      }),
      "Sep 14 · 1M",
    );
  });
});
