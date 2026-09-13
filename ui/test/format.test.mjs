// The words the panel puts on the numbers.
//
// These are the rules that would otherwise only be visible in a screenshot: what a window
// is called, what an old reading says about itself, and what happens to a bar nobody could
// measure. Every one of them is a contract rule rather than presentation taste, which is
// why they are tested rather than looked at.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  ageText,
  formatDuration,
  freshnessClass,
  labelText,
  meterWidth,
  resetClock,
  resolveMode,
  severityClass,
  windowLabel,
} from "../dist/lib/format.mjs";
import { createTranslator } from "../dist/lib/i18n.mjs";
import { catalogs } from "../dist/lib/locales.mjs";

const UI = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const en = createTranslator(catalogs, "en");
const tr = createTranslator(catalogs, "tr");

const window = (fields) => ({
  key: "seven_day",
  state: "ok",
  detailed: false,
  binding: false,
  severity: "ok",
  ...fields,
});

test("a duration is words, in the language the panel is in", () => {
  assert.equal(formatDuration(7_800_000, en), "2 h 10 m");
  assert.equal(formatDuration(600_000, en), "10 m");
  assert.equal(formatDuration(12_000, en), "12 s");
  assert.equal(formatDuration(-1, en), "0 s", "a reading from the future is not negative time");

  assert.equal(formatDuration(7_800_000, tr), "2 sa 10 dk");
  assert.equal(formatDuration(12_000, tr), "12 sn");
});

test("the Rust side and the panel say a duration the same way", () => {
  // `crates/nazar-tray/src/i18n.rs` formats the tray tooltip from these same three keys.
  // A panel that said "2 h 10 m" beside a tooltip that said "2:10:00" would look like two
  // programs.
  const rust = readFileSync(resolve(UI, "../crates/nazar-tray/src/i18n.rs"), "utf8");
  for (const key of ["time.hoursMinutes", "time.minutes", "time.seconds"]) {
    assert.ok(rust.includes(`"${key}"`), `the Rust duration formatter no longer uses ${key}`);
  }
});

test("a window is named by its length, not by its provider", () => {
  assert.equal(labelText(windowLabel(window({ windowMinutes: 300 })), en), "5-hour window");
  assert.equal(labelText(windowLabel(window({ windowMinutes: 10080 })), en), "Weekly window");
  // Codex's keys are `primary` and `secondary`; the same two lengths, so the same two names.
  assert.equal(
    labelText(windowLabel(window({ key: "primary", windowMinutes: 300 })), en),
    "5-hour window",
  );
  assert.equal(labelText(windowLabel(window({ windowMinutes: 300 })), tr), "5 saatlik pencere");
});

test("a model-scoped weekly says which model, because that is the point of it", () => {
  const scoped = window({ key: "seven_day_fable", windowMinutes: 10080, model: "Fable" });
  assert.equal(labelText(windowLabel(scoped), en), "Fable weekly");
  assert.equal(labelText(windowLabel(scoped), tr), "Fable haftalık");
});

test("a window length nobody recognises falls back to its key", () => {
  const odd = window({ key: "monthly", windowMinutes: 43200 });
  assert.equal(labelText(windowLabel(odd), en), "monthly");
  assert.equal(labelText(windowLabel(window({ key: "primary" })), en), "primary");
});

test("an unknown window has no bar at all, which is not the same as a bar of zero", () => {
  assert.equal(meterWidth(undefined), 0);
  assert.equal(meterWidth(Number.NaN), 0);
  assert.equal(meterWidth(0), 0);
  assert.equal(meterWidth(63.4), 63.4);
  assert.equal(meterWidth(140), 100, "a source that reports over 100 fills the bar, not the card");
  assert.equal(meterWidth(-5), 0);
});

test("the freshness line says how old, and stale says so in words", () => {
  assert.equal(ageText("fresh", 12_000, en), "updated 12 s ago");
  assert.equal(ageText("aging", 900_000, en), "updated 15 m ago");
  assert.equal(ageText("stale", 4_200_000, en), "stale · last data 1 h 10 m ago");
  assert.equal(ageText("unknown", undefined, en), "age unknown");
  assert.equal(
    ageText("fresh", undefined, en),
    "age unknown",
    "no age is no age, whatever the classification says",
  );
  assert.equal(ageText("stale", 4_200_000, tr), "bayat · son veri 1 sa 10 dk önce");
});

test("severity and freshness become classes, never English in the DOM", () => {
  assert.equal(severityClass("critical"), "severity-critical");
  assert.equal(severityClass("unknown"), "severity-unknown");
  assert.equal(freshnessClass("stale"), "age-stale");
});

test("a reset within a day is a time; one further out carries its weekday", () => {
  const at = new Date("2026-09-08T02:00:00Z");
  const soon = resetClock(at, 3 * 3600 * 1000, "en-GB");
  const later = resetClock(at, 4 * 86400 * 1000, "en-GB");

  assert.match(soon, /^\d{2}:\d{2}$/, `expected a clock time, got ${soon}`);
  assert.ok(
    later.length > soon.length,
    `a reset four days out needs its day: got ${later} against ${soon}`,
  );
  assert.ok(later.includes(soon), `the day is added to the time, not instead of it: ${later}`);
});

test("light and dark follow the system unless the settings say otherwise", () => {
  assert.equal(resolveMode("system", true), "dark");
  assert.equal(resolveMode("system", false), "light");
  assert.equal(resolveMode("dark", false), "dark");
  assert.equal(resolveMode("light", true), "light");
  // A value from a newer build is not a reason to paint the panel wrong.
  assert.equal(resolveMode("sepia", true), "dark");
});
