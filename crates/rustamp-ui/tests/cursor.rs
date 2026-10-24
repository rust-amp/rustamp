// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The cursor mapping: which cursor slot each control asks for.
//!
//! The mapping is Rustamp's own choice (S17 §8 leaves the cursor-to-region
//! mapping open), so these tests pin the choice, not a measurement.

use rustamp_skin::CursorSlot;
use rustamp_ui::cursor::{equalizer_cursor, main_window_cursor, playlist_cursor_at};
use rustamp_ui::equalizer::{EqSlider, EqWidget, EqualizerLayout, hit_test as eq_hit_test};
use rustamp_ui::layout::MainWindowLayout;
use rustamp_ui::widget::{Widget, hit_test};

/// The main window: the pointer position, the widget it hits and the cursor
/// slot the mapping asks the skin for.
#[test]
fn the_main_window_mapping_follows_the_hit_test() {
    let layout = MainWindowLayout::CLASSIC;
    let cases = [
        // The four window buttons.
        (10, 7, Widget::WindowMenu, CursorSlot::MainMenu),
        (248, 7, Widget::WindowMinimize, CursorSlot::Min),
        (258, 7, Widget::WindowShade, CursorSlot::WinBut),
        (268, 7, Widget::WindowClose, CursorSlot::Close),
        // The title band and the song title.
        (137, 7, Widget::TitleBar, CursorSlot::TitleBar),
        (120, 30, Widget::Marquee, CursorSlot::SongName),
        // The three sliders.
        (100, 77, Widget::PositionSlider, CursorSlot::PosBar),
        (140, 63, Widget::VolumeSlider, CursorSlot::VolBal),
        (190, 63, Widget::BalanceSlider, CursorSlot::VolBal),
        // Controls that own no cursor of their own.
        (50, 96, Widget::Play, CursorSlot::Normal),
        (230, 64, Widget::EqToggle, CursorSlot::Normal),
        (60, 30, Widget::TimeDisplay, CursorSlot::Normal),
    ];
    for (x, y, widget, slot) in cases {
        assert_eq!(hit_test(&layout, x, y), Some(widget), "at ({x},{y})");
        assert_eq!(
            main_window_cursor(hit_test(&layout, x, y)),
            slot,
            "at ({x},{y})"
        );
    }
}

/// The background of the main window — a point on no widget at all — is the
/// normal cursor.
#[test]
fn the_main_window_background_is_the_normal_cursor() {
    let layout = MainWindowLayout::CLASSIC;
    assert_eq!(hit_test(&layout, 5, 100), None);
    assert_eq!(main_window_cursor(None), CursorSlot::Normal);
}

/// Every widget of the main window maps to a slot, and no widget maps to a
/// slot outside the model's classic set.
#[test]
fn every_main_window_widget_maps_to_a_slot() {
    for widget in Widget::ALL {
        let slot = main_window_cursor(Some(*widget));
        assert!(
            CursorSlot::ALL.contains(&slot),
            "{widget:?} maps to {slot:?}, which is no slot"
        );
    }
}

/// The equalizer: the sliders and the title bar have their own cursors, the
/// buttons and the background share the equalizer's normal cursor.
#[test]
fn the_equalizer_mapping_follows_the_hit_test() {
    let layout = EqualizerLayout::CLASSIC;
    let cases = [
        (79, 60, CursorSlot::EqSlid),
        (97, 60, CursorSlot::EqSlid),
        (22, 60, CursorSlot::EqSlid),
        (100, 5, CursorSlot::EqTitle),
        (20, 24, CursorSlot::EqNormal),
        (230, 24, CursorSlot::EqNormal),
    ];
    for (x, y, slot) in cases {
        assert_eq!(
            equalizer_cursor(eq_hit_test(&layout, x, y, false)),
            slot,
            "at ({x},{y})"
        );
    }
    assert_eq!(equalizer_cursor(None), CursorSlot::EqNormal);
    // The shaded equalizer is its title band alone (S05 §1.2).
    assert_eq!(
        equalizer_cursor(eq_hit_test(&layout, 100, 5, true)),
        CursorSlot::EqTitle
    );
    assert_eq!(
        equalizer_cursor(eq_hit_test(&layout, 79, 60, true)),
        CursorSlot::EqNormal
    );
}

/// The playlist window: the title band, its two buttons, the scrollbar and
/// the rest of the window.
#[test]
fn the_playlist_mapping_covers_its_regions() {
    // The title band, without a button on it.
    assert_eq!(playlist_cursor_at(120, 5), CursorSlot::PTBar);
    // The close and the shade button.
    assert_eq!(playlist_cursor_at(268, 7), CursorSlot::PClose);
    assert_eq!(playlist_cursor_at(258, 7), CursorSlot::PWinBut);
    // The scrollbar: the track with the handle and the arrow pair.
    assert_eq!(playlist_cursor_at(263, 40), CursorSlot::PVScroll);
    assert_eq!(playlist_cursor_at(263, 82), CursorSlot::PVScroll);
    // The list and the bottom bar.
    assert_eq!(playlist_cursor_at(100, 40), CursorSlot::PNormal);
    assert_eq!(playlist_cursor_at(100, 100), CursorSlot::PNormal);
}

/// The slots the base skin ships are the ones the mapping asks for, except
/// the playlist's resize grip: the playlist editor has no resize region to
/// hit-test yet, so nothing asks for it.
#[test]
fn the_base_skin_slots_are_the_ones_the_mapping_uses() {
    let mut used: Vec<CursorSlot> = Vec::new();
    for widget in Widget::ALL {
        used.push(main_window_cursor(Some(*widget)));
    }
    for widget in EqWidget::ALL {
        used.push(equalizer_cursor(Some(*widget)));
    }
    for slider in EqSlider::ALL_BANDS {
        used.push(equalizer_cursor(Some(EqWidget::Slider(slider))));
    }
    for (x, y) in [
        (120, 5),   // the title band
        (268, 7),   // the close button
        (258, 7),   // the shade button
        (263, 40),  // the scrollbar
        (263, 82),  // the scrollbar's arrow pair
        (100, 40),  // the list
        (100, 100), // the bottom bar
    ] {
        used.push(playlist_cursor_at(x, y));
    }

    for name in [
        "normal", "close", "eqslid", "eqtitle", "posbar", "ptbar", "pvscroll", "titlebar",
        "volbal", "pnormal",
    ] {
        let slot = CursorSlot::from_file_name(&format!("{name}.cur")).unwrap();
        assert!(
            used.contains(&slot),
            "the mapping never asks for the base skin's {name}.cur"
        );
    }
    // The resize grip is the one base-skin cursor nothing asks for yet.
    assert!(!used.contains(&CursorSlot::PSize));
}
