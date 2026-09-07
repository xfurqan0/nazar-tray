/**
 * The whole i18n runtime: a catalogue lookup, `{placeholder}` interpolation and a
 * language guess. Deliberately about sixty lines — ICU is not needed for a tray panel,
 * and the policy in docs/PROJECT.md is flat key/value files with simple placeholders.
 *
 * The rule the rest of the panel is written against: **no string is ever hard-coded in
 * code**. Every visible word comes from `locales/<lang>.json` through `t()`.
 */

/** Every language v1 ships. EN and TR are written by hand; the rest land in WP6. */
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
 * Guess the language from the browser's preference list.
 *
 * **Stub, on purpose.** WP5 replaces it with the Windows UI language read in Rust plus
 * an explicit override in settings; a WebView2 page reports the browser's idea of the
 * user's languages, which is close but not the same thing.
 *
 * A language whose catalogue is still empty is skipped rather than selected: until WP6
 * fills ZH, KO, RU and ES, a speaker of one of them gets a fully English panel instead
 * of an English panel that claims to be translated.
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
