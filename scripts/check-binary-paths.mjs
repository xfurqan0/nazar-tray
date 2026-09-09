// Fails if a compiled binary carries the path of the machine that built it.
//
// Every `panic!`, every `unwrap()` and every `#[track_caller]` location is compiled in as a
// **string literal** holding the source file it came from — so `[profile.release]
// strip = true` does not touch them, and neither does any grep over the working tree.
// Measured here before this check existed: `nazar-tray.exe` carried **310** copies of
// `C:\Users\<account>\.cargo\registry\src\…` and `nazar-statusline.exe` **7**, inside an
// installer published for anyone to download. That is the maintainer's account name shipped
// to every user, and `docs/RELEASE.md` step 5 could not see it: it reads files, and this
// lives in an artefact no file in the repository contains.
//
// The fix is in `scripts/build-installer.mjs` — `--remap-path-prefix`, passed to every cargo
// invocation the installer build makes. This script is the proof that the fix is still
// working, and `build-installer.mjs` runs it itself before it will hand anybody an
// installer.
//
// Usage:
//   node scripts/check-binary-paths.mjs                     the release tray and wrapper
//   node scripts/check-binary-paths.mjs --profile debug     the debug ones
//   node scripts/check-binary-paths.mjs path\to\some.exe …  whatever you name instead
//
// Exit code 0 means every pattern counted zero. Anything else is a finding, printed with the
// first five in context so that the next question — *which* prefix escaped — is already
// answered.

import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// Honoured for the same reason `build-installer.mjs` honours it: a comparison build in a
// second target directory has to be checkable too.
const TARGET = process.env["CARGO_TARGET_DIR"]
  ? resolve(process.env["CARGO_TARGET_DIR"])
  : join(REPO, "target");

const argv = process.argv.slice(2);
let profile = "release";
const named = [];

for (let at = 0; at < argv.length; at += 1) {
  const argument = argv[at];
  if (argument === "--profile" || argument.startsWith("--profile=")) {
    const value = argument.includes("=") ? argument.slice(argument.indexOf("=") + 1) : argv[++at];
    if (!value || value.startsWith("--")) {
      process.stderr.write("--profile needs a value: release or debug\n");
      process.exit(2);
    }
    profile = value;
    continue;
  }
  if (argument.startsWith("--")) {
    process.stderr.write(`unknown option ${argument}\n`);
    process.exit(2);
  }
  named.push(resolve(argument));
}

const suffix = process.platform === "win32" ? ".exe" : "";
const files =
  named.length > 0
    ? named
    : [
        join(TARGET, profile, `nazar-tray${suffix}`),
        join(TARGET, profile, `nazar-statusline${suffix}`),
      ];

/**
 * What must not be in a binary, and why each one is on the list.
 *
 * Matched case-insensitively, because Windows spells the same directory several ways and a
 * check that only knows one of them is a check that can be walked past.
 *
 * `\Users\` rather than `C:\Users\`: the drive letter is not the interesting part, and a
 * GitHub runner's `C:\Users\runneradmin` has to trip this too. It is not personal data
 * there, but it is the same escape — if the runner's path survives the remap then so would
 * a contributor's, and the pull request is where that should be found.
 *
 * `.rustup` is a tripwire rather than a fix. The standard library's own paths already
 * arrive remapped by the Rust project as `/rustc/<hash>/library/…`, so nothing on this
 * toolchain produces it, and `build-installer.mjs` has no prefix for it. If it ever shows
 * up — a toolchain built from source, `-Zbuild-std` — this goes red and a fourth prefix is
 * the answer.
 */
const PATTERNS = [
  ["\\Users\\", "a Windows user profile"],
  ["/Users/", "a macOS home directory"],
  ["/home/", "a Linux home directory"],
  [".cargo\\registry", "the cargo registry checkout"],
  [".cargo/registry", "the cargo registry checkout"],
  [".rustup", "a rustup toolchain directory"],
  // The absolute path of this checkout, in both spellings, which is what catches a source
  // path from our own crates that escaped the remap. Deliberately not the bare string
  // `nazar-tray\`: the remapped paths of this workspace's own crates read
  // `crates\nazar-tray\src\main.rs`, which is relative, identical on every machine, and
  // exactly what is wanted.
  [REPO, "this checkout"],
  [REPO.split(sep).join("/"), "this checkout"],
].filter(
  // The two spellings of the checkout are the same string on a system whose separator is
  // already `/`, and counting it twice would report double what is there.
  ([needle], at, all) => all.findIndex(([earlier]) => earlier === needle) === at,
);

function human(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

/**
 * The three ways a path can be spelled inside a Windows executable.
 *
 * Rust compiles its source locations as UTF-8, which `latin1` reads back byte for byte, and
 * that is where all 310 of the original findings were. Resource and manifest data is UTF-16,
 * and it is not aligned to any particular offset in the file — hence the second decode from
 * one byte in, which is what makes an odd-offset wide string visible at all.
 */
function views(raw) {
  return [
    { encoding: "text", text: raw.toString("latin1").toLowerCase(), stride: 1, skew: 0 },
    { encoding: "utf-16", text: raw.toString("utf16le").toLowerCase(), stride: 2, skew: 0 },
    {
      encoding: "utf-16",
      text: raw.subarray(1).toString("utf16le").toLowerCase(),
      stride: 2,
      skew: 1,
    },
  ];
}

/** A printable one-line window around a hit, so the finding names its own cause. */
function excerpt(text, at, needleLength) {
  const from = Math.max(0, at - 16);
  const slice = text.slice(from, at + needleLength + 64);
  return slice.replace(/[^\x20-\x7e]+/g, ".");
}

let findings = 0;

for (const file of files) {
  if (!existsSync(file)) {
    process.stderr.write(`${relative(REPO, file)} is not there. Build it first.\n`);
    process.exit(2);
  }

  const raw = readFileSync(file);
  const decoded = views(raw);
  const counts = new Map();
  const examples = [];

  for (const [needle, what] of PATTERNS) {
    const lowered = needle.toLowerCase();
    let total = 0;
    for (const view of decoded) {
      let at = view.text.indexOf(lowered);
      while (at !== -1) {
        total += 1;
        if (examples.length < 5) {
          examples.push({
            offset: at * view.stride + view.skew,
            encoding: view.encoding,
            text: excerpt(view.text, at, lowered.length),
          });
        }
        at = view.text.indexOf(lowered, at + 1);
      }
    }
    if (total > 0) counts.set(`${needle}  (${what})`, total);
  }

  const total = [...counts.values()].reduce((sum, count) => sum + count, 0);
  findings += total;

  const name = relative(REPO, file).padEnd(48);
  const size = human(statSync(file).size).padStart(10);
  // Matches, not strings: one `C:\Users\…\.cargo\registry\…` answers to two of the patterns
  // below, and calling that two leaks would be a number nobody could reconcile with the
  // breakdown printed under it.
  process.stdout.write(`${name} ${size}  ${total} match(es)\n`);

  for (const [label, count] of counts) {
    process.stdout.write(`    ${String(count).padStart(5)}  ${label}\n`);
  }
  for (const example of examples) {
    process.stdout.write(`      @${example.offset} ${example.encoding}: ${example.text}\n`);
  }
}

if (findings > 0) {
  process.stderr.write(
    `\n${findings} match(es) for a build-machine path in the binaries above.\n` +
      "Release binaries are published; these are the build machine's own directories, and\n" +
      "`strip` does not remove them. `scripts/build-installer.mjs` passes\n" +
      "--remap-path-prefix for exactly this — check that it still reaches every cargo call.\n",
  );
  process.exit(1);
}

process.stdout.write("no machine-specific paths in any of them\n");
