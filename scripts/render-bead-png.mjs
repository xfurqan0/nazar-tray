// Rasterise ui/assets/bead.svg to a square PNG.
//
// Why not a library: the SVG is four concentric circles, and the alternative is pulling
// a rasteriser (and its transitive tree) into a project whose whole pitch is that it
// ships one small binary with nothing behind it. This reads the same SVG the panel
// shows, so the icon and the artwork cannot drift.
//
// Usage: node scripts/render-bead-png.mjs [size] [output.png]

import { deflateSync } from "node:zlib";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..");
const SOURCE = resolve(REPO, "ui/assets/bead.svg");

const SUPERSAMPLE = 4;

function parseCircles(svg) {
  const viewBox = /viewBox="0 0 (\d+) (\d+)"/.exec(svg);
  if (!viewBox) throw new Error("bead.svg has no square viewBox starting at the origin");

  const circles = [];
  const pattern = /<circle\s+cx="([\d.]+)"\s+cy="([\d.]+)"\s+r="([\d.]+)"\s+fill="#([0-9A-Fa-f]{6})"\s*\/>/g;
  for (const match of svg.matchAll(pattern)) {
    circles.push({
      cx: Number(match[1]),
      cy: Number(match[2]),
      r: Number(match[3]),
      rgb: [
        parseInt(match[4].slice(0, 2), 16),
        parseInt(match[4].slice(2, 4), 16),
        parseInt(match[4].slice(4, 6), 16),
      ],
    });
  }
  if (circles.length === 0) throw new Error("bead.svg contains no circles");
  return { extent: Number(viewBox[1]), circles };
}

function render(size, { extent, circles }) {
  const scale = extent / size;
  const step = scale / SUPERSAMPLE;
  const offset = step / 2;
  const pixels = Buffer.alloc(size * size * 4);

  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      let r = 0;
      let g = 0;
      let b = 0;
      let covered = 0;

      for (let sy = 0; sy < SUPERSAMPLE; sy += 1) {
        const py = y * scale + sy * step + offset;
        for (let sx = 0; sx < SUPERSAMPLE; sx += 1) {
          const px = x * scale + sx * step + offset;
          // Painter's order: the last circle that contains the sample wins.
          let hit = null;
          for (const circle of circles) {
            const dx = px - circle.cx;
            const dy = py - circle.cy;
            if (dx * dx + dy * dy <= circle.r * circle.r) hit = circle;
          }
          if (hit) {
            r += hit.rgb[0];
            g += hit.rgb[1];
            b += hit.rgb[2];
            covered += 1;
          }
        }
      }

      const at = (y * size + x) * 4;
      const samples = SUPERSAMPLE * SUPERSAMPLE;
      if (covered > 0) {
        pixels[at] = Math.round(r / covered);
        pixels[at + 1] = Math.round(g / covered);
        pixels[at + 2] = Math.round(b / covered);
        pixels[at + 3] = Math.round((covered / samples) * 255);
      }
    }
  }
  return pixels;
}

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
}

function toPng(size, pixels) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // colour type: RGBA
  // bytes 10..12 stay zero: deflate, adaptive filtering, no interlace

  const stride = size * 4;
  const raw = Buffer.alloc((stride + 1) * size);
  for (let y = 0; y < size; y += 1) {
    raw[y * (stride + 1)] = 0; // filter type 0 (none)
    pixels.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const size = Number(process.argv[2] ?? 1024);
const output = resolve(REPO, process.argv[3] ?? "ui/assets/bead-1024.png");
const bead = parseCircles(readFileSync(SOURCE, "utf8"));

mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, toPng(size, render(size, bead)));
process.stdout.write(`rendered ${size}x${size} bead to ${output}\n`);
