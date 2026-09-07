/**
 * Panel entry point.
 *
 * WP0's panel is honest about being empty: it names the app and its version, says there
 * is no data yet, and lists both providers as unknown. It does not draw a zero, an empty
 * bar or a placeholder percentage, because "unknown" and "0 %" mean opposite things to
 * someone deciding whether to start a long task.
 */

import { getCurrentWindow } from "@tauri-apps/api/window";

import { createTranslator, detectLocale } from "./i18n";
import { catalogs } from "./locales";
import { THEMES, applyTheme } from "./theme";

const languages: readonly string[] =
  navigator.languages && navigator.languages.length > 0
    ? navigator.languages
    : [navigator.language];

const locale = detectLocale(languages, catalogs);
const t = createTranslator(catalogs, locale);

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

const headline = document.querySelector<HTMLElement>("[data-headline]");
if (headline) {
  headline.textContent = t("panel.headline", {
    name: t("app.name"),
    version: __APP_VERSION__,
    status: t("panel.status.noData"),
  });
}

const bead = document.querySelector<SVGElement>("[data-bead] svg");
bead?.setAttribute("aria-label", t("panel.bead.alt"));

// Esc closes the panel. Clicking elsewhere closes it too, but that is handled in Rust:
// the window hides itself when it loses focus.
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    void getCurrentWindow().hide();
  }
});
