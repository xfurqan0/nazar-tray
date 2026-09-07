// Licence gate for every dependency, Rust and npm.
//
// The rule (unified plan, risk R6): permissive only. A copyleft dependency in an MIT
// product is not a licence question at release time, it is a rewrite, so it is caught
// on the day it is added. deny.toml holds the same policy for `cargo deny`, which does
// a stricter job; this script exists so the check runs with nothing installed but the
// toolchain the repo already needs.
//
// Usage: node scripts/check-licenses.mjs [--json]

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/**
 * SPDX identifiers this project accepts. Anything else stops the build.
 *
 * `CDLA-Permissive-2.0` is the odd one out and is here on purpose. It covers exactly one
 * dependency, `webpki-root-certs`, and what it licenses is *data*: Mozilla's root
 * certificate list. It grants unrestricted use with no share-alike clause, so it is
 * permissive in the sense this list cares about. It arrives through `reqwest`, which is
 * a dependency of `tauri` itself rather than of the updater plugin, so it cannot be
 * dropped without dropping Tauri.
 */
const ALLOWED = new Set([
  "0BSD",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "BSL-1.0",
  "CC0-1.0",
  "CDLA-Permissive-2.0",
  "ISC",
  "MIT",
  "MIT-0",
  "MPL-2.0",
  "Unicode-3.0",
  "Unicode-DFS-2016",
  "Unlicense",
  "Zlib",
]);

/**
 * Evaluate an SPDX expression against the allowlist.
 *
 * `A OR B` passes when either side does, because the user picks. `A AND B` needs both.
 * Exceptions (`Apache-2.0 WITH LLVM-exception`) narrow a licence, so the base decides.
 */
function isAllowed(expression) {
  if (!expression) return false;
  const normalised = expression.replace(/\//g, " OR ").replace(/[()]/g, " ");
  const terms = (clause) =>
    clause
      .split(/\s+/)
      .filter((word) => word && !["OR", "AND", "WITH"].includes(word.toUpperCase()))
      .filter((word) => !word.endsWith("-exception"));

  if (/\sAND\s/i.test(normalised)) {
    return normalised
      .split(/\sAND\s/i)
      .every((clause) => terms(clause).some((term) => ALLOWED.has(term)));
  }
  return terms(normalised).some((term) => ALLOWED.has(term));
}

function cargoDependencies() {
  const raw = execFileSync("cargo", ["metadata", "--format-version", "1", "--locked"], {
    cwd: REPO,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  const metadata = JSON.parse(raw);
  const workspace = new Set(metadata.workspace_members);

  return metadata.packages
    .filter((pkg) => !workspace.has(pkg.id))
    .map((pkg) => ({
      ecosystem: "cargo",
      name: pkg.name,
      version: pkg.version,
      license: pkg.license ?? (pkg.license_file ? `file: ${pkg.license_file}` : null),
    }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

function npmDependencies() {
  const modules = join(REPO, "ui", "node_modules");
  if (!existsSync(modules)) return [];

  const found = [];
  const visit = (dir, scope) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (!entry.isDirectory()) continue;
      if (entry.name === ".bin") continue;
      if (entry.name.startsWith("@") && !scope) {
        visit(join(dir, entry.name), entry.name);
        continue;
      }
      const manifest = join(dir, entry.name, "package.json");
      if (!existsSync(manifest)) continue;
      const pkg = JSON.parse(readFileSync(manifest, "utf8"));
      found.push({
        ecosystem: "npm",
        name: pkg.name ?? (scope ? `${scope}/${entry.name}` : entry.name),
        version: pkg.version ?? "unknown",
        license:
          typeof pkg.license === "string" ? pkg.license : (pkg.license?.type ?? pkg.licenses?.[0]?.type ?? null),
      });
    }
  };
  visit(modules, null);
  return found.sort((a, b) => a.name.localeCompare(b.name));
}

const dependencies = [...cargoDependencies(), ...npmDependencies()];
const rejected = dependencies.filter((dep) => !isAllowed(dep.license));
const asJson = process.argv.includes("--json");

if (asJson) {
  // Nothing but JSON on stdout, so the output stays pipeable.
  process.stdout.write(`${JSON.stringify(dependencies, null, 2)}\n`);
} else {
  const counts = new Map();
  for (const dep of dependencies) {
    counts.set(dep.license ?? "unknown", (counts.get(dep.license ?? "unknown") ?? 0) + 1);
  }
  process.stdout.write(`${dependencies.length} dependencies\n\n`);
  for (const [license, count] of [...counts].sort((a, b) => b[1] - a[1])) {
    process.stdout.write(`  ${String(count).padStart(4)}  ${license}\n`);
  }
}

if (rejected.length > 0) {
  process.stderr.write("\nnot on the allowlist:\n");
  for (const dep of rejected) {
    process.stderr.write(`  ${dep.ecosystem}  ${dep.name}@${dep.version}  ${dep.license ?? "no licence"}\n`);
  }
  process.stderr.write(
    "\nAdd the identifier to ALLOWED only if it is permissive, or drop the dependency.\n",
  );
  process.exit(1);
}

if (!asJson) {
  process.stdout.write("\nevery dependency is permissively licensed\n");
}
