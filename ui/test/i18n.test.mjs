// Locale files and the translation helper.
//
// The hard rule is EN/TR parity: those two are written by the maintainer and a gap in
// either is a bug, not a to-do. ZH, KO, RU and ES are machine-translated in WP6, so a
// missing key there is reported and counted, not failed on — otherwise the whole suite
// would be red from now until that package lands, and a red suite stops being read.

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { LOCALES, createTranslator, detectLocale, interpolate } from "../dist/lib/i18n.mjs";

const UI = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const LOCALE_DIR = resolve(UI, "locales");

/** Languages written by hand. A missing key in one of these fails the build. */
const AUTHORED = ["en", "tr"];

const load = (locale) => JSON.parse(readFileSync(resolve(LOCALE_DIR, `${locale}.json`), "utf8"));
const placeholders = (template) =>
  new Set([...template.matchAll(/\{(\w+)\}/g)].map((match) => match[1]));

test("every declared language has a locale file, and there are no stray ones", () => {
  const onDisk = readdirSync(LOCALE_DIR)
    .filter((name) => name.endsWith(".json"))
    .map((name) => name.replace(/\.json$/, ""))
    .sort();
  assert.deepEqual(onDisk, [...LOCALES].sort());
});

test("every locale file is a flat object of strings", () => {
  for (const locale of LOCALES) {
    const catalog = load(locale);
    assert.equal(typeof catalog, "object", `${locale}.json must hold an object`);
    assert.ok(!Array.isArray(catalog), `${locale}.json must not be an array`);
    for (const [key, value] of Object.entries(catalog)) {
      assert.equal(typeof value, "string", `${locale}.json: ${key} must be a string`);
    }
  }
});

test("Turkish covers every English key, and adds none of its own", () => {
  const en = Object.keys(load("en")).sort();
  const tr = Object.keys(load("tr")).sort();
  assert.deepEqual(tr, en);
});

test("authored translations keep the placeholders of the English original", () => {
  const en = load("en");
  for (const locale of AUTHORED) {
    const catalog = load(locale);
    for (const [key, template] of Object.entries(catalog)) {
      assert.deepEqual(
        [...placeholders(template)].sort(),
        [...placeholders(en[key])].sort(),
        `${locale}.json: ${key} has different placeholders from English`,
      );
    }
  }
});

test("no locale invents a key English does not have", () => {
  const en = load("en");
  for (const locale of LOCALES) {
    for (const key of Object.keys(load(locale))) {
      assert.ok(key in en, `${locale}.json has ${key}, which is not in en.json`);
    }
  }
});

test("machine-translated languages report their gaps without failing (WP6)", () => {
  const en = Object.keys(load("en"));
  const pending = LOCALES.filter((locale) => !AUTHORED.includes(locale));

  for (const locale of pending) {
    const missing = en.filter((key) => !(key in load(locale)));
    if (missing.length > 0) {
      console.warn(
        `warning: ${locale}.json is missing ${missing.length}/${en.length} keys (WP6): ${missing.join(", ")}`,
      );
    }
  }
  // The placeholder files exist and parse; that is all WP0 promises.
  assert.ok(pending.length > 0);
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
  assert.equal(tr("greeting", { name: "qarpus" }), "merhaba qarpus");
  assert.equal(tr("only"), "english");
  assert.equal(tr("panel.does.not.exist"), "panel.does.not.exist");
});

test("language detection skips languages that are not translated yet", () => {
  const catalogs = {
    en: { a: "a" },
    tr: { a: "a" },
    zh: {},
    ko: {},
    ru: {},
    es: {},
  };

  assert.equal(detectLocale(["tr-TR", "en-GB"], catalogs), "tr");
  assert.equal(detectLocale(["en-US"], catalogs), "en");
  assert.equal(detectLocale(["de-DE"], catalogs), "en");
  // ZH has a file but no strings: English is more useful than an empty panel.
  assert.equal(detectLocale(["zh-CN", "tr"], catalogs), "tr");
  assert.equal(detectLocale([], catalogs), "en");
});
