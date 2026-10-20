// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The built-in analyser draws with the skin's own `viscolor.txt` colours.
//!
//! Headless: every case renders the main window into a buffer and reads the
//! analyser rectangle back. No window, no device, no process is started.

use std::collections::HashSet;

use rustamp_skin::{
    BuiltinDefaultSkin, BuiltinLookalikeSkin, MemorySource, Skin, webamp::DEFAULT_VISCOLORS,
};
use rustamp_ui::visualization::{SPECTRUM_BANDS, VisInput, VisMode, VisState, slots};
use rustamp_ui::{MainWindowLayout, MainWindowState, render_main_window};

/// 24 distinct colours, one per slot.
fn palette() -> [[u8; 3]; 24] {
    std::array::from_fn(|slot| [slot as u8, 255 - slot as u8, slot as u8 * 7])
}

/// A second palette, disjoint from [`palette`].
fn other_palette() -> [[u8; 3]; 24] {
    std::array::from_fn(|slot| [255 - slot as u8, slot as u8, 3])
}

/// The `viscolor.txt` text for `colors`.
fn viscolor_text(colors: &[[u8; 3]; 24]) -> String {
    colors
        .iter()
        .map(|[r, g, b]| format!("{r},{g},{b}\n"))
        .collect()
}

/// A skin that carries nothing but a `viscolor.txt`; every sheet comes from
/// the built-in default skin.
fn skin_with_viscolors(colors: &[[u8; 3]; 24]) -> Skin {
    let mut source = MemorySource::new("synthetic viscolor skin");
    source.add("viscolor.txt", viscolor_text(colors).into_bytes());
    Skin::from_source(&mut source, &BuiltinDefaultSkin).expect("the synthetic skin loads")
}

/// A skin that ships no `viscolor.txt` at all.
fn skin_without_viscolors() -> Skin {
    let mut source = MemorySource::new("synthetic skin without a viscolor.txt");
    Skin::from_source(&mut source, &BuiltinDefaultSkin).expect("the synthetic skin loads")
}

/// A main window whose analyser draws full bars, no peaks and a fresh scope.
fn spectrum_state() -> MainWindowState {
    MainWindowState {
        visualization: VisState {
            mode: VisMode::Spectrum,
            bars: [16; SPECTRUM_BANDS],
            scope: [8; 76],
            input: VisInput::Fresh,
            ..VisState::default()
        },
        ..MainWindowState::default()
    }
}

/// The classic layout's analyser rectangle.
fn area() -> rustamp_gfx::Rect {
    MainWindowLayout::CLASSIC.visualization
}

/// The opaque pixel of one palette slot.
fn slot_pixel(colors: &[[u8; 3]; 24], slot: usize) -> [u8; 4] {
    let [r, g, b] = colors[slot];
    [r, g, b, 255]
}

#[test]
fn the_analyser_draws_with_the_skins_own_colours_only() {
    let colors = palette();
    let skin = skin_with_viscolors(&colors);
    assert_eq!(skin.viscolors(), &colors);
    let area = area();
    // The full bars hide the grid dots, so the empty state shows them.
    let empty = MainWindowState {
        visualization: VisState {
            mode: VisMode::Spectrum,
            input: VisInput::Fresh,
            ..VisState::default()
        },
        ..MainWindowState::default()
    };
    let mut seen: HashSet<[u8; 3]> = HashSet::new();
    for state in [spectrum_state(), empty] {
        let frame = render_main_window(&state, &skin, &MainWindowLayout::CLASSIC);
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let [r, g, b, _] = frame
                    .pixel(x, y)
                    .expect("the analyser rectangle is inside the frame");
                let color = [r, g, b];
                assert!(
                    colors.contains(&color),
                    "foreign colour {color:?} at {x},{y}"
                );
                seen.insert(color);
            }
        }
    }
    // Every slot the spectrum draws with is on the frame.
    for slot in [
        slots::BACKGROUND,
        slots::GRID,
        slots::SPECTRUM_TOP,
        slots::SPECTRUM_BOTTOM,
    ] {
        assert!(seen.contains(&colors[slot]), "slot {slot} is missing");
    }
}

#[test]
fn the_bars_use_the_gradient_slots_from_slot_2_at_the_top_to_slot_17_at_the_bottom() {
    let colors = palette();
    let skin = skin_with_viscolors(&colors);
    let frame = render_main_window(&spectrum_state(), &skin, &MainWindowLayout::CLASSIC);
    let area = area();
    for row in 0..16i32 {
        // S17 §6: slot 2 at the top of a bar, slot 17 at the bottom.
        let slot = slots::SPECTRUM_BOTTOM - row as usize;
        assert_eq!(
            frame.pixel(area.x, area.y + 15 - row),
            Some(slot_pixel(&colors, slot)),
            "row {row}"
        );
    }
}

#[test]
fn the_peak_marker_uses_slot_23() {
    let colors = palette();
    let skin = skin_with_viscolors(&colors);
    let mut state = spectrum_state();
    state.visualization.peaks = [8; SPECTRUM_BANDS];
    let frame = render_main_window(&state, &skin, &MainWindowLayout::CLASSIC);
    let area = area();
    // The marker rides above the bar's top: 16 rows minus a peak of 8.
    assert_eq!(
        frame.pixel(area.x, area.y + 8),
        Some(slot_pixel(&colors, slots::PEAK))
    );
}

#[test]
fn the_oscilloscope_uses_the_five_oscilloscope_slots() {
    let colors = palette();
    let skin = skin_with_viscolors(&colors);
    let mut state = MainWindowState {
        visualization: VisState {
            mode: VisMode::ScopeDots,
            scope: [8; 76],
            input: VisInput::Fresh,
            ..VisState::default()
        },
        ..MainWindowState::default()
    };
    // One sample on the centre line, one on the top row.
    state.visualization.scope[0] = 8;
    state.visualization.scope[1] = 0;
    let frame = render_main_window(&state, &skin, &MainWindowLayout::CLASSIC);
    let area = area();
    assert_eq!(
        frame.pixel(area.x, area.y + 8),
        Some(slot_pixel(&colors, slots::OSCILLOSCOPE_FIRST)),
        "the centre line wears the brightest oscilloscope colour"
    );
    assert_eq!(
        frame.pixel(area.x + 1, area.y),
        Some(slot_pixel(&colors, slots::OSCILLOSCOPE_LAST)),
        "the outermost row wears the dimmest one"
    );
}

#[test]
fn a_skin_without_a_viscolor_file_falls_back_to_the_built_in_defaults() {
    let skin = skin_without_viscolors();
    assert_eq!(skin.viscolors(), &DEFAULT_VISCOLORS);
    let frame = render_main_window(&spectrum_state(), &skin, &MainWindowLayout::CLASSIC);
    let area = area();
    assert_eq!(
        frame.pixel(area.x, area.y),
        Some(slot_pixel(&DEFAULT_VISCOLORS, slots::SPECTRUM_TOP))
    );
    assert_eq!(
        frame.pixel(area.x, area.y + 15),
        Some(slot_pixel(&DEFAULT_VISCOLORS, slots::SPECTRUM_BOTTOM))
    );
    // The classic look-alike skin ships no `viscolor.txt` either.
    let mut source = BuiltinLookalikeSkin::source();
    let lookalike =
        Skin::from_source(&mut source, &BuiltinLookalikeSkin).expect("the lookalike skin loads");
    assert_eq!(lookalike.viscolors(), &DEFAULT_VISCOLORS);
}

#[test]
fn a_skin_change_repaints_the_analyser_in_the_new_colours() {
    let first = palette();
    let second = other_palette();
    let skin_a = skin_with_viscolors(&first);
    let skin_b = skin_with_viscolors(&second);
    let state = spectrum_state();
    let area = area();
    let before = render_main_window(&state, &skin_a, &MainWindowLayout::CLASSIC);
    let after = render_main_window(&state, &skin_b, &MainWindowLayout::CLASSIC);
    for row in 0..16i32 {
        let slot = slots::SPECTRUM_BOTTOM - row as usize;
        let y = area.y + 15 - row;
        assert_eq!(
            before.pixel(area.x, y),
            Some(slot_pixel(&first, slot)),
            "row {row} of the first skin"
        );
        assert_eq!(
            after.pixel(area.x, y),
            Some(slot_pixel(&second, slot)),
            "row {row} of the second skin"
        );
    }
    assert_ne!(before.pixel(area.x, area.y), after.pixel(area.x, area.y));
}
