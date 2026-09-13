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
import { THEMES, applyTheme } from "./theme";
import {
  buildUsageView,
  formatColumn,
  formatDay,
  formatNumber,
  formatTokens,
  isUsageError,
  requestsKey,
  usageErrorKey,
  usageWindow,
  type UsageBar,
  type UsageRow,
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
const usageTabs = [...document.querySelectorAll<HTMLButtonElement>("[data-usage-range]")];
const usageSummary = document.querySelector<HTMLElement>("[data-usage-summary]");
const usageTotal = document.querySelector<HTMLElement>("[data-usage-total]");
const usageCache = document.querySelector<HTMLElement>("[data-usage-cache]");
const usageStrip = document.querySelector<HTMLElement>("[data-usage-strip]");
const usageRows = document.querySelector<HTMLElement>("[data-usage-rows]");
const usageStateLine = document.querySelector<HTMLElement>("[data-usage-state]");
const usageErrorLine = document.querySelector<HTMLElement>("[data-usage-error]");
const usageDetailLine = document.querySelector<HTMLElement>("[data-usage-detail]");
const usageSinceLine = document.querySelector<HTMLElement>("[data-usage-since]");
const usageScannedLine = document.querySelector<HTMLElement>("[data-usage-scanned]");
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

/** The usage answer being drawn, the range it answers, and what went wrong instead. */
let usageRange: UsageRange = "week";
let usageAnswer: UsageResponse | undefined;
let usageProblem: { kind: string; detail: string } | undefined;
let usageLoading = false;

/**
 * Which request the view is waiting for.
 *
 * A cold scan takes a second or two, which is long enough for somebody to press *Month* and
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
function usageRowItem(row: UsageRow): HTMLElement {
  const item = document.createElement("li");
  item.className = "usage-row";

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

  const foot = document.createElement("div");
  foot.className = "usage-row-foot";
  const records = document.createElement("span");
  // Two labels rather than one: on the Claude side a record is a reply, on the Codex side it
  // is a `token_count` event and several of them make one turn.
  records.textContent = t(requestsKey(row.provider), {
    requests: formatNumber(row.requests, locale),
  });
  foot.append(records);
  item.append(foot);

  return item;
}

/** One column of the strip: a local day, or a local week on the *all* tab. */
function usageColumn(bar: UsageBar): HTMLElement {
  const column = document.createElement("span");
  column.className = "usage-bar";
  column.title = t("usage.bar", {
    date: formatColumn(bar.at, locale),
    tokens: formatTokens(bar.total, locale),
  });
  if (bar.height > 0) {
    const fill = document.createElement("span");
    fill.className = "usage-bar-fill";
    fill.style.height = `${bar.height}%`;
    column.append(fill);
  }
  return column;
}

/** Draw the usage view from the last answer, the last failure, or neither. */
function paintUsage(): void {
  if (!usageRows) return;

  for (const tab of usageTabs) {
    const own = tab.dataset["usageRange"] === usageRange;
    tab.classList.toggle("primary", own);
    tab.setAttribute("aria-selected", String(own));
  }

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

  if (usageStateLine) {
    let word = "";
    // A cold scan takes a second or two. A line saying so beats a panel that looks broken,
    // and it beats last week's numbers sitting under this week's heading while it waits.
    if (usageLoading) word = t("usage.loading");
    else if (view && view.empty && !failed) word = t("usage.empty");
    usageStateLine.hidden = word === "";
    usageStateLine.textContent = word;
  }

  const numbers = view !== undefined && !view.empty && !usageLoading && !failed;
  if (usageSummary) usageSummary.hidden = !numbers;
  if (usageStrip) usageStrip.hidden = !numbers;

  if (numbers && view) {
    if (usageTotal) usageTotal.textContent = formatTokens(view.total, locale);
    if (usageCache) {
      // Never inside the headline: cache reads were 98.5 % of the raw total over six days of
      // real work, so a number with them folded in is a number about the cache.
      usageCache.textContent = t("usage.cacheRead", {
        tokens: formatTokens(view.cacheRead, locale),
      });
    }
    if (usageStrip) usageStrip.replaceChildren(...view.bars.map(usageColumn));

    const items: HTMLElement[] = [];
    for (const group of view.groups) {
      // A heading for one provider is a heading that says nothing; it earns its line only
      // when there are two of them to tell apart.
      if (view.grouped) {
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
      items.push(...group.rows.map(usageRowItem));
    }
    usageRows.replaceChildren(...items);
  } else {
    usageRows.replaceChildren();
    if (usageStrip) usageStrip.replaceChildren();
  }

  if (usageSinceLine) {
    // Only on the *all* tab, where it is the honest boundary of the word: the store holds
    // what the first scan could still see, not everything that ever happened.
    const show = view !== undefined && view.range === "all" && view.since !== undefined;
    usageSinceLine.hidden = !show;
    usageSinceLine.textContent = show
      ? t("usage.since", { date: formatDay(view?.since, locale) })
      : "";
  }
  if (usageScannedLine) {
    const at = view?.scannedAt === undefined ? Number.NaN : Date.parse(view.scannedAt);
    const show = Number.isFinite(at);
    usageScannedLine.hidden = !show;
    usageScannedLine.textContent = show
      ? t("usage.scanned", { age: formatDuration(Date.now() - at, t) })
      : "";
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
 * Ask for one range and draw what comes back.
 *
 * `force` is the view's own Refresh: the throttle on the Rust side lets a scan run at most
 * every five minutes, and somebody who has just finished a long session should not be told
 * to wait for numbers that are already on disk.
 */
async function loadUsage(range: UsageRange, force = false): Promise<void> {
  usageRange = range;
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
  void loadUsage(usageRange);
});

document.querySelector("[data-usage-back]")?.addEventListener("click", () => {
  showView("quota");
});

for (const tab of usageTabs) {
  tab.addEventListener("click", () => {
    const range = tab.dataset["usageRange"];
    if (range === "week" || range === "month" || range === "all") void loadUsage(range);
  });
}

usageRefresh?.addEventListener("click", () => {
  void loadUsage(usageRange, true);
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
// first, so a user who opened one by accident is not thrown out of the panel entirely.
window.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
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
