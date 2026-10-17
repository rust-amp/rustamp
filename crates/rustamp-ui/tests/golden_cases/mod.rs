// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Shared golden-test cases and helpers for headlessly rendered skin suites.

#![allow(dead_code)] // Shared across two test binaries (lookalike and rust-theme), each using their own skin loader and namespace.

use rustamp_core::command::SelectOp;
use rustamp_core::playlist::EntryId;
use rustamp_skin::{BuiltinDefaultSkin, BuiltinLookalikeSkin, Skin};
use rustamp_testkit::{Snapshot, assert_golden};
use rustamp_ui::equalizer::{EqSlider, EqWidget};
use rustamp_ui::{
    DialogKind, EqualizerLayout, EqualizerState, MainWindowLayout, MainWindowState, MonoStereo,
    PlaylistWindow, TransportState, eq_frame_size, frame_size, render_dialog,
    render_equalizer_window, render_main_window, render_playlist_window,
};

/// Namespace for the look-alike skin.
pub const SKIN_LOOKALIKE: &str = rustamp_testkit::SKIN_LOOKALIKE;
/// Namespace for the built-in default Rust theme skin.
pub const SKIN_RUST_THEME: &str = "rust-theme";

/// Loads the classic look-alike skin via its memory source.
pub fn lookalike_skin() -> Skin {
    let mut source = BuiltinLookalikeSkin::source();
    Skin::from_source(&mut source, &BuiltinLookalikeSkin).expect("the lookalike skin must load")
}

/// Loads the built-in default Rust-themed skin via its memory source.
pub fn rust_theme_skin() -> Skin {
    let mut source = BuiltinDefaultSkin::source();
    Skin::from_source(&mut source, &BuiltinDefaultSkin)
        .expect("the built-in default skin must load")
}

/// Renders on demand, because `Image::into_raw` consumes the image.
pub struct Shot<F: Fn() -> rustamp_gfx::Image>(pub F);

impl<F: Fn() -> rustamp_gfx::Image> Snapshot for Shot<F> {
    fn render_rgba(&self) -> (u32, u32, Vec<u8>) {
        (self.0)().into_raw()
    }
}

/// A playing main window: a track title in the marquee (scrolled, so the
/// `marquee_offset` path is pinned too), a running time, a bitrate and sample
/// rate, a lit stereo label and a moved position and volume.
pub fn playing_state() -> MainWindowState {
    MainWindowState {
        transport: TransportState::Playing,
        // 3:42, so the leading tens-of-minutes cell stays blank (S01 §2.3).
        time_seconds: 222,
        title: "1. Rustamp - the look-alike default skin (3:42)".to_string(),
        // Shifted left, as a scrolling title is (S02 §3, 5 px per cell).
        marquee_offset: 10,
        kbps: Some(192),
        khz: Some(44),
        channels: MonoStereo::Stereo,
        position: 37,
        volume: 80,
        ..MainWindowState::default()
    }
}

pub fn equalizer_golden(
    name: &str,
    namespace: &str,
    skin: &Skin,
    state: &EqualizerState,
    dimensions: (u32, u32),
) {
    assert_eq!(eq_frame_size(state), dimensions);
    let image = render_equalizer_window(state, skin, &EqualizerLayout::CLASSIC);
    assert_eq!((image.width(), image.height()), dimensions);
    if state.double_size {
        let single = EqualizerState {
            double_size: false,
            ..state.clone()
        };
        let single = render_equalizer_window(&single, skin, &EqualizerLayout::CLASSIC);
        assert_eq!(image, single.scaled(2), "exact whole-frame pixel doubling");
    }
    let shot = Shot(|| image.clone());
    assert_golden(name, namespace, &shot.render_image());
}

pub fn case_main_window(skin: &Skin, namespace: &str) {
    let shot = Shot(|| {
        render_main_window(
            &MainWindowState::default(),
            skin,
            &MainWindowLayout::CLASSIC,
        )
    });
    assert_golden("main_window", namespace, &shot.render_image());
}

pub fn case_equalizer_window(skin: &Skin, namespace: &str) {
    let shot = Shot(|| {
        render_equalizer_window(&EqualizerState::default(), skin, &EqualizerLayout::CLASSIC)
    });
    assert_golden("equalizer_window", namespace, &shot.render_image());
}

pub fn case_playlist_window(skin: &Skin, namespace: &str) {
    let labels = vec![
        "1. Rustamp - the look-alike default skin".to_string(),
        "2. A second entry, so the rows are visible".to_string(),
        "3. 3:42".to_string(),
    ];
    let shot = Shot(|| render_playlist_window(skin, &PlaylistWindow::default(), &labels, true));
    assert_golden("playlist_window", namespace, &shot.render_image());
}

pub fn case_main_window_playing(skin: &Skin, namespace: &str) {
    let state = playing_state();
    let shot = Shot(|| render_main_window(&state, skin, &MainWindowLayout::CLASSIC));
    assert_golden("main_window_playing", namespace, &shot.render_image());
}

pub fn case_main_window_double_size(skin: &Skin, namespace: &str) {
    let state = MainWindowState {
        double_size: true,
        ..MainWindowState::default()
    };
    assert_eq!(frame_size(&state), (550, 232), "S05 §2.1 double-size size");
    let shot = Shot(|| render_main_window(&state, skin, &MainWindowLayout::CLASSIC));
    assert_golden("main_window_double_size", namespace, &shot.render_image());
}

pub fn case_equalizer_window_raised_band_and_preamp(skin: &Skin, namespace: &str) {
    let state = EqualizerState {
        bands: [8, 31, 31, 45, 31, 31, 20, 31, 31, 55],
        preamp: 14,
        ..EqualizerState::default()
    };
    let shot = Shot(|| render_equalizer_window(&state, skin, &EqualizerLayout::CLASSIC));
    assert_golden(
        "equalizer_window_raised_band_preamp",
        namespace,
        &shot.render_image(),
    );
}

pub fn case_equalizer_window_curve_and_track_pixels(skin: &Skin) {
    let sheet = skin.sheet(rustamp_skin::Sheet::EqMain).unwrap();
    let sheet =
        rustamp_gfx::Image::from_rgba(sheet.width(), sheet.height(), sheet.as_rgba().to_vec())
            .unwrap();
    let image =
        render_equalizer_window(&EqualizerState::default(), skin, &EqualizerLayout::CLASSIC);
    for x in 0..113 {
        assert_eq!(image.pixel(86 + x, 25), sheet.pixel(115, 302));
        assert_eq!(image.pixel(86 + x, 26), sheet.pixel(x, 314));
    }
    for slider in EqSlider::ALL_BANDS {
        let x = EqualizerLayout::slider_x(slider);
        // Track sample at its unfilled left edge, body sample below the thumb.
        assert_eq!(image.pixel(x, 50), sheet.pixel(13, 241));
        assert_eq!(image.pixel(x + 2, 74), sheet.pixel(115, 303));
    }
}

pub fn case_equalizer_window_single_raised_band(skin: &Skin, namespace: &str) {
    let mut state = EqualizerState::default();
    state.bands[4] = 0;
    equalizer_golden(
        "equalizer_window_single_raised_band",
        namespace,
        skin,
        &state,
        (275, 116),
    );
}

pub fn case_equalizer_window_extremes(skin: &Skin, namespace: &str) {
    for (name, bands, preamp) in [
        ("equalizer_window_minimum", [0; 10], 0),
        ("equalizer_window_maximum", [63; 10], 63),
        (
            "equalizer_window_alternating",
            [0, 63, 0, 63, 0, 63, 0, 63, 0, 63],
            31,
        ),
        ("equalizer_window_preamp_minimum", [31; 10], 0),
        ("equalizer_window_preamp_maximum", [31; 10], 63),
    ] {
        let state = EqualizerState {
            bands,
            preamp,
            ..EqualizerState::default()
        };
        equalizer_golden(name, namespace, skin, &state, (275, 116));
    }
}

pub fn case_equalizer_window_mid_travel(skin: &Skin, namespace: &str) {
    let mut state = EqualizerState {
        preamp: 15,
        ..EqualizerState::default()
    };
    state.bands[4] = 46;
    equalizer_golden(
        "equalizer_window_mid_travel",
        namespace,
        skin,
        &state,
        (275, 116),
    );
    state.held = Some(EqWidget::Slider(EqSlider::Band(4)));
    state.thumb_dragged = true;
    equalizer_golden(
        "equalizer_window_mid_travel_dragged",
        namespace,
        skin,
        &state,
        (275, 116),
    );
    state = EqualizerState {
        held: Some(EqWidget::Slider(EqSlider::Preamp)),
        thumb_dragged: true,
        ..EqualizerState::default()
    };
    equalizer_golden(
        "equalizer_window_preamp_held",
        namespace,
        skin,
        &state,
        (275, 116),
    );
}

pub fn case_equalizer_window_held_buttons(skin: &Skin, namespace: &str) {
    for (name, widget, engaged) in [
        ("equalizer_window_on_held", EqWidget::On, false),
        ("equalizer_window_on_held_engaged", EqWidget::On, true),
        ("equalizer_window_auto_held", EqWidget::Auto, false),
        ("equalizer_window_auto_held_engaged", EqWidget::Auto, true),
    ] {
        let state = EqualizerState {
            held: Some(widget),
            enabled: widget != EqWidget::On || engaged,
            auto: widget == EqWidget::Auto && engaged,
            ..EqualizerState::default()
        };
        equalizer_golden(name, namespace, skin, &state, (275, 116));
    }
}

pub fn case_equalizer_window_focus_scale_and_provisional_shade(skin: &Skin, namespace: &str) {
    for (name, focused, shade, double_size, dimensions) in [
        (
            "equalizer_window_unfocused",
            false,
            false,
            false,
            (275, 116),
        ),
        (
            "equalizer_window_double_size",
            true,
            false,
            true,
            (550, 232),
        ),
        ("equalizer_window_shade", true, true, false, (275, 14)),
        (
            "equalizer_window_shade_unfocused",
            false,
            true,
            false,
            (275, 14),
        ),
        (
            "equalizer_window_shade_double_size",
            true,
            true,
            true,
            (550, 28),
        ),
    ] {
        let state = EqualizerState {
            focused,
            windowshade: shade,
            double_size,
            ..EqualizerState::default()
        };
        equalizer_golden(name, namespace, skin, &state, dimensions);
    }
}

pub fn case_playlist_window_selection(skin: &Skin, namespace: &str) {
    let mut window = PlaylistWindow::new();
    window.set_entries((0..60u64).map(EntryId).collect());
    window.scroll_to(56);
    // The second visible row (entry 57), through the model's public selection
    // API; `draw_rows` maps selected ids back to the visible row band.
    window.apply_select(&SelectOp::Set(vec![EntryId(57)]));
    let labels = vec![
        "57. Rustamp - the look-alike default skin".to_string(),
        "58. A second entry, so the rows are visible".to_string(),
        "59. 3:42".to_string(),
        "60. The last entry on the bottom page".to_string(),
    ];
    let shot = Shot(|| render_playlist_window(skin, &window, &labels, true));
    assert_golden("playlist_window_selection", namespace, &shot.render_image());
}

/// The playlist mid-list: the view scrolled to offset 28 of 60, so the
/// handle sits at its settled middle point (S01 §4.2: y = 40), with one row
/// selected through the page-key path (`move_selection_down`).
///
/// This pins the own-choice linear handle interpolation between the settled
/// points and the S03 §2 provisional Down behaviour on the same frame.
pub fn case_playlist_window_mid_page(skin: &Skin, namespace: &str) {
    let mut window = PlaylistWindow::new();
    window.set_entries((0..60u64).map(EntryId).collect());
    window.scroll_to(28);
    window.apply_select(&SelectOp::Set(vec![EntryId(28)]));
    // One Down step through the new key path: entry 29, view unchanged.
    window.move_selection_down();
    let labels = vec![
        "29. Rustamp - the look-alike default skin".to_string(),
        "30. A second entry, so the rows are visible".to_string(),
        "31. 3:42".to_string(),
        "32. The fourth row on this page".to_string(),
    ];
    let shot = Shot(|| render_playlist_window(skin, &window, &labels, true));
    assert_golden("playlist_window_mid_page", namespace, &shot.render_image());
}

/// The load-playlist dialog with a typed path: the face the BETA-09 UI half
/// types into (`DialogManager::type_text` on `LoadPlaylist`).
///
/// The body lines mirror what the app assembles for this dialog (the prompt
/// plus the field); the dialog frame itself is the renderer's own choice
/// (S04 open), pinned here at its 300 x 100 client size.
pub fn case_dialog_load_playlist(skin: &Skin, namespace: &str) {
    let body = ["Playlist file:", r"C:\Music\mix.m3u"];
    let shot = Shot(|| render_dialog(DialogKind::LoadPlaylist, skin, &body));
    assert_golden("dialog_load_playlist", namespace, &shot.render_image());
}

macro_rules! register_golden_tests {
    (
        $skin_fn:expr,
        $namespace:expr,
        $main_window:ident,
        $equalizer_window:ident,
        $playlist_window:ident,
        $main_window_playing:ident,
        $main_window_double_size:ident,
        $equalizer_window_raised_band_and_preamp:ident,
        $equalizer_window_curve_and_track_pixels:ident,
        $equalizer_window_single_raised_band:ident,
        $equalizer_window_extremes:ident,
        $equalizer_window_mid_travel:ident,
        $equalizer_window_held_buttons:ident,
        $equalizer_window_focus_scale_and_provisional_shade:ident,
        $playlist_window_selection:ident,
        $playlist_window_mid_page:ident,
        $dialog_load_playlist:ident $(,)?
    ) => {
        #[test]
        fn $main_window() {
            let skin = $skin_fn();
            $crate::golden_cases::case_main_window(&skin, $namespace);
        }

        #[test]
        fn $equalizer_window() {
            let skin = $skin_fn();
            $crate::golden_cases::case_equalizer_window(&skin, $namespace);
        }

        #[test]
        fn $playlist_window() {
            let skin = $skin_fn();
            $crate::golden_cases::case_playlist_window(&skin, $namespace);
        }

        #[test]
        fn $main_window_playing() {
            let skin = $skin_fn();
            $crate::golden_cases::case_main_window_playing(&skin, $namespace);
        }

        #[test]
        fn $main_window_double_size() {
            let skin = $skin_fn();
            $crate::golden_cases::case_main_window_double_size(&skin, $namespace);
        }

        #[test]
        fn $equalizer_window_raised_band_and_preamp() {
            let skin = $skin_fn();
            $crate::golden_cases::case_equalizer_window_raised_band_and_preamp(&skin, $namespace);
        }

        #[test]
        fn $equalizer_window_curve_and_track_pixels() {
            let skin = $skin_fn();
            $crate::golden_cases::case_equalizer_window_curve_and_track_pixels(&skin);
        }

        #[test]
        fn $equalizer_window_single_raised_band() {
            let skin = $skin_fn();
            $crate::golden_cases::case_equalizer_window_single_raised_band(&skin, $namespace);
        }

        #[test]
        fn $equalizer_window_extremes() {
            let skin = $skin_fn();
            $crate::golden_cases::case_equalizer_window_extremes(&skin, $namespace);
        }

        #[test]
        fn $equalizer_window_mid_travel() {
            let skin = $skin_fn();
            $crate::golden_cases::case_equalizer_window_mid_travel(&skin, $namespace);
        }

        #[test]
        fn $equalizer_window_held_buttons() {
            let skin = $skin_fn();
            $crate::golden_cases::case_equalizer_window_held_buttons(&skin, $namespace);
        }

        #[test]
        fn $equalizer_window_focus_scale_and_provisional_shade() {
            let skin = $skin_fn();
            $crate::golden_cases::case_equalizer_window_focus_scale_and_provisional_shade(
                &skin, $namespace,
            );
        }

        #[test]
        fn $playlist_window_selection() {
            let skin = $skin_fn();
            $crate::golden_cases::case_playlist_window_selection(&skin, $namespace);
        }

        #[test]
        fn $playlist_window_mid_page() {
            let skin = $skin_fn();
            $crate::golden_cases::case_playlist_window_mid_page(&skin, $namespace);
        }

        #[test]
        fn $dialog_load_playlist() {
            let skin = $skin_fn();
            $crate::golden_cases::case_dialog_load_playlist(&skin, $namespace);
        }
    };
}

pub(crate) use register_golden_tests;
