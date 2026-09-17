/**
 * The settings form, as values and as a payload — and nothing that touches the DOM.
 *
 * There are two shapes of the same settings and this file is the only place that knows how
 * to get from one to the other:
 *
 * * **{@link SettingsForm}** is what Rust reads and writes. It is the `config.json` document
 *   minus the keys the form does not own, and `quietHours` is either an object or `null`.
 * * **{@link FormValues}** is what a form full of `<input>` elements holds: strings for the
 *   numbers, because that is what a text field contains, and a separate checkbox for "are
 *   there quiet hours at all" because an empty pair of time fields is not the same statement
 *   as no quiet hours.
 *
 * Keeping the conversion here rather than in `main.ts` is what makes it testable without a
 * browser — `test/settings.test.mjs` imports this file and nothing else — and the round trip
 * is the property that matters: a form drawn from settings and read straight back must be
 * the same settings, or the act of *opening* the page would change something.
 *
 * **The validation is a mirror, not the gate.** `Config::validate` in Rust is what actually
 * refuses to save, and it runs on the document after the form has been applied to it. The
 * copy here exists so the page can say what is wrong while the user is still typing, and
 * `test/settings.test.mjs` checks the two agree on the cases that matter.
 */

/** A stretch of local time in which notifications are not shown. */
export interface QuietHours {
  readonly from: string;
  readonly to: string;
}

/** The three percentages a notification fires at. */
export interface Thresholds {
  readonly warn: number;
  readonly critical: number;
  readonly exhausted: number;
}

/** Which providers are read at all. */
export interface ProviderSwitches {
  readonly claude: boolean;
  readonly codex: boolean;
}

/** The settings form as Rust reads and writes it. */
export interface SettingsForm {
  readonly locale: string;
  readonly theme: string;
  readonly themeMode: string;
  readonly notifications: boolean;
  readonly quietHours: QuietHours | null;
  readonly thresholds: Thresholds;
  readonly providers: ProviderSwitches;
  readonly detailedWindows: boolean;
  /** Show the per-line numbers `/usage` shows, rather than the deduplicated spend. */
  readonly usageCountLikeClaudeCode: boolean;
  /** Show the days before the transcripts, as Claude Code reported them. */
  readonly usageFillHistoryFromStats: boolean;
  /** Draw the binding percentage beside the tray icon. Linux only; the row is hidden
   * elsewhere, and the value still round-trips so a saved form cannot clear it. */
  readonly trayShowLabel: boolean;
  /** Open the panel through XWayland, where a position is honoured. Linux only. */
  readonly windowX11Positioning: boolean;
}

/** Where the files live, with `~` already collapsed by the Rust side. */
export interface SettingsPaths {
  readonly config: string;
  readonly limits: string;
  readonly captures: string;
  readonly alerts: string;
}

/** Everything `get_config` returns. */
export interface SettingsView {
  readonly form: SettingsForm;
  readonly resolvedLocale: string;
  readonly languages: readonly string[];
  readonly firstRunHintDismissed: boolean;
  readonly suggestDetailed: boolean;
  readonly paths: SettingsPaths;
  readonly version: string;
  readonly demo: boolean;
  /** Whether the two shell-specific rows belong on this page. Rust's answer, not a guess
   * from `navigator.platform` — the rule the startup row's label already follows. */
  readonly desktopSwitches: boolean;
  readonly writable: boolean;
}

/** What `set_config` rejects a form for. The same words `nazar_core::config::Invalid` uses. */
export type Invalid = "thresholds" | "locale" | "themeMode" | "quietHours" | "freshness";

/** The value the language and mode selects use for "whatever the machine says". */
export const SYSTEM = "system";

/** The quiet hours a user gets when they tick the box and say nothing else. */
export const DEFAULT_QUIET: QuietHours = { from: "22:00", to: "07:00" };

/** What a page full of form controls holds. */
export interface FormValues {
  locale: string;
  theme: string;
  themeMode: string;
  notifications: boolean;
  quietHoursEnabled: boolean;
  quietFrom: string;
  quietTo: string;
  /** As typed. A field being emptied is a state the user passes through. */
  warn: string;
  critical: string;
  exhausted: string;
  claude: boolean;
  codex: boolean;
  detailedWindows: boolean;
  usageCountLikeClaudeCode: boolean;
  usageFillHistoryFromStats: boolean;
  trayShowLabel: boolean;
  windowX11Positioning: boolean;
}

/** A number as a form field should show it: `85`, not `85.0`. */
export function showNumber(value: number): string {
  return Number.isFinite(value) ? String(Number(value.toFixed(2))) : "";
}

/** Fill a form from the settings. */
export function toValues(form: SettingsForm): FormValues {
  return {
    locale: form.locale,
    theme: form.theme,
    themeMode: form.themeMode,
    notifications: form.notifications,
    // The times survive the checkbox being unticked and re-ticked within one visit, which
    // is what stops "I turned it off to see" from costing the user their hours.
    quietHoursEnabled: form.quietHours !== null,
    quietFrom: form.quietHours?.from ?? DEFAULT_QUIET.from,
    quietTo: form.quietHours?.to ?? DEFAULT_QUIET.to,
    warn: showNumber(form.thresholds.warn),
    critical: showNumber(form.thresholds.critical),
    exhausted: showNumber(form.thresholds.exhausted),
    claude: form.providers.claude,
    codex: form.providers.codex,
    detailedWindows: form.detailedWindows,
    usageCountLikeClaudeCode: form.usageCountLikeClaudeCode,
    usageFillHistoryFromStats: form.usageFillHistoryFromStats,
    trayShowLabel: form.trayShowLabel,
    windowX11Positioning: form.windowX11Positioning,
  };
}

/** A threshold field as a number, or `NaN` when it is not one. */
export function readNumber(text: string): number {
  const trimmed = text.trim();
  if (trimmed === "") return Number.NaN;
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : Number.NaN;
}

/** Read a form back into the settings Rust expects. */
export function toForm(values: FormValues): SettingsForm {
  return {
    locale: values.locale,
    theme: values.theme,
    themeMode: values.themeMode,
    notifications: values.notifications,
    quietHours: values.quietHoursEnabled
      ? { from: values.quietFrom, to: values.quietTo }
      : null,
    thresholds: {
      warn: readNumber(values.warn),
      critical: readNumber(values.critical),
      exhausted: readNumber(values.exhausted),
    },
    providers: { claude: values.claude, codex: values.codex },
    detailedWindows: values.detailedWindows,
    usageCountLikeClaudeCode: values.usageCountLikeClaudeCode,
    usageFillHistoryFromStats: values.usageFillHistoryFromStats,
    trayShowLabel: values.trayShowLabel,
    windowX11Positioning: values.windowX11Positioning,
  };
}

/** `HH:MM` on a 24-hour clock, and nothing else. The same rule `config.rs` applies. */
export function isTime(text: string): boolean {
  const match = /^(\d{2}):(\d{2})$/.exec(text);
  if (!match) return false;
  return Number(match[1]) <= 23 && Number(match[2]) <= 59;
}

/**
 * Everything wrong with a form, in the words Rust uses.
 *
 * A mirror of `Config::validate`, so the page can complain while the user is typing rather
 * than only after a round trip. Rust still has the last word — it validates the whole
 * document, including keys this form does not own — and the two must not disagree about
 * these cases, which is what `test/settings.test.mjs` checks.
 */
export function validate(values: FormValues, languages: readonly string[]): Invalid[] {
  const problems: Invalid[] = [];
  const steps = [readNumber(values.warn), readNumber(values.critical), readNumber(values.exhausted)];
  const inRange = steps.every((value) => Number.isFinite(value) && value > 0 && value <= 100);
  const ascending = steps[0]! < steps[1]! && steps[1]! < steps[2]!;
  if (!inRange || !ascending) problems.push("thresholds");

  if (values.locale !== SYSTEM && !languages.includes(values.locale)) problems.push("locale");
  if (!["system", "light", "dark"].includes(values.themeMode)) problems.push("themeMode");
  if (values.quietHoursEnabled && !(isTime(values.quietFrom) && isTime(values.quietTo))) {
    problems.push("quietHours");
  }
  return problems;
}

/** The message key that explains a problem. */
export function invalidKey(problem: Invalid): string {
  return `settings.invalid.${problem}`;
}

/** The message key for a language's own name in the picker. */
export function languageKey(tag: string): string {
  return `settings.language.${tag}`;
}
