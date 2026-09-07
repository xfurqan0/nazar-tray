// Generates THIRD-PARTY-NOTICES.md — the attribution that ships inside the installer.
//
// Every permissive licence this product depends on asks for the same thing: keep the notice
// and the copyright with the copies you hand out. `LICENSE` covers our own code. This file
// covers everybody else's, and the installer puts it next to the binaries so that a copy of
// nazar-tray on someone else's machine carries it too.
//
// Why it is generated rather than written: 500 crates, and the answer changes every time
// `Cargo.lock` does. `cargo about` does this job well, and `deny.toml` already documents the
// policy for `cargo deny`, but both are extra installs — and this file has to be
// regeneratable by anyone who can build the product, with nothing beyond the toolchain the
// repository already needs. Same argument as `scripts/check-licenses.mjs`, which enforces
// the allow-list this file then reports on.
//
// What is counted:
//
//   * The **normal** dependencies of the two binaries that ship, `nazar-tray` and
//     `nazar-statusline`, resolved for the **target the installer is built for**. Build
//     dependencies are excluded because none of their code is in the binaries; dev
//     dependencies are excluded for the same reason. Optional dependencies that the default
//     features do not turn on are already absent from a `--filter-platform` resolve.
//   * The panel's **runtime** npm dependencies — `@tauri-apps/api`, and nothing else. The
//     two dev dependencies build the bundle and are not in it.
//   * The panel's **image assets**, whose licence is a file rather than a manifest field.
//
// Usage: node scripts/third-party-notices.mjs [--check]
//
// `--check` regenerates in memory and fails if the file on disk differs, which is how CI
// notices that a dependency was added without the notices being refreshed.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUTPUT = join(REPO, "THIRD-PARTY-NOTICES.md");

/** The two crates that become the two files in the bundle. */
const SHIPPED = ["nazar-tray", "nazar-statusline"];

/**
 * When a crate offers a choice, this is the one taken, in order of preference.
 *
 * A dual `MIT OR Apache-2.0` crate is used under one of the two, not both, and the notice
 * has to say which. MIT first because it is what this product is under and what the shortest
 * notice needs; Apache-2.0 next, which brings its NOTICE requirement with it.
 */
const PREFERENCE = [
  "MIT",
  "MIT-0",
  "0BSD",
  "ISC",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "Zlib",
  "Unlicense",
  "CC0-1.0",
  "BSL-1.0",
  "Apache-2.0",
  "Unicode-3.0",
  "Unicode-DFS-2016",
  "CDLA-Permissive-2.0",
  "MPL-2.0",
];

/** File names that hold a licence text, in the order a crate usually means them. */
const LICENCE_FILE = /^(LICEN[CS]E|COPYING|NOTICE|UNLICEN[CS]E)/i;

/** The host triple, so the resolve matches the binaries that are actually built. */
function hostTriple() {
  const verbose = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const line = verbose.split(/\r?\n/).find((row) => row.startsWith("host: "));
  if (!line) throw new Error("rustc -vV printed no host line");
  return line.slice("host: ".length).trim();
}

/** Split an SPDX expression into the terms this project could choose between. */
function terms(expression) {
  if (!expression) return [];
  return expression
    .replace(/\//g, " OR ")
    .replace(/[()]/g, " ")
    .split(/\s+(?:OR|AND)\s+/i)
    .map((clause) => clause.split(/\s+WITH\s+/i)[0].trim())
    .filter(Boolean);
}

/** The single licence a crate is used under here. */
function chosen(expression) {
  const available = terms(expression);
  for (const preferred of PREFERENCE) {
    if (available.includes(preferred)) return preferred;
  }
  return available[0] ?? "unknown";
}

/** Every licence text a crate ships, longest first — the longest is the licence itself. */
function licenceTexts(manifestPath) {
  const directory = dirname(manifestPath);
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && LICENCE_FILE.test(entry.name))
    .map((entry) => ({
      name: entry.name,
      text: readFileSync(join(directory, entry.name), "utf8").replace(/\r\n/g, "\n").trim(),
    }))
    .sort((a, b) => b.text.length - a.text.length);
}

/**
 * The copyright lines out of a licence text.
 *
 * This is the part a permissive licence actually asks to be carried, and it is the part a
 * generated notice most often loses: the licence body is the same for everyone, the
 * copyright line is not.
 */
function copyrights(texts) {
  // A real notice, not a licence body. Three rules, and each was written because the naive
  // version put something wrong in the table: capital `Copyright` (Apache's own text says
  // "copyright notice that is included in…" in lower case, mid-sentence), a four-digit year
  // or "All rights reserved" (so a sentence about copyright is not mistaken for one), and no
  // template placeholder (Apache's appendix carries `Copyright [yyyy] [name of copyright
  // owner]`, which names nobody).
  const PLACEHOLDER = /\[yyyy]|\{yyyy}|<year>|\{year}|name of copyright owner|<name of author>/i;
  const found = new Set();
  for (const { text } of texts) {
    for (const line of text.split("\n")) {
      const trimmed = line.trim().replace(/\s+/g, " ");
      if (trimmed.length === 0 || trimmed.length > 200) continue;
      if (!/^(Copyright|©|\(c\) )/.test(trimmed)) continue;
      if (!/\b(19|20)\d{2}\b/.test(trimmed) && !/all rights reserved/i.test(trimmed)) continue;
      if (PLACEHOLDER.test(trimmed)) continue;
      found.add(trimmed);
    }
  }
  return [...found];
}

/** Walk the resolved graph from the shipped crates, following normal dependencies only. */
function shippedCrates(metadata) {
  const byId = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
  const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
  const workspace = new Set(metadata.workspace_members);

  const roots = metadata.packages
    .filter((pkg) => workspace.has(pkg.id) && SHIPPED.includes(pkg.name))
    .map((pkg) => pkg.id);
  if (roots.length !== SHIPPED.length) {
    throw new Error(`expected ${SHIPPED.join(" and ")} in the workspace`);
  }

  const seen = new Set();
  const queue = [...roots];
  while (queue.length > 0) {
    const id = queue.shift();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const dependency of nodes.get(id)?.deps ?? []) {
      // `kind: null` is a normal dependency. A build dependency compiles a build script and
      // a dev dependency compiles a test; neither is linked into a shipped binary.
      const normal = dependency.dep_kinds.some((kind) => kind.kind === null);
      if (normal) queue.push(dependency.pkg);
    }
  }

  return [...seen]
    .filter((id) => !workspace.has(id))
    .map((id) => byId.get(id))
    .filter(Boolean)
    .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
}

/** The panel's runtime npm dependencies, read from the tree that built `ui/dist`. */
function panelDependencies() {
  const manifest = JSON.parse(readFileSync(join(REPO, "ui", "package.json"), "utf8"));
  const names = Object.keys(manifest.dependencies ?? {});
  return names
    .map((name) => {
      const directory = join(REPO, "ui", "node_modules", ...name.split("/"));
      const path = join(directory, "package.json");
      if (!existsSync(path)) return null;
      const pkg = JSON.parse(readFileSync(path, "utf8"));
      const texts = licenceTexts(path);
      return {
        name,
        version: pkg.version ?? "unknown",
        license: typeof pkg.license === "string" ? pkg.license : (pkg.license?.type ?? "unknown"),
        repository:
          typeof pkg.repository === "string" ? pkg.repository : (pkg.repository?.url ?? ""),
        texts,
      };
    })
    .filter(Boolean);
}

/** Turn a repository field into something worth printing. */
function homepage(pkg) {
  return (pkg.repository ?? pkg.homepage ?? "").replace(/^git\+/, "").replace(/\.git$/, "");
}

function build() {
  const triple = hostTriple();
  const metadata = JSON.parse(
    execFileSync(
      "cargo",
      ["metadata", "--format-version", "1", "--locked", "--filter-platform", triple],
      { cwd: REPO, encoding: "utf8", maxBuffer: 128 * 1024 * 1024 },
    ),
  );

  const crates = shippedCrates(metadata).map((pkg) => {
    const texts = licenceTexts(pkg.manifest_path);
    return {
      name: pkg.name,
      version: pkg.version,
      expression: pkg.license ?? "",
      licence: chosen(pkg.license ?? ""),
      url: homepage(pkg),
      copyrights: copyrights(texts),
      texts,
    };
  });

  const npm = panelDependencies().map((pkg) => ({
    ...pkg,
    licence: chosen(pkg.license),
    url: homepage(pkg),
    copyrights: copyrights(pkg.texts),
  }));

  const everything = [...crates, ...npm];
  const licences = [...new Set(everything.map((pkg) => pkg.licence))].sort();

  const lines = [];
  lines.push("# Third-party notices");
  lines.push("");
  lines.push(
    "nazar-tray is MIT licensed; `LICENSE` is its own notice. This file is everybody else's.",
  );
  lines.push("");
  lines.push(
    "It is generated by `scripts/third-party-notices.mjs` from `Cargo.lock` and " +
      "`ui/package.json`, and it lists the dependencies that are **in the shipped binaries** " +
      "— the normal dependencies of `nazar-tray` and `nazar-statusline` resolved for the " +
      "build target, plus the panel's one runtime package and its image assets. Build-time " +
      "and test-only dependencies are not listed, because none of their code is distributed.",
  );
  lines.push("");
  lines.push(
    "Where a crate offers a choice of licences, the one named below is the one it is used " +
      "under here. `scripts/check-licenses.mjs` fails the build on anything that is not " +
      "permissive, and `deny.toml` holds the same policy for `cargo deny`.",
  );
  lines.push("");
  lines.push(`Generated for \`${triple}\`.`);
  lines.push("");
  lines.push("## Summary");
  lines.push("");
  lines.push("| Licence | Packages |");
  lines.push("|---|---|");
  for (const licence of licences) {
    lines.push(`| ${licence} | ${everything.filter((pkg) => pkg.licence === licence).length} |`);
  }
  lines.push(`| **Total** | **${everything.length}** |`);
  lines.push("");

  for (const licence of licences) {
    const group = everything.filter((pkg) => pkg.licence === licence);
    lines.push(`## ${licence}`);
    lines.push("");
    lines.push("| Package | Version | Copyright | Source |");
    lines.push("|---|---|---|---|");
    for (const pkg of group) {
      const holders = pkg.copyrights.length > 0 ? pkg.copyrights.join("<br>") : "—";
      const source = pkg.url ? `[${pkg.url}](${pkg.url})` : "—";
      lines.push(`| ${pkg.name} | ${pkg.version} | ${holders.replace(/\|/g, "\\|")} | ${source} |`);
    }
    lines.push("");

    const representative = representativeFor(licence, group);
    if (representative) {
      lines.push(`### ${licence} — full text`);
      lines.push("");
      lines.push("```");
      lines.push(representative.text);
      lines.push("```");
      lines.push("");
    }
  }

  lines.push("## Panel assets");
  lines.push("");
  lines.push(
    "The Claude Code and Codex marks in the panel are from `@lobehub/icons`, MIT licensed. " +
      "They identify the two products whose numbers the panel shows; they are their owners' " +
      "trade marks and are used for identification only, and no endorsement is implied.",
  );
  lines.push("");
  const lobehub = join(REPO, "ui", "assets", "LICENSE-lobehub.txt");
  if (existsSync(lobehub)) {
    lines.push("```");
    lines.push(readFileSync(lobehub, "utf8").replace(/\r\n/g, "\n").trim());
    lines.push("```");
    lines.push("");
  }

  lines.push("## WebView2");
  lines.push("");
  lines.push(
    "The panel is drawn by the Microsoft Edge WebView2 runtime, which is part of Windows 10 " +
      "1803 and later and of Windows 11. It is **not** redistributed in this installer: the " +
      "package uses Tauri's `downloadBootstrapper` mode, which fetches Microsoft's own " +
      "installer only on a machine that does not already have the runtime. It is licensed by " +
      "Microsoft under its own terms.",
  );
  lines.push("");

  return `${lines.join("\n")}\n`;
}

/**
 * How to recognise each licence's own text, and how long it has to be.
 *
 * Picking by file name does not work: hundreds of crates call the file `LICENSE` and put
 * whichever of their two licences they felt like in it, several concatenate both, and a few
 * put a five-line pointer to a licence rather than the licence. So the text is matched on
 * its own wording and on a plausible length, and the **shortest** match wins — the shortest
 * text that is unmistakably the MIT licence is the MIT licence, while the longest is
 * whatever somebody stapled to it.
 */
const SIGNATURE = {
  MIT: [/Permission is hereby granted, free of charge/, 700],
  "MIT-0": [/Permission is hereby granted, free of charge/, 300],
  "Apache-2.0": [/TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION/, 8000],
  ISC: [/Permission to use, copy, modify/, 300],
  "BSD-2-Clause": [/Redistribution and use in source and binary forms/, 700],
  "BSD-3-Clause": [/Neither the name/, 700],
  Zlib: [/misrepresented as being the original software/, 400],
  "Unicode-3.0": [/UNICODE LICENSE/i, 500],
  "MPL-2.0": [/Mozilla Public License Version 2\.0/, 8000],
  "CDLA-Permissive-2.0": [/Community Data License Agreement/, 1000],
  Unlicense: [/This is free and unencumbered software released into the public domain/, 500],
  "CC0-1.0": [/CREATIVE COMMONS/i, 1000],
  "BSL-1.0": [/Boost Software License/, 500],
  "0BSD": [/Permission to use, copy, modify/, 300],
};

/** The one text printed under a licence heading, or nothing if no file looks like it. */
function representativeFor(licence, group) {
  const files = group.flatMap((pkg) => pkg.texts);
  const rule = SIGNATURE[licence];
  if (!rule) return files.sort((a, b) => b.text.length - a.text.length)[0];
  const [pattern, minimum] = rule;
  return files
    .filter((file) => file.text.length >= minimum && pattern.test(file.text))
    .sort((a, b) => a.text.length - b.text.length)[0];
}

const content = build();

if (process.argv.includes("--check")) {
  const onDisk = existsSync(OUTPUT) ? readFileSync(OUTPUT, "utf8") : "";
  if (onDisk.replace(/\r\n/g, "\n") !== content) {
    process.stderr.write(
      "THIRD-PARTY-NOTICES.md is out of date. Run: node scripts/third-party-notices.mjs\n",
    );
    process.exit(1);
  }
  process.stdout.write("THIRD-PARTY-NOTICES.md is up to date\n");
} else {
  writeFileSync(OUTPUT, content, "utf8");
  process.stdout.write(`wrote ${OUTPUT}\n`);
}
