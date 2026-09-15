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
//     `nazar-statusline`, resolved for **each of the three targets in `TARGETS`**. Build
//     dependencies are excluded because none of their code is in the binaries; dev
//     dependencies are excluded for the same reason. Optional dependencies that the default
//     features do not turn on are already absent from a `--filter-platform` resolve.
//   * The panel's **runtime** npm dependencies — `@tauri-apps/api`, and nothing else. The
//     two dev dependencies build the bundle and are not in it.
//   * The panel's **image assets**, whose licence is a file rather than a manifest field.
//
// **Three targets, one file (T-WP-L5).** It used to resolve for the host triple, which was
// fine while the only thing that shipped was built on Windows. It is not fine now: a Linux
// build pulls in the gtk, webkit, dbus and zbus half of the tree that Windows never sees,
// so a notices file resolved on one of them is wrong for the other, and `--check` could only
// ever be a gate on the job that happened to generate it. Resolving all three and saying
// which is which makes one file true everywhere and makes the check a gate on every job.
//
// **The output is byte-for-byte the same on every machine, and that is a requirement rather
// than a nicety** — it is the whole reason two CI jobs can check one file. Nothing here may
// read the host: not the triple, not the locale, not a directory's listing order. Every sort
// is by code point and every tie is broken by something written down.
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

/**
 * The targets a shipped binary is resolved for, and what each section is called.
 *
 * Windows is what ships today, Linux is what T-WP-L0 put in CI and T-WP-L3 packaged, and
 * macOS is the roadmap build — the same three `deny.toml` has been checking since WP0, for
 * the same reason: the licence question is answered before the build that asks it arrives.
 *
 * `aarch64-apple-darwin` rather than the Intel one, because it is the Mac that gets built
 * first and because the two resolve the same set anyway — the split in that tree is by
 * operating system, not by word size.
 *
 * The order is the order of the sections in the file, and it is the order things shipped in.
 */
const TARGETS = [
  { triple: "x86_64-pc-windows-msvc", section: "Windows" },
  { triple: "x86_64-unknown-linux-gnu", section: "Linux" },
  { triple: "aarch64-apple-darwin", section: "macOS" },
];

/** What a package is called when the three resolves are compared. */
const identity = (pkg) => `${pkg.name}@${pkg.version}`;

/**
 * Ordering that does not depend on the machine that runs this.
 *
 * `localeCompare` was here, and it is exactly the kind of thing that makes one file into
 * two: it collates by the host's rules, which decide whether `windows-sys` sorts before or
 * after `windows_x86_64_msvc` depending on how much a runtime cares about hyphens. Code
 * points are the same everywhere.
 */
const byCodePoint = (left, right) => (left < right ? -1 : left > right ? 1 : 0);

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
    // Longest first -- the longest text is the licence and the rest is whatever somebody
    // stapled to it -- and by name where two are the same length, because a directory
    // listing is in whatever order the filesystem felt like.
    .sort((a, b) => b.text.length - a.text.length || byCodePoint(a.name, b.name));
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
    .sort((a, b) => byCodePoint(a.name, b.name) || byCodePoint(a.version, b.version));
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

/** Every shipped crate for one target, with its licence, copyrights and texts. */
function cratesFor(triple) {
  const metadata = JSON.parse(
    execFileSync(
      "cargo",
      ["metadata", "--format-version", "1", "--locked", "--filter-platform", triple],
      { cwd: REPO, encoding: "utf8", maxBuffer: 128 * 1024 * 1024 },
    ),
  );
  return shippedCrates(metadata).map((pkg) => {
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
}

/** One package's row in a table. */
function row(pkg) {
  const holders = pkg.copyrights.length > 0 ? pkg.copyrights.join("<br>") : "—";
  const source = pkg.url ? `[${pkg.url}](${pkg.url})` : "—";
  return `| ${pkg.name} | ${pkg.version} | ${holders.replace(/\|/g, "\\|")} | ${source} |`;
}

/** A section's packages, grouped by the licence each is used under. */
function tables(lines, packages, depth) {
  const heading = "#".repeat(depth);
  for (const licence of [...new Set(packages.map((pkg) => pkg.licence))].sort(byCodePoint)) {
    lines.push(`${heading} ${licence}`);
    lines.push("");
    lines.push("| Package | Version | Copyright | Source |");
    lines.push("|---|---|---|---|");
    for (const pkg of packages.filter((pkg) => pkg.licence === licence)) {
      lines.push(row(pkg));
    }
    lines.push("");
  }
}

function build() {
  // One resolve per target. The three are compared by name and version, so a crate every
  // one of them pulls in is written down once and the rest are written down where they
  // come from -- which is also the answer to "why is gtk-sys in a Windows installer's
  // notices", asked and answered before anybody has to ask it.
  const perTarget = new Map(TARGETS.map(({ triple }) => [triple, cratesFor(triple)]));

  const npm = panelDependencies().map((pkg) => ({
    ...pkg,
    licence: chosen(pkg.license),
    url: homepage(pkg),
    copyrights: copyrights(pkg.texts),
  }));

  // The panel's own package is drawn by every target, so it belongs with the crates that
  // are: it is JavaScript, and the webview that runs it is the only platform it has.
  const common = [];
  const only = new Map(TARGETS.map(({ triple }) => [triple, []]));
  const everywhere = [...perTarget.values()]
    .map((list) => new Set(list.map(identity)))
    .reduce((left, right) => new Set([...left].filter((id) => right.has(id))));
  const claimed = new Set();
  for (const [triple, list] of perTarget) {
    for (const pkg of list) {
      if (!everywhere.has(identity(pkg))) {
        only.get(triple).push(pkg);
      } else if (!claimed.has(identity(pkg))) {
        claimed.add(identity(pkg));
        common.push(pkg);
      }
    }
  }
  common.push(...npm);
  common.sort((a, b) => byCodePoint(a.name, b.name) || byCodePoint(a.version, b.version));

  const sections = [
    { title: "All platforms", packages: common },
    ...TARGETS.map(({ triple, section }) => ({ title: section, packages: only.get(triple) })),
  ];
  const everything = sections.flatMap((section) => section.packages);
  const licences = [...new Set(everything.map((pkg) => pkg.licence))].sort(byCodePoint);

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
      "— the normal dependencies of `nazar-tray` and `nazar-statusline`, plus the panel's one " +
      "runtime package and its image assets. Build-time and test-only dependencies are not " +
      "listed, because none of their code is distributed.",
  );
  lines.push("");
  lines.push(
    "Where a crate offers a choice of licences, the one named below is the one it is used " +
      "under here. `scripts/check-licenses.mjs` fails the build on anything that is not " +
      "permissive, and `deny.toml` holds the same policy for `cargo deny`.",
  );
  lines.push("");
  lines.push(
    "**Resolved for " +
      TARGETS.map(({ triple }) => `\`${triple}\``).join(", ").replace(/, (?=[^,]*$)/, " and ") +
      ".** A package all three pull in is listed once under **All platforms**; the rest are " +
      "listed under the platform that brings them in, so a reader can see what a Linux " +
      "package contains that a Windows one does not. The file is byte-for-byte identical " +
      "whichever of the three a machine generates it on, which is what lets " +
      "`--check` gate more than one CI job.",
  );
  lines.push("");
  lines.push("## Summary");
  lines.push("");
  lines.push("| Section | Packages |");
  lines.push("|---|---|");
  for (const section of sections) {
    lines.push(`| ${section.title} | ${section.packages.length} |`);
  }
  lines.push(`| **Total** | **${everything.length}** |`);
  lines.push("");
  lines.push("| Licence | Packages |");
  lines.push("|---|---|");
  for (const licence of licences) {
    lines.push(`| ${licence} | ${everything.filter((pkg) => pkg.licence === licence).length} |`);
  }
  lines.push(`| **Total** | **${everything.length}** |`);
  lines.push("");

  for (const section of sections) {
    lines.push(`## ${section.title}`);
    lines.push("");
    if (section.packages.length === 0) {
      lines.push(`Nothing this target pulls in is absent from the other two.`);
      lines.push("");
      continue;
    }
    if (section.title !== "All platforms") {
      lines.push(`Only what this target adds to **All platforms** above.`);
      lines.push("");
    }
    tables(lines, section.packages, 3);
  }

  // The licence bodies, once each rather than once per section. They are the longest part
  // of this file and they do not vary by platform; repeating them four times would make a
  // 300 KB document out of an 85 KB one and would say nothing new in any of the copies.
  lines.push("## Licence texts");
  lines.push("");
  lines.push(
    "One copy of each licence named above. The copyright lines are in the tables, because " +
      "those are the part that differs by package and the part a permissive licence actually " +
      "asks to be carried.",
  );
  lines.push("");
  for (const licence of licences) {
    const representative = representativeFor(
      licence,
      everything.filter((pkg) => pkg.licence === licence),
    );
    if (!representative) continue;
    lines.push(`### ${licence}`);
    lines.push("");
    lines.push("```");
    lines.push(representative.text);
    lines.push("```");
    lines.push("");
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
      "Microsoft under its own terms. On Linux the panel is drawn by the system's WebKitGTK, " +
      "which the `.deb` and the `.rpm` depend on and neither redistributes.",
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
