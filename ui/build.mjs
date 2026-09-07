// Build the popup panel into ui/dist, which tauri.conf.json points `frontendDist` at.
//
// Three outputs:
//   dist/index.html   the panel, with the bead SVG inlined from ui/assets/bead.svg
//   dist/panel.js     the bundled module, locale and theme JSON included
//   dist/lib/*.mjs    the same modules as plain ESM, so test/ can import them
//
// No framework and no dev server (decision K2): the panel is a few hundred lines of
// TypeScript, and the resource budget is part of what the product claims.

import { build } from "esbuild";
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const SRC = resolve(HERE, "src");
const DIST = resolve(HERE, "dist");

const pkg = JSON.parse(readFileSync(resolve(HERE, "package.json"), "utf8"));

const shared = {
  bundle: true,
  format: "esm",
  target: ["chrome110"], // WebView2 is evergreen Chromium; this is the floor, not the aim
  define: { __APP_VERSION__: JSON.stringify(pkg.version) },
  logLevel: "warning",
};

rmSync(DIST, { recursive: true, force: true });
mkdirSync(DIST, { recursive: true });

await build({
  ...shared,
  entryPoints: [resolve(SRC, "main.ts")],
  outfile: resolve(DIST, "panel.js"),
});

// The tests exercise the same modules the panel runs, not a copy of their logic.
await build({
  ...shared,
  entryPoints: [
    resolve(SRC, "i18n.ts"),
    resolve(SRC, "theme.ts"),
    resolve(SRC, "locales.ts"),
    resolve(SRC, "snapshot.ts"),
  ],
  outdir: resolve(DIST, "lib"),
  outExtension: { ".js": ".mjs" },
});

const bead = readFileSync(resolve(HERE, "assets/bead.svg"), "utf8").trim();
const html = readFileSync(resolve(SRC, "index.html"), "utf8");
if (!html.includes("<!--BEAD_SVG-->")) {
  throw new Error("src/index.html lost its <!--BEAD_SVG--> placeholder");
}
writeFileSync(resolve(DIST, "index.html"), html.replace("<!--BEAD_SVG-->", bead));

cpSync(resolve(SRC, "styles.css"), resolve(DIST, "styles.css"));

process.stdout.write(`built panel ${pkg.version} into ${DIST}\n`);
