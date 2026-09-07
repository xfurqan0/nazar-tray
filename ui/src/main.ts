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
import { createTranslator, detectLocale, isLocale, type Locale } from "./i18n";
import { catalogs } from "./locales";
import {
  displayPercent,
  formatClock,
  type ProviderView,
  type SnapshotView,
  type WindowView,
} from "./snapshot";
import { THEMES, applyTheme } from "./theme";

/** What `get_ui_state` returns: the choices, as against the measurements. */
interface UiState {
  readonly theme: string;
  readonly mode: string;
  readonly locale: string | null;
  readonly hintDismissed: boolean;
  readonly demo: boolean;
}

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

let ui: UiState = {
  theme: "nazar",
  mode: "system",
  locale: null,
  hintDismissed: true,
  demo: false,
};
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
  if (themeButton) {
    themeButton.setAttribute("aria-label", t("panel.action.theme"));
    themeButton.title = t("panel.action.theme");
  }
}

/** Paint the panel in the chosen theme, following the system for light and dark. */
function applyChosenTheme(): void {
  const theme = THEMES[ui.theme] ?? THEMES["nazar"];
  if (!theme) return;
  applyTheme(root, theme, resolveMode(ui.mode, darkQuery.matches));
  if (themeButton) themeButton.textContent = theme.label;
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
  reportHeight();
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

  const preferred =
    ui.locale && isLocale(ui.locale) ? [ui.locale] : [...navigator.languages, navigator.language];
  locale = detectLocale(preferred, catalogs);
  t = createTranslator(catalogs, locale);

  applyLanguage();
  applyChosenTheme();
  await refresh();
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

// Esc closes the panel. Clicking elsewhere closes it too, but that is handled in Rust: the
// window hides itself when it loses focus.
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    void getCurrentWindow().hide();
  }
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
