/**
 * The panel: what the tray shows when you click the bead.
 *
 * One card per provider, one row per window, a bar in the bead's colours, a countdown that
 * ticks locally, and a line saying how old the reading is. The Rust side derives everything
 * that depends on the clock (`get_snapshot`); this file turns it into words and pixels and
 * does the arithmetic for the seconds in between.
 *
 * What is deliberate here, and why:
 *
 * * **Nothing is hard-coded.** Every word comes from `locales/<lang>.json` through `t()`.
 *   The numerals, colons and percent signs are punctuation and need no translation.
 * * **Unknown is a word and an empty track**, never a bar of zero length (audit B03).
 * * **The countdown is local arithmetic.** The document changes when the numbers change,
 *   which can be minutes apart; a display that stood still for a minute would look broken.
 * * **The panel measures itself.** After every render it asks the Rust side for a window
 *   that fits, so the first-run hint can appear and disappear without leaving a gap.
 * * **The theme is the user's, and it is remembered.** The toggle writes through
 *   `set_theme` into `config.json`; light and dark follow `prefers-color-scheme` unless
 *   the settings say otherwise.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  ageText,
  CLOCK_LIMIT,
  formatDuration,
  freshnessClass,
  labelText,
  meterWidth,
  nextTheme,
  resetClock,
  resetTitle,
  resolveMode,
  severityClass,
  windowLabel,
  type Translate,
} from "./format";
import { createTranslator, isLocale, type Locale } from "./i18n";
import { catalogs } from "./locales";
import {
  displayPercent,
  formatClock,
  type ProviderView,
  type SnapshotView,
  type WindowView,
} from "./snapshot";
import {
  DEFAULT_QUIET,
  SYSTEM,
  invalidKey,
  languageKey,
  toForm,
  toValues,
  validate,
  type FormValues,
  type Invalid,
  type SettingsView,
} from "./settings";
import { THEMES, applyTheme } from "./theme";

/** What `get_ui_state` returns: the choices, as against the measurements. */
interface UiState {
  readonly theme: string;
  readonly mode: string;
  readonly locale: string | null;
  /**
   * The language Rust decided on: the override, then the machine, then English.
   *
   * The panel used to guess this itself from `navigator.languages` while the tray fell
   * back to English, so the two could disagree — WP4's open risk. There is one answer now
   * and this is it.
   */
  readonly resolvedLocale: string;
  readonly hintDismissed: boolean;
  readonly demo: boolean;
  /** `--view settings`: open on the settings page rather than on the numbers. */
  readonly openSettings: boolean;
}

/**
 * Where to go for the source and for the contract.
 *
 * Written here rather than in the locale files because an address is data, not a word:
 * translating `github.com/xfurqan0/nazar-tray` would be translating a street name. They are
 * shown as selectable text rather than as links — the panel's content security policy is
 * `default-src 'self'`, and opening a browser from a webview needs a plugin and a permission
 * this application does not have.
 */
const LINKS: Readonly<Record<string, string>> = {
  repository: "github.com/xfurqan0/nazar-tray",
  contract: "docs/limits-contract.md",
};

/** The provider badges, from `@lobehub/icons` (MIT — see assets/LICENSE-lobehub.txt). */
const BADGES: Readonly<Record<string, string>> = {
  claude: "assets/claudecode-color.png",
  codex: "assets/codex-color.png",
};

const root = document.documentElement;
const panel = document.querySelector<HTMLElement>("[data-panel]");
const providersList = document.querySelector<HTMLElement>("[data-providers]");
const emptyLine = document.querySelector<HTMLElement>("[data-empty]");
const hintBox = document.querySelector<HTMLElement>("[data-hint]");
const demoPill = document.querySelector<HTMLElement>("[data-demo]");
const themeButton = document.querySelector<HTMLButtonElement>("[data-theme]");
const refreshButton = document.querySelector<HTMLButtonElement>("[data-refresh]");
const dismissButton = document.querySelector<HTMLButtonElement>("[data-dismiss]");
const versionLabel = document.querySelector<HTMLElement>("[data-version]");

// The settings view. Every one of these is inside the same window as the quota view; see
// index.html for why there is not a second window.
const views = document.querySelectorAll<HTMLElement>("[data-view]");
const settingsForm = document.querySelector<HTMLFormElement>("[data-settings]");
const settingsErrors = document.querySelector<HTMLElement>("[data-settings-errors]");
const settingsVersion = document.querySelector<HTMLElement>("[data-settings-version]");
const readOnlyNote = document.querySelector<HTMLElement>("[data-read-only]");
const savedNote = document.querySelector<HTMLElement>("[data-saved]");
const quietTimes = document.querySelector<HTMLElement>("[data-quiet-times]");
const autostartBox = document.querySelector<HTMLInputElement>("[data-autostart]");
const autostartError = document.querySelector<HTMLElement>("[data-autostart-error]");
const hintResetNote = document.querySelector<HTMLElement>("[data-hint-reset-note]");
const suggestionBox = document.querySelector<HTMLElement>("[data-suggestion]");

let ui: UiState = {
  theme: "nazar",
  mode: "system",
  locale: null,
  resolvedLocale: "en",
  hintDismissed: true,
  demo: false,
  openSettings: false,
};

/** The settings as `get_config` last reported them, and the form's own working copy. */
let settings: SettingsView | undefined;
let values: FormValues | undefined;
let locale: Locale = "en";
let t: Translate = createTranslator(catalogs, locale);

/** The last snapshot, and the local instant it was derived for. */
let latest: SnapshotView | undefined;
let derivedAt = 0;

/** How far the local clock has moved since the snapshot was derived. */
function drift(): number {
  return latest ? Date.now() - derivedAt : 0;
}

/** A piece of text that has to be rewritten every second. */
type Ticker = (elapsed: number) => void;

let tickers: Ticker[] = [];

/** The dark-mode query, kept so the panel follows the system while it is open. */
const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

// ---------------------------------------------------------------- rendering

/** Apply the language to every static string in the document. */
function applyLanguage(): void {
  root.lang = locale;
  for (const node of document.querySelectorAll<HTMLElement>("[data-i18n]")) {
    const key = node.dataset["i18n"];
    if (key) node.textContent = t(key);
  }
  document
    .querySelector<SVGElement>("[data-bead] svg")
    ?.setAttribute("aria-label", t("panel.bead.alt"));
  if (versionLabel) versionLabel.textContent = __APP_VERSION__;
  for (const node of document.querySelectorAll<HTMLElement>("[data-link]")) {
    const key = node.dataset["link"];
    if (key && LINKS[key]) node.textContent = LINKS[key];
  }
  if (themeButton) {
    themeButton.setAttribute("aria-label", t("panel.action.theme"));
    themeButton.title = t("panel.action.theme");
  }
  // The language picker's options are generated rather than written in the markup, so
  // `[data-i18n]` cannot reach them; they are rebuilt in the language that was just chosen.
  if (settings) paintForm();
}

/** Paint the panel in the chosen theme, following the system for light and dark. */
function applyChosenTheme(): void {
  const theme = THEMES[ui.theme] ?? THEMES["nazar"];
  if (!theme) return;
  applyTheme(root, theme, resolveMode(ui.mode, darkQuery.matches));
  // The theme's own `label` is English in both theme files. The settings picker has always
  // named the same two themes through message keys, so the footer toggle would otherwise
  // read "Graphite" beneath a dropdown reading "Grafit". The key wins; `label` stays in the
  // theme files as the name the brand definition gives them, which is not a UI string.
  if (themeButton) themeButton.textContent = t(`settings.theme.${theme.name}`);
}

/** One window row. */
function windowRow(usage: WindowView): HTMLElement {
  const row = document.createElement("li");
  row.className = `window ${severityClass(usage.severity)}`;
  if (usage.binding) {
    row.classList.add("binding");
    row.title = t("panel.window.binding");
  }

  const head = document.createElement("div");
  head.className = "window-head";

  const label = document.createElement("span");
  label.className = "window-label";
  label.textContent = labelText(windowLabel(usage), t);
  head.append(label);

  if (usage.detailed) {
    const mark = document.createElement("span");
    mark.className = "pill detailed";
    mark.textContent = t("panel.window.detailed");
    mark.title = t("panel.window.detailedTitle");
    head.append(mark);
  }

  // `?? undefined` throughout: the Rust side leaves an empty field out of the JSON
  // altogether, and a `null` that slipped through would become `0` after one `Math.floor` —
  // which is the difference between "nobody read this" and "you have used none of it".
  const used = usage.percent ?? undefined;

  const percent = document.createElement("span");
  percent.className = "window-percent";
  percent.textContent =
    used === undefined
      ? t("panel.window.unknown")
      : t("panel.window.percent", { percent: displayPercent(used) });
  head.append(percent);
  row.append(head);

  // The bar repeats the number beside it, so it is decoration to a screen reader.
  const meter = document.createElement("div");
  meter.className = "meter";
  meter.setAttribute("aria-hidden", "true");
  if (used !== undefined) {
    const fill = document.createElement("span");
    fill.className = "meter-fill";
    fill.style.width = `${meterWidth(used)}%`;
    meter.append(fill);
  }
  row.append(meter);

  if (usage.remainingMs != null && usage.resetsAt) {
    const foot = document.createElement("div");
    foot.className = "window-foot";

    const countdown = document.createElement("span");
    foot.append(countdown);

    const at = document.createElement("span");
    at.className = "reset-at";
    const resetsAt = new Date(usage.resetsAt);
    at.textContent = resetClock(resetsAt, usage.remainingMs, locale);
    at.title = resetTitle(resetsAt, locale);
    foot.append(at);

    const remaining = usage.remainingMs;
    tickers.push((elapsed) => {
      const left = remaining - elapsed;
      // A clock below a day, words above it: a weekly window has four days left, and
      // `98:59:59` is not something anybody reads as four days.
      const time = left < CLOCK_LIMIT ? formatClock(left) : formatDuration(left, t);
      countdown.textContent =
        left <= 0 ? t("panel.window.resetDue") : t("panel.window.resetsIn", { time });
    });
    row.append(foot);
  }

  if (usage.error) {
    const error = document.createElement("p");
    error.className = "window-error";
    // The reader's own sentence. Not a message key: it says which file said what, and no
    // translation could know that in advance.
    error.textContent = usage.error;
    row.append(error);
  }

  return row;
}

/** One provider card. */
function providerCard(provider: ProviderView): HTMLElement {
  const card = document.createElement("li");
  card.className = "provider";

  const head = document.createElement("div");
  head.className = "provider-head";

  const badge = BADGES[provider.name];
  if (badge) {
    const image = document.createElement("img");
    image.className = "badge";
    image.src = badge;
    image.width = 15;
    image.height = 15;
    image.alt = "";
    head.append(image);
  }

  const name = document.createElement("span");
  name.className = "provider-name";
  name.textContent = t(`panel.provider.${provider.name}`);
  head.append(name);

  if (provider.plan) {
    const plan = document.createElement("span");
    plan.className = "pill plan";
    // The plan name as the source reported it. Never normalised, never translated.
    plan.textContent = provider.plan;
    head.append(plan);
  }

  const age = document.createElement("span");
  age.className = `age ${freshnessClass(provider.freshness)}`;
  if (provider.configured) {
    const ageMs = provider.ageMs ?? undefined;
    const freshness = provider.freshness;
    tickers.push((elapsed) => {
      age.textContent = ageText(freshness, ageMs === undefined ? undefined : ageMs + elapsed, t);
    });
  } else {
    age.textContent = t("panel.provider.notConfigured");
  }
  head.append(age);
  card.append(head);

  if (provider.windows.length > 0) {
    const windows = document.createElement("ul");
    windows.className = "windows";
    windows.append(...provider.windows.map(windowRow));
    card.append(windows);
  }
  return card;
}

/** Rebuild the whole panel from the last snapshot. */
function draw(): void {
  if (!providersList) return;
  tickers = [];

  const providers = latest?.providers ?? [];
  providersList.replaceChildren(...providers.map(providerCard));

  const anyWindows = providers.some((provider) => provider.windows.length > 0);
  if (emptyLine) emptyLine.hidden = anyWindows;
  if (hintBox) hintBox.hidden = ui.hintDismissed;
  if (demoPill) demoPill.hidden = !ui.demo;

  tick();
  // The settings page is measured by `showView`; a redraw behind it must not shrink the
  // window to the size of a view nobody is looking at.
  if (!settingsOpen()) reportHeight();
}

/** Rewrite the countdowns and the ages. Called every second; touches text, not structure. */
function tick(): void {
  const elapsed = drift();
  for (const ticker of tickers) ticker(elapsed);
}

/** Ask for a window that fits the content, when the content has changed size. */
let lastHeight = 0;
function reportHeight(): void {
  if (!panel) return;
  const height = Math.ceil(panel.getBoundingClientRect().height);
  if (height <= 0 || height === lastHeight) return;
  lastHeight = height;
  void invoke("set_panel_height", { height }).catch(() => {
    // Opened outside the tray (a browser on dist/index.html); there is no window to size.
  });
}

// ------------------------------------------------------------------- settings

/** Show one of the two views and give the window a height that fits it. */
function showView(name: "quota" | "settings"): void {
  for (const view of views) view.hidden = view.dataset["view"] !== name;
  // The window is measured from whatever is on screen, so switching views has to be
  // followed by a measurement or the settings page opens inside a panel-sized window.
  lastHeight = 0;
  reportHeight();
}

/** Whether the settings view is the one showing. */
function settingsOpen(): boolean {
  return settingsForm?.hidden === false;
}

/** The control that holds one field of the form. */
function field(name: string): HTMLInputElement | HTMLSelectElement | null {
  return document.querySelector<HTMLInputElement | HTMLSelectElement>(`[data-field="${name}"]`);
}

/** Every control the form owns. */
function fields(): (HTMLInputElement | HTMLSelectElement)[] {
  return [...document.querySelectorAll<HTMLInputElement | HTMLSelectElement>("[data-field]")];
}

/** A text or select field's contents. */
function text(name: string): string {
  return field(name)?.value ?? "";
}

/** A checkbox's state. */
function checked(name: string): boolean {
  const control = field(name);
  return control instanceof HTMLInputElement ? control.checked : false;
}

/** Put the working copy into the controls. */
function paintForm(): void {
  if (!values || !settings) return;

  // The language picker is built from what Rust says this build can paint, so the day WP6
  // fills a catalogue the option appears without either side being edited.
  const picker = document.querySelector<HTMLSelectElement>('[data-field="locale"]');
  if (picker) {
    const options = [SYSTEM, ...settings.languages];
    picker.replaceChildren(
      ...options.map((tag) => {
        const option = document.createElement("option");
        option.value = tag;
        option.textContent = tag === SYSTEM ? t("settings.language.system") : t(languageKey(tag));
        return option;
      }),
    );
  }

  for (const control of fields()) {
    const name = control.dataset["field"] as keyof FormValues | undefined;
    if (!name) continue;
    const value = values[name];
    if (control instanceof HTMLInputElement && control.type === "checkbox") {
      control.checked = Boolean(value);
    } else {
      control.value = String(value);
    }
    control.disabled = !settings.writable;
  }

  if (autostartBox) autostartBox.disabled = !settings.writable;
  if (quietTimes) quietTimes.dataset["disabled"] = String(!values.quietHoursEnabled);
  if (readOnlyNote) readOnlyNote.hidden = settings.writable;
  if (settingsVersion) settingsVersion.textContent = settings.version;
  for (const node of document.querySelectorAll<HTMLElement>("[data-path]")) {
    const key = node.dataset["path"] as keyof SettingsView["paths"] | undefined;
    if (key) node.textContent = settings.paths[key];
  }
  if (hintResetNote) hintResetNote.hidden = settings.firstRunHintDismissed;
  showProblems(validate(values, settings.languages));
}

/**
 * Read the controls back into the working copy.
 *
 * Written out field by field rather than looped over with an index signature: `FormValues`
 * is thirteen named fields of two types, and a loop that assigned into it would have to be
 * cast to something that no longer checks either.
 */
function readForm(): void {
  if (!values) return;
  values = {
    locale: text("locale"),
    theme: text("theme"),
    themeMode: text("themeMode"),
    notifications: checked("notifications"),
    quietHoursEnabled: checked("quietHoursEnabled"),
    quietFrom: text("quietFrom"),
    quietTo: text("quietTo"),
    warn: text("warn"),
    critical: text("critical"),
    exhausted: text("exhausted"),
    claude: checked("claude"),
    codex: checked("codex"),
    detailedWindows: checked("detailedWindows"),
  };
  if (quietTimes) quietTimes.dataset["disabled"] = String(!values.quietHoursEnabled);
}

/** Say what is wrong, in one line, or nothing at all. */
function showProblems(problems: readonly Invalid[]): void {
  if (!settingsErrors) return;
  settingsErrors.hidden = problems.length === 0;
  settingsErrors.textContent = problems.map((problem) => t(invalidKey(problem))).join(" ");
}

/** Apply what `get_config` (or `set_config`) returned. */
function useSettings(next: SettingsView): void {
  settings = next;
  values = toValues(next.form);
  if (suggestionBox) suggestionBox.hidden = !next.suggestDetailed;
  paintForm();
}

/** Read the settings and the autostart switch, which lives outside `config.json`. */
async function loadSettings(): Promise<void> {
  try {
    useSettings(await invoke<SettingsView>("get_config"));
  } catch {
    // Opened outside the tray. The form stays as the markup left it.
    return;
  }
  await loadAutostart();
}

/** Ask the plugin whether we start with Windows. */
async function loadAutostart(): Promise<void> {
  if (!autostartBox) return;
  try {
    autostartBox.checked = await invoke<boolean>("get_autostart");
    if (autostartError) autostartError.hidden = true;
  } catch {
    // The registry could not be read. Saying so beats a switch that lies about the machine.
    autostartBox.checked = false;
    autostartBox.disabled = true;
    if (autostartError) autostartError.hidden = false;
  }
}

// ------------------------------------------------------------------ loading

/** Ask the Rust side for the derived view and redraw. */
async function refresh(): Promise<void> {
  try {
    latest = await invoke<SnapshotView>("get_snapshot");
    derivedAt = Date.now();
  } catch {
    // The command is defined by this same binary, so a failure here means the window is
    // being torn down. Keeping the last drawing beats blanking the panel.
  }
  draw();
}

/** Read the settings, choose the language and the theme, then draw. */
async function load(): Promise<void> {
  try {
    ui = await invoke<UiState>("get_ui_state");
  } catch {
    // Outside the tray: the defaults, and the hint stays hidden because there is no
    // config.json to remember dismissing it in.
  }

  // One answer, worked out in Rust, used by the panel and by the tray tooltip alike.
  // `navigator.languages` was the panel's own guess in WP4 and could disagree with the
  // tray's; it survives here only as the fallback for a page opened outside the tray.
  locale = isLocale(ui.resolvedLocale) ? ui.resolvedLocale : "en";
  t = createTranslator(catalogs, locale);

  applyLanguage();
  applyChosenTheme();
  await loadSettings();
  await refresh();
  // A screenshot run has no user to click `Settings`. Read from the state rather than
  // waited for as an event, because an event emitted while this file was still loading
  // would have had nobody listening for it.
  if (ui.openSettings) showView("settings");
}

// ------------------------------------------------------------------ actions

themeButton?.addEventListener("click", () => {
  const theme = nextTheme(ui.theme, Object.keys(THEMES));
  ui = { ...ui, theme };
  applyChosenTheme();
  void invoke<UiState>("set_theme", { theme, mode: ui.mode })
    .then((state) => {
      ui = state;
      applyChosenTheme();
    })
    .catch(() => {
      // Nothing to remember it in. The panel is already painted; that is the visible half.
    });
});

refreshButton?.addEventListener("click", () => {
  void invoke("refresh_now").catch(() => {});
});

dismissButton?.addEventListener("click", () => {
  ui = { ...ui, hintDismissed: true };
  if (hintBox) hintBox.hidden = true;
  reportHeight();
  void invoke<UiState>("dismiss_hint")
    .then((state) => {
      ui = state;
    })
    .catch(() => {});
  // The hint was the only reason to look at the top of the panel; give the focus back to
  // something that still exists rather than to a button that has just been hidden.
  refreshButton?.focus();
});

// ------------------------------------------------------------- settings actions

document.querySelector("[data-open-settings]")?.addEventListener("click", () => {
  void loadSettings().then(() => showView("settings"));
});

document.querySelector("[data-settings-back]")?.addEventListener("click", () => {
  showView("quota");
});

// Typing changes the working copy and re-judges it, so the user is told what is wrong while
// they are still in the field rather than after a round trip.
settingsForm?.addEventListener("input", () => {
  readForm();
  if (values?.quietHoursEnabled) {
    // Ticking the box with empty fields is a request for quiet hours, not for a broken
    // pair of times; the defaults are what the README documents.
    if (!values.quietFrom) values.quietFrom = DEFAULT_QUIET.from;
    if (!values.quietTo) values.quietTo = DEFAULT_QUIET.to;
    const from = document.querySelector<HTMLInputElement>('[data-field="quietFrom"]');
    const to = document.querySelector<HTMLInputElement>('[data-field="quietTo"]');
    if (from && !from.value) from.value = values.quietFrom;
    if (to && !to.value) to.value = values.quietTo;
  }
  if (values && settings) showProblems(validate(values, settings.languages));
  if (savedNote) savedNote.hidden = true;
});

settingsForm?.addEventListener("submit", (event) => {
  event.preventDefault();
  if (!values || !settings) return;
  readForm();
  const problems = validate(values, settings.languages);
  showProblems(problems);
  if (problems.length > 0) return;

  void invoke<SettingsView>("set_config", { form: toForm(values) })
    .then((next) => {
      useSettings(next);
      // The language may have changed, which changes every word on this page.
      locale = isLocale(next.resolvedLocale) ? next.resolvedLocale : "en";
      t = createTranslator(catalogs, locale);
      ui = { ...ui, theme: next.form.theme, mode: next.form.themeMode };
      applyLanguage();
      applyChosenTheme();
      draw();
      if (savedNote) savedNote.hidden = false;
    })
    .catch((problems: unknown) => {
      // Rust refused the whole document, so nothing was written. It has the last word: it
      // validates keys this form does not own.
      showProblems(Array.isArray(problems) ? (problems as Invalid[]) : []);
    });
});

// The startup entry is not part of `config.json`, so it is saved the moment it is flipped
// and the answer is read back from the plugin rather than assumed.
autostartBox?.addEventListener("change", () => {
  const wanted = autostartBox.checked;
  void invoke<boolean>("set_autostart", { enabled: wanted })
    .then((enabled) => {
      autostartBox.checked = enabled;
      if (autostartError) autostartError.hidden = true;
    })
    .catch(() => {
      autostartBox.checked = !wanted;
      if (autostartError) autostartError.hidden = false;
    });
});

document.querySelector("[data-reset-hint]")?.addEventListener("click", () => {
  void invoke<SettingsView>("reset_hint")
    .then((next) => {
      useSettings(next);
      ui = { ...ui, hintDismissed: false };
      if (hintResetNote) hintResetNote.hidden = false;
    })
    .catch(() => {});
});

// The one-time Max-plan offer. Both answers are final: it is asked once.
document.querySelector("[data-suggestion-accept]")?.addEventListener("click", () => {
  if (!values || !settings) return;
  values = { ...values, detailedWindows: true };
  void invoke<SettingsView>("set_config", { form: toForm(values) })
    .then(useSettings)
    .catch(() => {});
  if (suggestionBox) suggestionBox.hidden = true;
  reportHeight();
});

document.querySelector("[data-suggestion-dismiss]")?.addEventListener("click", () => {
  void invoke<SettingsView>("dismiss_detailed_suggestion").then(useSettings).catch(() => {});
  if (suggestionBox) suggestionBox.hidden = true;
  reportHeight();
});

// The tray menu's `Settings` cannot open a view inside a webview, so it asks for one.
void listen("open-settings", () => {
  void loadSettings().then(() => showView("settings"));
});

// Esc closes the panel — except on the settings page, where it goes back one step first,
// so a user who opened the settings by accident is not thrown out of the panel entirely.
window.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  if (settingsOpen()) {
    showView("quota");
    return;
  }
  void getCurrentWindow().hide();
});

// The loop tells the panel when the document moved; the panel counts the seconds itself.
void listen("snapshot-changed", () => void refresh());
window.setInterval(tick, 1000);

// Redraw whenever the panel comes back into view: it is hidden rather than closed, so it
// can have been out of sight for hours with a countdown that stopped mattering.
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) void refresh();
});

// The panel is open while the user changes Windows' light/dark setting: follow it.
darkQuery.addEventListener("change", () => applyChosenTheme());

void load();
