// The formatting the panel does to the derived snapshot.
//
// The rules being checked are contract rules rather than presentation taste: a quota is
// floored rather than rounded (audit finding B15 — 99.6 % is not "you have run out"), an
// unknown window has no number at all (B03), and a countdown that has gone past zero is
// clamped rather than printed as a negative clock.

import assert from "node:assert/strict";
import test from "node:test";

import { displayPercent, formatClock, worstSeverity } from "../dist/lib/snapshot.mjs";

test("a duration reads as a clock, in numerals that need no translation", () => {
  assert.equal(formatClock(0), "0:00");
  assert.equal(formatClock(7_000), "0:07");
  assert.equal(formatClock(65_000), "1:05");
  assert.equal(formatClock(3_600_000), "1:00:00");
  assert.equal(formatClock(14_527_000), "4:02:07");
});

test("a countdown that has gone past zero does not print a negative clock", () => {
  assert.equal(formatClock(-1), "0:00");
  assert.equal(formatClock(-28_800_000), "0:00");
});

test("a percentage is floored, never rounded up", () => {
  assert.equal(displayPercent(0), 0);
  assert.equal(displayPercent(54), 54);
  assert.equal(displayPercent(99.6), 99, "99.6 % has not run out and must not say 100");
  assert.equal(displayPercent(100), 100);
});

test("a whole percentage is not floored through its own rounding error", () => {
  // T-WP29. Claude Code reports a fraction times a hundred, and `0.29 * 100` is
  // 28.999999999999996 — which a bare floor turned into 28 %. Of the 101 whole percentages,
  // 29, 57 and 58 land below themselves; every one of them must read as itself.
  for (let whole = 0; whole <= 100; whole += 1) {
    assert.equal(displayPercent((whole / 100) * 100), whole, `${(whole / 100) * 100}`);
  }
  assert.equal(displayPercent(14.000000000000002), 14);
  assert.equal(displayPercent(28.9999), 28, "a real fraction is still rounded down");
});

const provider = (name, severity) => ({
  name,
  configured: true,
  freshness: "fresh",
  severity,
  detailed: false,
  windows: [],
});

test("the worst severity across providers is the one the panel reports", () => {
  assert.equal(
    worstSeverity({ updatedAt: "", now: "", providers: [] }),
    "unknown",
    "nothing read at all is unknown, which is the grey state",
  );
  assert.equal(
    worstSeverity({
      updatedAt: "",
      now: "",
      providers: [provider("claude", "ok"), provider("codex", "warn")],
    }),
    "warn",
  );
  assert.equal(
    worstSeverity({
      updatedAt: "",
      now: "",
      providers: [provider("claude", "unknown"), provider("codex", "ok")],
    }),
    "ok",
    "a provider nobody could read must not decide the colour on its own",
  );
  assert.equal(
    worstSeverity({
      updatedAt: "",
      now: "",
      providers: [provider("claude", "exhausted"), provider("codex", "critical")],
    }),
    "exhausted",
  );
});
