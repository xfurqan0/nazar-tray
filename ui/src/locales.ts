import type { Catalogs } from "./i18n";

import en from "../locales/en.json";
import es from "../locales/es.json";
import ko from "../locales/ko.json";
import ru from "../locales/ru.json";
import tr from "../locales/tr.json";
import zh from "../locales/zh.json";

/**
 * Every catalogue, bundled into the panel.
 *
 * Bundled rather than fetched: six flat files are a few hundred bytes together, and a
 * panel that fetches its own strings would need a network permission it should not have.
 */
export const catalogs: Catalogs = { en, tr, zh, ko, ru, es };
