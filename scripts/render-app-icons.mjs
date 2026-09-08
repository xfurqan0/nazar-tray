// The application icon set, rendered from ui/assets/bead.svg exactly.
//
// **Why this is not `cargo tauri icon`.** It was, until the bead moved on to the pixel
// grid. The Tauri CLI takes one large PNG and resamples it down with a smooth filter, which
// is the right thing for a vector mark and the wrong thing for this one: a smooth
// downscale of 8-bit art gives every cell a soft halo and hundreds of partly transparent
// pixels, which is worse than either the art or the circles it replaced. The mark is
// authored on a 16-cell grid precisely so that no resampler is ever involved, so every size
// is rendered from the grid and packed into the two containers Windows and macOS want.
//
// Same output layout as the CLI produced — same file names, same PNG-in-ICO entries
// (16, 24, 32, 48, 64, 256), same ICNS types — so `tauri.conf.json`, the NSIS installer and
// the MSIX logo list needed no change.
//
// The nine Store logos are the awkward ones: 30, 44, 71, 89, 107, 142, 150, 284 and 310 are
// none of them multiples of 16. `fitToBox` draws the bead at the largest whole cell size
// that fits and leaves the rest transparent, centred. Padding, never blur.
//
// Zero dependencies, no network, no browser.
//
// Usage: node scripts/render-app-icons.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { GRID, beadGrid, fitToBox, renderBead, toPng } from "./render-bead-png.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..");
const ICONS = resolve(REPO, "crates/nazar-tray/icons");

/** The loose PNGs `tauri.conf.json` names, plus the one Tauri uses as a window icon. */
const LOOSE = [
  ["32x32.png", 32],
  ["64x64.png", 64],
  ["128x128.png", 128],
  ["128x128@2x.png", 256],
  ["icon.png", 512],
];

/** The MSIX logos. Every one of these boxes gets a centred bead and transparent padding. */
const LOGOS = [
  ["Square30x30Logo.png", 30],
  ["Square44x44Logo.png", 44],
  ["Square71x71Logo.png", 71],
  ["Square89x89Logo.png", 89],
  ["Square107x107Logo.png", 107],
  ["Square142x142Logo.png", 142],
  ["Square150x150Logo.png", 150],
  ["Square284x284Logo.png", 284],
  ["Square310x310Logo.png", 310],
  ["StoreLogo.png", 50],
];

/** The sizes inside `icon.ico`. Exactly what the Tauri CLI used to put there. */
const ICO_SIZES = [16, 24, 32, 48, 64, 256];

/**
 * The sizes inside `icon.icns`, by four-character type.
 *
 * Only the PNG-carrying types, which is everything macOS 10.7 and later reads. The CLI also
 * wrote `is32`/`il32`/`s8mk`/`l8mk` — RLE-packed 24-bit art with a separate mask, for
 * Mac OS X 10.0 — and nothing this project ships has ever run there.
 */
const ICNS_TYPES = [
  ["ic11", 32],
  ["ic12", 64],
  ["ic07", 128],
  ["ic13", 256],
  ["ic08", 256],
  ["ic14", 512],
  ["ic09", 512],
  ["ic10", 1024],
];

/** One PNG of the mark at `size`, padded if `size` is not a whole number of cells. */
function beadPng(size, cells) {
  const pixels = size % GRID === 0 ? renderBead(size, cells) : fitToBox(size, cells);
  return toPng(size, size, pixels);
}

/**
 * Pack PNGs into an `.ico`.
 *
 * `ICONDIR` then one 16-byte `ICONDIRENTRY` per image then the images. A dimension of 256
 * is written as the byte 0, which is how the format says "256" in one byte. The entries
 * carry PNG rather than a DIB at every size, which is what the Tauri CLI wrote here before
 * and what the NSIS installer reads.
 */
function toIco(images) {
  const directory = Buffer.alloc(6 + images.length * 16);
  directory.writeUInt16LE(0, 0); // reserved
  directory.writeUInt16LE(1, 2); // type: icon
  directory.writeUInt16LE(images.length, 4);

  let offset = directory.length;
  images.forEach(({ size, png }, index) => {
    const at = 6 + index * 16;
    directory[at] = size >= 256 ? 0 : size;
    directory[at + 1] = size >= 256 ? 0 : size;
    directory[at + 2] = 0; // palette entries: none
    directory[at + 3] = 0; // reserved
    directory.writeUInt16LE(1, at + 4); // colour planes
    directory.writeUInt16LE(32, at + 6); // bits per pixel
    directory.writeUInt32LE(png.length, at + 8);
    directory.writeUInt32LE(offset, at + 12);
    offset += png.length;
  });

  return Buffer.concat([directory, ...images.map((image) => image.png)]);
}

/** Pack PNGs into an `.icns`: the magic, the total length, then type/length/data runs. */
function toIcns(entries) {
  const blocks = entries.map(({ type, png }) => {
    const header = Buffer.alloc(8);
    header.write(type, 0, 4, "ascii");
    header.writeUInt32BE(png.length + 8, 4);
    return Buffer.concat([header, png]);
  });
  const total = 8 + blocks.reduce((sum, block) => sum + block.length, 0);
  const header = Buffer.alloc(8);
  header.write("icns", 0, 4, "ascii");
  header.writeUInt32BE(total, 4);
  return Buffer.concat([header, ...blocks]);
}

const cells = beadGrid();
mkdirSync(ICONS, { recursive: true });

const written = [];
for (const [name, size] of [...LOOSE, ...LOGOS]) {
  writeFileSync(resolve(ICONS, name), beadPng(size, cells));
  written.push(`${name} (${size})`);
}

writeFileSync(
  resolve(ICONS, "icon.ico"),
  toIco(ICO_SIZES.map((size) => ({ size, png: beadPng(size, cells) }))),
);
writeFileSync(
  resolve(ICONS, "icon.icns"),
  toIcns(ICNS_TYPES.map(([type, size]) => ({ type, png: beadPng(size, cells) }))),
);

process.stdout.write(
  `icons: ${written.join(", ")}, icon.ico (${ICO_SIZES.join("/")}), ` +
    `icon.icns (${ICNS_TYPES.length} entries)\n`,
);
