// The icon strip in docs/design, checked by a decoder nobody in this repository wrote.
//
// `crates/nazar-tray/src/icon.rs` encodes PNG by hand — a fixed-Huffman deflate stream in
// about eighty lines, because an image crate is a dependency tree in a product that ships
// one small binary. Its own tests pin the bytes, which proves the encoder is *stable*; they
// cannot prove it is *correct*, since they compare its output with itself.
//
// Node's zlib can. Inflating the strip here is an independent implementation saying the
// stream is well formed and the right length, and it is why the pinned checksums over there
// are worth anything.

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { inflateSync } from "node:zlib";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const STRIP = resolve(REPO, "docs/design/bead-states.png");

/** Split a PNG into its chunks. */
function chunks(bytes) {
  assert.deepEqual(
    [...bytes.subarray(0, 8)],
    [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a],
    "not a PNG",
  );
  const out = [];
  let at = 8;
  while (at + 8 <= bytes.length) {
    const length = bytes.readUInt32BE(at);
    out.push({ type: bytes.toString("ascii", at + 4, at + 8), data: bytes.subarray(at + 8, at + 8 + length) });
    at += 12 + length;
  }
  return out;
}

test("the icon strip is a PNG that a real decoder can read", () => {
  assert.ok(
    existsSync(STRIP),
    "docs/design/bead-states.png is missing; regenerate it with `nazar-tray --icons docs/design`",
  );
  const parts = chunks(readFileSync(STRIP));

  const header = parts.find((part) => part.type === "IHDR");
  assert.ok(header, "no IHDR");
  const width = header.data.readUInt32BE(0);
  const height = header.data.readUInt32BE(4);
  assert.equal(header.data[8], 8, "8 bits per channel");
  assert.equal(header.data[9], 6, "RGBA");
  assert.equal(header.data[10], 0, "deflate");
  assert.equal(header.data[12], 0, "not interlaced");

  const idat = Buffer.concat(parts.filter((part) => part.type === "IDAT").map((part) => part.data));
  const raw = inflateSync(idat);
  assert.equal(
    raw.length,
    (width * 4 + 1) * height,
    "the decompressed image is not one filter byte plus one row of RGBA per row",
  );

  assert.equal(parts.at(-1)?.type, "IEND", "a PNG ends with IEND");
});

test("the strip has the shape the rasteriser says it does", () => {
  const rust = readFileSync(resolve(REPO, "crates/nazar-tray/src/icon.rs"), "utf8");
  // The names of the states, from the body of `DOCUMENTED_STATES`. Matched on the quoted
  // names rather than on the shape of the literal, because rustfmt wraps it one way at one
  // length and another way at another, and a test that breaks when the formatter runs is a
  // test nobody trusts.
  const body = /pub const DOCUMENTED_STATES[\s\S]*?\];/.exec(rust);
  assert.ok(body, "icon.rs no longer has DOCUMENTED_STATES");
  const states = [...body[0].matchAll(/"([a-z0-9-]+)"/g)].map((match) => match[1]);
  // Two of them, since 2026-09-09: the icon carries no state beyond "was anything read".
  assert.deepEqual(states, ["mark", "unknown"], `unexpected states ${states.join(", ")}`);

  const header = chunks(readFileSync(STRIP)).find((part) => part.type === "IHDR");
  const width = header.data.readUInt32BE(0);
  const height = header.data.readUInt32BE(4);
  // Two columns of 64 + 6, plus a margin; three rows of 16, 32 and 64 plus four gaps.
  // The sizes are the ones size_for_scale hands out at 100 %, 200 % and 400 %, and they are
  // whole multiples of the sixteen-cell grid because nothing else is allowed to be.
  assert.equal(width, states.length * 70 + 6, `unexpected width ${width}`);
  assert.equal(height, 16 + 32 + 64 + 6 * 4, `unexpected height ${height}`);
});
