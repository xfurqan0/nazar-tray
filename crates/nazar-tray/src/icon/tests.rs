//! What the bead is allowed to look like.
//!
//! The rasteriser is a pure function, so these are pixels rather than screenshots: a
//! percentage in, a fill height out. Three of them are contract rules rather than taste —
//! unknown never draws a fill, a spent window is red *and* carries a mark that survives a
//! greyscale screenshot, and the palette is the theme file's rather than this file's.

use super::*;
use nazar_core::state::{Freshness, ProviderView, Severity, SnapshotView, WindowView};

/// The theme file the bead colours are copied from, read at compile time.
const THEME: &str = include_str!("../../../../ui/theme.nazar.json");

/// Colour of a pixel, ignoring alpha.
fn colour_at(bitmap: &Bitmap, x: u32, y: u32) -> Rgb {
    let (red, green, blue, _) = bitmap.pixel(x, y);
    Rgb::new(red, green, blue)
}

/// How many pixels of the centre column are exactly the fill colour.
fn fill_height(bitmap: &Bitmap, fill: Rgb) -> u32 {
    let centre = bitmap.width / 2;
    (0..bitmap.height)
        .filter(|y| colour_at(bitmap, centre, *y) == fill)
        .count() as u32
}

/// The topmost row of the centre column that is exactly the fill colour.
fn fill_top(bitmap: &Bitmap, fill: Rgb) -> Option<u32> {
    let centre = bitmap.width / 2;
    (0..bitmap.height).find(|y| colour_at(bitmap, centre, *y) == fill)
}

fn ok_at(percent: f64) -> IconState {
    IconState {
        percent: Some(percent),
        severity: Severity::Ok,
        freshness: Freshness::Fresh,
    }
}

#[test]
fn the_fill_height_follows_the_percentage() {
    // The chamber is the drawable range: a full bead is its diameter, an empty one nothing.
    let size = 64;
    let radius = f64::from(size) / 2.0 - (f64::from(size) * INSET).max(0.5);
    let chamber = 2.0 * radius * CHAMBER;

    for percent in [10.0, 25.0, 50.0, 75.0, 90.0] {
        let drawn = f64::from(fill_height(&render(size, &ok_at(percent)), LIGHT_BLUE));
        let wanted = chamber * percent / 100.0;
        assert!(
            (drawn - wanted).abs() <= 2.0,
            "at {percent} % the fill is {drawn} px where the chamber asks for {wanted:.1}"
        );
    }
}

#[test]
fn an_empty_window_draws_no_fill_and_a_full_one_fills_the_chamber() {
    let empty = render(64, &ok_at(0.0));
    assert_eq!(
        fill_height(&empty, LIGHT_BLUE),
        0,
        "0 % must draw an empty bead, not a sliver"
    );

    let full = render(
        64,
        &IconState {
            percent: Some(100.0),
            severity: Severity::Exhausted,
            freshness: Freshness::Fresh,
        },
    );
    // The pupil interrupts the centre column of a spent bead, so the fill is measured
    // beside it.
    let column = full.width / 2 + full.width / 4;
    let painted = (0..full.height)
        .filter(|y| colour_at(&full, column, *y) == RED)
        .count();
    assert!(painted > 0, "a spent bead must be red");
}

#[test]
fn a_half_full_bead_is_filled_from_the_middle_down() {
    let bead = render(64, &ok_at(50.0));
    let centre = bead.height / 2;
    assert_eq!(
        fill_top(&bead, LIGHT_BLUE),
        Some(centre),
        "50 % must start exactly at the middle row"
    );
    assert_eq!(
        colour_at(&bead, bead.width / 2, centre - 1),
        WHITE,
        "the half above it is the empty chamber"
    );
}

#[test]
fn more_quota_used_never_lowers_the_surface() {
    let mut previous = u32::MAX;
    for percent in [5.0, 20.0, 40.0, 60.0, 80.0, 99.0] {
        let top = fill_top(&render(64, &ok_at(percent)), LIGHT_BLUE).expect("a fill");
        assert!(
            top < previous,
            "{percent} % starts at row {top}, which is no higher than the step before it"
        );
        previous = top;
    }
}

#[test]
fn each_severity_paints_its_own_colour() {
    for (severity, wanted) in [
        (Severity::Ok, LIGHT_BLUE),
        (Severity::Warn, AMBER),
        (Severity::Critical, RED),
        (Severity::Exhausted, RED),
    ] {
        let bead = render(
            64,
            &IconState {
                percent: Some(90.0),
                severity,
                freshness: Freshness::Fresh,
            },
        );
        assert_eq!(
            fill_colour(severity),
            wanted,
            "{severity:?} takes the wrong colour from the palette"
        );
        assert!(
            fill_height(&bead, wanted) > 0,
            "{severity:?} did not paint {}",
            wanted.hex()
        );
    }
}

#[test]
fn unknown_draws_no_fill_whatever_it_is_handed() {
    // Both shapes of unknown: no percentage at all, and — defensively — a percentage that
    // arrived with an unknown severity, which no honest snapshot produces.
    for state in [
        IconState::default(),
        IconState {
            percent: Some(90.0),
            severity: Severity::Unknown,
            freshness: Freshness::Fresh,
        },
    ] {
        let bead = render(64, &state);
        for fill in [LIGHT_BLUE, AMBER, RED] {
            assert_eq!(
                fill_height(&bead, fill),
                0,
                "an unknown bead painted {} — finding B03 is exactly this",
                fill.hex()
            );
        }
        assert!(
            fill_height(&bead, WHITE) > 0,
            "the chamber of an unknown bead is empty, not absent"
        );
        assert!(
            fill_height(&bead, GREY.desaturated(fade(state.freshness))) > 0,
            "an unknown bead carries a grey ring so it reads as a state, not as an empty one"
        );
    }
}

#[test]
fn a_spent_bead_carries_a_dark_pupil() {
    let spent = render(
        64,
        &IconState {
            percent: Some(100.0),
            severity: Severity::Exhausted,
            freshness: Freshness::Fresh,
        },
    );
    assert_eq!(
        colour_at(&spent, 32, 32),
        BLACK_DOT,
        "a spent window needs a mark that survives a greyscale screenshot"
    );

    let critical = render(
        64,
        &IconState {
            percent: Some(90.0),
            severity: Severity::Critical,
            freshness: Freshness::Fresh,
        },
    );
    assert_eq!(
        colour_at(&critical, 32, 32),
        RED,
        "only a spent window gets the pupil"
    );
}

#[test]
fn age_drains_the_colour_without_changing_the_level() {
    let fresh = render(
        64,
        &IconState {
            percent: Some(63.0),
            severity: Severity::Warn,
            freshness: Freshness::Fresh,
        },
    );
    let stale = render(
        64,
        &IconState {
            percent: Some(63.0),
            severity: Severity::Warn,
            freshness: Freshness::Stale,
        },
    );

    let faded = AMBER.desaturated(fade(Freshness::Stale));
    assert_ne!(faded, AMBER, "a stale bead must not look as confident");
    assert_eq!(
        fill_height(&fresh, AMBER),
        fill_height(&stale, faded),
        "staleness changes the colour, never the number"
    );

    let spread = |colour: Rgb| {
        i32::from(colour.red.max(colour.green).max(colour.blue))
            - i32::from(colour.red.min(colour.green).min(colour.blue))
    };
    assert!(
        spread(faded) < spread(AMBER),
        "desaturating must move the colour towards grey"
    );
    assert_eq!(
        AMBER.desaturated(0.0),
        AMBER,
        "a fresh reading is not touched at all"
    );
}

#[test]
fn every_scale_gets_the_size_the_shell_asks_for() {
    assert_eq!(size_for_scale(1.0), 16);
    assert_eq!(size_for_scale(1.25), 20);
    assert_eq!(size_for_scale(1.5), 24);
    assert_eq!(size_for_scale(2.0), 32);
    // A scale nobody has ever seen must still produce a drawable icon.
    assert!(size_for_scale(0.0) >= 8);
    assert!(size_for_scale(99.0) <= 256);

    for scale in STRIP_SCALES {
        let size = size_for_scale(scale);
        let bead = render(size, &ok_at(50.0));
        assert_eq!((bead.width, bead.height), (size, size));
        assert_eq!(bead.rgba.len(), (size as usize).pow(2) * 4);
        assert!(
            fill_height(&bead, LIGHT_BLUE) > 0,
            "the fill has to survive down to {size} px, which is where it matters most"
        );
        assert_eq!(
            bead.pixel(0, 0).3,
            0,
            "the corners stay transparent: the taskbar shows through"
        );
    }
}

#[test]
fn the_icon_takes_the_highest_binding_window_across_both_providers() {
    let window = |percent: Option<f64>, severity: Severity| WindowView {
        key: "seven_day".to_owned(),
        percent,
        window_minutes: Some(10080),
        resets_at: None,
        remaining_ms: None,
        state: "ok".to_owned(),
        error: None,
        model: None,
        detailed: false,
        binding: true,
        severity,
    };
    let provider = |name: &str, window: WindowView, freshness: Freshness| ProviderView {
        name: name.to_owned(),
        configured: true,
        plan: None,
        source: None,
        source_at: None,
        binding: Some(window.key.clone()),
        age_ms: None,
        freshness,
        severity: window.severity,
        windows: vec![window],
    };
    let view = |providers: Vec<ProviderView>| SnapshotView {
        updated_at: String::new(),
        now: String::new(),
        providers,
    };

    let state = IconState::from_view(&view(vec![
        provider("claude", window(Some(12.0), Severity::Ok), Freshness::Fresh),
        provider(
            "codex",
            window(Some(88.0), Severity::Critical),
            Freshness::Stale,
        ),
    ]));
    assert_eq!(state.percent, Some(88.0));
    assert_eq!(state.severity, Severity::Critical);
    assert_eq!(
        state.freshness,
        Freshness::Stale,
        "the age shown belongs to the number shown"
    );

    let nothing = IconState::from_view(&view(vec![provider(
        "claude",
        window(None, Severity::Unknown),
        Freshness::Unknown,
    )]));
    assert_eq!(nothing.percent, None);
    assert_eq!(nothing.severity, Severity::Unknown);
    assert_eq!(IconState::from_view(&view(vec![])), IconState::default());
}

#[test]
fn the_strip_covers_every_severity_and_every_freshness() {
    let states = strip_states();
    for severity in [
        Severity::Unknown,
        Severity::Ok,
        Severity::Warn,
        Severity::Critical,
        Severity::Exhausted,
    ] {
        assert!(
            states.iter().any(|(_, state)| state.severity == severity),
            "the strip does not show {severity:?}"
        );
    }
    for freshness in [Freshness::Fresh, Freshness::Aging, Freshness::Stale] {
        assert!(
            states.iter().any(|(_, state)| state.freshness == freshness),
            "the strip does not show {freshness:?}"
        );
    }

    let sheet = strip();
    assert!(sheet.width > 0 && sheet.height > 0);
    assert!(
        sheet.rgba.iter().skip(3).step_by(4).any(|alpha| *alpha > 0),
        "the strip is blank"
    );
}

#[test]
fn the_png_is_the_same_bytes_every_run() {
    let bead = render(16, &ok_at(50.0));
    let once = encode_png(&bead);
    let twice = encode_png(&render(16, &ok_at(50.0)));
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
        ("half full", ok_at(50.0), 16u32, 455usize, 0x2297_71EBu32),
        ("unknown", IconState::default(), 16, 592, 0x68EB_E67F),
        (
            "spent",
            IconState {
                percent: Some(100.0),
                severity: Severity::Exhausted,
                freshness: Freshness::Stale,
            },
            32,
            817,
            0x42AB_3B58,
        ),
    ] {
        let bytes = encode_png(&render(size, &state));
        assert_eq!(bytes.len(), length, "{label}: PNG length moved");
        assert_eq!(crc32(&bytes), checksum, "{label}: PNG bytes moved");
    }
}

#[test]
fn the_palette_is_the_theme_files_and_not_this_files() {
    let theme: serde_json::Value = serde_json::from_str(THEME).expect("theme.nazar.json");
    let bead = &theme["bead"];
    let light = &theme["modes"]["light"];

    for (key, wanted) in [
        ("deepBlue", DEEP_BLUE),
        ("lightBlue", LIGHT_BLUE),
        ("white", WHITE),
        ("blackDot", BLACK_DOT),
    ] {
        assert_eq!(
            bead[key].as_str(),
            Some(wanted.hex().as_str()),
            "bead.{key} drifted from the theme file"
        );
    }
    for (key, wanted) in [("warn", AMBER), ("danger", RED), ("unknownGrey", GREY)] {
        assert_eq!(
            light[key].as_str(),
            Some(wanted.hex().as_str()),
            "modes.light.{key} drifted from the theme file"
        );
    }
}

#[test]
fn drawing_one_bitmap_into_another_keeps_the_transparent_parts() {
    let mut sheet = Bitmap::blank(8, 8);
    let bead = render(4, &ok_at(100.0));
    sheet.draw(&bead, 2, 2);

    assert_eq!(
        sheet.pixel(0, 0).3,
        0,
        "the sheet stays transparent around it"
    );
    assert_eq!(sheet.pixel(4, 4), bead.pixel(2, 2));
    // Out of bounds is a no-op rather than a panic.
    sheet.draw(&bead, 7, 7);
}
