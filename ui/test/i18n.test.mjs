// Locale files, the translation helper, and the rule that keeps them honest.
//
// WP0 wrote this file for two languages and let the other four be empty: a missing key in
// ZH, KO, RU or ES was reported and counted rather than failed on, because a suite that is
// red for a whole work package stops being read. WP6 filled them, so the exception is gone
// and the rule `docs/PROJECT.md` states is enforced as written:
//
//   * **Exact parity.** Every language has exactly English's keys — a missing one fails,
//     and so does an invented one.
//   * **Nothing empty, nothing nested.** Every value is a non-blank string. A nested object
//     is worse than a missing key: the Rust side reads these files as
//     `BTreeMap<String, String>`, so one of them makes the whole file fail to parse and
//     that language silently stops being offered. That is why the review status lives in
//     `locales/README.md` rather than in a `_meta` key inside the files.
//   * **The same holes.** A translation's placeholders are English's placeholders, or the
//     panel renders `{percent}` at somebody.
//   * **No hard-coded text.** The last two tests grep `ui/src` and `crates/nazar-tray/src`
//     for user-facing prose that never passes through a catalogue. The allow-list is
//     written out below, with a reason for each entry.

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  LOCALES,
  createTranslator,
  detectLocale,
  interpolate,
  plural,
  pluralCategory,
} from "../dist/lib/i18n.mjs";

const UI = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const REPO = resolve(UI, "..");
const LOCALE_DIR = resolve(UI, "locales");

const load = (locale) => JSON.parse(readFileSync(resolve(LOCALE_DIR, `${locale}.json`), "utf8"));
const placeholders = (template) =>
  [...template.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();

const english = load("en");

test("every declared language has a locale file, and there are no stray ones", () => {
  const onDisk = readdirSync(LOCALE_DIR)
    .filter((name) => name.endsWith(".json"))
    .map((name) => name.replace(/\.json$/, ""))
    .sort();
  assert.deepEqual(onDisk, [...LOCALES].sort());
});

test("every locale file is a flat object of non-empty strings", () => {
  for (const locale of LOCALES) {
    const catalog = load(locale);
    assert.equal(typeof catalog, "object", `${locale}.json must hold an object`);
    assert.ok(!Array.isArray(catalog), `${locale}.json must not be an array`);
    for (const [key, value] of Object.entries(catalog)) {
      assert.equal(
        typeof value,
        "string",
        `${locale}.json: ${key} is not a string, which makes the whole file unreadable to Rust`,
      );
      assert.notEqual(value.trim(), "", `${locale}.json: ${key} is blank`);
      assert.equal(value, value.trim(), `${locale}.json: ${key} has leading or trailing space`);
    }
  }
});

test("no locale carries metadata of its own", () => {
  // Checked rather than assumed: `crates/nazar-tray/src/i18n.rs` parses these files into
  // `BTreeMap<String, String>` and turns a parse failure into an *empty* catalogue, so a
  // `_meta` object would not raise an error — it would quietly drop that language out of
  // the settings list. Verified by trying it. The review status is a table in
  // `locales/README.md`, where nothing has to parse it.
  for (const locale of LOCALES) {
    for (const key of Object.keys(load(locale))) {
      assert.ok(!key.startsWith("_"), `${locale}.json carries ${key}; metadata belongs in README.md`);
    }
  }
});

test("every language covers exactly the English key set", () => {
  const expected = Object.keys(english).sort();
  for (const locale of LOCALES) {
    const keys = Object.keys(load(locale)).sort();
    const missing = expected.filter((key) => !keys.includes(key));
    const extra = keys.filter((key) => !expected.includes(key));
    assert.deepEqual(missing, [], `${locale}.json is missing ${missing.length} keys`);
    assert.deepEqual(extra, [], `${locale}.json invents keys English does not have`);
  }
});

test("every translation keeps the placeholders of the English original", () => {
  for (const locale of LOCALES) {
    const catalog = load(locale);
    for (const [key, template] of Object.entries(catalog)) {
      assert.deepEqual(
        placeholders(template),
        placeholders(english[key]),
        `${locale}.json: ${key} has different placeholders from English`,
      );
    }
  }
});

test("product names are never translated", () => {
  // A translated brand is a wrong brand, and a translated command name is a broken
  // instruction. `Nazar` is both the sibling application and the default theme.
  const verbatim = {
    "app.name": "nazar-tray",
    "panel.provider.claude": "Claude Code",
    "panel.provider.codex": "Codex",
    "tray.provider.claude": "Claude",
    "tray.provider.codex": "Codex",
    "settings.theme.nazar": "Nazar",
  };
  for (const locale of LOCALES) {
    const catalog = load(locale);
    for (const [key, value] of Object.entries(verbatim)) {
      assert.equal(catalog[key], value, `${locale}.json translated ${key}`);
    }
    for (const [key, value] of Object.entries(catalog)) {
      if (!key.startsWith("settings.language.") || key.endsWith(".system")) continue;
      assert.equal(
        value,
        english[key],
        `${locale}.json: ${key} — a language is named in its own language, in every file`,
      );
    }
  }
});

test("the Russian plural rule is the one the grammar books give", () => {
  // 1, then 2–4, then 5 and up; 11–14 belong to neither of the first two. Nothing in v1
  // calls this — see the next test — and it exists so that the first counted *word* is
  // written right rather than discovered wrong.
  const category = (n) => pluralCategory(n, "ru");
  assert.deepEqual([1, 21, 101, 131].map(category), ["one", "one", "one", "one"]);
  assert.deepEqual([2, 3, 4, 22, 34].map(category), ["few", "few", "few", "few", "few"]);
  assert.deepEqual([0, 5, 9, 11, 12, 14, 25, 111].map(category), Array(8).fill("other"));

  // English and Spanish have two forms; Chinese, Korean and Turkish leave the noun alone.
  assert.equal(pluralCategory(1, "en"), "one");
  assert.equal(pluralCategory(2, "es"), "other");
  for (const locale of ["zh", "ko", "tr"]) {
    assert.deepEqual([1, 2, 5].map((n) => pluralCategory(n, locale)), ["other", "other", "other"]);
  }

  // Magnitude decides, so a countdown that has gone negative does not pick a fourth form.
  assert.equal(pluralCategory(-1, "ru"), "one");
  assert.equal(plural(2, { one: "минута", few: "минуты", other: "минут" }, "ru"), "минуты");
  assert.equal(plural(5, { one: "минута", few: "минуты", other: "минут" }, "ru"), "минут");
  assert.equal(plural(3, { other: "minutes" }, "ru"), "minutes", "a missing form falls back");
});

test("the keys that carry a count are frozen, so a new one is a decision", () => {
  // Every one of these renders its number next to a unit **abbreviation**, which is the
  // same word after 1 as after 5 in all six languages — that is why this product needs no
  // plural forms in any message. Adding a counted string that spells its unit out would
  // break Russian silently, so adding one to this list has to be deliberate: either keep
  // it an abbreviation, or route it through `plural()` from `src/i18n.ts`.
  const counted = new Set(["days", "hours", "minutes", "seconds", "percent", "count", "n"]);
  const found = Object.entries(english)
    .filter(([, template]) => placeholders(template).some((name) => counted.has(name)))
    .map(([key]) => key)
    .sort();

  assert.deepEqual(found, [
    "alert.title",
    "panel.window.percent",
    "time.daysHours",
    "time.hoursMinutes",
    "time.minutes",
    "time.seconds",
    "tray.label.percent",
    "tray.tooltip.entry",
  ]);
});

test("no language this build ships is written right to left", () => {
  // No RTL locale is in scope for v1, and the panel therefore sets `<html lang>` and never
  // `dir`. Adding Arabic, Hebrew, Persian or Urdu means a `dir` table in `src/i18n.ts` and
  // an audit of `styles.css`, which still uses physical `left`/`right` properties in
  // places. This test is the reminder that both are owed.
  const rtl = ["ar", "he", "fa", "ur", "yi", "dv", "ps"];
  for (const locale of LOCALES) {
    assert.ok(!rtl.includes(locale), `${locale} is right-to-left and styles.css is not ready`);
  }
});

test("interpolation fills what it can and leaves the rest visible", () => {
  assert.equal(interpolate("{a} and {b}", { a: "one", b: 2 }), "one and 2");
  assert.equal(interpolate("{a} and {b}", { a: "one" }), "one and {b}");
  assert.equal(interpolate("nothing to fill"), "nothing to fill");
});

test("a missing translation falls back to English, then to the key itself", () => {
  const catalogs = {
    en: { greeting: "hello {name}", only: "english" },
    tr: { greeting: "merhaba {name}" },
    zh: {},
    ko: {},
    ru: {},
    es: {},
  };

  const tr = createTranslator(catalogs, "tr");
  assert.equal(tr("greeting", { name: "Furkan" }), "merhaba Furkan");
  assert.equal(tr("only"), "english");
  assert.equal(tr("panel.does.not.exist"), "panel.does.not.exist");
});

test("language detection skips a catalogue that is empty", () => {
  // No shipped catalogue is empty any more, so this guards a build somebody has broken:
  // an English panel beats a panel full of message keys.
  const catalogs = { en: { a: "a" }, tr: { a: "a" }, zh: {}, ko: { a: "a" }, ru: {}, es: {} };

  assert.equal(detectLocale(["tr-TR", "en-GB"], catalogs), "tr");
  assert.equal(detectLocale(["ko-KR"], catalogs), "ko");
  assert.equal(detectLocale(["de-DE"], catalogs), "en");
  assert.equal(detectLocale(["zh-CN", "tr"], catalogs), "tr");
  assert.equal(detectLocale([], catalogs), "en");
});

test("the real catalogues are all offered to a speaker who asks for them", () => {
  const catalogs = Object.fromEntries(LOCALES.map((locale) => [locale, load(locale)]));
  for (const locale of LOCALES) {
    assert.equal(detectLocale([`${locale}-XX`], catalogs), locale);
    const t = createTranslator(catalogs, locale);
    assert.equal(t("tray.menu.quit"), load(locale)["tray.menu.quit"]);
  }
});

// ------------------------------------------------------------ no hard-coded text
//
// WP6's acceptance criterion is "every UI string comes from locale files; no hard-coded
// text in code". These two tests are that criterion. They read the sources, throw away
// everything that is not a string literal a user could see, and compare what is left with
// an allow-list.
//
// What is thrown away by rule rather than listed:
//
//   * **Comments and doc comments.** They are prose by design.
//   * **Rust test modules**, both `#[cfg(test)] mod tests { … }` blocks and the whole
//     `*/tests.rs` files they include. A test's assertion message is written for whoever
//     reads the failure, and that reader is a developer.
//   * **Diagnostics.** The literal argument of `eprintln!`, `println!`, `panic!`,
//     `unreachable!`, `todo!`, `write!`, `writeln!` and `.expect(…)`. These reach a
//     terminal or a crash report, never the panel — and standard output is this
//     application's command-line surface, which `docs/PROJECT.md` keeps in English
//     deliberately: `--print` emits JSON a script parses, and `--autostart` answers a
//     question a maintainer asked in English. There is no `--help` in this build; when
//     WP8 adds one it stays English for the same reason.

/** Comments out, string literals intact. */
function stripComments(source) {
  let out = "";
  let index = 0;
  while (index < source.length) {
    const character = source[index];
    if (character === '"' || character === "'" || character === "`") {
      out += character;
      index++;
      while (index < source.length) {
        if (source[index] === "\\") {
          out += source[index] + (source[index + 1] ?? "");
          index += 2;
          continue;
        }
        out += source[index];
        if (source[index] === character) {
          index++;
          break;
        }
        index++;
      }
      continue;
    }
    if (character === "/" && source[index + 1] === "/") {
      while (index < source.length && source[index] !== "\n") index++;
      continue;
    }
    if (character === "/" && source[index + 1] === "*") {
      index += 2;
      while (index < source.length && !(source[index] === "*" && source[index + 1] === "/")) index++;
      index += 2;
      continue;
    }
    out += character;
    index++;
  }
  return out;
}

/** Every `#[cfg(test)] … { … }` block removed, by counting braces. */
function stripTestModules(source) {
  let out = source;
  for (;;) {
    const at = out.indexOf("#[cfg(test)]");
    if (at < 0) return out;
    const opening = out.indexOf("{", at);
    if (opening < 0) return out.slice(0, at);
    let depth = 0;
    let index = opening;
    for (; index < out.length; index++) {
      if (out[index] === "{") depth++;
      else if (out[index] === "}" && --depth === 0) {
        index++;
        break;
      }
    }
    out = out.slice(0, at) + out.slice(index);
  }
}

/** Every string literal in a comment-free source. */
function literals(source, quotes) {
  const found = [];
  let index = 0;
  while (index < source.length) {
    const quote = source[index];
    if (!quotes.includes(quote)) {
      index++;
      continue;
    }
    let value = "";
    index++;
    while (index < source.length) {
      if (source[index] === "\\") {
        value += source[index + 1] ?? "";
        index += 2;
        continue;
      }
      if (source[index] === quote) {
        index++;
        break;
      }
      value += source[index];
      index++;
    }
    found.push(value);
  }
  return found;
}

/** Two runs of letters with a space between them: what a sentence looks like. */
const PROSE = /[A-Za-z]{2,}[ ][A-Za-z]{2,}/;

/** Every file under `dir` with this extension, deepest first. */
function sources(dir, extension) {
  const found = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) found.push(...sources(path, extension));
    else if (entry.name.endsWith(extension)) found.push(path);
  }
  return found;
}

test("the panel has no words of its own", () => {
  /**
   * Allow-listed literals in `ui/src`, with the reason each is not a UI string.
   *
   * CSS class names. They are selectors the stylesheet matches on, they never reach a
   * screen, and translating one would unstyle the element it names.
   */
  const ALLOWED = new Set(["pill detailed", "pill plan"]);

  const offenders = [];
  for (const file of sources(resolve(UI, "src"), ".ts")) {
    const text = stripComments(readFileSync(file, "utf8"));
    for (const value of literals(text, ['"', "'", "`"])) {
      if (PROSE.test(value) && !ALLOWED.has(value)) {
        offenders.push(`${file.replace(REPO, "")}: ${JSON.stringify(value)}`);
      }
    }
  }
  assert.deepEqual(offenders, [], "every visible word must come from a locale file");
});

test("the tray has no words of its own", () => {
  /**
   * Allow-listed literals in `crates/nazar-tray/src`, with the reason for each.
   *
   * `demo.rs` — the sentence a demo window carries in its `error` field. A reader's error
   * text is the one thing the panel prints verbatim (see the comment on `windowRow` in
   * `ui/src/main.ts`): it says which file said what, and no translation can know that in
   * advance. The real ones come from `nazar-core`; this is the demo's stand-in for one,
   * and it is the string in every `--demo` screenshot.
   *
   * `desktop.rs` — the three answers `Mode::status` gives. They are standard-error
   * diagnostics, printed at start-up and by `--print`, and `ui/locales/README.md` states
   * the rule they follow: anything on the command line stays in English, because standard
   * output and standard error are a maintainer's surface rather than a user's. The thing
   * the *user* is told when the tray has nowhere to go is a notification, and it comes from
   * `tray.hidden.title` and `tray.hidden.body` like every other word this product shows.
   * They are here rather than inside an `eprintln!` — which the regex below would have
   * stripped — because two call sites read them and a sentence written twice is a sentence
   * that will one day say two things.
   *
   * `desktop.rs` again — the two spellings of the startup entry, for the same surface and
   * the same reason. `--autostart status` prints one of them on standard error, and which
   * one is a compile-time decision an `eprintln!` cannot make inline. What the *user* reads
   * is the settings row, and that comes from `settings.autostart` or
   * `settings.autostart.session` like every other word this product shows.
   */
  const ALLOWED = new Set([
    "no quota line in the newest session log",
    "tray icon shown",
    "engine mode (--headless): no tray icon, limits.json still written",
    "engine mode: no StatusNotifierWatcher on the session bus; limits.json still written",
    "start with Windows",
    "start with the desktop session",
  ]);

  const diagnostic =
    /(?:eprintln!|println!|panic!|unreachable!|todo!|unimplemented!|write!|writeln!|\.expect)\s*\(\s*(?:[^,()"]*,\s*)?"(?:[^"\\]|\\[\s\S])*"/g;

  const offenders = [];
  for (const file of sources(resolve(REPO, "crates/nazar-tray/src"), ".rs")) {
    // A file that is nothing but a test module — `src/icon/tests.rs` and its kind — is
    // included by `#[cfg(test)] mod tests;` and never compiled into the product.
    if (file.endsWith("tests.rs")) continue;
    let text = stripTestModules(readFileSync(file, "utf8"));
    text = stripComments(text).replace(diagnostic, "");
    for (const value of literals(text, ['"'])) {
      if (PROSE.test(value) && !ALLOWED.has(value)) {
        offenders.push(`${file.replace(REPO, "")}: ${JSON.stringify(value)}`);
      }
    }
  }
  assert.deepEqual(offenders, [], "the tooltip, the menu and the toasts read locale files");
});

test("the markup carries no text either, and every key it names exists", () => {
  const html = readFileSync(resolve(UI, "src/index.html"), "utf8").replace(/<!--[\s\S]*?-->/g, "");

  for (const [, key] of html.matchAll(/data-i18n="([^"]+)"/g)) {
    assert.ok(key in english, `index.html asks for ${key}, which is not in en.json`);
  }

  // `<title>` is the window title and is the product's own name, which is not translated.
  const title = /<title>([^<]*)<\/title>/.exec(html);
  assert.equal(title?.[1], "nazar-tray");

  const stray = [...html.matchAll(/>([^<]+)</g)]
    .map((match) => match[1].trim())
    .filter((text) => text !== "" && text !== "nazar-tray");
  assert.deepEqual(stray, [], "a word typed into index.html is a word no translation can reach");
});
