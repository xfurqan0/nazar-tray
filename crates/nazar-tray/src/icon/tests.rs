//! What the bead is allowed to look like.
//!
//! Short, because the icon has almost nothing to say. Five of these are the contract: the
//! render **is** the shipped artwork, cell for cell, at every size the tray asks for; the
//! pupil is always there; unknown is a grey rim and a hollow ring and nothing else; a
//! sixteen-pixel bead is sixteen cells; and the palette is the theme file's rather than this
//! file's. The rest pin the encoder and the exported pictures.
//!
//! Everything is measured in **cells** rather than in pixels. The mark is sixteen rows of
//! sixteen, [`size_for_scale`] only ever hands out whole multiples of that, and a test that
//! counted pixels would be re-deriving the scale factor in order to say the same thing.

use super::*;
use nazar_core::state::{Freshness, ProviderView, Severity, SnapshotView, WindowView};

/// The theme file the bead colours are copied from, read at compile time.
const THEME: &str = include_str!("../../../../ui/theme.nazar.json");

/// The artwork the panel shows and the icon set is cut from, read at compile time.
const ARTWORK: &str = include_str!("../../../../ui/assets/bead.svg");

/// The colour and the alpha of one grid cell, sampled at the middle of its block.
///
/// Only meaningful for a render whose size is a whole multiple of [`GRID`], which is every
/// size the tray ever asks for.
fn cell(bitmap: &Bitmap, x: usize, y: usize) -> (Rgb, u8) {
    let scale = bitmap.width as usize / GRID;
    let at = |value: usize| (value * scale + scale / 2) as u32;
    let (red, green, blue, alpha) = bitmap.pixel(at(x), at(y));
    (Rgb::new(red, green, blue), alpha)
}

/// Every grid cell that carries `colour`, as `(x, y)`.
fn cells_of(bitmap: &Bitmap, colour: Rgb) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    for y in 0..GRID {
        for x in 0..GRID {
            if cell(bitmap, x, y) == (colour, 255) {
                found.push((x, y));
            }
        }
    }
    found
}

/// The colour one character of the grid stands for, or `None` where nothing is drawn.
fn layer(glyph: u8) -> Option<Rgb> {
    match glyph {
        b'R' => Some(DEEP_BLUE),
        b'W' => Some(WHITE),
        b'I' => Some(IRIS),
        b'P' => Some(BLACK_DOT),
        _ => None,
    }
}

/// Parse the bead SVG into the same sixteen strings the grid is written as.
///
/// A deliberately small parser: the artwork is a hand-authored file of
/// `<rect x y width height fill>` on integer coordinates, and anything else appearing in it
/// should fail this test rather than be quietly tolerated.
fn grid_of(svg: &str) -> Vec<String> {
    let code = |fill: &str| match fill {
        "#0E2A5A" => Some('R'),
        "#FFFFFF" => Some('W'),
        "#F2A93B" => Some('I'),
        "#0A0A0F" => Some('P'),
        _ => None,
    };

    let mut cells = vec![vec!['.'; GRID]; GRID];
    for line in svg.lines() {
        let line = line.trim();
        if !line.starts_with("<rect ") {
            continue;
        }
        let value = |name: &str| -> String {
            let key = format!("{name}=\"");
            let start = line.find(&key).expect("attribute is present") + key.len();
            let rest = &line[start..];
            rest[..rest.find('"').expect("attribute is closed")].to_string()
        };
        let number = |name: &str| value(name).parse::<usize>().expect("integer attribute");
        let (x, y) = (number("x"), number("y"));
        let (width, height) = (number("width"), number("height"));
        let fill = value("fill").to_ascii_uppercase();
        let character = code(&fill).unwrap_or_else(|| panic!("unexpected fill {fill}"));
        for row in cells.iter_mut().skip(y).take(height) {
            for cell in row.iter_mut().skip(x).take(width) {
                assert_eq!(*cell, '.', "two rects cover one cell");
                *cell = character;
            }
        }
    }
    cells
        .into_iter()
        .map(|row| row.into_iter().collect())
        .collect()
}

/// The grid here and the artwork the panel draws are one mark written twice.
#[test]
fn the_grid_has_not_drifted_from_the_shipped_artwork() {
    let cells = grid_of(ARTWORK);
    for (y, row) in cells.iter().enumerate() {
        assert_eq!(
            row.as_bytes(),
            BEAD[y].as_slice(),
            "row {y} of ui/assets/bead.svg is not row {y} of the tray's grid"
        );
    }
}

/// The whole of rule one: the icon **is** the still mark, at every size, always.
///
/// Cell for cell against the shipped SVG rather than against [`BEAD`], so this fails if the
/// grid and the artwork agree with each other and both disagree with the palette — which is
/// the one way the previous test could pass on a wrong drawing.
#[test]
fn the_mark_is_the_artwork_at_every_size() {
    let cells = grid_of(ARTWORK);
    for size in [16, 32, 48, 64] {
        let bead = render(size, IconState::Mark);
        assert_eq!((bead.width, bead.height), (size, size));
        for (y, row) in cells.iter().enumerate() {
            for (x, glyph) in row.bytes().enumerate() {
                let wanted = layer(glyph).map_or((Rgb::new(0, 0, 0), 0), |colour| (colour, 255));
                assert_eq!(
                    cell(&bead, x, y),
                    wanted,
                    "cell {x},{y} of the {size} px mark is not the artwork's"
                );
            }
        }
    }
}

/// The iris is whole and the pupil is on it, whatever was read. No percentage reaches this
/// module any more, so there is nothing that could hollow either one out.
#[test]
fn the_iris_is_whole_and_the_pupil_is_always_there() {
    for size in [16, 32, 64] {
        let bead = render(size, IconState::Mark);
        assert_eq!(
            cells_of(&bead, BLACK_DOT),
            vec![(7, 7), (8, 7), (7, 8), (8, 8)],
            "the pupil is the grid's own two by two and nothing else, at {size} px"
        );
        let iris = BEAD
            .iter()
            .flat_map(|row| row.iter())
            .filter(|glyph| **glyph == b'I')
            .count();
        assert_eq!(
            cells_of(&bead, IRIS).len(),
            iris,
            "every iris cell is filled at {size} px, or the icon is carrying a level again"
        );
    }
}

/// Finding B03, and the only state the icon has.
#[test]
fn unknown_is_a_grey_rim_a_hollow_ring_and_no_pupil() {
    let bead = render(16, IconState::Unknown);

    assert_eq!(
        cell(&bead, 0, 7),
        (GREY, 255),
        "an unknown bead's rim is grey, not deep blue"
    );
    for colour in [DEEP_BLUE, IRIS, BLACK_DOT] {
        assert!(
            cells_of(&bead, colour).is_empty(),
            "an unknown bead painted {} — it must not look like a reading",
            colour.hex()
        );
    }
    assert!(
        !cells_of(&bead, WHITE).is_empty(),
        "the chamber of an unknown bead is empty, not absent"
    );

    // The ring: one cell thick around a six-by-six box, so the hole is four by four and the
    // ring is twenty cells. It stays inside the chamber, and the band around it stays white.
    for (y, row) in BEAD.iter().enumerate().take(RING_TO + 1).skip(RING_FROM) {
        for (x, glyph) in row.iter().enumerate().take(RING_TO + 1).skip(RING_FROM) {
            let edge = x == RING_FROM || x == RING_TO || y == RING_FROM || y == RING_TO;
            let wanted = if edge { GREY } else { WHITE };
            assert_eq!(cell(&bead, x, y), (wanted, 255), "cell {x},{y} of the ring");
            assert!(
                matches!(glyph, b'W' | b'I' | b'P'),
                "the ring must stay inside the chamber, and {x},{y} is not in it"
            );
        }
    }
    let ring: Vec<(usize, usize)> = cells_of(&bead, GREY)
        .into_iter()
        .filter(|(x, y)| (RING_FROM..=RING_TO).contains(x) && (RING_FROM..=RING_TO).contains(y))
        .collect();
    assert_eq!(ring.len(), 20, "a six-by-six ring one cell thick");

    // And the default is this one: before anything is read, nothing has been read.
    assert_eq!(IconState::default(), IconState::Unknown);
    assert_eq!(render(16, IconState::default()), bead);
}

/// At the tray's own size the render *is* the grid, cell for cell and pixel for pixel.
#[test]
fn sixteen_pixels_is_sixteen_cells() {
    for state in [IconState::Mark, IconState::Unknown] {
        let bitmap = render(16, state);
        for y in 0..GRID {
            for x in 0..GRID {
                let at = (y * GRID + x) * 4;
                let pixel = Rgb::new(bitmap.rgba[at], bitmap.rgba[at + 1], bitmap.rgba[at + 2]);
                let alpha = bitmap.rgba[at + 3];
                match cell_colour(state, x, y) {
                    Some(colour) => {
                        assert_eq!(alpha, 255, "cell {x},{y} should be drawn");
                        assert_eq!(pixel, colour, "cell {x},{y} is the wrong colour");
                    }
                    None => assert_eq!(alpha, 0, "cell {x},{y} should be empty"),
                }
            }
        }
    }
}

/// The opposite of what the round bead asserted, and the reason for the grid: the old test
/// demanded an antialiased rim, because a circle without one looks like a cog. A mark drawn
/// on cells must have no soft pixel anywhere, at any size.
#[test]
fn every_pixel_is_opaque_or_absent_and_never_in_between() {
    for size in [16, 32, 48, 64] {
        for state in [IconState::Mark, IconState::Unknown] {
            let soft = render(size, state)
                .rgba
                .chunks_exact(4)
                .filter(|pixel| pixel[3] != 0 && pixel[3] != 255)
                .count();
            assert_eq!(soft, 0, "{soft} antialiased pixels at {size} px");
        }
    }
}

#[test]
fn the_size_is_always_a_whole_number_of_cells() {
    assert_eq!(size_for_scale(1.0), 16);
    assert_eq!(
        size_for_scale(1.25),
        16,
        "20 px would make four cells wider than the rest"
    );
    assert_eq!(size_for_scale(1.5), 16);
    assert_eq!(size_for_scale(2.0), 32);
    assert_eq!(size_for_scale(3.0), 48);
    assert_eq!(size_for_scale(4.0), 64);
    assert_eq!(
        size_for_scale(0.5),
        16,
        "never smaller than the tray's unit"
    );
    assert_eq!(size_for_scale(99.0), 64, "and never absurd");

    for scale in [0.0, 1.0, 1.1, 1.25, 1.4, 1.5, 1.75, 2.0, 2.5, 3.0, 8.0] {
        let size = size_for_scale(scale);
        assert_eq!(
            size as usize % GRID,
            0,
            "{size} px is not whole cells at {scale}"
        );
        let bead = render(size, IconState::Mark);
        assert_eq!((bead.width, bead.height), (size, size));
        assert_eq!(bead.rgba.len(), (size as usize).pow(2) * 4);
        assert_eq!(
            bead.pixel(0, 0).3,
            0,
            "the corners stay transparent: the taskbar shows through"
        );
    }
}

/// The still mark lives in `ui/assets/bead.svg` and is drawn by the panel and by
/// `scripts/render-app-icons.mjs` as well as by this module. What has to hold on the Rust
/// side is that the four hexes the artwork paints are the four constants above, so a change
/// to one is a failure rather than a silent divergence between three drawings of one mark.
#[test]
fn the_artwork_paints_the_four_constants_and_nothing_else() {
    let fills: Vec<String> = ARTWORK
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("<rect "))
        .map(|line| {
            let at = line.find("fill=\"").expect("a fill") + 6;
            line[at..at + 7].to_ascii_uppercase()
        })
        .collect();

    let mut used: Vec<String> = fills.clone();
    used.sort();
    used.dedup();
    let mut wanted = [DEEP_BLUE, WHITE, IRIS, BLACK_DOT].map(|colour| colour.hex());
    wanted.sort();
    assert_eq!(
        used, wanted,
        "the artwork paints something the tray does not know"
    );

    for (glyph, colour) in [
        (b'R', DEEP_BLUE),
        (b'W', WHITE),
        (b'I', IRIS),
        (b'P', BLACK_DOT),
    ] {
        let cells = BEAD
            .iter()
            .flat_map(|row| row.iter())
            .filter(|c| **c == glyph)
            .count();
        assert!(cells > 0, "the grid has no {} cells", colour.hex());
        assert!(
            fills.iter().any(|fill| *fill == colour.hex()),
            "the artwork never paints {}",
            colour.hex()
        );
    }
}

#[test]
fn the_icon_is_unknown_only_when_nothing_could_be_read() {
    let window = |percent: Option<f64>, binding: bool| WindowView {
        key: "seven_day".to_owned(),
        percent,
        window_minutes: Some(10080),
        resets_at: None,
        remaining_ms: None,
        state: "ok".to_owned(),
        error: None,
        model: None,
        detailed: false,
        binding,
        severity: if percent.is_some() {
            Severity::Ok
        } else {
            Severity::Unknown
        },
    };
    let provider = |name: &str, window: WindowView| ProviderView {
        name: name.to_owned(),
        configured: true,
        plan: None,
        source: None,
        source_at: None,
        binding: Some(window.key.clone()),
        age_ms: None,
        freshness: Freshness::Unknown,
        severity: window.severity,
        windows: vec![window],
    };
    let view = |providers: Vec<ProviderView>| SnapshotView {
        updated_at: String::new(),
        now: String::new(),
        providers,
    };

    // One provider read, one not: something is known, so the icon is the mark.
    assert_eq!(
        IconState::from_view(&view(vec![
            provider("claude", window(Some(12.0), true)),
            provider("codex", window(None, true)),
        ])),
        IconState::Mark
    );
    // Nothing read at all, and nothing at all: unknown, which is also the default.
    assert_eq!(
        IconState::from_view(&view(vec![provider("claude", window(None, true))])),
        IconState::Unknown
    );
    assert_eq!(IconState::from_view(&view(vec![])), IconState::default());
    // A percentage on a window that does not bind is not the window the user is stopped by,
    // and it does not by itself mean the machine could be read.
    assert_eq!(
        IconState::from_view(&view(vec![provider("claude", window(Some(40.0), false))])),
        IconState::Unknown
    );
    // Neither is a percentage that is not a number.
    assert_eq!(
        IconState::from_view(&view(vec![provider(
            "claude",
            window(Some(f64::NAN), true)
        )])),
        IconState::Unknown
    );
}

/// The pictures in `docs/design/` are both states at all three sizes, and nothing else.
#[test]
fn the_documented_pictures_are_the_two_states_the_icon_has() {
    let names: Vec<&str> = DOCUMENTED_STATES.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, vec!["mark", "unknown"]);
    let states: Vec<IconState> = DOCUMENTED_STATES.iter().map(|(_, state)| *state).collect();
    assert_eq!(states, vec![IconState::Mark, IconState::Unknown]);

    assert_eq!(DOCUMENTED_SIZES, [16, 32, 64]);
    for size in DOCUMENTED_SIZES {
        assert_eq!(size as usize % GRID, 0, "{size} px is not whole cells");
        assert_eq!(
            size,
            size_for_scale(f64::from(size) / 16.0),
            "{size} px is not a size the tray would ever draw"
        );
    }

    let sheet = strip();
    assert_eq!(
        (sheet.width, sheet.height),
        (2 * 70 + 6, 16 + 32 + 64 + 6 * 4),
        "two columns of 64 + 6, and one row per size"
    );
    assert!(
        sheet.rgba.iter().skip(3).step_by(4).any(|alpha| *alpha > 0),
        "the strip is blank"
    );
}

#[test]
fn the_png_is_the_same_bytes_every_run() {
    let once = encode_png(&render(16, IconState::Mark));
    let twice = encode_png(&render(16, IconState::Mark));
    assert_eq!(once, twice, "the same bead must encode to the same file");

    assert_eq!(
        &once[..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        "PNG signature"
    );
    assert_eq!(&once[12..16], b"IHDR");
    assert_eq!(
        u32::from_be_bytes([once[16], once[17], once[18], once[19]]),
        16
    );
    assert_eq!(&once[once.len() - 8..once.len() - 4], b"IEND");

    // Pinned: a change to the drawing or to the encoder has to be a deliberate one.
    for (label, state, size, length, checksum) in [
        ("the mark", IconState::Mark, 16u32, PIN_MARK.0, PIN_MARK.1),
        (
            "unknown",
            IconState::Unknown,
            16,
            PIN_UNKNOWN.0,
            PIN_UNKNOWN.1,
        ),
        (
            "the mark, doubled",
            IconState::Mark,
            32,
            PIN_MARK_32.0,
            PIN_MARK_32.1,
        ),
    ] {
        let bytes = encode_png(&render(size, state));
        assert_eq!(bytes.len(), length, "{label}: PNG length moved");
        assert_eq!(crc32(&bytes), checksum, "{label}: PNG bytes moved");
    }
}

/// Length and CRC-32 of the three pinned PNGs. Written out here rather than inline so the
/// three that have to be regenerated together are visibly one group.
const PIN_MARK: (usize, u32) = (238, 0x2302_3606);
const PIN_UNKNOWN: (usize, u32) = (212, 0x33DC_618B);
const PIN_MARK_32: (usize, u32) = (262, 0x89BD_915E);

#[test]
fn the_palette_is_the_theme_files_and_not_this_files() {
    let theme: serde_json::Value = serde_json::from_str(THEME).expect("theme.nazar.json");
    let bead = &theme["bead"];

    for (key, wanted) in [
        ("deepBlue", DEEP_BLUE),
        ("white", WHITE),
        ("iris", IRIS),
        ("blackDot", BLACK_DOT),
    ] {
        assert_eq!(
            bead[key].as_str(),
            Some(wanted.hex().as_str()),
            "bead.{key} drifted from the theme file"
        );
    }
    assert_eq!(
        theme["modes"]["light"]["unknownGrey"].as_str(),
        Some(GREY.hex().as_str()),
        "modes.light.unknownGrey drifted from the theme file"
    );

    // The bead block is the four layers and nothing else. `warnFill` was a fill colour for
    // a gauge the icon no longer has, and a key nothing reads is a key that starts lying.
    assert!(
        bead.get("warnFill").is_none(),
        "the theme still carries bead.warnFill, and nothing draws it"
    );
    assert_eq!(
        bead.as_object().map(serde_json::Map::len),
        Some(4),
        "the bead block is deepBlue, white, iris and blackDot"
    );

    // Nothing the artwork paints is Nazar's iris. The comment at the top of the module names
    // that hex on purpose, so this looks at the fills and not at the bytes.
    assert!(
        !ARTWORK.to_ascii_uppercase().contains("FILL=\"#3FA9F5\""),
        "Nazar's iris is back in this repository's mark"
    );
}

#[test]
fn drawing_one_bitmap_into_another_keeps_the_transparent_parts() {
    let mut sheet = Bitmap::blank(48, 48);
    let bead = render(16, IconState::Mark);
    sheet.draw(&bead, 16, 16);

    assert_eq!(
        sheet.pixel(0, 0).3,
        0,
        "the sheet stays transparent around it"
    );
    assert_eq!(sheet.pixel(24, 24), bead.pixel(8, 8));
    // Out of bounds is a no-op rather than a panic.
    sheet.draw(&bead, 47, 47);
}
