//! The tray bead, drawn at run time.
//!
//! Decision K3 of `docs/PROJECT.md` section 8, closed here: the icon is **rendered in
//! Rust for the current scale factor**, not shipped as a set of pre-rendered PNGs. A
//! pre-rendered set would need one file per fill level per severity per freshness per
//! scale — a few hundred images that a theme change invalidates all at once — and the
//! theme hexes would then live in two places. Drawing it costs the sixty lines below and
//! keeps `ui/theme.nazar.json` the only place a colour is written down.
//!
//! `tiny-skia` was the leaning in the plan and is not used: the whole picture is four
//! circles and a horizontal cut, the sibling `scripts/render-bead-png.mjs` already
//! rasterises the same shapes in JavaScript, and this product's pitch is one small binary
//! with nothing behind it. Nothing here needs a path, a transform or a blend mode.
//!
//! ```text
//!            ╭─────────╮        rim      deep blue, or grey when nothing was read
//!           │  ╭─────╮  │       chamber  white — this is what "empty" looks like
//!           │  │▓▓▓▓▓│  │       fill     rises from the bottom with the binding window,
//!           │  │▓▓▓▓▓│  │                coloured by severity
//!            ╰─────────╯        pupil    black, only when a window is spent
//! ```
//!
//! Four rules, each of them a finding from the audit or a line of the plan:
//!
//! 1. **Unknown never draws a fill.** A grey rim and a hollow ring, never a reassuring
//!    empty-blue bead that looks like "0 % used" (finding B03).
//! 2. **The fill is the binding window**, the highest percentage across both providers —
//!    the number that actually constrains the user (finding B04).
//! 3. **Age shows.** A reading that is ageing or stale is drawn desaturated, so an icon
//!    that has not been fed in an hour does not look as confident as one from a second ago.
//! 4. **The colours are the brand's, written once.** The constants below are copied from
//!    `ui/theme.nazar.json`; [`tests`] parses that file and fails if they drift.

use nazar_core::state::{Freshness, Severity, SnapshotView};

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

    /// Blend towards this colour's own grey by `amount` (0 = untouched, 1 = fully grey).
    ///
    /// Rec. 601 luma, because the eye weighs green more than blue and a naive average
    /// turns the deep blue rim almost black.
    #[must_use]
    fn desaturated(self, amount: f64) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let luma = 0.299 * f64::from(self.red)
            + 0.587 * f64::from(self.green)
            + 0.114 * f64::from(self.blue);
        let mix = |channel: u8| {
            let value = f64::from(channel) * (1.0 - amount) + luma * amount;
            value.round().clamp(0.0, 255.0) as u8
        };
        Rgb::new(mix(self.red), mix(self.green), mix(self.blue))
    }
}

/// The bead's rim, and the fill of a window that is comfortably inside its quota.
/// `bead.deepBlue` in the theme files.
pub const DEEP_BLUE: Rgb = Rgb::new(0x0E, 0x2A, 0x5A);
/// The fill below the warning threshold. `bead.lightBlue`.
pub const LIGHT_BLUE: Rgb = Rgb::new(0x3F, 0xA9, 0xF5);
/// The empty chamber. `bead.white`.
pub const WHITE: Rgb = Rgb::new(0xFF, 0xFF, 0xFF);
/// The pupil, drawn only on a spent window. `bead.blackDot`.
pub const BLACK_DOT: Rgb = Rgb::new(0x0A, 0x0A, 0x0F);
/// The fill at or above the warning threshold. `modes.light.warn`.
///
/// The **light**-mode tones are the ones used here, in both themes and whatever the panel
/// is set to: the fill sits on the white chamber, not on the taskbar, so it is legibility
/// against white that decides. The dark-mode amber is chosen to glow on navy and would be
/// a pale wash inside the bead.
pub const AMBER: Rgb = Rgb::new(0xB8, 0x74, 0x00);
/// The fill at or above the critical threshold. `modes.light.danger`.
pub const RED: Rgb = Rgb::new(0xC0, 0x39, 0x2B);
/// The rim and the hollow ring when nothing could be read. `modes.light.unknownGrey`.
pub const GREY: Rgb = Rgb::new(0x78, 0x87, 0x9A);

/// Bead radius as a fraction of half the icon, leaving room for the antialiased edge.
const INSET: f64 = 1.0 / 32.0;
/// The chamber, as a fraction of the bead's radius. What is left is the rim.
const CHAMBER: f64 = 0.72;
/// The pupil of a spent bead, as a fraction of the bead's radius.
const PUPIL: f64 = 0.26;
/// Outer edge of the hollow ring that means "unknown", as a fraction of the radius.
const RING_OUTER: f64 = 0.46;
/// Inner edge of that ring.
const RING_INNER: f64 = 0.24;
/// Samples per pixel per axis. Sixteen samples give seventeen alpha levels, which is
/// enough for a 16-pixel circle and is what `scripts/render-bead-png.mjs` uses.
const SUPERSAMPLE: u32 = 4;

/// How much of the colour is drained at each freshness.
///
/// `Unknown` — a reading with no `sourceAt` at all — is treated as nearly stale rather than
/// as fresh: an age nobody can work out is not a recent one.
///
/// The numbers were set by looking at a 16-pixel bead on a real taskbar. At 0.6 a stale
/// amber is brown and the severity stops being readable, which trades one signal for
/// another; 0.45 still says "this is old" while leaving the colour recognisable.
fn fade(freshness: Freshness) -> f64 {
    match freshness {
        Freshness::Fresh => 0.0,
        Freshness::Aging => 0.20,
        Freshness::Stale => 0.45,
        Freshness::Unknown => 0.35,
    }
}

/// Everything the drawing needs, and nothing else.
///
/// Deliberately not a `SnapshotView`: the rasteriser is a pure function of three values,
/// so its tests are three values in and pixels out. [`IconState::from_view`] is the one
/// place that knows how to get them out of a snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconState {
    /// The binding window's percentage. `None` is unknown, and draws no fill at all.
    pub percent: Option<f64>,
    /// Severity of that window, which picks the fill colour.
    pub severity: Severity,
    /// How old the reading behind it is, which drains the colour.
    pub freshness: Freshness,
}

impl Default for IconState {
    /// What the tray shows before anything has been read.
    fn default() -> Self {
        IconState {
            percent: None,
            severity: Severity::Unknown,
            freshness: Freshness::Unknown,
        }
    }
}

impl IconState {
    /// The state the icon should be in for a derived snapshot.
    ///
    /// The bead shows **one** number: the highest binding percentage across the providers,
    /// which is the window that will stop the user first. The severity and the freshness
    /// come from that same provider rather than from the worst of each — an icon that took
    /// its colour from one provider and its age from another would describe a machine that
    /// does not exist.
    #[must_use]
    pub fn from_view(view: &SnapshotView) -> Self {
        let mut chosen: Option<(f64, Severity, Freshness)> = None;
        for provider in &view.providers {
            let Some(window) = provider.windows.iter().find(|window| window.binding) else {
                continue;
            };
            let Some(percent) = window.percent.filter(|value| value.is_finite()) else {
                continue;
            };
            if chosen.is_none_or(|(best, _, _)| percent > best) {
                chosen = Some((percent, window.severity, provider.freshness));
            }
        }
        match chosen {
            Some((percent, severity, freshness)) => IconState {
                percent: Some(percent),
                severity,
                freshness,
            },
            None => IconState::default(),
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

/// The physical size of the tray icon at a scale factor.
///
/// Windows asks for a 16-pixel icon at 100 %, and `SM_CXSMICON` grows with the display
/// scale: 20 at 125 %, 24 at 150 %, 32 at 200 %. Handing the shell exactly that size is
/// what makes the bead crisp instead of resampled, and it is why this is drawn rather
/// than shipped.
#[must_use]
pub fn size_for_scale(scale: f64) -> u32 {
    let size = (16.0 * scale).round();
    (size.clamp(8.0, 256.0) as u32).max(8)
}

/// Draw the bead.
///
/// `size` is physical pixels; the picture is defined in fractions of it, so 16, 20, 24 and
/// 32 are the same drawing at four resolutions rather than four drawings.
#[must_use]
pub fn render(size: u32, state: &IconState) -> Bitmap {
    let mut bitmap = Bitmap::blank(size, size);
    let extent = f64::from(size);
    let centre = extent / 2.0;
    let radius = centre - (extent * INSET).max(0.5);
    if radius <= 0.0 {
        return bitmap;
    }

    let unknown = state.severity == Severity::Unknown || state.percent.is_none();
    let drained = fade(state.freshness);
    let rim = if unknown { GREY } else { DEEP_BLUE }.desaturated(drained);
    let fill = fill_colour(state.severity).desaturated(drained);
    let ring = GREY.desaturated(drained);

    let chamber = radius * CHAMBER;
    let pupil = radius * PUPIL;
    let ring_outer = radius * RING_OUTER;
    let ring_inner = radius * RING_INNER;

    // Percentages above 100 exist — a source can report 104 — and the bead is simply full.
    let percent = state.percent.unwrap_or(0.0).clamp(0.0, 100.0);
    let surface = centre + chamber - 2.0 * chamber * percent / 100.0;

    let samples = f64::from(SUPERSAMPLE * SUPERSAMPLE);
    let step = 1.0 / f64::from(SUPERSAMPLE);
    let offset = step / 2.0;

    for y in 0..size {
        for x in 0..size {
            let (mut red, mut green, mut blue, mut covered) = (0.0, 0.0, 0.0, 0.0);

            for sub_y in 0..SUPERSAMPLE {
                let py = f64::from(y) + f64::from(sub_y) * step + offset;
                for sub_x in 0..SUPERSAMPLE {
                    let px = f64::from(x) + f64::from(sub_x) * step + offset;
                    let distance = ((px - centre).powi(2) + (py - centre).powi(2)).sqrt();

                    // Painter's order, from the rim inwards. The last shape that contains
                    // the sample wins, exactly like the SVG the panel draws.
                    let colour = if distance > radius {
                        None
                    } else if distance > chamber {
                        Some(rim)
                    } else if unknown {
                        // No fill, ever: a hollow ring inside an empty chamber.
                        Some(if distance <= ring_outer && distance >= ring_inner {
                            ring
                        } else {
                            WHITE
                        })
                    } else if state.severity == Severity::Exhausted && distance <= pupil {
                        Some(BLACK_DOT)
                    } else if py >= surface {
                        Some(fill)
                    } else {
                        Some(WHITE)
                    };

                    if let Some(colour) = colour {
                        red += f64::from(colour.red);
                        green += f64::from(colour.green);
                        blue += f64::from(colour.blue);
                        covered += 1.0;
                    }
                }
            }

            if covered > 0.0 {
                let at = ((y as usize) * (size as usize) + x as usize) * 4;
                bitmap.rgba[at] = (red / covered).round() as u8;
                bitmap.rgba[at + 1] = (green / covered).round() as u8;
                bitmap.rgba[at + 2] = (blue / covered).round() as u8;
                bitmap.rgba[at + 3] = (covered / samples * 255.0).round() as u8;
            }
        }
    }

    bitmap
}

/// The fill colour for a severity. Unknown has none, which is the whole point.
#[must_use]
pub fn fill_colour(severity: Severity) -> Rgb {
    match severity {
        Severity::Unknown => GREY,
        Severity::Ok => LIGHT_BLUE,
        Severity::Warn => AMBER,
        Severity::Critical | Severity::Exhausted => RED,
    }
}

/// The states the icon strip in `docs/screenshots/wp4-icons.png` shows, left to right.
///
/// Kept here rather than in the exporter so that a test can assert the strip covers every
/// severity and every freshness: a screenshot nobody checks is a screenshot that quietly
/// stops matching the code.
#[must_use]
pub fn strip_states() -> Vec<(&'static str, IconState)> {
    let at = |percent: f64, severity: Severity, freshness: Freshness| IconState {
        percent: Some(percent),
        severity,
        freshness,
    };
    vec![
        ("unknown", IconState::default()),
        ("ok-8", at(8.0, Severity::Ok, Freshness::Fresh)),
        ("ok-42", at(42.0, Severity::Ok, Freshness::Fresh)),
        ("warn-63", at(63.0, Severity::Warn, Freshness::Fresh)),
        (
            "critical-88",
            at(88.0, Severity::Critical, Freshness::Fresh),
        ),
        (
            "exhausted-100",
            at(100.0, Severity::Exhausted, Freshness::Fresh),
        ),
        ("warn-63-ageing", at(63.0, Severity::Warn, Freshness::Aging)),
        ("warn-63-stale", at(63.0, Severity::Warn, Freshness::Stale)),
        (
            "critical-92-stale",
            at(92.0, Severity::Critical, Freshness::Stale),
        ),
    ]
}

/// The scales the strip and the screenshots are taken at: 100 %, 150 %, 200 %.
pub const STRIP_SCALES: [f64; 3] = [1.0, 1.5, 2.0];

/// Compose the icon strip: one column per state, one row per scale.
#[must_use]
pub fn strip() -> Bitmap {
    let states = strip_states();
    let sizes: Vec<u32> = STRIP_SCALES.iter().copied().map(size_for_scale).collect();
    let gap = 6;
    let cell = sizes.iter().copied().max().unwrap_or(32) + gap;

    let width = cell * u32::try_from(states.len()).unwrap_or(1) + gap;
    let height = sizes.iter().sum::<u32>() + gap * (u32::try_from(sizes.len()).unwrap_or(1) + 1);
    let mut sheet = Bitmap::blank(width, height);

    let mut y = gap;
    for size in sizes {
        for (column, (_, state)) in states.iter().enumerate() {
            let x = gap + cell * u32::try_from(column).unwrap_or(0) + (cell - gap - size) / 2;
            sheet.draw(&render(size, state), x, y);
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
/// this exists for `--icons`, which produces the strip in `docs/screenshots`, and for the
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
/// input — flat circles on transparency — it does the job: the icon strip goes from 130 KB
/// stored to a few kilobytes. On input it cannot match it degrades to literals, which is
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

/// Write the icon strip into a directory. `--icons` calls this.
///
/// One file, not twenty-seven: the strip already carries every state at every scale, one
/// row per scale, and a documentation directory full of 16-pixel PNGs is a directory
/// nobody opens.
///
/// # Errors
/// Whatever the file system says: a directory that cannot be created, a file that cannot
/// be written.
pub fn export(directory: &std::path::Path) -> std::io::Result<Vec<std::path::PathBuf>> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join("wp4-icons.png");
    std::fs::write(&path, encode_png(&strip()))?;
    Ok(vec![path])
}

#[cfg(test)]
mod tests;
