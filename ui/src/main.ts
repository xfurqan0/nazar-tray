/**
 * Panel entry point.
 *
 * WP0's panel was honest about being empty. WP3's is honest about being undesigned: it
 * asks the Rust side for the derived snapshot, prints the raw values — provider, each
 * window's percentage or the word "unknown", the countdown, how old the reading is — and
 * redraws when the loop says the document moved. There is no bar, no bead fill and no
 * layout worth the name; that is WP4, and a placeholder design here would only have to be
 * deleted.
 *
 * What it does do, and what WP4 inherits:
 *
 * * **Nothing is hard-coded.** Every word comes from `locales/<lang>.json`. The numbers are
 *   numerals and colons, which need no translation.
 * * **Unknown is a word, not a zero.** A window with no percentage says so.
 * * **The countdown ticks locally.** The document changes when the numbers change, which
 *   may be minutes apart; the seconds in between are arithmetic the panel can do itself.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { createTranslator, detectLocale, type Translate } from "./i18n";
import { catalogs } from "./locales";
import {
  displayPercent,
  formatClock,
  freshnessKey,
  severityClass,
  type ProviderView,
  type SnapshotView,
  type WindowView,
} from "./snapshot";
import { THEMES, applyTheme } from "./theme";

const languages: readonly string[] =
  navigator.languages && navigator.languages.length > 0
    ? navigator.languages
    : [navigator.language];

const locale = detectLocale(languages, catalogs);
const t: Translate = createTranslator(catalogs, locale);

const root = document.documentElement;
root.lang = locale;

const nazarTheme = THEMES["nazar"];
if (nazarTheme) {
  // The panel is the navy face of the product; WP5 adds the theme and mode settings.
  applyTheme(root, nazarTheme, "dark");
}

for (const node of document.querySelectorAll<HTMLElement>("[data-i18n]")) {
  const key = node.dataset["i18n"];
  if (key) node.textContent = t(key);
}

const bead = document.querySelector<SVGElement>("[data-bead] svg");
bead?.setAttribute("aria-label", t("panel.bead.alt"));

const headline = document.querySelector<HTMLElement>("[data-headline]");
const providers = document.querySelector<HTMLElement>("[data-providers]");

/**
 * The last snapshot, and the local instant it was derived for.
 *
 * Kept so the countdown can tick between refreshes: the document only changes when the
 * numbers change, and a display that stood still for a minute at a time would look broken.
 */
let latest: SnapshotView | undefined;
let derivedAt = 0;

/** How far the local clock has moved since the snapshot was derived. */
function drift(): number {
  return latest ? Date.now() - derivedAt : 0;
}

/** One window's line: `12 % · 4:12:07`, or the word for "unknown". */
function windowLine(window: WindowView): string {
  const parts: string[] = [];
  parts.push(
    window.percent === undefined
      ? t("panel.window.unknown")
      : t("panel.window.percent", { percent: displayPercent(window.percent) }),
  );
  if (window.remainingMs !== undefined) {
    const remaining = window.remainingMs - drift();
    parts.push(
      remaining <= 0
        ? t("panel.window.resetDue")
        : t("panel.window.resetsIn", { time: formatClock(remaining) }),
    );
  }
  return parts.join(" · ");
}

/** One provider's block: a heading and a line per window. */
function providerNode(provider: ProviderView): HTMLElement {
  const item = document.createElement("li");
  item.className = "provider";

  const heading = document.createElement("div");
  heading.className = "provider-heading";

  const name = document.createElement("span");
  name.className = "provider-name";
  name.textContent = t(`panel.provider.${provider.name}`);
  heading.append(name);

  const age = document.createElement("span");
  age.className = "provider-age";
  age.textContent = provider.configured
    ? t(freshnessKey(provider.freshness))
    : t("panel.provider.notConfigured");
  heading.append(age);
  item.append(heading);

  const list = document.createElement("ul");
  list.className = "windows";
  for (const usage of provider.windows) {
    const row = document.createElement("li");
    row.className = `window ${severityClass(usage.severity)}`;
    if (usage.binding) row.classList.add("binding");

    const key = document.createElement("span");
    key.className = "window-key";
    // The window key is data, not a message: `five_hour`, `seven_day_fable`, `primary`.
    // WP4 gives them labels; naming them here would put six words in the code that no
    // translation could reach.
    key.textContent = usage.model ? `${usage.key} (${usage.model})` : usage.key;
    row.append(key);

    const value = document.createElement("span");
    value.className = "window-value";
    value.textContent = windowLine(usage);
    row.append(value);

    list.append(row);
  }
  item.append(list);
  return item;
}

/** Redraw everything from the last snapshot. */
function draw(): void {
  if (!headline || !providers) return;

  if (!latest) {
    headline.textContent = t("panel.headline", {
      name: t("app.name"),
      version: __APP_VERSION__,
      status: t("panel.status.noData"),
    });
    return;
  }

  const anyWindows = latest.providers.some((provider) => provider.windows.length > 0);
  headline.textContent = t("panel.headline", {
    name: t("app.name"),
    version: __APP_VERSION__,
    status: anyWindows
      ? t("panel.status.updated", { age: formatClock(drift()) })
      : t("panel.status.noData"),
  });

  providers.replaceChildren(...latest.providers.map(providerNode));
}

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

draw();
void refresh();

// The loop tells the panel when the document moved; the panel counts the seconds itself.
void listen("snapshot-changed", () => void refresh());
window.setInterval(draw, 1000);

// Redraw whenever the panel comes back into view: it is hidden rather than closed, so it
// can have been out of sight for hours with a countdown that stopped mattering.
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) void refresh();
});

// Esc closes the panel. Clicking elsewhere closes it too, but that is handled in Rust: the
// window hides itself when it loses focus.
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    void getCurrentWindow().hide();
  }
});
