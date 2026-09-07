/**
 * The whole i18n runtime: a catalogue lookup, `{placeholder}` interpolation, a language
 * guess and a plural rule. Deliberately about a hundred lines — ICU is not needed for a
 * tray panel, and the policy in docs/PROJECT.md is flat key/value files with simple
 * placeholders.
 *
 * The rule the rest of the panel is written against: **no string is ever hard-coded in
 * code**. Every visible word comes from `locales/<lang>.json` through `t()`, and
 * `test/i18n.test.mjs` greps `ui/src` and `crates/nazar-tray/src` to prove it.
 *
 * **Flat strings only.** A locale file is a JSON object whose every value is a string.
 * Not a stylistic rule: the Rust side reads the same files as `BTreeMap<String, String>`
 * (`crates/nazar-tray/src/i18n.rs`), so one nested object anywhere in a file makes the
 * whole file fail to parse and that language falls back to English **without a word of
 * complaint** — it simply stops being offered in the settings. That is why the
 * translation status lives in `locales/README.md` and not in a `_meta` key.
 *
 * **Writing direction.** All six languages v1 ships are left-to-right, so the panel sets
 * `<html lang>` and never `dir`. Adding Arabic, Hebrew or Persian means setting
 * `document.documentElement.dir` from a per-locale table here **and** auditing
 * `styles.css`, which still uses physical `margin-left` / `text-align: right` in places;
 * neither is done, because doing it against no RTL locale would be untested code.
 */

/** Every language v1 ships. All six are complete; EN and TR are the authored pair. */
export const LOCALES = ["en", "tr", "zh", "ko", "ru", "es"] as const;

/** One of {@link LOCALES}. */
export type Locale = (typeof LOCALES)[number];

/** The language every missing string falls back to. */
export const FALLBACK_LOCALE: Locale = "en";

/** A flat map of message key to message template. */
export type Catalog = Readonly<Record<string, string>>;

/** Every catalogue, keyed by language. */
export type Catalogs = Readonly<Record<Locale, Catalog>>;

/** Values substituted into `{placeholder}` slots. */
export type Params = Readonly<Record<string, string | number>>;

/** A bound lookup: key in, finished string out. */
export type Translate = (key: string, params?: Params) => string;

/** Narrow an arbitrary string to a supported language. */
export function isLocale(value: string): value is Locale {
  return (LOCALES as readonly string[]).includes(value);
}

/**
 * Replace `{name}` with `params.name`.
 *
 * A placeholder with no matching parameter is left as written rather than blanked, so a
 * missing value shows up as `{version}` in the panel instead of disappearing quietly.
 */
export function interpolate(template: string, params?: Params): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (whole, name: string) => {
    const value = params[name];
    return value === undefined ? whole : String(value);
  });
}

/**
 * Bind a catalogue set to a language.
 *
 * Lookup order: the chosen language, then English, then the key itself. Returning the
 * key makes an untranslated string obvious in the panel instead of rendering as a gap.
 */
export function createTranslator(catalogs: Catalogs, locale: Locale): Translate {
  return (key, params) => {
    const template = catalogs[locale][key] ?? catalogs[FALLBACK_LOCALE][key] ?? key;
    return interpolate(template, params);
  };
}

/**
 * The plural category a count falls into, for the languages this product ships.
 *
 * Three of the six need no rule at all — Chinese, Korean and Turkish leave a noun alone
 * after a numeral — English and Spanish need two forms, and Russian needs the three the
 * grammar books give: **1** (`one`), **2–4** (`few`), **5 and up** (`other`), with the
 * teens 11–14 taken out of both, so 21 is `one` and 111 is `other`.
 *
 * `Intl.PluralRules` would answer the same question, and is deliberately not used: it
 * would put the correctness of a shipped string at the mercy of the WebView2 version on
 * the user's machine, and the Rust half of this product (tooltip, menu, toasts) has no
 * `Intl` to agree with. Fifteen lines that both halves can read is the cheaper contract.
 */
export type PluralCategory = "one" | "few" | "other";

/** The plural category of `count` in `locale`. Negatives and fractions count by magnitude. */
export function pluralCategory(count: number, locale: Locale): PluralCategory {
  const n = Math.abs(Math.trunc(count));
  if (locale === "ru") {
    const tens = n % 100;
    const units = n % 10;
    if (tens >= 11 && tens <= 14) return "other";
    if (units === 1) return "one";
    if (units >= 2 && units <= 4) return "few";
    return "other";
  }
  if (locale === "en" || locale === "es") return n === 1 ? "one" : "other";
  return "other";
}

/**
 * Pick the form of a counted word that goes with `count`.
 *
 * **Nothing in v1 calls this**, and that is a fact about the strings rather than an
 * oversight: every counted string in this product is a unit *abbreviation* — `4 d 2 h`,
 * `{minutes} мин`, `{seconds}초` — and an abbreviation does not inflect in any of the six.
 * The helper exists because the first counted **word** anybody writes will otherwise be
 * written wrong in Russian, and `test/i18n.test.mjs` freezes the set of keys that carry a
 * count so that adding one is a decision somebody makes on purpose.
 */
export function plural(
  count: number,
  forms: Readonly<Partial<Record<PluralCategory, string>>>,
  locale: Locale,
): string {
  return forms[pluralCategory(count, locale)] ?? forms.other ?? "";
}

/**
 * Guess the language from the browser's preference list.
 *
 * **Stub, on purpose.** WP5 replaced it with the Windows UI language read in Rust plus
 * an explicit override in settings; a WebView2 page reports the browser's idea of the
 * user's languages, which is close but not the same thing. It survives as the fallback
 * for a page opened outside the tray.
 *
 * A language whose catalogue is empty is skipped rather than selected, so a half-shipped
 * translation gives a speaker a fully English panel instead of an English panel that
 * claims to be translated. Since WP6 filled ZH, KO, RU and ES, no shipped catalogue is
 * empty and the guard only fires for a build somebody has broken.
 */
export function detectLocale(preferred: readonly string[], catalogs: Catalogs): Locale {
  for (const tag of preferred) {
    const primary = tag.toLowerCase().split("-")[0] ?? "";
    if (isLocale(primary) && Object.keys(catalogs[primary]).length > 0) {
      return primary;
    }
  }
  return FALLBACK_LOCALE;
}
