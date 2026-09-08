//! The tray bead, drawn at run time on Nazar's sixteen-cell grid.
//!
//! **The icon carries no state.** It is the mark — deep blue rim, white band, a whole yellow
//! iris, black pupil — and it looks the same at 3 % as at 97 %. There is exactly one
//! exception, and it is the last section of this file: when nothing could be read at all the
//! rim goes grey and a hollow square stands where the iris would be.
//!
//! That is a reversal of what this module did for one evening, and the reasons are worth
//! writing down, because they are what a gauge inside a tray icon actually costs.
//!
//! * **The tray is where the icon is recognised. The panel is where the quota is read.**
//!   Nobody reads a percentage off sixteen pixels. The number is in the tooltip on hover and
//!   in the panel on click, to the decimal, in both cases about a second away — so the fill
//!   was never the only route to it, only the least precise one. What the tray icon is for
//!   is being *found*, and a mark is found by being always the same.
//! * **A fill over the whole chamber ate the mark.** Band and iris filling together meant the
//!   white ring was gone past about 70 %: an orange disc with a blue rim, sitting beside
//!   Nazar's bead in the same tray, at exactly the percentage that most needs recognising.
//! * **Confining the fill to the iris did not save it.** With the band held white the mark
//!   survived, but the iris then carried the level alone — eight rows, part yellow and part
//!   white — and at 16 pixels a half-coloured iris does not read as a measurement. It reads
//!   as a bead with a piece missing: two sibling icons in one tray, one of them always whole
//!   and the other apparently drawn wrong. A signal that looks like a rendering fault is
//!   worse than no signal, because the user has to rule the fault out before reading it.
//!
//! Severity and freshness left with the fill. An icon with no level has nothing to colour and
//! nothing to drain, so the orange and red fills, the desaturation and the row arithmetic are
//! all gone from this file. Neither signal was lost: the tooltip names the binding window's
//! percentage and the panel shows every window, its severity colour and its age.
//!
//! Decision K3 of `docs/PROJECT.md` section 8 still holds, on thinner ice than before: the
//! icon is **rendered in Rust for the current scale factor** rather than shipped as PNGs.
//! With the states down to two, a pre-rendered set would be eight small files rather than the
//! few hundred the fill implied, so this is no longer the obvious call — it is kept because
//! the theme hexes then live in one place instead of two, and because the shell asks for a
//! size rather than picking one from a list.
//!
//! ```text
//!      ████████        rim      deep blue, or grey when nothing was read
//!    ██▒▒▒▒▒▒▒▒██      band     white
//!   ██▒▒░░░░░░▒▒██     iris     whole, at every reading
//!   ██▒▒░░██░░▒▒██     pupil    the grid's own 2x2 centre, black, always
//!    ██▒▒░░░░▒▒██
//!      ████████
//! ```
//!
//! **The iris is yellow here, and only here.** Rim `#0E2A5A`, band `#FFFFFF` and pupil
//! `#0A0A0F` are Nazar's, shared on purpose: two programs by the same hand, one mark. The
//! iris is `#F2A93B` rather than Nazar's `#3FA9F5` so that the two beads sitting side by
//! side in one Windows tray are told apart at 16 pixels, where a shape difference would not
//! survive. That hex is borrowed rather than invented: it is Nazar's amber, the colour its
//! bar bead turns past the warning threshold — `modes.dark.warn` in the theme files both
//! repositories carry — so the one difference between the marks still comes out of the
//! family's own palette. `docs/PROJECT.md` has the decision; the Nazar repository's
//! `docs/design/` has the six directions the grid came out of and why this one won.
//!
//! Three rules, and the middle one is the only state there is:
//!
//! 1. **The mark is whole.** Every cell of [`BEAD`] takes its own colour, at every size and
//!    at every reading. The render at 16 pixels is `ui/assets/bead.svg` pixel for pixel, and
//!    [`tests`] parses that file and fails if the two drift.
//! 2. **Unknown is drawn, and drawn as unknown.** Nothing read means a grey rim and a hollow
//!    six-by-six ring where the iris and pupil are — never a confident bead that could be
//!    mistaken for a healthy reading (finding B03).
//! 3. **The colours are the brand's, written once.** The constants below are copied from
//!    `ui/theme.nazar.json`; [`tests`] parses that file and fails if they drift.

use nazar_core::state::SnapshotView;

/// A straight (non-premultiplied) RGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    /// Red channel.
    pub red: u8,
    /// Green channel.
    pub green: u8,
    /// Blue channel.
    pub blue: u8,
}

impl Rgb {
    /// A colour from its three channels.
    #[must_use]
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Rgb { red, green, blue }
    }

    /// The colour as `#RRGGBB`, which is how the theme files spell it.
    ///
    /// Only the drift test needs this — the running tray never turns a colour back into
    /// text — so it is compiled only for the tests rather than shipped unused.
    #[cfg(test)]
    #[must_use]
    pub fn hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.red, self.green, self.blue)
    }
}

/// The bead's rim. `bead.deepBlue`, and Nazar's — the two programs share a mark.
pub const DEEP_BLUE: Rgb = Rgb::new(0x0E, 0x2A, 0x5A);
/// The iris, whole, at every reading. `bead.iris`.
///
/// This is the one hex the two programs do **not** share: Nazar's iris is `#3FA9F5`. A tray
/// holding both beads has 16 pixels to tell them apart with, and colour is the only channel
/// that survives at that size.
///
/// The tone itself is the family's, not this repository's: `#F2A93B` is Nazar's amber, the
/// colour its bar bead turns past the warning threshold — `modes.dark.warn` in the theme
/// files both repositories carry. Sibling applications, one palette, one hex used for two
/// different jobs.
pub const IRIS: Rgb = Rgb::new(0xF2, 0xA9, 0x3B);
/// The band between the rim and the iris. `bead.white`.
pub const WHITE: Rgb = Rgb::new(0xFF, 0xFF, 0xFF);
/// The pupil, the grid's own 2x2 centre. `bead.blackDot`.
pub const BLACK_DOT: Rgb = Rgb::new(0x0A, 0x0A, 0x0F);
/// The rim and the hollow ring when nothing could be read. `modes.light.unknownGrey`.
pub const GREY: Rgb = Rgb::new(0x78, 0x87, 0x9A);

/// The grid's extent, in cells, on both axes. Also the tray's native pixel size.
pub const GRID: usize = 16;

/// The mark, one character a cell: `R` rim, `W` band, `I` iris, `P` pupil, `.` nothing.
///
/// Cell for cell the grid Nazar draws from — `packages/ui/src/bead.ts` there, which is
/// `docs/design/icon-04-pixel-bead.svg` transcribed. This repository keeps its own copy in
/// `ui/assets/bead.svg`, and [`tests`] parses that file and fails if the two disagree; the
/// only difference between the two programs' artwork is which hex the `I` cells take.
const BEAD: [&[u8; GRID]; GRID] = [
    b"......RRRR......",
    b"....RRRRRRRR....",
    b"..RRRRWWWWRRRR..",
    b"..RRWWWWWWWWRR..",
    b".RRWWWWIIWWWWRR.",
    b".RRWWIIIIIIWWRR.",
    b"RRWWWIIIIIIWWWRR",
    b"RRWWIIIPPIIIWWRR",
    b"RRWWIIIPPIIIWWRR",
    b"RRWWWIIIIIIWWWRR",
    b".RRWWIIIIIIWWRR.",
    b".RRWWWWIIWWWWRR.",
    b"..RRWWWWWWWWRR..",
    b"..RRRRWWWWRRRR..",
    b"....RRRRRRRR....",
    b"......RRRR......",
];

/// The hollow square that means "nothing was read": the ring around the box from
/// `(RING_FROM, RING_FROM)` to `(RING_TO, RING_TO)`, one cell thick.
///
/// A square rather than a circle because there is no circle to be had at this size — a
/// ring of cells six across and one thick is what a 16-pixel grid can spell, and pretending
/// otherwise would mean antialiasing, which this module has none of. Six across is even, so
/// the ring is centred on the same seam the pupil is.
const RING_FROM: usize = 5;
/// The far edge of that ring, inclusive.
const RING_TO: usize = 10;

/// Which of the two drawings to rasterise.
///
/// Deliberately not a `SnapshotView` and deliberately not a percentage: the rasteriser is a
/// pure function of this one value, so its tests are one value in and pixels out.
/// [`IconState::from_view`] is the one place that knows how to get it out of a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconState {
    /// Something was read. The mark, whole, whatever the number turned out to be.
    Mark,
    /// Nothing could be read: a grey rim and a hollow ring (finding B03).
    Unknown,
}

impl Default for IconState {
    /// What the tray shows before anything has been read.
    fn default() -> Self {
        IconState::Unknown
    }
}

impl IconState {
    /// The state the icon should be in for a derived snapshot.
    ///
    /// One question: **did any provider produce a binding window with a percentage in it?**
    /// If so the icon is the mark, and which provider, how high the number was and how old
    /// the reading is are all the tooltip's and the panel's business rather than the icon's.
    /// If not, nothing on this machine could be read, and the icon says so.
    #[must_use]
    pub fn from_view(view: &SnapshotView) -> Self {
        let read = view.providers.iter().any(|provider| {
            provider
                .windows
                .iter()
                .filter(|window| window.binding)
                .any(|window| window.percent.is_some_and(f64::is_finite))
        });
        if read {
            IconState::Mark
        } else {
            IconState::Unknown
        }
    }
}

/// A rendered icon: straight RGBA, row-major, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
    /// `width * height * 4` bytes of non-premultiplied RGBA.
    pub rgba: Vec<u8>,
}

impl Bitmap {
    /// A transparent bitmap of the given size.
    #[must_use]
    pub fn blank(width: u32, height: u32) -> Self {
        Bitmap {
            width,
            height,
            rgba: vec![0; (width as usize) * (height as usize) * 4],
        }
    }

    /// The pixel at `(x, y)` as `(red, green, blue, alpha)`.
    ///
    /// Out of bounds is transparent rather than a panic: the callers are tests and the
    /// strip composer, and neither has anything useful to do with a panic.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> (u8, u8, u8, u8) {
        if x >= self.width || y >= self.height {
            return (0, 0, 0, 0);
        }
        let at = ((y as usize) * (self.width as usize) + x as usize) * 4;
        (
            self.rgba[at],
            self.rgba[at + 1],
            self.rgba[at + 2],
            self.rgba[at + 3],
        )
    }

    /// Draw `other` into this bitmap with its top-left corner at `(x, y)`, over-operator.
    pub fn draw(&mut self, other: &Bitmap, x: u32, y: u32) {
        for row in 0..other.height {
            for column in 0..other.width {
                let (red, green, blue, alpha) = other.pixel(column, row);
                if alpha == 0 {
                    continue;
                }
                let (tx, ty) = (x + column, y + row);
                if tx >= self.width || ty >= self.height {
                    continue;
                }
                let at = ((ty as usize) * (self.width as usize) + tx as usize) * 4;
                self.rgba[at] = red;
                self.rgba[at + 1] = green;
                self.rgba[at + 2] = blue;
                self.rgba[at + 3] = alpha;
            }
        }
    }
}

/// The physical size of the tray icon at a scale factor: always a whole multiple of the
/// grid, so a cell is always a whole square block of pixels.
///
/// Windows asks for a 16-pixel icon at 100 %, and `SM_CXSMICON` grows with the display
/// scale: 20 at 125 %, 24 at 150 %, 32 at 200 %. Nazar hands the shell exactly that number
/// and lets four of its sixteen cells come out a pixel wider than the rest.
///
/// This icon takes the other side of that trade, and it kept taking it when the fill was
/// removed — the reason simply changed. It used to be arithmetic: rows of unequal height
/// stopped 50 % from being half. What is left is the drawing. At 20 pixels, four of the
/// sixteen cells are two pixels wide and twelve are one, and the four land wherever the
/// division puts them: a band two cells thick that is three pixels on one side and two on
/// the other, a 2x2 pupil that comes out 3x2. A 16-pixel bead scaled by the shell into a
/// 20-pixel slot is soft; a 20-pixel bead drawn on uneven cells is crooked, and at this size
/// soft beats crooked.
///
/// Capped at 64: past 400 % the taskbar is not asking for an icon any more.
#[must_use]
pub fn size_for_scale(scale: f64) -> u32 {
    let asked = (16.0 * scale).round().clamp(16.0, 256.0) as u32;
    let cells = (asked / GRID as u32).clamp(1, 4);
    cells * GRID as u32
}

/// The colour of one grid cell, or `None` where the mark draws nothing at all.
///
/// The whole of the difference between the two states is here, and it is four lines long.
fn cell_colour(state: IconState, x: usize, y: usize) -> Option<Rgb> {
    let glyph = BEAD[y][x];
    if glyph == b'.' {
        return None;
    }
    if state == IconState::Unknown {
        // No confident bead, ever: a grey rim around an empty chamber with a hollow ring in
        // it. The ring sits entirely on iris and pupil cells, so the band stays white here
        // too, exactly as it is in the mark.
        let on_ring = (RING_FROM..=RING_TO).contains(&x)
            && (RING_FROM..=RING_TO).contains(&y)
            && (x == RING_FROM || x == RING_TO || y == RING_FROM || y == RING_TO);
        return Some(if glyph == b'R' || on_ring {
            GREY
        } else {
            WHITE
        });
    }
    match glyph {
        b'R' => Some(DEEP_BLUE),
        b'W' => Some(WHITE),
        b'I' => Some(IRIS),
        b'P' => Some(BLACK_DOT),
        _ => None,
    }
}

/// Draw the bead.
///
/// `size` is physical pixels. Nearest neighbour and nothing else: the cell a pixel belongs
/// to is `x * 16 / size` in integer arithmetic, so a pixel is inside exactly one cell and
/// takes its colour whole. No supersampling, no blending, no partial alpha — every pixel is
/// opaque or absent, which is the entire point of authoring on the grid.
///
/// [`size_for_scale`] only ever asks for a multiple of sixteen, where every cell is the
/// same square block. Any other size still renders, with cells a pixel wider here and there;
/// the icon-set script leans on that for the Store logos, which are not multiples of anything.
#[must_use]
pub fn render(size: u32, state: IconState) -> Bitmap {
    let mut bitmap = Bitmap::blank(size, size);
    if size == 0 {
        return bitmap;
    }
    let grid = GRID as u32;

    for y in 0..size {
        let row = (y * grid / size) as usize;
        for x in 0..size {
            let Some(colour) = cell_colour(state, (x * grid / size) as usize, row) else {
                continue;
            };
            let at = ((y as usize) * (size as usize) + x as usize) * 4;
            bitmap.rgba[at] = colour.red;
            bitmap.rgba[at + 1] = colour.green;
            bitmap.rgba[at + 2] = colour.blue;
            bitmap.rgba[at + 3] = 0xFF;
        }
    }

    bitmap
}

/// Both states the icon has, and the names their files take in `docs/design/`.
///
/// Kept here rather than in the exporter so a test can assert the pictures in the repository
/// are the states the code can actually be in: documentation nobody checks is documentation
/// that quietly stops matching.
pub const DOCUMENTED_STATES: [(&str, IconState); 2] =
    [("mark", IconState::Mark), ("unknown", IconState::Unknown)];

/// The sizes those pictures are written at, and the strip's three rows: the tray's own unit
/// at 100 %, and what [`size_for_scale`] hands out at 200 % and 400 %.
///
/// Not 125 % or 150 %: both round down to the same 16-pixel bead, so a row for either would
/// be a duplicate. 400 % is where a reader can count the cells instead.
pub const DOCUMENTED_SIZES: [u32; 3] = [16, 32, 64];

/// Compose the icon strip: one column per state, one row per size.
#[must_use]
pub fn strip() -> Bitmap {
    let gap = 6;
    let cell = DOCUMENTED_SIZES.iter().copied().max().unwrap_or(64) + gap;

    let width = cell * u32::try_from(DOCUMENTED_STATES.len()).unwrap_or(1) + gap;
    let height = DOCUMENTED_SIZES.iter().sum::<u32>()
        + gap * (u32::try_from(DOCUMENTED_SIZES.len()).unwrap_or(1) + 1);
    let mut sheet = Bitmap::blank(width, height);

    let mut y = gap;
    for size in DOCUMENTED_SIZES {
        for (column, (_, state)) in DOCUMENTED_STATES.iter().enumerate() {
            let x = gap + cell * u32::try_from(column).unwrap_or(0) + (cell - gap - size) / 2;
            sheet.draw(&render(size, *state), x, y);
        }
        y += size + gap;
    }
    sheet
}

/// A PNG of a bitmap, byte for byte the same every run.
///
/// Written by hand for the same reason the rasteriser is: the alternative is an image crate
/// and its tree in a product that ships one small binary.
///
/// Nothing in the running tray calls this. The tray hands its RGBA straight to the shell;
/// this exists for `--icons`, which produces the pictures in `docs/design`, and for the
/// tests that pin the drawing.
#[must_use]
pub fn encode_png(bitmap: &Bitmap) -> Vec<u8> {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&bitmap.width.to_be_bytes());
    header.extend_from_slice(&bitmap.height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, deflate, no filter, no interlace
    png.extend_from_slice(&chunk(b"IHDR", &header));

    // One filter byte (0: none) in front of every row, which is what the PNG format calls
    // a raw scanline.
    let stride = bitmap.width as usize * 4 + 1;
    let mut raw = Vec::with_capacity(stride * bitmap.height as usize);
    for row in 0..bitmap.height as usize {
        raw.push(0);
        raw.extend_from_slice(&bitmap.rgba[row * (stride - 1)..(row + 1) * (stride - 1)]);
    }
    png.extend_from_slice(&chunk(b"IDAT", &zlib(&raw, stride)));
    png.extend_from_slice(&chunk(b"IEND", &[]));
    png
}

/// Base length, extra bits and symbol for every deflate length code (RFC 1951, 3.2.5).
const LENGTHS: [(u16, u32, u32); 29] = [
    (3, 0, 257),
    (4, 0, 258),
    (5, 0, 259),
    (6, 0, 260),
    (7, 0, 261),
    (8, 0, 262),
    (9, 0, 263),
    (10, 0, 264),
    (11, 1, 265),
    (13, 1, 266),
    (15, 1, 267),
    (17, 1, 268),
    (19, 2, 269),
    (23, 2, 270),
    (27, 2, 271),
    (31, 2, 272),
    (35, 3, 273),
    (43, 3, 274),
    (51, 3, 275),
    (59, 3, 276),
    (67, 4, 277),
    (83, 4, 278),
    (99, 4, 279),
    (115, 4, 280),
    (131, 5, 281),
    (163, 5, 282),
    (195, 5, 283),
    (227, 5, 284),
    (258, 0, 285),
];

/// Base distance and extra bits for every deflate distance code.
const DISTANCES: [(u16, u32); 30] = [
    (1, 0),
    (2, 0),
    (3, 0),
    (4, 0),
    (5, 1),
    (7, 1),
    (9, 2),
    (13, 2),
    (17, 3),
    (25, 3),
    (33, 4),
    (49, 4),
    (65, 5),
    (97, 5),
    (129, 6),
    (193, 6),
    (257, 7),
    (385, 7),
    (513, 8),
    (769, 8),
    (1025, 9),
    (1537, 9),
    (2049, 10),
    (3073, 10),
    (4097, 11),
    (6145, 11),
    (8193, 12),
    (12289, 12),
    (16385, 13),
    (24577, 13),
];

/// Longest match deflate can encode.
const MAX_MATCH: usize = 258;

/// Bits into bytes, the way deflate wants them: least significant bit of the byte first,
/// but Huffman codes written most significant bit first.
struct Bits {
    out: Vec<u8>,
    partial: u8,
    filled: u32,
}

impl Bits {
    fn new() -> Self {
        Bits {
            out: Vec::new(),
            partial: 0,
            filled: 0,
        }
    }

    fn bit(&mut self, bit: u32) {
        self.partial |= ((bit & 1) as u8) << self.filled;
        self.filled += 1;
        if self.filled == 8 {
            self.out.push(self.partial);
            self.partial = 0;
            self.filled = 0;
        }
    }

    /// `count` bits of `value`, least significant first. Block headers and extra bits.
    fn value(&mut self, value: u32, count: u32) {
        for shift in 0..count {
            self.bit(value >> shift);
        }
    }

    /// A Huffman code: `count` bits of `code`, most significant first.
    fn code(&mut self, code: u32, count: u32) {
        for shift in (0..count).rev() {
            self.bit(code >> shift);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.filled > 0 {
            self.out.push(self.partial);
        }
        self.out
    }
}

/// One literal byte, in the fixed Huffman alphabet.
fn literal(bits: &mut Bits, byte: u8) {
    let value = u32::from(byte);
    if value < 144 {
        bits.code(0x30 + value, 8);
    } else {
        bits.code(0x190 + value - 144, 9);
    }
}

/// One length or end-of-block symbol, in the fixed Huffman alphabet.
fn symbol(bits: &mut Bits, symbol: u32) {
    if symbol < 280 {
        bits.code(symbol - 256, 7);
    } else {
        bits.code(0xC0 + symbol - 280, 8);
    }
}

/// A zlib stream: one fixed-Huffman deflate block, with an Adler-32 after it.
///
/// The matcher is three candidate distances rather than a search: **1** (a run of equal
/// bytes, which is what transparency is), **4** (the same pixel repeated, which is what a
/// flat colour is) and **the row stride** (a row identical to the one above, which is what
/// the empty half of a strip is). That is a few lines instead of a hash chain, and on this
/// input — flat colour on transparency — it does the job: the icon strip goes from tens of
/// kilobytes stored to a few. On input it cannot match it degrades to literals, which is
/// exactly what a stored block would have cost.
fn zlib(data: &[u8], stride: usize) -> Vec<u8> {
    // 0x78 0x01: deflate, 32 KiB window, no preset dictionary. (0x7801 % 31 == 0.)
    let mut out = vec![0x78, 0x01];

    let mut bits = Bits::new();
    bits.value(1, 1); // final block
    bits.value(1, 2); // fixed Huffman codes

    let candidates = [1usize, 4, stride];
    let mut at = 0;
    while at < data.len() {
        let (length, distance) = candidates
            .iter()
            .filter(|distance| **distance <= at)
            .map(|distance| (match_length(data, at, *distance), *distance))
            .max_by_key(|(length, _)| *length)
            .unwrap_or((0, 0));

        if length >= 3 {
            let (base, extra, code) = *LENGTHS
                .iter()
                .rev()
                .find(|(base, _, _)| usize::from(*base) <= length)
                .unwrap_or(&LENGTHS[0]);
            symbol(&mut bits, code);
            bits.value((length - usize::from(base)) as u32, extra);

            let (distance_code, (distance_base, distance_extra)) = DISTANCES
                .iter()
                .enumerate()
                .rev()
                .find(|(_, (base, _))| usize::from(*base) <= distance)
                .map_or((0, (1u16, 0u32)), |(index, entry)| (index, *entry));
            bits.code(distance_code as u32, 5);
            bits.value(
                (distance - usize::from(distance_base)) as u32,
                distance_extra,
            );

            at += length;
        } else {
            literal(&mut bits, data[at]);
            at += 1;
        }
    }
    symbol(&mut bits, 256); // end of block

    out.extend_from_slice(&bits.finish());
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// How many bytes at `at` repeat the bytes `distance` earlier, capped at [`MAX_MATCH`].
fn match_length(data: &[u8], at: usize, distance: usize) -> usize {
    if distance == 0 || distance > at {
        return 0;
    }
    let limit = MAX_MATCH.min(data.len() - at);
    (0..limit)
        .take_while(|offset| data[at + offset] == data[at - distance + offset])
        .count()
}

/// A PNG chunk: length, type, payload, CRC.
fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 12);
    out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[4..]);
    out.extend_from_slice(&crc.to_be_bytes());
    out
}

/// CRC-32 as PNG defines it.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                0xEDB8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xFFFF_FFFF
}

/// Adler-32, the checksum a zlib stream ends with.
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Write the icon strip and both states into a directory.
/// `--icons docs/design` produces everything in that folder: seven files.
///
/// The strip is the picture for a README — both states at all three sizes, one row per
/// size — and beside it each state is written out on its own, so a maintainer can look at
/// one bead large enough to count its cells.
///
/// # Errors
/// Whatever the file system says: a directory that cannot be created, a file that cannot
/// be written.
pub fn export(directory: &std::path::Path) -> std::io::Result<Vec<std::path::PathBuf>> {
    std::fs::create_dir_all(directory)?;

    let mut written = Vec::new();
    let path = directory.join("bead-states.png");
    std::fs::write(&path, encode_png(&strip()))?;
    written.push(path);

    for (name, state) in DOCUMENTED_STATES {
        for size in DOCUMENTED_SIZES {
            let path = directory.join(format!("bead-{name}-{size}.png"));
            std::fs::write(&path, encode_png(&render(size, state)))?;
            written.push(path);
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests;
