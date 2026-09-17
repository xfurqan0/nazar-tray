// The settings form: what it holds, what it sends, and what it refuses.
//
// Two properties, and the first is the one that would be a real bug:
//
// 1. **Opening the page changes nothing.** A form drawn from the settings and read straight
//    back has to be the same settings. If it were not, the act of looking at the page and
//    pressing Save would move something the user never touched.
// 2. **The panel and Rust agree about what is invalid.** `Config::validate` is what actually
//    refuses to save; `validate` here exists so the page can complain while the user is
//    still typing. Two validators that disagreed would produce a form that says it is fine
//    and a Save that says it is not.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  DEFAULT_QUIET,
  SYSTEM,
  invalidKey,
  isTime,
  languageKey,
  readNumber,
  showNumber,
  toForm,
  toValues,
  validate,
} from "../dist/lib/settings.mjs";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const read = (relative) => readFileSync(resolve(REPO, relative), "utf8");
const english = JSON.parse(read("ui/locales/en.json"));

/** The settings a fresh machine has, in the shape `get_config` sends them. */
const DEFAULTS = {
  locale: SYSTEM,
  theme: "nazar",
  themeMode: "system",
  notifications: true,
  quietHours: null,
  thresholds: { warn: 60, critical: 85, exhausted: 100 },
  providers: { claude: true, codex: true },
  detailedWindows: false,
  usageCountLikeClaudeCode: false,
  usageFillHistoryFromStats: false,
  // Linux's two, which every build still round-trips: the rows are hidden elsewhere and
  // the values are not, so a Windows Save cannot clear what a Linux session switched on.
  trayShowLabel: true,
  windowX11Positioning: false,
};

const LANGUAGES = ["en", "tr"];

test("a form drawn from the settings and read straight back is the same settings", () => {
  assert.deepEqual(toForm(toValues(DEFAULTS)), DEFAULTS);

  const chosen = {
    locale: "tr",
    theme: "graphite",
    themeMode: "dark",
    notifications: false,
    quietHours: { from: "22:30", to: "07:15" },
    thresholds: { warn: 50, critical: 80, exhausted: 95 },
    providers: { claude: false, codex: true },
    detailedWindows: true,
    usageCountLikeClaudeCode: true,
    usageFillHistoryFromStats: true,
    trayShowLabel: false,
    windowX11Positioning: true,
  };
  assert.deepEqual(toForm(toValues(chosen)), chosen);
});

test("the two shell switches are on the page, explained, and hidden off Linux", () => {
  // The rows exist in the markup on every platform — the panel is one bundle — and the
  // fieldset around them starts `hidden`, so a build whose Rust side says `desktopSwitches`
  // is false never draws them at all. That is `desktop::DESKTOP_SWITCHES`, the mirror of
  // the rule that keeps the Windows overflow hint off a Linux panel.
  const html = read("ui/src/index.html");
  assert.match(html, /<fieldset[^>]*data-desktop-switches[^>]*hidden/);
  for (const field of ["trayShowLabel", "windowX11Positioning"]) {
    assert.ok(html.includes(`data-field="${field}"`), `the form has no ${field} control`);
  }
  for (const key of [
    "settings.section.desktop",
    "settings.tray.showLabel",
    "settings.tray.showLabel.help",
    "settings.window.x11Positioning",
    "settings.window.x11Positioning.help",
  ]) {
    assert.ok(html.includes(`data-i18n="${key}"`), `${key} is not named in the markup`);
    assert.ok(english[key], `${key} is not in the English catalogue`);
  }
  // The cost is in the sentence rather than only in the README: the whole application moves
  // to XWayland and the change waits for a restart.
  assert.match(english["settings.window.x11Positioning.help"], /XWayland/);
  assert.match(english["settings.window.x11Positioning.help"], /next time/);
});

test("the panel draws its own dropdowns, because one desktop will not draw ours", () => {
  // On WebKitGTK a `<select>` is rendered by the platform: the background is the GTK combo's
  // and only the text colour is ours, which on a dark theme is near-white on near-white.
  // Measured on Fedora 44 / GNOME 50 with the language, theme and light/dark rows all blank.
  // `appearance: none` is what makes the box the one `.control` describes on both platforms.
  const css = read("ui/src/styles.css");
  const rule = /select\.control\s*\{([^}]*)\}/.exec(css);
  assert.ok(rule, "there is no select.control rule");
  assert.match(rule[1], /appearance:\s*none/);
  // And the arrow it takes away is drawn back, in a colour that follows the theme — a data
  // URI could not read a custom property, which is why this is gradients.
  assert.match(rule[1], /--color-text-muted/);
  assert.match(rule[1], /padding-right/, "the arrow needs room, or it sits on the text");
});

test("the gear is the door to the settings, and it is the only one in the header", () => {
  // The maintainer asked for a gear rather than the footer word it replaces; the word is
  // gone, so there is exactly one control that opens the page and it is the icon.
  const html = read("ui/src/index.html");
  const openers = [...html.matchAll(/data-open-settings/g)];
  assert.equal(openers.length, 1, "there should be exactly one control that opens settings");
  assert.match(html, /<button[^>]*class="gear"[^>]*data-open-settings/);
  // The icon carries no word, so its accessible name comes from the catalogue like every
  // other string on the page.
  assert.match(html, /data-i18n-label="settings.title"/);
  assert.ok(english["settings.title"], "settings.title is not in the English catalogue");
});

test("the two usage switches survive the round trip on their own", () => {
  // Off is the shipped answer on both, and the pair is independent: turning on the count
  // that matches `/usage` must not quietly turn on the imported history beside it.
  const values = toValues(DEFAULTS);
  assert.equal(values.usageCountLikeClaudeCode, false);
  assert.equal(values.usageFillHistoryFromStats, false);

  const counted = toForm({ ...values, usageCountLikeClaudeCode: true });
  assert.equal(counted.usageCountLikeClaudeCode, true);
  assert.equal(counted.usageFillHistoryFromStats, false);

  const filled = toForm({ ...values, usageFillHistoryFromStats: true });
  assert.equal(filled.usageCountLikeClaudeCode, false);
  assert.equal(filled.usageFillHistoryFromStats, true);
});

test("the settings page offers both usage switches and explains each of them", () => {
  const html = read("ui/src/index.html");
  for (const field of ["usageCountLikeClaudeCode", "usageFillHistoryFromStats"]) {
    assert.ok(
      html.includes(`data-field="${field}"`),
      `${field} has no control on the settings page`,
    );
  }
  // A switch with no sentence under it is a switch nobody can decide about: both of these
  // trade a property of the default for agreement with another program.
  for (const key of [
    "settings.section.usage",
    "settings.usage.perLine",
    "settings.usage.perLine.help",
    "settings.usage.history",
    "settings.usage.history.help",
  ]) {
    assert.ok(key in english, `${key} is missing from en.json`);
    assert.ok(html.includes(`data-i18n="${key}"`), `${key} is not drawn on the page`);
  }
  assert.match(english["settings.usage.perLine.help"], /1\.7/, "the factor is the point");
});

test("the numbers become fields and come back as numbers", () => {
  const values = toValues(DEFAULTS);
  assert.equal(values.warn, "60", "a text field holds text, and 60 is not written 60.0");
  assert.equal(values.critical, "85");
  assert.equal(values.exhausted, "100");
  assert.equal(toForm(values).thresholds.warn, 60);

  assert.equal(showNumber(62.5), "62.5");
  assert.equal(showNumber(Number.NaN), "");
  assert.equal(readNumber(" 85 "), 85);
  assert.ok(Number.isNaN(readNumber("")), "an emptied field is not zero");
  assert.ok(Number.isNaN(readNumber("eighty")), "and neither is a word");
});

test("quiet hours are a checkbox and two times, and null means there are none", () => {
  const off = toValues(DEFAULTS);
  assert.equal(off.quietHoursEnabled, false);
  assert.equal(
    off.quietFrom,
    DEFAULT_QUIET.from,
    "the fields still hold something, so ticking the box does not produce two empty times",
  );
  assert.equal(toForm(off).quietHours, null);

  const on = { ...off, quietHoursEnabled: true };
  assert.deepEqual(toForm(on).quietHours, DEFAULT_QUIET);

  // Unticking the box keeps the times in the form, so "I turned it off to look" does not
  // cost the user their hours.
  const kept = toValues({ ...DEFAULTS, quietHours: { from: "23:00", to: "06:30" } });
  assert.deepEqual(toForm({ ...kept, quietHoursEnabled: false }).quietHours, null);
  assert.equal(kept.quietFrom, "23:00");
});

test("`system` is the word the form uses for a language nobody chose", () => {
  assert.equal(toValues(DEFAULTS).locale, SYSTEM);
  assert.equal(toForm(toValues(DEFAULTS)).locale, SYSTEM);
  // Rust turns it into the absence of the key; see `SettingsForm::apply_to`. The panel only
  // has to send the word.
  const rust = read("crates/nazar-tray/src/state.rs");
  assert.match(rust, /LOCALE_SYSTEM\)\.then\(\|\| self\.locale\.clone\(\)\)/);
});

test("valid settings produce no complaints", () => {
  assert.deepEqual(validate(toValues(DEFAULTS), LANGUAGES), []);
  assert.deepEqual(
    validate(toValues({ ...DEFAULTS, locale: "tr", quietHours: DEFAULT_QUIET }), LANGUAGES),
    [],
  );
});

test("the thresholds have to climb, and stay between 1 and 100", () => {
  const base = toValues(DEFAULTS);
  for (const [warn, critical, exhausted] of [
    ["85", "60", "100"],
    ["60", "60", "100"],
    ["0", "85", "100"],
    ["60", "85", "120"],
    ["60", "", "100"],
    ["sixty", "85", "100"],
  ]) {
    assert.deepEqual(
      validate({ ...base, warn, critical, exhausted }, LANGUAGES),
      ["thresholds"],
      `${warn}/${critical}/${exhausted} should have been refused`,
    );
  }
  assert.deepEqual(validate({ ...base, warn: "1", critical: "2", exhausted: "100" }, LANGUAGES), []);
});

test("a language the build cannot paint is refused rather than quietly ignored", () => {
  const base = toValues(DEFAULTS);
  assert.deepEqual(validate({ ...base, locale: "de" }, LANGUAGES), ["locale"]);
  assert.deepEqual(validate({ ...base, locale: SYSTEM }, LANGUAGES), []);
  assert.deepEqual(
    validate({ ...base, locale: "ko" }, LANGUAGES),
    ["locale"],
    "the list Rust sends is the whole rule: a language it did not offer is refused, whatever \
     catalogues this build happens to carry",
  );
  // And the six it does offer are all accepted, which is what changed in WP6.
  for (const locale of ["en", "tr", "zh", "ko", "ru", "es"]) {
    assert.deepEqual(
      validate({ ...base, locale }, ["en", "tr", "zh", "ko", "ru", "es"]),
      [],
      `${locale} is a language v1 ships and the form must accept it`,
    );
  }
});

test("quiet hours need two times, and a typo is a refusal rather than silence", () => {
  const base = { ...toValues(DEFAULTS), quietHoursEnabled: true };
  assert.deepEqual(validate({ ...base, quietFrom: "10pm" }, LANGUAGES), ["quietHours"]);
  assert.deepEqual(validate({ ...base, quietTo: "7:00" }, LANGUAGES), ["quietHours"]);
  assert.deepEqual(
    validate({ ...base, quietHoursEnabled: false, quietFrom: "nonsense" }, LANGUAGES),
    [],
    "a time nobody is using is not a problem",
  );

  for (const good of ["00:00", "07:05", "23:59"]) assert.ok(isTime(good), good);
  for (const bad of ["7:00", "24:00", "22:60", "2200", "", "aa:bb"]) assert.ok(!isTime(bad), bad);
});

test("light or dark has to be one of the three the panel knows", () => {
  const base = toValues(DEFAULTS);
  assert.deepEqual(validate({ ...base, themeMode: "sepia" }, LANGUAGES), ["themeMode"]);
  for (const mode of ["system", "light", "dark"]) {
    assert.deepEqual(validate({ ...base, themeMode: mode }, LANGUAGES), []);
  }
});

test("the panel and Rust agree about what is invalid", () => {
  const rust = read("crates/nazar-core/src/config.rs");

  // Every variant Rust can return has to have a message, or the form would show a blank
  // complaint. The names are the same words on both sides: serde renames to camelCase.
  const variants = /pub enum Invalid \{([\s\S]*?)\n\}/.exec(rust);
  assert.ok(variants, "config.rs no longer has an Invalid enum");
  const names = [...variants[1].matchAll(/^\s{4}([A-Z]\w+),$/gm)].map((match) => match[1]);
  assert.ok(names.length >= 5, `expected the five problems, found ${names.join(", ")}`);

  for (const name of names) {
    const camel = name[0].toLowerCase() + name.slice(1);
    assert.ok(
      invalidKey(camel) in english,
      `Invalid::${name} has no message: en.json needs ${invalidKey(camel)}`,
    );
  }

  // And the two agree on the rule that is easiest to get subtly different.
  assert.match(rust, /pair\[0\] < pair\[1\]/, "Rust checks the thresholds strictly ascend");
  assert.match(rust, /\*value > 0\.0 && \*value <= 100\.0/, "and that each is in 0 < v <= 100");
});

test("every language the settings can offer has a name of its own", () => {
  const rust = read("crates/nazar-core/src/config.rs");
  const declared = /pub const LOCALES: \[&str; \d+\] = \[([^\]]+)\]/.exec(rust);
  assert.ok(declared, "config.rs no longer declares LOCALES");
  const locales = [...declared[1].matchAll(/"(\w+)"/g)].map((match) => match[1]);

  assert.ok(
    "settings.language.system" in english,
    "`system` is an option in the picker too, and it is the only one that is a sentence",
  );
  assert.ok(!locales.includes(SYSTEM), "`system` is a word the form uses, never a locale");
  for (const locale of locales) {
    assert.ok(languageKey(locale) in english, `${languageKey(locale)} is missing from en.json`);
  }
});

test("the toast says the sentence the plan asks for, in both languages", () => {
  // The words come from the locale files that Rust compiles in, so this is the same
  // catalogue `crates/nazar-tray/src/alerts.rs` reads. What is checked here is the
  // *template*: that the placeholders are the ones the Rust side fills in, and that the
  // finished sentence reads the way `docs/PROJECT.md` says it should.
  const turkish = JSON.parse(read("ui/locales/tr.json"));
  const fill = (template, params) =>
    template.replace(/\{(\w+)\}/g, (whole, name) => params[name] ?? whole);

  assert.equal(
    fill(english["alert.title"], {
      provider: english["panel.provider.claude"],
      window: english["alert.window.weekly"],
      percent: "85",
    }),
    "Claude Code · weekly window 85 %",
  );
  assert.equal(
    fill(english["alert.body.resets"], { time: fill(english["time.hoursMinutes"], { hours: 2, minutes: 10 }) }),
    "Resets in 2 h 10 m",
  );

  assert.equal(
    fill(turkish["alert.title"], {
      provider: turkish["panel.provider.claude"],
      window: turkish["alert.window.weekly"],
      percent: "85",
    }),
    "Claude Code · haftalık pencere %85",
  );
  assert.equal(
    fill(turkish["alert.body.resets"], { time: fill(turkish["time.hoursMinutes"], { hours: 2, minutes: 10 }) }),
    "2 sa 10 dk sonra sıfırlanır",
  );

  // The units are in the locale file too: "h" is not "sa", and a body assembled from an
  // English duration would be half-translated.
  assert.notEqual(english["time.hoursMinutes"], turkish["time.hoursMinutes"]);
  assert.equal(
    fill(english["alert.window.modelWeekly"], { model: "Fable" }),
    "Fable weekly window",
    "a model-scoped weekly says which model; two 'weekly window's in one account would lie",
  );
});
