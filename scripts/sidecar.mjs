// Puts `nazar-statusline` where the Tauri build expects to find it.
//
// The bundle ships two programs, and Tauri picks the second one up through
// `bundle.externalBin`. That mechanism wants the file on disk, under its target-triple name,
// **before the tray crate compiles** — `tauri-build` checks for it in the build script, so a
// missing sidecar fails `cargo clippy` and `cargo test` too, not only the bundler. Cargo will
// not put it there on its own: the wrapper is a separate crate.
//
// So: build it, and copy it. Run this before anything that compiles `nazar-tray`.
//
//   node scripts/sidecar.mjs            release build (what the installer ships)
//   node scripts/sidecar.mjs --debug    debug build, for lint and test runs
//
// The copy lands in `crates/nazar-tray/binaries/`, which is git-ignored: it is a build
// artefact, and committing a binary to satisfy a build step is how a repository ends up
// shipping a stale one.

import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// Honoured so a comparison build can be made in a second directory without disturbing the
// first — which is how the size table in docs/RELEASE.md was measured.
const TARGET = process.env["CARGO_TARGET_DIR"]
  ? resolve(process.env["CARGO_TARGET_DIR"])
  : join(REPO, "target");
const debug = process.argv.includes("--debug");
const profile = debug ? "debug" : "release";
const suffix = process.platform === "win32" ? ".exe" : "";

/** The triple `bundle.externalBin` expects in the file name. */
function hostTriple() {
  const verbose = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const line = verbose.split(/\r?\n/).find((row) => row.startsWith("host: "));
  if (!line) throw new Error("rustc -vV printed no host line");
  return line.slice("host: ".length).trim();
}

const args = ["build", "-p", "nazar-statusline", ...(debug ? [] : ["--release"])];
process.stdout.write(`> cargo ${args.join(" ")}\n`);
const built = spawnSync("cargo", args, {
  cwd: REPO,
  stdio: "inherit",
  shell: process.platform === "win32",
});
if (built.status !== 0) process.exit(built.status ?? 1);

const source = join(TARGET, profile, `nazar-statusline${suffix}`);
if (!existsSync(source)) {
  process.stderr.write(`cargo did not produce ${source}\n`);
  process.exit(1);
}

const directory = join(REPO, "crates", "nazar-tray", "binaries");
mkdirSync(directory, { recursive: true });
const target = join(directory, `nazar-statusline-${hostTriple()}${suffix}`);
copyFileSync(source, target);

const size = statSync(target).size;
process.stdout.write(`${relative(REPO, target)}  ${(size / 1024).toFixed(1)} KB\n`);
