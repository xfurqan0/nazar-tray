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
 * * **The theme is the user's, and it is remembered.** It is chosen on the settings page
 *   and written into `config.json` with the rest of the form; light and dark follow
 *   `prefers-color-scheme` unless the settings say otherwise.
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
  type UsageRange,
  type UsageResponse,
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
import { THEMES, applyTheme, seriesPalette } from "./theme";
import {
  FIRST_USAGE_STATE,
  HEAT_LEVELS,
  buildUsageChart,
  buildUsageDetail,
  buildUsageView,
  cellLine,
  chartLevels,
  chartPoints,
  chartTicks,
  closeDetail,
  detailTitle,
  footerLine,
  formatColumn,
  formatMonth,
  formatNumber,
  formatTokens,
  isUsageError,
  needsFetch,
  openDetail,
  partsLine,
  requestsKey,
  tabRange,
  modeTagKey,
  usageErrorKey,
  usageWindow,
  weekTotal,
  weekLabel,
  withSpan,
  withTab,
  type ChartBox,
  type UsageBar,
  type UsageCell,
  type UsageChart,
  type UsageGrid,
  type UsageRow,
  type UsageScope,
  type UsageSpan,
  type UsageState,
  type UsageTab,
  type UsageTotals,
  type UsageWeekRow,
} from "./usage";

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
  /**
   * `--view usage --usage-tab <name>`: open on the usage view, on that tab.
   *
   * `week`, `weeks`, `all` and `models` are the tabs; `day` is the Week tab with its newest
   * day already opened, which is the one state of this view a tab name cannot reach. `null`
   * on every run that did not ask, which is every run but a screenshot.
   */
  readonly openUsage: string | null;
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
const refreshButton = document.querySelector<HTMLButtonElement>("[data-refresh]");
const dismissButton = document.querySelector<HTMLButtonElement>("[data-dismiss]");
const versionLabel = document.querySelector<HTMLElement>("[data-version]");

// The settings and usage views. Every one of these is inside the same window as the quota
// view; see index.html for why there is not a second window.
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

// The status-line section. The only control on this page that changes a file belonging to
// another program, which is why it is three buttons rather than a checkbox.
const statuslineState = document.querySelector<HTMLElement>("[data-statusline-state]");
const statuslinePreview = document.querySelector<HTMLButtonElement>("[data-statusline-preview]");
const statuslineApply = document.querySelector<HTMLButtonElement>("[data-statusline-apply]");
const statuslineCancel = document.querySelector<HTMLButtonElement>("[data-statusline-cancel]");
const statuslineNote = document.querySelector<HTMLElement>("[data-statusline-note]");
const statuslineOutput = document.querySelector<HTMLElement>("[data-statusline-output]");

// The usage view. Everything on it is drawn from one `get_usage` answer and the instant it
// is drawn at; nothing here ticks, because history does not move.
const usageTabs = [...document.querySelectorAll<HTMLButtonElement>("[data-usage-tab]")];
const usageTabsRow = document.querySelector<HTMLElement>("[data-usage-tabs]");
const usageSpanTabs = [...document.querySelectorAll<HTMLButtonElement>("[data-usage-span]")];
const usageBackButton = document.querySelector<HTMLButtonElement>("[data-usage-back]");
const usageTitle = document.querySelector<HTMLElement>("[data-usage-title]");
const usageWeekRows = document.querySelector<HTMLElement>("[data-usage-week-rows]");
const usageModels = document.querySelector<HTMLElement>("[data-usage-models]");
const usageChartBox = document.querySelector<HTMLElement>("[data-usage-chart]");
const usageKeys = document.querySelector<HTMLElement>("[data-usage-keys]");
const usageSummary = document.querySelector<HTMLElement>("[data-usage-summary]");
const usageTotal = document.querySelector<HTMLElement>("[data-usage-total]");
const usagePartsLine = document.querySelector<HTMLElement>("[data-usage-parts]");
const usageTag = document.querySelector<HTMLElement>("[data-usage-tag]");
const usageInfoLine = document.querySelector<HTMLElement>("[data-usage-info]");
const usageStrip = document.querySelector<HTMLElement>("[data-usage-strip]");
const usageGrid = document.querySelector<HTMLElement>("[data-usage-grid]");
const usageMonths = document.querySelector<HTMLElement>("[data-usage-months]");
const usageWeeks = document.querySelector<HTMLElement>("[data-usage-weeks]");
const usageScale = document.querySelector<HTMLElement>("[data-usage-scale]");
const usageHoverLine = document.querySelector<HTMLElement>("[data-usage-hover]");
const usageLegend = document.querySelector<HTMLElement>("[data-usage-legend]");
const usageRows = document.querySelector<HTMLElement>("[data-usage-rows]");
const usageStateLine = document.querySelector<HTMLElement>("[data-usage-state]");
const usageErrorLine = document.querySelector<HTMLElement>("[data-usage-error]");
const usageDetailLine = document.querySelector<HTMLElement>("[data-usage-detail]");
const usageDamagedLine = document.querySelector<HTMLElement>("[data-usage-damaged]");
const usageRefresh = document.querySelector<HTMLButtonElement>("[data-usage-refresh]");

let ui: UiState = {
  theme: "nazar",
  mode: "system",
  locale: null,
  resolvedLocale: "en",
  hintDismissed: true,
  demo: false,
  openSettings: false,
  openUsage: null,
};

/**
 * One run of `nazar-statusline`, as `crates/nazar-tray/src/statusline.rs` reports it.
 *
 * `outcome` is a key rather than a sentence, so the words on the page are translated like
 * every other word; `output` is the wrapper's own bytes — a unified diff and a backup path
 * — and is shown verbatim.
 */
interface StatuslineOutcome {
  readonly outcome: "ok" | "missing" | "failed" | "readOnly";
  readonly installed: boolean | null;
  readonly output: string;
  readonly program: string;
}

/** The settings as `get_config` last reported them, and the form's own working copy. */
let settings: SettingsView | undefined;
let values: FormValues | undefined;
let locale: Locale = "en";
let t: Translate = createTranslator(catalogs, locale);

/** The last snapshot, and the local instant it was derived for. */
let latest: SnapshotView | undefined;
let derivedAt = 0;

/** Which of the three views is on screen. The markup opens on the numbers. */
type ViewName = "quota" | "settings" | "usage";
let shown: ViewName = "quota";

/**
 * The usage answer being drawn, where the view is inside it, and what went wrong instead.
 *
 * The state is one value rather than three variables so that the moves between its parts —
 * a tab, a span, a day opened, *Back* — are the functions in `usage.ts` that
 * `test/usage.test.mjs` can exercise without a browser, rather than four assignments spread
 * through the listeners at the bottom of this file.
 */
let usage: UsageState = FIRST_USAGE_STATE;
let usageAnswer: UsageResponse | undefined;
let usageProblem: { kind: string; detail: string } | undefined;
let usageLoading = false;

/**
 * Which request the view is waiting for.
 *
 * A cold scan takes a second or two, which is long enough for somebody to press *Week* and
 * then *All*. Without this the slower answer would land last and draw the wrong tab's
 * numbers under the right tab's heading.
 */
let usageAsked = 0;

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
  // The language picker's options are generated rather than written in the markup, so
  // `[data-i18n]` cannot reach them; they are rebuilt in the language that was just chosen.
  if (settings) paintForm();
  // Same reason: the status-line section's state line and its first button say different
  // things depending on the machine, so neither can carry a `data-i18n` attribute.
  if (statusline) paintStatusline();
  // And the usage view's heading, which is `Usage` on the tabs and a date on a detail. It is
  // set here as well as in `paintUsage`, because a language chosen before the view was ever
  // opened would otherwise leave the heading blank until the first answer arrived.
  paintUsageTitle();
  // And the same again for the usage view: every number on it is formatted for a language —
  // `22.3M` is `22,3 Mn` in Turkish — so a language change redraws it rather than leaving
  // English digits grouped the English way.
  if (usageAnswer || usageProblem) paintUsage();
}

/** Paint the panel in the chosen theme, following the system for light and dark. */
function applyChosenTheme(): void {
  const theme = THEMES[ui.theme] ?? THEMES["nazar"];
  if (!theme) return;
  applyTheme(root, theme, resolveMode(ui.mode, darkQuery.matches));
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
  // The other two views are measured by `showView`; a redraw behind one of them must not
  // shrink the window to the size of a view nobody is looking at. It says "is the quota view
  // the one showing" rather than "is the settings page closed" because there are three of
  // them now, and the usage view is the taller one.
  if (shown === "quota") reportHeight();
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

/** Show one of the three views and give the window a height that fits it. */
function showView(name: ViewName): void {
  shown = name;
  for (const view of views) view.hidden = view.dataset["view"] !== name;
  // The window is measured from whatever is on screen, so switching views has to be
  // followed by a measurement or the settings page opens inside a panel-sized window.
  lastHeight = 0;
  reportHeight();
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
    usageCountLikeClaudeCode: checked("usageCountLikeClaudeCode"),
    usageFillHistoryFromStats: checked("usageFillHistoryFromStats"),
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
  await loadStatusline();
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

// ------------------------------------------------------- the status-line wrapper

/** What the machine last said, and whether a preview is waiting for a second click. */
let statusline: StatuslineOutcome | undefined;
let statuslinePending: "install" | "remove" | undefined;

/** Draw the section from `statusline` and `statuslinePending`, in the current language. */
function paintStatusline(): void {
  if (!statuslineState || !statuslinePreview) return;

  const missing = !statusline || statusline.outcome === "missing";
  const installed = statusline?.installed ?? null;
  // A run that was told what to look like says so on this page already, and the Rust side
  // refuses the write in any case. Here it means: do not offer the button.
  const readOnly = settings?.writable === false;

  statuslineState.textContent = missing
    ? t("settings.statusline.missing")
    : installed === null
      ? t("settings.statusline.unreadable")
      : installed
        ? t("settings.statusline.installed")
        : t("settings.statusline.notInstalled");

  // A machine with no wrapper beside the tray has nothing to offer: the button that would
  // install it needs the very binary that is missing.
  statuslinePreview.hidden = missing || readOnly || statuslinePending !== undefined;
  statuslinePreview.textContent = installed
    ? t("settings.statusline.remove")
    : t("settings.statusline.install");

  if (statuslineApply) {
    statuslineApply.hidden = statuslinePending === undefined;
    statuslineApply.textContent = t("settings.statusline.apply");
  }
  if (statuslineCancel) statuslineCancel.hidden = statuslinePending === undefined;

  if (statuslineNote) {
    const note =
      statuslinePending !== undefined
        ? "settings.statusline.preview"
        : statusline?.outcome === "failed"
          ? "settings.statusline.failed"
          : readOnly && !missing
            ? "settings.readOnly"
            : undefined;
    statuslineNote.hidden = note === undefined;
    statuslineNote.textContent = note ? t(note) : "";
    statuslineNote.classList.toggle("help-error", statusline?.outcome === "failed");
  }

  if (statuslineOutput) {
    // `status` prints three or four lines of its own on every load, and showing them
    // permanently would make the section a wall of text. The output is shown when it is
    // the answer to something the user just pressed.
    const interesting =
      statuslinePending !== undefined || statusline?.outcome === "failed";
    const text = interesting ? (statusline?.output.trimEnd() ?? "") : "";
    statuslineOutput.hidden = text === "";
    statuslineOutput.textContent = text;
  }
  reportHeight();
}

/** Ask the machine what Claude Code's status line is now. */
async function loadStatusline(): Promise<void> {
  if (!statuslineState) return;
  try {
    statusline = await invoke<StatuslineOutcome>("statusline_status");
  } catch {
    // Opened outside the tray, where the command does not exist.
    return;
  }
  statuslinePending = undefined;
  paintStatusline();
}

// --------------------------------------------------------------------- usage

/**
 * One model's row: the id as the source spelled it, what it spent, and how much of the
 * window that is.
 *
 * The id is printed raw — never translated, never merged with a model that is probably the
 * same one under another name — for the reason `docs/usage-contract.md` gives: an alias table
 * has to be right about names nobody here controls, and a wrong merge cannot be undone.
 */
function usageRowItem(row: UsageRow, share: boolean): HTMLElement {
  const item = document.createElement("li");
  item.className = "usage-row";
  if (row.reported) item.classList.add("reported");

  const head = document.createElement("div");
  head.className = "usage-row-head";

  const model = document.createElement("span");
  model.className = "usage-model";
  model.textContent = row.model;
  head.append(model);

  const value = document.createElement("span");
  value.className = "usage-row-total";
  value.textContent = formatTokens(row.total, locale);
  head.append(value);
  item.append(head);

  // The same empty-track rule as a window nobody could read: a bar of zero length and a bar
  // with no number behind it look identical, and one of them is a lie.
  const meter = document.createElement("div");
  meter.className = "meter";
  meter.setAttribute("aria-hidden", "true");
  if (row.total !== undefined) {
    const fill = document.createElement("span");
    fill.className = "meter-fill";
    fill.style.width = `${row.share}%`;
    meter.append(fill);
  }
  item.append(meter);

  // The four counters the total above is a sum of, on their own smaller line. Without it the
  // headline is a number about the cache with the work lost in the rounding — cache reads
  // were 98.5 % of it over six days of real work — and the reader has no way to see that.
  const parts = document.createElement("div");
  parts.className = "usage-row-parts";
  parts.textContent = partsLine(row.parts, locale, t);
  item.append(parts);

  const foot = document.createElement("div");
  foot.className = "usage-row-foot";
  const records = document.createElement("span");
  // Two labels rather than one: on the Claude side a record is a reply, on the Codex side it
  // is a `token_count` event and several of them make one turn. A reported day has neither —
  // the file it came from holds one number per model and no count of anything — so the line
  // says where the number is from instead of claiming a count it does not have.
  records.textContent = row.reported
    ? t("usage.reported")
    : t(requestsKey(row.provider), { requests: formatNumber(row.requests, locale) });
  foot.append(records);

  // How much of the span this model took, on the *Models* tab alone — the question that tab
  // exists to answer. `panel.window.percent` rather than a seventh key of its own: it is the
  // product's one spelling of *a number and a percent sign*, and the three languages that
  // write `%88` or `88%` rather than `88 %` already have it right there.
  if (share && row.percent !== undefined) {
    const percent = document.createElement("span");
    percent.className = "usage-row-share";
    percent.textContent = t("panel.window.percent", {
      percent: formatNumber(row.percent, locale),
    });
    foot.append(percent);
  }
  item.append(foot);

  return item;
}

/**
 * One week of the *Weeks* list: Monday to Sunday, what it spent, and a way into it.
 *
 * A button rather than a row with a button on it, so the whole card answers the pointer and
 * one `Tab` reaches it. The three pieces inside are the model row's three pieces in the same
 * order — a head with a total, a bar, the four counters — because the two lists sit one tab
 * apart and reading one should teach you how to read the other.
 */
function usageWeekItem(week: UsageWeekRow): HTMLElement {
  const item = document.createElement("li");

  const card = document.createElement("button");
  card.type = "button";
  card.className = "usage-week-row";
  if (week.current) card.classList.add("current");
  // A week nobody has a transcript for any more, drawn as the outline a reported day is
  // drawn as, so the two readings of history never share a shade or a bar.
  if (week.reported) card.classList.add("reported");
  card.dataset["usageWeek"] = week.key;
  card.dataset["usageAt"] = String(week.start);

  const head = document.createElement("div");
  head.className = "usage-week-head";

  const when = document.createElement("span");
  when.className = "usage-week-when";
  when.textContent = weekLabel(week, locale);
  head.append(when);

  const value = document.createElement("span");
  value.className = "usage-week-total";
  value.textContent = formatTokens(weekTotal(week), locale);
  head.append(value);
  card.append(head);

  // The same empty-track rule the model rows keep: a week nobody recorded anything for has
  // no bar at all, rather than a bar of zero length that looks like a measurement. A reported
  // week has none either, because the scale it would be drawn against is not its scale.
  const meter = document.createElement("div");
  meter.className = "meter";
  meter.setAttribute("aria-hidden", "true");
  if (week.total !== undefined && !week.reported) {
    const fill = document.createElement("span");
    fill.className = "meter-fill";
    fill.style.width = `${week.share}%`;
    meter.append(fill);
  }
  card.append(meter);

  const parts = document.createElement("div");
  parts.className = "usage-row-parts";
  // A reported week has no breakdown to print — one total per model per day is all the file
  // holds — so it says where its number came from in the line the parts would have been on.
  parts.textContent = week.reported ? t("usage.reported") : partsLine(week.parts, locale, t);
  card.append(parts);

  item.append(card);
  return item;
}

/** The model rows, under provider headings when there are two providers to tell apart. */
function paintRows(totals: UsageTotals, share: boolean): void {
  if (!usageRows) return;
  const items: HTMLElement[] = [];
  for (const group of totals.groups) {
    // A heading for one provider is a heading that says nothing; it earns its line only
    // when there are two of them to tell apart.
    if (totals.grouped) {
      const heading = document.createElement("li");
      heading.className = "usage-provider";
      const key = `panel.provider.${group.provider}`;
      const name = t(key);
      heading.textContent = name === key ? group.provider : name;
      const sum = document.createElement("span");
      sum.className = "usage-provider-total";
      sum.textContent = formatTokens(group.total, locale);
      heading.append(sum);
      items.push(heading);
    }
    items.push(...group.rows.map((row) => usageRowItem(row, share)));
  }
  usageRows.replaceChildren(...items);
}

// --------------------------------------------------------------- the calendar

/**
 * Every cell and bar that can be inspected, in the order they are drawn.
 *
 * A grid of a year is 366 cells, and 366 tab stops between the tabs and the model rows would
 * make the keyboard useless. So the list holds one tab stop — {@link roving} keeps it on the
 * cell that has been looked at last — and the arrow keys walk this array: up and down a week,
 * left and right a column, which is what the shape on screen makes them mean.
 */
let usageStops: HTMLElement[] = [];

/** Move the single tab stop onto one cell, and optionally put the focus there too. */
function roving(next: HTMLElement | undefined, focus: boolean): void {
  if (!next) return;
  for (const stop of usageStops) stop.tabIndex = -1;
  next.tabIndex = 0;
  if (focus) next.focus();
}

/** Say what a cell holds, on the panel's own line rather than in a tooltip of the shell's. */
function sayCell(element: HTMLElement | null): void {
  if (!usageHoverLine) return;
  const line = element?.dataset["usageLine"];
  usageHoverLine.textContent = line ?? "";
}

/**
 * Make one cell or bar answer to the pointer, to the keyboard and to being activated.
 *
 * A real `<button>` since T-WP21. It used to be a `role="img"` with a label, and the reason
 * was written down here: nothing happened when one was activated, and a button that does
 * nothing is a promise the panel does not keep. A day now opens that day's models, so the
 * promise is kept and the element is the one the platform already knows how to focus, to
 * activate with Enter and Space, and to announce. The label is still the sentence the hover
 * line shows, so a screen reader and a pair of eyes are told the same thing.
 */
function inspectable(button: HTMLButtonElement, line: string, key: string, at: number): void {
  button.dataset["usageLine"] = line;
  button.dataset["usageDay"] = key;
  button.dataset["usageAt"] = String(at);
  button.setAttribute("aria-label", line);
  button.tabIndex = -1;
  usageStops.push(button);
}

/** One column of the week strip: a local day, from this Monday to today. */
function usageColumn(bar: UsageBar): HTMLElement {
  const column = document.createElement("button");
  column.type = "button";
  column.className = "usage-bar";
  inspectable(column, cellLine(bar, locale, t), bar.key, bar.at);
  if (bar.height > 0) {
    const fill = document.createElement("span");
    fill.className = "usage-bar-fill";
    fill.style.height = `${bar.height}%`;
    column.append(fill);
  }
  return column;
}

/**
 * One day of the calendar.
 *
 * A cell outside the range — before the store began, after today — is a hole: no track, no
 * shade, no label, nothing the keyboard can land on and nothing to open. February has no 30th
 * and next Friday has not happened, and an empty track would say both were days with no work
 * on them.
 */
function usageDay(cell: UsageCell): HTMLElement {
  if (!cell.present) {
    const hole = document.createElement("span");
    hole.className = "usage-cell";
    hole.classList.add("blank");
    hole.setAttribute("aria-hidden", "true");
    return hole;
  }
  const box = document.createElement("button");
  box.type = "button";
  box.className = "usage-cell";
  box.classList.add(`usage-level-${cell.level}`);
  // An outline rather than a shade: the number is real and is one hover away, but it was
  // counted by another program in another unit, and a shade is a comparison.
  if (cell.reported) box.classList.add("reported");
  inspectable(box, cellLine(cell, locale, t), cell.key, cell.at);
  return box;
}

/**
 * Draw the calendar: weeks as columns, Monday at the top, month names over the top of it.
 *
 * The two rows share one column template so the names line up with the weeks under them, and
 * a label is stretched to where the next one begins — the same trick a contribution graph
 * uses, and the reason a month three weeks wide does not push the one after it sideways.
 *
 * The cell size is the one thing decided here rather than in the stylesheet: a month is five
 * or six columns and would be a postage stamp at the size a year has to be drawn at, and a
 * year is fifty-three columns and cannot be drawn at the size a month can afford. Above ten
 * columns the grid goes small and scrolls sideways inside its own box. 18 px rather than
 * something more comfortable because seven rows of it are 144 px of a window that is clamped
 * at 720 and already has a summary, a strip of tabs, a list of models and two footers in it.
 */
function paintGrid(grid: UsageGrid): void {
  if (!usageWeeks || !usageMonths) return;
  const wide = grid.columns.length > 10;
  const size = wide ? "11px" : "18px";
  for (const row of [usageWeeks, usageMonths]) {
    row.style.setProperty("--usage-cell", size);
  }

  const cells: HTMLElement[] = [];
  for (const column of grid.columns) for (const day of column.days) cells.push(usageDay(day));
  usageWeeks.replaceChildren(...cells);

  const labels: HTMLElement[] = [];
  grid.months.forEach((month, index) => {
    const label = document.createElement("span");
    label.className = "usage-month";
    label.textContent = formatMonth(month.at, locale);
    const ends = grid.months[index + 1]?.column ?? grid.columns.length;
    label.style.gridColumn = `${month.column + 1} / ${ends + 1}`;
    labels.push(label);
  });
  usageMonths.replaceChildren(...labels);
}

/** The four shades and the empty one, so the scale is a legend and not a guess. */
function paintLegend(): void {
  if (!usageLegend) return;
  const keys: HTMLElement[] = [];
  for (let level = 0; level <= HEAT_LEVELS; level++) {
    const key = document.createElement("span");
    key.className = "usage-cell";
    key.classList.add(`usage-level-${level}`);
    keys.push(key);
  }
  usageLegend.replaceChildren(...keys);
}

// ------------------------------------------------------------------ the chart

/** Where an `<svg>` element lives. Not a word, and not something a catalogue translates. */
const SVG_NS = "http://www.w3.org/2000/svg";

/** One SVG element, in the namespace the browser insists on for them. */
function shape<K extends keyof SVGElementTagNameMap>(name: K): SVGElementTagNameMap[K] {
  return document.createElementNS(SVG_NS, name);
}

/**
 * The chart's coordinate space, and the box the lines are drawn inside it.
 *
 * Fixed numbers rather than measured pixels: the `<svg>` scales to whatever width the panel
 * gives it, so one space works at 330 px and at whatever a future window is. 22 at the bottom
 * is one row of date labels, and 96 of height is what is left of the clamp once a tab strip, a
 * span strip, a legend and a scrolling list of models have been paid for.
 *
 * **50 on the left is a measurement, not a guess.** It was 36 — enough for `78.9M` — and the
 * Russian panel drew `78,9 млрд` clipped to `8,9 млрд`, which is not a smaller number, it is a
 * wrong one. A magnitude mark is a *word* in four of the six languages, so the gutter was
 * measured at 9 px in the panel's own font stack across every language and every magnitude the
 * store can reach: the widest is Spanish `78,9 mil M` at **41.6 px**, then Russian `78,9 млрд`
 * at 40.2. 45 of drawing room, and the number is anchored to its right edge.
 */
const CHART_VIEW = { width: 320, height: 124 };
const CHART_BOX: ChartBox = { left: 50, top: 6, width: 266, height: 96 };

/**
 * The *Models* chart: one polyline per model, three rules, three dates and three numbers.
 *
 * **Written by hand, which is decision K2 again.** A charting library is a dependency, a
 * bundle and a second set of colours for a drawing that is six polylines and nine bits of
 * text. What it is drawn from is in `usage.ts`, where a test can read it without a browser;
 * this function is the geometry and nothing else.
 */
function paintChart(chart: UsageChart, colours: readonly string[]): void {
  if (!usageChartBox) return;

  const figure = shape("svg");
  figure.setAttribute("viewBox", `0 0 ${CHART_VIEW.width} ${CHART_VIEW.height}`);
  figure.setAttribute("role", "img");
  figure.setAttribute("aria-label", t("usage.chart.daily"));

  // The y axis: the peak, half of it and zero, each with a rule the eye can read a height
  // against. The numbers are compact and in the reader's own language, like every other
  // number in this view.
  const levels = chartLevels(chart.peak);
  levels.forEach((value, index) => {
    const y =
      levels.length > 1
        ? CHART_BOX.top + (index / (levels.length - 1)) * CHART_BOX.height
        : CHART_BOX.top + CHART_BOX.height;

    const rule = shape("line");
    rule.setAttribute("class", "usage-chart-rule");
    rule.setAttribute("x1", String(CHART_BOX.left));
    rule.setAttribute("x2", String(CHART_BOX.left + CHART_BOX.width));
    rule.setAttribute("y1", String(y));
    rule.setAttribute("y2", String(y));
    figure.append(rule);

    const label = shape("text");
    label.setAttribute("class", "usage-chart-label");
    label.setAttribute("x", String(CHART_BOX.left - 5));
    label.setAttribute("y", String(y + 3));
    label.setAttribute("text-anchor", "end");
    label.textContent = formatTokens(value, locale);
    figure.append(label);
  });

  // The x axis: where the span begins, roughly its middle, and where it ends. The two at the
  // ends are anchored inwards so that neither is drawn off the edge of the panel.
  const last = chart.days.length - 1;
  for (const index of chartTicks(chart.days.length)) {
    const day = chart.days[index];
    if (!day) continue;
    const x =
      last > 0
        ? CHART_BOX.left + (index / last) * CHART_BOX.width
        : CHART_BOX.left + CHART_BOX.width / 2;

    const label = shape("text");
    label.setAttribute("class", "usage-chart-label");
    label.setAttribute("x", String(Math.round(x)));
    label.setAttribute("y", String(CHART_BOX.top + CHART_BOX.height + 15));
    label.setAttribute(
      "text-anchor",
      index === 0 ? "start" : index === last ? "end" : "middle",
    );
    label.textContent = formatColumn(day.at, locale);
    figure.append(label);
  }

  chart.series.forEach((series, index) => {
    const colour = colours[index % colours.length] ?? "";
    const line = shape("polyline");
    line.setAttribute("class", "usage-chart-line");
    line.setAttribute("points", chartPoints(series.points, chart.peak, CHART_BOX));
    line.setAttribute("stroke", colour);
    figure.append(line);

    // A span one day wide has no line to draw — two points make a segment and one makes
    // nothing — so the single reading is drawn as the dot it is.
    if (chart.days.length === 1) {
      const [only = 0] = series.points;
      const dot = shape("circle");
      dot.setAttribute("cx", String(CHART_BOX.left + CHART_BOX.width / 2));
      dot.setAttribute(
        "cy",
        String(
          chart.peak > 0
            ? CHART_BOX.top + CHART_BOX.height - (only / chart.peak) * CHART_BOX.height
            : CHART_BOX.top + CHART_BOX.height,
        ),
      );
      dot.setAttribute("r", "2");
      dot.setAttribute("fill", colour);
      figure.append(dot);
    }
  });

  usageChartBox.replaceChildren(figure);
}

/** The legend: every line named beside its colour, because colour is never the only channel. */
function paintKeys(chart: UsageChart, colours: readonly string[]): void {
  if (!usageKeys) return;
  usageKeys.replaceChildren(
    ...chart.series.map((series, index) => {
      const item = document.createElement("li");
      item.className = "usage-key";

      const swatch = document.createElement("span");
      swatch.className = "usage-key-swatch";
      swatch.style.background = colours[index % colours.length] ?? "";
      item.append(swatch);

      const model = document.createElement("span");
      model.className = "usage-key-model";
      model.textContent = series.model;
      item.append(model);

      return item;
    }),
  );
}

/** The six line colours of the theme and mode the panel is painted in right now. */
function chartColours(): readonly string[] {
  const theme = THEMES[ui.theme] ?? THEMES["nazar"];
  if (!theme) return [];
  return seriesPalette(theme, resolveMode(ui.mode, darkQuery.matches));
}

/**
 * The usage view's one heading: the day or the week that is open, or the word *Usage*.
 *
 * It is the only title in the panel that is not a fixed word, which is why the markup gives
 * it no `[data-i18n]` and why a language change has to come back through here. Together with
 * the single *Back* beside it this is the whole of the view's navigation — the reader is told
 * where they are, and the one control goes one level up from there.
 */
function paintUsageTitle(): void {
  if (!usageTitle) return;
  usageTitle.textContent = usage.detail ? detailTitle(usage.detail, locale, t) : t("usage.title");
}

/** Draw the usage view from the last answer, the last failure, or neither. */
function paintUsage(): void {
  if (!usageRows) return;

  const opened = usage.detail !== undefined;
  for (const tab of usageTabs) {
    const own = !opened && tab.dataset["usageTab"] === usage.tab;
    tab.classList.toggle("primary", own);
    tab.setAttribute("aria-selected", String(own));
  }
  for (const span of usageSpanTabs) {
    const own = span.dataset["usageSpan"] === usage.span;
    span.classList.toggle("primary", own);
    span.setAttribute("aria-selected", String(own));
  }
  // A detail is not a fifth tab: the row of four is put away while one is open, and the
  // heading above says which day or which week is on screen instead.
  if (usageTabsRow) usageTabsRow.hidden = opened;
  paintUsageTitle();

  const view = usageAnswer ? buildUsageView(usageAnswer, new Date()) : undefined;
  const failed = usageProblem !== undefined;

  if (usageErrorLine) {
    usageErrorLine.hidden = !failed;
    usageErrorLine.textContent = usageProblem ? t(usageErrorKey(usageProblem.kind)) : "";
  }
  if (usageDetailLine) {
    // The reader's own words, printed verbatim: it names the value that was refused or
    // repeats what a file said, and no catalogue can know that in advance.
    const detail = usageProblem?.detail.trim() ?? "";
    usageDetailLine.hidden = detail === "";
    usageDetailLine.textContent = detail;
  }

  // What the answer is being cut into: a day or a week that was opened, a span under
  // *Models*, or the whole window. All three are the same arithmetic in `usage.ts`, which is
  // what keeps a detail from adding up differently from the row it was opened from.
  const opening =
    usageAnswer && usage.detail ? buildUsageDetail(usageAnswer, usage.detail) : undefined;
  const chart =
    usageAnswer && !opened && usage.tab === "models"
      ? buildUsageChart(usageAnswer, usage.span, new Date())
      : undefined;
  const shownTotals: UsageTotals | undefined = opening ?? chart?.totals ?? view;

  // Whether the window holds anything at all, and whether the cut of it on screen does. They
  // differ on *Models*, where a store full of work can still have a quiet *Last 7 days* — and
  // the span selector has to stay on screen then, or there is no way back out of the span.
  const answered = view !== undefined && !view.empty && !usageLoading && !failed;
  const numbers = answered && shownTotals !== undefined && shownTotals.rows.length > 0;

  if (usageStateLine) {
    let word = "";
    // A cold scan takes a second or two. A line saying so beats a panel that looks broken,
    // and it beats last week's numbers sitting under this week's heading while it waits.
    if (usageLoading) word = t("usage.loading");
    else if (view && !failed && !numbers) word = t("usage.empty");
    usageStateLine.hidden = word === "";
    usageStateLine.textContent = word;
  }

  // Exactly one drawing of the days is on screen at a time, which is what keeps the hover
  // line underneath unambiguous.
  const strip = numbers && !opened && usage.tab === "week";
  const calendar = numbers && !opened && usage.tab === "all";
  const weeks = numbers && !opened && usage.tab === "weeks";
  const models = answered && !opened && usage.tab === "models";
  if (usageSummary) usageSummary.hidden = !numbers || models;
  if (usageStrip) usageStrip.hidden = !strip;
  if (usageGrid) usageGrid.hidden = !calendar;
  if (usageWeekRows) usageWeekRows.hidden = !weeks;
  if (usageModels) usageModels.hidden = !models;
  if (usageScale) usageScale.hidden = !strip && !calendar;

  usageStops = [];
  sayCell(null);

  // Everything drawn at run time is emptied before anything is drawn again, so a tab never
  // leaves last tab's cells sitting in a hidden box — where they would still be found by the
  // delegated listeners and by anything walking the document.
  if (usageStrip) usageStrip.replaceChildren();
  if (usageWeeks) usageWeeks.replaceChildren();
  if (usageMonths) usageMonths.replaceChildren();
  if (usageLegend) usageLegend.replaceChildren();
  if (usageWeekRows) usageWeekRows.replaceChildren();
  if (!models) {
    if (usageChartBox) usageChartBox.replaceChildren();
    if (usageKeys) usageKeys.replaceChildren();
  }

  if (numbers && shownTotals) {
    if (usageTotal) usageTotal.textContent = formatTokens(shownTotals.total, locale);
    // All four counters, every time. The headline is their sum — the same definition
    // `/usage` calls *total tokens* — and this is where the cache is told from the work. A
    // reported day has no split to show and prints four em dashes, which is the same rule an
    // absent counter has always followed.
    if (usagePartsLine) usagePartsLine.textContent = partsLine(shownTotals.parts, locale, t);
  }
  if (usageTag) {
    // What the headline is, when it is not this product's own answer: the count `/usage`
    // shows, or a day another program reported. Nothing when it is.
    const key = opening?.reported ? "usage.reported" : view && modeTagKey(view.mode);
    usageTag.hidden = !numbers || !key;
    usageTag.textContent = key ? t(key) : "";
  }
  // Every tab has exactly one list, and on *Weeks* that list is the weeks. Drawing the model
  // rows underneath them as well would put two scrolling lists of 236 px in a window clamped
  // at 720 — and the models of a week are one click away in the week itself.
  if (numbers && shownTotals && !weeks) paintRows(shownTotals, chart !== undefined);
  else usageRows.replaceChildren();

  if (!opened && view) {
    if (strip && usageStrip) usageStrip.replaceChildren(...view.bars.map(usageColumn));
    if (calendar) paintGrid(view.grid);
    if (strip || calendar) {
      paintLegend();
      // The tab stop lands on the most recent day, which is the one somebody opening this
      // view wanted to know about.
      roving(usageStops[usageStops.length - 1], false);
    }
    if (weeks && usageWeekRows) usageWeekRows.replaceChildren(...view.weeks.map(usageWeekItem));
  }

  if (models && chart) {
    const colours = chartColours();
    paintChart(chart, colours);
    paintKeys(chart, colours);
  }

  if (usageInfoLine) {
    // Where the history begins and when it was last counted, on one line at the size of the
    // rest of the view. Two footnotes at 10 px was T-WP16's answer and nobody could read it.
    //
    // A detail keeps the freshness and loses the floor, for the reason *since* has never been
    // drawn under the *Week* tab: `Since Jul 29` under a page headed *Sep 13, 2026* reads as a
    // claim about the day being shown rather than about the store behind it.
    const carries = opened && view ? { ...view, since: undefined } : view;
    const line = carries ? footerLine(carries, locale, t, Date.now()) : "";
    usageInfoLine.hidden = line === "";
    usageInfoLine.textContent = line;
  }
  if (usageDamagedLine) {
    // A damaged month is not an error: the months beside it loaded, and this says which one
    // is missing from the numbers above rather than repairing a file that may be the only
    // copy of it.
    const damaged = view?.damaged ?? [];
    usageDamagedLine.hidden = damaged.length === 0;
    usageDamagedLine.textContent =
      damaged.length > 0 ? t("usage.damaged", { months: damaged.join(", ") }) : "";
  }

  if (shown === "usage") reportHeight();
}

/**
 * Ask for the window the tab wants and draw what comes back.
 *
 * `force` is the view's own Refresh: the throttle on the Rust side lets a scan run at most
 * every five minutes, and somebody who has just finished a long session should not be told
 * to wait for numbers that are already on disk.
 */
async function askUsage(force = false): Promise<void> {
  const range: UsageRange = tabRange(usage.tab);
  const asked = ++usageAsked;
  usageLoading = true;
  paintUsage();

  const span = usageWindow(range, new Date());
  try {
    const answer = await invoke<UsageResponse>("get_usage", {
      request: { range, from: span.from, to: span.to, force },
    });
    if (asked !== usageAsked) return;
    usageAnswer = answer;
    usageProblem = undefined;
  } catch (problem) {
    if (asked !== usageAsked) return;
    usageAnswer = undefined;
    // A rejection that is not the command's own error shape — a window being torn down, a
    // page opened outside the tray — still gets a sentence rather than a blank view.
    usageProblem = isUsageError(problem)
      ? { kind: problem.kind, detail: problem.detail }
      : { kind: "", detail: String(problem) };
  }
  usageLoading = false;
  paintUsage();
}

/**
 * Move the view, and ask the store again only when the move needs a different window.
 *
 * Three of the four tabs are readings of the same widest answer, and so are a span, a day
 * opened and *Back*. Redrawing them from what is already in hand is what keeps a loading line
 * — and a scan behind it — out of a move the panel can make instantly, and it is also what
 * keeps two tabs from reporting two scans of the same history. {@link needsFetch} in
 * `usage.ts` is the whole rule, and the tests exercise it there.
 */
function showUsage(next: UsageState): void {
  const before = usage;
  usage = next;
  if (usageAnswer === undefined || needsFetch(before, next)) {
    void askUsage();
    return;
  }
  paintUsage();
}

/**
 * Open a day or a week, and take the focus with it.
 *
 * The element that was activated is about to be replaced by the detail, so without this the
 * focus falls to the document body and a keyboard user is stranded at the top of the panel
 * with no idea that anything happened. It lands on *Back* — the control that, a moment ago,
 * left the view altogether and now closes the day instead.
 */
function openScope(scope: UsageScope): void {
  showUsage(openDetail(usage, scope));
  usageBackButton?.focus();
}

/** *Back*, and the focus with it: onto the tab whose list is coming back. */
function closeScope(): void {
  showUsage(closeDetail(usage));
  usageTabs.find((tab) => tab.dataset["usageTab"] === usage.tab)?.focus();
}

/**
 * The usage view's one way back, and it goes exactly one level.
 *
 * T-WP21 gave the view a second header of its own, so a day opened from the calendar stacked
 * two *Back* buttons in the top-left corner — one out of the day, one out of the view — and
 * neither said which. There is one now: with a detail open it closes the detail and the tabs
 * come back; with no detail open it leaves for the quota view. Esc is routed through this
 * same function, so the key and the button can never come to mean different things.
 */
function usageBack(): void {
  if (usage.detail !== undefined) closeScope();
  else showView("quota");
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
  else if (ui.openUsage) await openUsageAt(ui.openUsage);
}

/**
 * Open the usage view on one tab, for `--view usage`.
 *
 * The tab is set before the answer is asked for, so the window asked of the store is the one
 * that tab wants and the picture is never of a tab drawn from another tab's range. `day` is
 * the Week tab with its **last** cell opened — the day the history ends on, which is today —
 * because a detail is a state of the view no tab name reaches and a screenshot of it is the
 * only way the *Back* button appears in the documentation.
 */
async function openUsageAt(tab: string): Promise<void> {
  showView("usage");
  const name = tab === "day" ? "week" : tab;
  if (name === "week" || name === "weeks" || name === "all" || name === "models") {
    usage = withTab(closeDetail(usage), name satisfies UsageTab);
  }
  await askUsage();
  if (tab !== "day") return;

  // The cells are drawn by the paint above, so the day to open is read off the page rather
  // than recomputed here: one definition of "the last day with anything in it", and it is
  // the one the reader would have clicked.
  const cells = [...document.querySelectorAll<HTMLElement>("[data-usage-strip] [data-usage-day]")];
  const cell = cells[cells.length - 1];
  const key = cell?.dataset["usageDay"];
  const at = Number(cell?.dataset["usageAt"] ?? Number.NaN);
  if (key === undefined || !Number.isFinite(at)) return;
  openScope({ kind: "day", key, at });
}

// ------------------------------------------------------------------ actions

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

// ---------------------------------------------------------------- usage actions

// The button sits where the theme toggle used to (T-WP12). Every open asks again rather than
// showing what was drawn an hour ago: the call is cheap unless the five-minute throttle has
// expired, and the answer is what the store holds either way.
document.querySelector("[data-open-usage]")?.addEventListener("click", () => {
  showView("usage");
  // A detail belongs to the visit it was opened in, so an open lands on a list rather than
  // on the Tuesday somebody looked at yesterday.
  usage = closeDetail(usage);
  void askUsage();
});

usageBackButton?.addEventListener("click", usageBack);

for (const tab of usageTabs) {
  tab.addEventListener("click", () => {
    const name = tab.dataset["usageTab"];
    if (name === "week" || name === "weeks" || name === "all" || name === "models") {
      showUsage(withTab(usage, name satisfies UsageTab));
    }
  });
}

for (const span of usageSpanTabs) {
  span.addEventListener("click", () => {
    const name = span.dataset["usageSpan"];
    if (name === "all" || name === "days7" || name === "days30") {
      showUsage(withSpan(usage, name satisfies UsageSpan));
    }
  });
}

// A week of the list opens that week. Delegated, for the reason the cells are: fifty-three
// rows are fifty-three pairs of listeners to remove on the next redraw.
usageWeekRows?.addEventListener("click", (event) => {
  const card = (event.target as HTMLElement | null)?.closest<HTMLElement>("[data-usage-week]");
  const key = card?.dataset["usageWeek"];
  const at = Number(card?.dataset["usageAt"] ?? Number.NaN);
  if (key === undefined || !Number.isFinite(at)) return;
  openScope({ kind: "week", key, at });
});

usageRefresh?.addEventListener("click", () => {
  void askUsage(true);
});

// The hover line, for both drawings of the days. Delegated rather than bound per cell: a year
// is 366 cells and 366 pairs of listeners is 732 things to remove on the next redraw.
for (const surface of [usageStrip, usageGrid]) {
  surface?.addEventListener("pointerover", (event) => {
    sayCell((event.target as HTMLElement | null)?.closest("[data-usage-line]") ?? null);
  });
  surface?.addEventListener("pointerleave", () => sayCell(null));
  surface?.addEventListener("focusin", (event) => {
    const cell = (event.target as HTMLElement | null)?.closest<HTMLElement>("[data-usage-line]");
    sayCell(cell ?? null);
    roving(cell ?? undefined, false);
  });
  surface?.addEventListener("focusout", () => sayCell(null));

  // And a day opens that day. Enter and Space arrive here too, because the cells are real
  // buttons rather than the labelled images T-WP20 drew.
  surface?.addEventListener("click", (event) => {
    const cell = (event.target as HTMLElement | null)?.closest<HTMLElement>("[data-usage-day]");
    const key = cell?.dataset["usageDay"];
    const at = Number(cell?.dataset["usageAt"] ?? Number.NaN);
    if (key === undefined || !Number.isFinite(at)) return;
    openScope({ kind: "day", key, at });
  });
}

/**
 * Walking the days with the arrow keys.
 *
 * The calendar's cells are drawn column by column, Monday first, so one step along the array
 * is one day down a column and seven is one week across — exactly what the shape on screen
 * makes up, down, left and right mean. The arithmetic survives the holes because a grid only
 * ever has them at its two ends: a month that starts on a Wednesday is missing the Monday and
 * the Tuesday of its first column and nothing in between, so every step after that first
 * column is a constant. The strip is one row, so left and right are one day.
 *
 * A step that lands outside is ignored rather than wrapped. Wrapping from the bottom of one
 * week to the top of the next moves the focus somewhere the eye is not looking.
 */
function walkDays(event: KeyboardEvent, across: number): void {
  const steps: Readonly<Record<string, number>> = {
    ArrowUp: -1,
    ArrowDown: 1,
    ArrowLeft: -across,
    ArrowRight: across,
  };
  const step = steps[event.key];
  if (step === undefined || step === 0) return;
  const here = usageStops.indexOf(event.target as HTMLElement);
  if (here < 0) return;
  const next = usageStops[here + step];
  if (!next) return;
  event.preventDefault();
  roving(next, true);
}

usageGrid?.addEventListener("keydown", (event) => walkDays(event, 7));
usageStrip?.addEventListener("keydown", (event) => walkDays(event, 1));

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

// The status-line wrapper asks twice. The first press writes nothing and prints the diff;
// the second is the one that edits Claude Code's settings file, and it only exists on the
// page while a preview is on screen.
statuslinePreview?.addEventListener("click", () => {
  const remove = statusline?.installed === true;
  statuslinePreview.disabled = true;
  void invoke<StatuslineOutcome>("statusline_preview", { remove })
    .then((outcome) => {
      statusline = outcome;
      // A dry run that failed changed nothing either, so there is nothing to confirm.
      statuslinePending = outcome.outcome === "ok" ? (remove ? "remove" : "install") : undefined;
    })
    .catch(() => {})
    .finally(() => {
      statuslinePreview.disabled = false;
      paintStatusline();
    });
});

statuslineApply?.addEventListener("click", () => {
  const remove = statuslinePending === "remove";
  statuslineApply.disabled = true;
  void invoke<StatuslineOutcome>("statusline_apply", { remove })
    .then((outcome) => {
      statusline = outcome;
      statuslinePending = undefined;
    })
    .catch(() => {})
    .finally(() => {
      statuslineApply.disabled = false;
      paintStatusline();
    });
});

statuslineCancel?.addEventListener("click", () => {
  statuslinePending = undefined;
  void loadStatusline();
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

// Esc closes the panel — except on the settings and usage pages, where it goes back one step
// first, so a user who opened one by accident is not thrown out of the panel entirely. On the
// usage page it is the *Back* button's own function, which is how the key and the button stay
// one behaviour: a day or a week that is open is a step of its own, and it is the one Esc
// undoes first.
window.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  if (shown === "usage") {
    usageBack();
    return;
  }
  if (shown !== "quota") {
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
