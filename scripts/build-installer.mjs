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
//   4. `cargo tauri build`, which builds the tray and produces the installer;
//   5. `scripts/check-binary-paths.mjs`, which reads the two binaries back and deletes the
//      package rather than hand anybody an installer built from binaries that carry the
//      path of the machine that built them.
//
// Then it prints what came out, with sizes and SHA-256, because those are the numbers the
// release checklist asks for and computing them by hand is how they end up wrong.
//
// Usage:
//   node scripts/build-installer.mjs                 release build, NSIS installer
//   node scripts/build-installer.mjs --debug         the same, unoptimised, for a quick check
//   node scripts/build-installer.mjs --bundles nsis,msi
//   node scripts/build-installer.mjs --bundles deb,rpm       on Linux
//   node scripts/build-installer.mjs --skip-panel
//
// On Linux `--bundles` is not optional: `bundle.targets` names `nsis`, which no Linux
// machine can produce, so the target has to come from the command line. That is the safer
// half of the trade -- `"all"` would make a Windows release produce the MSI that WP7
// deliberately does not ship.
//
// Nothing here signs, tags, uploads or installs anything. `docs/RELEASE.md` is the checklist
// that does the rest, by hand.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, rmSync, statSync } from "node:fs";
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

/** Run a command, let its output through, and return its exit code. */
function attempt(command, args, cwd, env) {
  process.stdout.write(`\n> ${command} ${args.join(" ")}\n`);
  const finished = spawnSync(command, args, {
    cwd,
    env,
    stdio: "inherit",
    shell: process.platform === "win32",
  });
  return finished.status ?? 1;
}

/** The same, but the script stops here if it fails. */
function run(command, args, cwd, env) {
  const status = attempt(command, args, cwd, env);
  if (status !== 0) {
    process.stderr.write(`\n${command} failed with exit code ${status}\n`);
    process.exit(status);
  }
}

/**
 * The environment every cargo invocation below is given: the same one, with the build
 * machine's own directories mapped to names that mean the same thing everywhere.
 *
 * Panic locations and `#[track_caller]` sites are compiled in as **string literals** naming
 * the source file they came from, so `[profile.release] strip = true` leaves them alone.
 * Before this existed, the published `nazar-tray.exe` carried 310 copies of
 * `C:\Users\<account>\.cargo\registry\src\…` and the wrapper 7 — the maintainer's account
 * name inside an installer anyone can download, invisible to every grep in
 * `docs/RELEASE.md` step 5 because it reads the working tree and this is only in the
 * artefact. `scripts/check-binary-paths.mjs` is the check that it is gone.
 *
 * `[profile.release] trim-paths = "all"` would be the tidy way and is not available:
 * still nightly-only on the toolchain `rust-toolchain.toml` pins. `--remap-path-prefix`
 * does the same job on stable, and the prefixes have to be computed here rather than
 * written into a config file because they are different on every machine.
 *
 * Three prefixes, which is every one that shows up in practice on this toolchain. The
 * standard library arrives already remapped by the Rust project as `/rustc/<hash>/…`, and
 * nothing here builds it from source; `.rustup` is a pattern in the checker rather than a
 * prefix here, so that a toolchain which ever did would go red instead of quietly shipping.
 *
 * The registry maps to `cargo` and not to `crates`, which is the one place this parts
 * company with nazar's `scripts/build-desktop.mjs`: this workspace keeps its own code in
 * `crates/`, so a dependency panicking from `crates\serde_json-1.0.x\src\…` would read like
 * one of ours.
 *
 * **`CFLAGS` is the same rule for the half of the tree that is not Rust**, and it is not
 * hypothetical either: on Linux, `--remap-path-prefix` left 214 copies of
 * `/home/<account>/.cargo/registry/.../ring` in the tray binary, because `ring` builds its
 * assembly and C through `cc` and the compiler writes its own debug info, which a rustc
 * flag never sees. Windows never showed it - the paths go to a `.pdb` there rather than
 * into the image. `-ffile-prefix-map` is the C compiler's word for the same thing, and the
 * caller's `CFLAGS` are kept in front of it as `RUSTFLAGS` are above.
 *
 * **Every profile, not only release.** nazar applies its remap to release builds alone, to
 * keep `cargo test` and `cargo clippy` on one fingerprint; here the debug installer is a
 * downloadable CI artefact and, more to the point, the debug bundle job is where CI gets to
 * prove on every pull request that the remap still works. A rule that holds for one profile
 * is a rule nobody can check until release day.
 */
function remappedEnvironment() {
  const home =
    process.platform === "win32"
      ? (process.env["USERPROFILE"] ?? process.env["HOME"] ?? "")
      : (process.env["HOME"] ?? "");
  const cargoHome = process.env["CARGO_HOME"]
    ? resolve(process.env["CARGO_HOME"])
    : join(home, ".cargo");

  const remaps = [
    `--remap-path-prefix=${join(cargoHome, "registry", "src")}=cargo`,
    `--remap-path-prefix=${join(cargoHome, "git", "checkouts")}=git`,
    `--remap-path-prefix=${REPO}=nazar-tray`,
  ];

  // Cargo reads `CARGO_ENCODED_RUSTFLAGS` in preference to `RUSTFLAGS` and ignores the
  // second one entirely when the first is set, so whatever the caller asked for is carried
  // across rather than replaced. (Both of them also mask `build.rustflags` from a config
  // file; this repository has none, and a machine that does gets told by cargo's own
  // precedence rules rather than by anything here.)
  const inherited = process.env["CARGO_ENCODED_RUSTFLAGS"]
    ? process.env["CARGO_ENCODED_RUSTFLAGS"].split("\u001f")
    : (process.env["RUSTFLAGS"] ?? "").split(/\s+/);

  // The same two prefixes for anything `cc` compiles. Windows is left alone: MSVC has no
  // such flag, its debug information goes to a `.pdb` beside the binary rather than into
  // it, and the checker measures zero there.
  const compilerFlags =
    process.platform === "win32"
      ? undefined
      : [
          process.env["CFLAGS"] ?? "",
          `-ffile-prefix-map=${join(cargoHome, "registry", "src")}=cargo`,
          `-ffile-prefix-map=${REPO}=nazar-tray`,
        ]
          .filter(Boolean)
          .join(" ");

  // `CARGO_ENCODED_RUSTFLAGS` and not `RUSTFLAGS`: the unencoded variable is split on
  // whitespace, and a Windows home directory with a space in it would break every flag
  // above in a way that looks like a compiler bug.
  return {
    ...process.env,
    CARGO_ENCODED_RUSTFLAGS: [...inherited.filter(Boolean), ...remaps].join("\u001f"),
    ...(compilerFlags ? { CFLAGS: compilerFlags } : {}),
    // Read by `tauri-cli` on Linux and ignored everywhere else. See
    // `requireAyatanaHeaders` below for what it decides and why it is not left to chance.
    ...(process.platform === "linux" ? { TAURI_LINUX_AYATANA_APPINDICATOR: "true" } : {}),
  };
}

const cargoEnvironment = remappedEnvironment();

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

// 0. On Linux, the one thing about the packages that is decided by the machine building
//    them rather than by this repository.
//
// `tauri-cli` writes the tray's dependency on the appindicator library from whichever
// development package it can see through pkg-config. With `ayatana-appindicator3-0.1` the
// `.deb` asks for `libayatana-appindicator3-1` and the `.rpm` for
// `libayatana-appindicator3.so.1`; without it, both silently fall back to the 2018-era
// `libappindicator3` names. The Debian one is the damaging half: that package was removed
// in bookworm and in Ubuntu 24.04, so the `.deb` **cannot be installed** on any current
// Debian or Ubuntu, and nothing about the build says so. It succeeds, it produces a file of
// the right size, and the failure is the user's.
//
// Two lines answer it, and they belong together. The environment variable settles the
// question for every builder rather than leaving it to what happens to be installed -- the
// package says the same thing from a release runner, a maintainer's laptop and a machine
// carrying both libraries -- and this check turns the missing headers into a sentence
// naming the package to install, instead of the panic from inside `tauri-cli` that the
// variable would otherwise produce.
//
// Ayatana rather than the original is the product decision behind both: it is the
// maintained successor, it is what current distributions ship, and it is already the first
// name `libappindicator-sys` tries to dlopen. Nothing at run time depends on this choice --
// the tray loads whichever library is on the machine -- so it is a packaging requirement
// and not a build one.
function requireAyatanaHeaders() {
  if (process.platform !== "linux" || !bundles) return;
  if (spawnSync("pkg-config", ["--exists", "ayatana-appindicator3-0.1"]).status === 0) return;
  process.stderr.write(
    [
      "",
      "pkg-config cannot see ayatana-appindicator3-0.1. Packages built without it ask for",
      "libappindicator3-1, which no longer exists in Debian 12 or Ubuntu 24.04 -- a",
      ".deb that cannot be installed on any current Debian or Ubuntu.",
      "",
      "  Debian / Ubuntu   sudo apt install libayatana-appindicator3-dev",
      "  Fedora            sudo dnf install libayatana-appindicator-gtk3-devel",
      "",
      "Nothing at run time needs it: the tray dlopens whichever library is present. This is",
      "a requirement of the build machine, for the packages alone.",
      "",
    ].join("\n"),
  );
  process.exit(1);
}
requireAyatanaHeaders();

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

// 3. The wrapper, and the copy the bundler looks for. `sidecar.mjs` runs cargo itself, and
//    gets the same environment through the one it inherits from here — the wrapper ships in
//    the same installer and is held to the same rule.
run(
  "node",
  [join("scripts", "sidecar.mjs"), ...(debug ? ["--debug"] : [])],
  REPO,
  cargoEnvironment,
);

// 4. The tray and the installer.
run(
  "cargo",
  ["tauri", "build", ...(debug ? ["--debug"] : []), ...(bundles ? ["--bundles", bundles] : [])],
  TAURI_DIR,
  cargoEnvironment,
);

// 5. The binaries, read back. A package built from binaries that name the build machine is
//    not a package to hand anybody, so the bundle goes in the bin rather than sitting on
//    disk looking finished — the next person to walk past this directory has no way of
//    knowing the check failed, and `docs/RELEASE.md` step 2 says to keep the output.
const checked = attempt(
  "node",
  [join("scripts", "check-binary-paths.mjs"), "--profile", profile],
  REPO,
);
if (checked !== 0) {
  const bundle = join(TARGET, profile, "bundle");
  if (existsSync(bundle)) {
    rmSync(bundle, { recursive: true, force: true });
    process.stderr.write(`removed ${relative(REPO, bundle)}\n`);
  }
  process.exit(checked);
}

// What came out.
const artefacts = [
  join(TARGET, profile, `nazar-tray${suffix}`),
  join(TARGET, profile, `nazar-statusline${suffix}`),
  join(TARGET, profile, "bundle", "nsis"),
  join(TARGET, profile, "bundle", "msi"),
  join(TARGET, profile, "bundle", "deb"),
  join(TARGET, profile, "bundle", "rpm"),
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
