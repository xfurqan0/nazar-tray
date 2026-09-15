// The Waybar face, against the repository's own sample document.
//
// `faces/waybar/nazar-waybar.sh` is the first consumer of `limits.json` that is not this
// product, and the reason it lives in this repository rather than in one of its own is
// exactly this file: it is checked against `fixtures/limits.sample.json` in the same CI run
// as the writer, so the two cannot drift. A change to the document that the face mishandles
// fails the build instead of being discovered by somebody whose bar went blank.
//
// **The variants are derived, not committed.** Every case below starts from the sample and
// edits one thing — 99.6 %, an unreadable provider, a provider that is not configured, a
// damaged file, no file at all. A committed copy of a document is a second document, and
// two documents are two things to keep in step; a derivation is the sample plus a sentence
// saying what this case is about.
//
// Skipped on Windows, where there is no `sh`. Everywhere else `jq` is required rather than
// detected: the face needs it, the CI Linux job installs it, and a suite that silently
// skips when a dependency is missing is a suite that is green on a machine where the thing
// it tests does not work.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const UI = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const REPO = resolve(UI, "..");
const FACE = resolve(REPO, "faces/waybar");
const SCRIPT = join(FACE, "nazar-waybar.sh");
const SAMPLE = JSON.parse(readFileSync(resolve(REPO, "fixtures/limits.sample.json"), "utf8"));

const windows = process.platform === "win32";

/** A throwaway `NAZAR_HOME` holding whatever this case is about. */
function home({ limits, lock }) {
  const dir = mkdtempSync(join(tmpdir(), "nazar-waybar-"));
  if (limits !== undefined) {
    writeFileSync(join(dir, "limits.json"), typeof limits === "string" ? limits : JSON.stringify(limits, null, 2));
  }
  if (lock !== undefined) {
    writeFileSync(join(dir, "limits.lock"), JSON.stringify(lock, null, 2));
  }
  return dir;
}

/** A lock that says the tray is alive: this very process, beating just now. */
const alive = () => ({
  schemaVersion: 1,
  pid: process.pid,
  startedAt: new Date(Date.now() - 60_000).toISOString().replace(/\.\d+Z$/, "Z"),
  heartbeatAt: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
});

/** Run the face and parse its one line. Asserts the two things Waybar needs first. */
function run(dir) {
  const stdout = execFileSync("sh", [SCRIPT], {
    env: { ...process.env, NAZAR_HOME: dir },
    encoding: "utf8",
  });
  assert.equal(stdout.split("\n").filter(Boolean).length, 1, `one line, got ${JSON.stringify(stdout)}`);
  return JSON.parse(stdout);
}

/** The sample with one edit, as a fresh document. */
function variant(edit) {
  const copy = structuredClone(SAMPLE);
  edit(copy);
  return copy;
}

test("the binding window is the number in the bar", { skip: windows }, () => {
  // Claude binds on its Fable weekly at 23 %, Codex on its weekly at 70 %. The number that
  // belongs in a bar is the one closest to full, because that is the one that stops you.
  const module = run(home({ limits: SAMPLE, lock: alive() }));
  assert.equal(module.text, "nazar 70%");
  assert.equal(module.percentage, 70);
  assert.equal(module.class, "warn");
  assert.equal(module.alt, module.class, "the icon and the stylesheet follow the same state");
  assert.match(module.tooltip, /codex secondary 70 %/);
  assert.match(module.tooltip, /claude seven_day_fable 23 %/);
});

test("a percentage is rounded down, never up", { skip: windows }, () => {
  // Rule 5 of the contract. 99.6 % is not 100 %, and a bar that says a window is spent when
  // it is not is wrong at the exact moment it matters most.
  const limits = variant((doc) => {
    doc.providers.codex.windows.secondary.percent = 99.6;
  });
  const module = run(home({ limits, lock: alive() }));
  assert.equal(module.text, "nazar 99%");
  assert.equal(module.percentage, 99);
  assert.equal(module.class, "crit");
});

test("a window that could not be read is a question mark and never a zero", { skip: windows }, () => {
  // Rule 2. Codex goes unreadable — no percent, no binding, state error — and the bar falls
  // back to the provider that can still be read rather than to a reassuring 0 %.
  const limits = variant((doc) => {
    delete doc.providers.codex.binding;
    for (const key of Object.keys(doc.providers.codex.windows)) {
      doc.providers.codex.windows[key] = { state: "error", error: "no quota line in the newest session log" };
    }
  });
  const module = run(home({ limits, lock: alive() }));
  assert.equal(module.text, "nazar 23%");
  assert.match(module.tooltip, /codex primary \?/);
  assert.match(module.tooltip, /codex secondary \?/);
  assert.doesNotMatch(module.tooltip, /codex (primary|secondary) 0 %/);
});

test("when nothing can be read the bar says so in both places", { skip: windows }, () => {
  const limits = variant((doc) => {
    doc.providers.claude = { configured: false };
    delete doc.providers.codex.binding;
    for (const key of Object.keys(doc.providers.codex.windows)) {
      doc.providers.codex.windows[key] = { state: "error" };
    }
  });
  const module = run(home({ limits, lock: alive() }));
  assert.equal(module.text, "nazar ?");
  assert.equal(module.class, "unknown");
  assert.equal(module.percentage, 0, "Waybar needs a number here, which is why text and class carry the truth");
  assert.match(module.tooltip, /claude: not configured/);
});

test("the tray being gone is a state of its own, in all three of its shapes", { skip: windows }, () => {
  // The lock answers "is the tray running", in the order the contract gives: a pid the
  // kernel has never heard of is stale at once, and only a pid that cannot be asked about
  // falls back to the heartbeat's five minutes.
  const stale = new Date(Date.now() - 30 * 60_000).toISOString().replace(/\.\d+Z$/, "Z");
  const cases = {
    "no lock at all": undefined,
    "a pid nothing owns": { ...alive(), pid: 0x3fffff },
    "a heartbeat half an hour old": { ...alive(), heartbeatAt: stale },
  };
  for (const [what, lock] of Object.entries(cases)) {
    const module = run(home({ limits: SAMPLE, lock }));
    assert.equal(module.class, "stale", what);
    assert.equal(module.text, "nazar 70%", `${what}: the last numbers stay, they were true when written`);
    assert.match(module.tooltip.split("\r")[0], /not running/, what);
  }
});

test("no document, or a damaged one, still prints one valid line and exits 0", { skip: windows }, () => {
  // Waybar hides a module whose script prints nothing or prints something it cannot parse,
  // which is the worst failure shape there is: the user sees an empty bar and no reason.
  for (const limits of [undefined, '{ "providers": ', "", "not json at all"]) {
    const module = run(home({ limits }));
    assert.equal(typeof module.text, "string");
    assert.equal(typeof module.tooltip, "string");
    assert.equal(typeof module.percentage, "number");
    assert.match(module.class, /^(ok|warn|crit|stale|unknown)$/);
  }
});

test("the face writes nothing at all", { skip: windows }, () => {
  // `~/.nazar` has one writer and this is not it. Checked by the directory's own metadata
  // rather than by inotify: a file created, touched or replaced moves the directory's
  // mtime, and the two files it reads would change their own.
  const dir = home({ limits: SAMPLE, lock: alive() });
  const before = [dir, join(dir, "limits.json"), join(dir, "limits.lock")].map((path) => {
    const info = statSync(path);
    return `${info.mtimeMs} ${info.size}`;
  });
  run(dir);
  const after = [dir, join(dir, "limits.json"), join(dir, "limits.lock")].map((path) => {
    const info = statSync(path);
    return `${info.mtimeMs} ${info.size}`;
  });
  assert.deepEqual(after, before);
});

test("the face runs no program but jq", { skip: windows }, () => {
  // A grep rather than a trace, and it is the right shape of check: what it rules out is a
  // future line that shells out. The names are the ones the eight community faces of the
  // tool this one is measured against reach for — a CLI, a clock, a process table, an HTTP
  // client — plus the text tools a shell script grows when somebody stops trusting jq.
  //
  // Matched only where a command can start, because the words themselves are allowed to
  // appear in a string: the last line of the script prints a tooltip that names nazar-tray.
  const body = readFileSync(SCRIPT, "utf8")
    .split("\n")
    .filter((line) => !line.trim().startsWith("#"))
    .join("\n");
  for (const program of ["nazar-tray", "date", "ps", "curl", "wget", "notify-send", "cat", "awk", "sed"]) {
    const invoked = new RegExp(`(^|[;&|(]|\\$\\()\\s*${program}\\b`, "m");
    assert.ok(!invoked.test(body), `${program} has no business in a face`);
  }
  const jqs = [...body.matchAll(/(^|[;&|(]|\$\()\s*jq\b/gm)];
  assert.equal(jqs.length, 2, "one jq for the lock's pid, one for everything else");
});

test("the face stays short enough to read in one sitting", { skip: windows }, () => {
  // Sixty lines is not a style rule, it is the measurement the plan was made on: the eight
  // community faces of the tool this one is measured against spend 793 to 23 881 lines
  // normalising a CLI's output, and this one spends none, because the contract did it.
  const lines = readFileSync(SCRIPT, "utf8").split("\n").filter((line) => line !== "").length;
  assert.ok(lines <= 60, `the face is ${lines} lines`);
});

test("the example module never asks Waybar for {percentage}", { skip: windows }, () => {
  // The one trap. `percentage` has to be a number, so it is 0 when nothing is known, and a
  // bar reading "nazar 0%" tells somebody about to start a long task the opposite of the
  // truth. An example that gets this wrong is worse than no example.
  const jsonc = readFileSync(join(FACE, "config.jsonc"), "utf8");
  const config = JSON.parse(jsonc.replace(/^\s*\/\/.*$/gm, ""));
  const module = config["custom/nazar"];
  assert.equal(module["return-type"], "json");
  assert.ok(!module.format.includes("{percentage}"), "see the comment above format");
  assert.ok(module.exec.endsWith("nazar-waybar.sh"));
  assert.ok(Number.isInteger(module.interval) && module.interval > 0);
  assert.ok(Number.isInteger(module.signal));
});

test("the example stylesheet has a rule for every state the script can set", { skip: windows }, () => {
  const css = readFileSync(join(FACE, "style.css"), "utf8");
  for (const state of ["ok", "warn", "crit", "stale", "unknown"]) {
    assert.match(css, new RegExp(`#custom-nazar\\.${state}\\s*\\{`), state);
  }
});
