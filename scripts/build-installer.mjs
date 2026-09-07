// Builds the Windows installer, from a checkout to a signed-nothing NSIS package.
//
// `cargo tauri build` on its own is not enough, and the reason is the second binary. The
// bundle ships two programs — the tray and `nazar-statusline`, the status-line wrapper — and
// Tauri picks the second one up through `bundle.externalBin`, which wants it on disk under
// its target-triple name before the tray crate even compiles. Cargo will not put it there:
// the wrapper is a separate crate, and `cargo tauri build` builds one package.
//
// So this script is the build. In order:
//
//   1. the panel, because `tauri::generate_context!` reads `ui/dist` at compile time and a
//      stale `dist` produces a stale installer with no error anywhere;
//   2. `THIRD-PARTY-NOTICES.md`, which the bundle carries as a resource, regenerated from
//      the lock file so it cannot describe a different dependency set from the one built;
//   3. the sidecar (`scripts/sidecar.mjs`);
//   4. `cargo tauri build`, which builds the tray and produces the installer.
//
// Then it prints what came out, with sizes and SHA-256, because those are the numbers the
// release checklist asks for and computing them by hand is how they end up wrong.
//
// Usage:
//   node scripts/build-installer.mjs                 release build, NSIS installer
//   node scripts/build-installer.mjs --debug         the same, unoptimised, for a quick check
//   node scripts/build-installer.mjs --bundles nsis,msi
//   node scripts/build-installer.mjs --skip-panel
//
// Nothing here signs, tags, uploads or installs anything. `docs/RELEASE.md` is the checklist
// that does the rest, by hand.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// Honoured so a comparison build can be made in a second directory without disturbing the
// first — which is how the size table in docs/RELEASE.md was measured.
const TARGET = process.env["CARGO_TARGET_DIR"]
  ? resolve(process.env["CARGO_TARGET_DIR"])
  : join(REPO, "target");
const TAURI_DIR = join(REPO, "crates", "nazar-tray");

const argv = process.argv.slice(2);
const debug = argv.includes("--debug");
const skipPanel = argv.includes("--skip-panel");
const bundles = (() => {
  const at = argv.indexOf("--bundles");
  return at >= 0 ? argv[at + 1] : undefined;
})();

const profile = debug ? "debug" : "release";
const suffix = process.platform === "win32" ? ".exe" : "";

/** Run a command, let its output through, and stop the script if it fails. */
function run(command, args, cwd) {
  process.stdout.write(`\n> ${command} ${args.join(" ")}\n`);
  const finished = spawnSync(command, args, {
    cwd,
    stdio: "inherit",
    shell: process.platform === "win32",
  });
  if (finished.status !== 0) {
    process.stderr.write(`\n${command} failed with exit code ${finished.status}\n`);
    process.exit(finished.status ?? 1);
  }
}

function human(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function report(file) {
  const name = relative(REPO, file).padEnd(58);
  process.stdout.write(`${name} ${human(statSync(file).size).padStart(10)}  ${sha256(file)}\n`);
}

// 1. The panel.
if (!skipPanel) {
  if (!existsSync(join(REPO, "ui", "node_modules"))) {
    process.stderr.write("ui/node_modules is missing. Run `npm ci` in ui/ first.\n");
    process.exit(1);
  }
  run("npm", ["run", "build"], join(REPO, "ui"));
}

// 2. The notices, from the lock file this build resolves.
run("node", [join("scripts", "third-party-notices.mjs")], REPO);

// 3. The wrapper, and the copy the bundler looks for.
run("node", [join("scripts", "sidecar.mjs"), ...(debug ? ["--debug"] : [])], REPO);

// 4. The tray and the installer.
run(
  "cargo",
  ["tauri", "build", ...(debug ? ["--debug"] : []), ...(bundles ? ["--bundles", bundles] : [])],
  TAURI_DIR,
);

// What came out.
const artefacts = [
  join(TARGET, profile, `nazar-tray${suffix}`),
  join(TARGET, profile, `nazar-statusline${suffix}`),
  join(TARGET, profile, "bundle", "nsis"),
  join(TARGET, profile, "bundle", "msi"),
];

process.stdout.write("\n--- artefacts ---\n");
for (const path of artefacts) {
  if (!existsSync(path)) continue;
  if (statSync(path).isDirectory()) {
    for (const entry of readdirSync(path).sort()) {
      const file = join(path, entry);
      if (statSync(file).isFile()) report(file);
    }
  } else {
    report(path);
  }
}
