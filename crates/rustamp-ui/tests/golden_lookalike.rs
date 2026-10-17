// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Golden images for the built-in skin, so the shipped default can be looked at
//! and compared pixel for pixel.
//!
//! The skin namespace is `lookalike` (`SKIN_LOOKALIKE`), whose goldens are
//! **committed** (`tests/golden/lookalike/`): they are our own artwork, unlike
//! the `classic-base` namespace, which holds the original skin's references and
//! must never be committed.
//!
//! Write or refresh them with:
//!
//! ```text
//! cargo xtask golden --skin lookalike --bless
//! ```
//!
//! and review the PNGs before committing. `cargo xtask golden` compares them
//! exactly (tolerance 0) and writes `target/golden-out/lookalike/<name>.diff.png`
//! on a mismatch.
//!
//! Every case pins one rendered state through the public `rustamp-ui` API only
//! (state-struct fields plus the window models' public methods); a state that
//! would need a private field is left out and reported instead. The cases are:
//! the default main window, it playing with a title/time/bitrate/stereo, the
//! default main window at double size, the equalizer's provisional curve and
//! body at flat/raised/extreme/mid-travel values, held/dragged/focus/scale and
//! provisional shade states, plus empty/selected playlists.

use rustamp_core::command::SelectOp;
use rustamp_core::playlist::EntryId;
use rustamp_skin::{BuiltinDefaultSkin, MemorySource, Skin};
use rustamp_testkit::{SKIN_LOOKALIKE, Snapshot, assert_golden};
use rustamp_ui::equalizer::{EqSlider, EqWidget};
use rustamp_ui::{
    DialogKind, EqualizerLayout, EqualizerState, MainWindowLayout, MainWindowState, MonoStereo,
    PlaylistWindow, TransportState, eq_frame_size, frame_size, render_dialog,
    render_equalizer_window, render_main_window, render_playlist_window,
};

/// The built-in skin, loaded the way the app loads it: an empty source with the
/// built-in skin as the fallback, so this is exactly what a first launch shows.
fn builtin_skin() -> Skin {
    let mut source = MemorySource::new("built-in");
    Skin::from_source(&mut source, &BuiltinDefaultSkin).expect("the built-in skin must load")
}

/// Renders on demand, because `Image::into_raw` consumes the image.
struct Shot<F: Fn() -> rustamp_gfx::Image>(F);

impl<F: Fn() -> rustamp_gfx::Image> Snapshot for Shot<F> {
    fn render_rgba(&self) -> (u32, u32, Vec<u8>) {
        (self.0)().into_raw()
    }
}

#[test]
fn golden_lookalike_main_window() {
    let skin = builtin_skin();
    let shot = Shot(|| {
        render_main_window(
            &MainWindowState::default(),
            &skin,
            &MainWindowLayout::CLASSIC,
        )
    });
    assert_golden("main_window", SKIN_LOOKALIKE, &shot.render_image());
}

#[test]
fn golden_lookalike_equalizer_window() {
    let skin = builtin_skin();
    let shot = Shot(|| {
        render_equalizer_window(&EqualizerState::default(), &skin, &EqualizerLayout::CLASSIC)
    });
    assert_golden("equalizer_window", SKIN_LOOKALIKE, &shot.render_image());
}

#[test]
fn golden_lookalike_playlist_window() {
    let skin = builtin_skin();
    let labels = vec![
        "1. Rustamp - the look-alike default skin".to_string(),
        "2. A second entry, so the rows are visible".to_string(),
        "3. 3:42".to_string(),
    ];
    let shot = Shot(|| render_playlist_window(&skin, &PlaylistWindow::default(), &labels, true));
    assert_golden("playlist_window", SKIN_LOOKALIKE, &shot.render_image());
}

/// A playing main window: a track title in the marquee (scrolled, so the
/// `marquee_offset` path is pinned too), a running time, a bitrate and sample
/// rate, a lit stereo label and a moved position and volume.
///
/// The state fields are the ones S02 measures: `transport` drives the play and
/// decoder indicators (§1), `time_seconds` the four time cells (§4), `kbps` /
/// `khz` the text fields (§5) and `channels` the mono/stereo labels (§6).
fn playing_state() -> MainWindowState {
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

#[test]
fn golden_lookalike_main_window_playing() {
    let skin = builtin_skin();
    let state = playing_state();
    let shot = Shot(|| render_main_window(&state, &skin, &MainWindowLayout::CLASSIC));
    assert_golden("main_window_playing", SKIN_LOOKALIKE, &shot.render_image());
}

/// The default state at double size: the exact 2x of `main_window`.
///
/// S05 §2.1 settles that double-size is a whole-window 2x scale (550 x 232 for
/// the main window); the same default state as `main_window` is framed here, so
/// the committed pair differs only by the scale.
#[test]
fn golden_lookalike_main_window_double_size() {
    let skin = builtin_skin();
    let state = MainWindowState {
        double_size: true,
        ..MainWindowState::default()
    };
    assert_eq!(frame_size(&state), (550, 232), "S05 §2.1 double-size size");
    let shot = Shot(|| render_main_window(&state, &skin, &MainWindowLayout::CLASSIC));
    assert_golden(
        "main_window_double_size",
        SKIN_LOOKALIKE,
        &shot.render_image(),
    );
}

/// A non-flat equalizer: the preamp sits above its flat centre and the bands
/// carry a raised/lowered profile, so every thumb lands off the flat row
/// (S01 §3.1: saved value 0 -> y 38, 31 -> y 63, 63 -> y 89).
///
/// This pins the own-choice polyline and slider body recipe, not a measured
/// classic response: S07 §7.1..§7.4 leave their exact laws open.
#[test]
fn golden_lookalike_equalizer_window_raised_band_and_preamp() {
    let skin = builtin_skin();
    let state = EqualizerState {
        // 0..=63, flat is 31: band 0 is raised (+8.9 dB), band 3 lowered
        // (-5.25 dB), bands 6 and 9 sit a little above and below the flat row
        // (`saved_to_gain_db`), so their thumbs are not on the flat row.
        bands: [8, 31, 31, 45, 31, 31, 20, 31, 31, 55],
        preamp: 14,
        ..EqualizerState::default()
    };
    let shot = Shot(|| render_equalizer_window(&state, &skin, &EqualizerLayout::CLASSIC));
    assert_golden(
        "equalizer_window_raised_band_preamp",
        SKIN_LOOKALIKE,
        &shot.render_image(),
    );
}

fn equalizer_golden(name: &str, state: &EqualizerState, dimensions: (u32, u32)) {
    let skin = builtin_skin();
    assert_eq!(eq_frame_size(state), dimensions);
    let image = render_equalizer_window(state, &skin, &EqualizerLayout::CLASSIC);
    assert_eq!((image.width(), image.height()), dimensions);
    if state.double_size {
        let single = EqualizerState {
            double_size: false,
            ..state.clone()
        };
        let single = render_equalizer_window(&single, &skin, &EqualizerLayout::CLASSIC);
        assert_eq!(image, single.scaled(2), "exact whole-frame pixel doubling");
    }
    let shot = Shot(|| image.clone());
    assert_golden(name, SKIN_LOOKALIKE, &shot.render_image());
}

/// Flat band control points occupy row8 (own choice); preamp independently
/// occupies row9, sampled across the entire source strip rather than one colour.
#[test]
fn golden_lookalike_equalizer_window_curve_and_track_pixels() {
    let skin = builtin_skin();
    let sheet = skin.sheet(rustamp_skin::Sheet::EqMain).unwrap();
    let sheet =
        rustamp_gfx::Image::from_rgba(sheet.width(), sheet.height(), sheet.as_rgba().to_vec())
            .unwrap();
    let image =
        render_equalizer_window(&EqualizerState::default(), &skin, &EqualizerLayout::CLASSIC);
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
    // The existing flat case owns the PNG; do not race its bless write here.
}

#[test]
fn golden_lookalike_equalizer_window_single_raised_band() {
    let mut state = EqualizerState::default();
    state.bands[4] = 0;
    equalizer_golden("equalizer_window_single_raised_band", &state, (275, 116));
}

#[test]
fn golden_lookalike_equalizer_window_extremes() {
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
        equalizer_golden(name, &state, (275, 116));
    }
}

#[test]
fn golden_lookalike_equalizer_window_mid_travel() {
    let mut state = EqualizerState {
        preamp: 15,
        ..EqualizerState::default()
    };
    state.bands[4] = 46;
    equalizer_golden("equalizer_window_mid_travel", &state, (275, 116));
    state.held = Some(EqWidget::Slider(EqSlider::Band(4)));
    state.thumb_dragged = true;
    equalizer_golden("equalizer_window_mid_travel_dragged", &state, (275, 116));
    // S07 §8.2: preamp's dragged sprite also applies on a lone down, before
    // motion. Render the flat value to isolate that sprite from body movement.
    state = EqualizerState {
        held: Some(EqWidget::Slider(EqSlider::Preamp)),
        thumb_dragged: true,
        ..EqualizerState::default()
    };
    equalizer_golden("equalizer_window_preamp_held", &state, (275, 116));
}

#[test]
fn golden_lookalike_equalizer_window_held_buttons() {
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
        equalizer_golden(name, &state, (275, 116));
    }
}

#[test]
fn golden_lookalike_equalizer_window_focus_scale_and_provisional_shade() {
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
        equalizer_golden(name, &state, dimensions);
    }
}

/// The playlist at its bottom page with one row selected, so the selection
/// band (the `SelectedBG` fill of S01 §4) is pinned on a row other than the
/// first and the scroll handle sits at its lowest settled point (S01 §4.2:
/// y = 60 for a 60-entry list).
#[test]
fn golden_lookalike_playlist_window_selection() {
    let skin = builtin_skin();
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
    let shot = Shot(|| render_playlist_window(&skin, &window, &labels, true));
    assert_golden(
        "playlist_window_selection",
        SKIN_LOOKALIKE,
        &shot.render_image(),
    );
}

/// The playlist mid-list: the view scrolled to offset 28 of 60, so the
/// handle sits at its settled middle point (S01 §4.2: y = 40), with one row
/// selected through the page-key path (`move_selection_down`).
///
/// This pins the own-choice linear handle interpolation between the settled
/// points and the S03 §2 provisional Down behaviour on the same frame.
#[test]
fn golden_lookalike_playlist_window_mid_page() {
    let skin = builtin_skin();
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
    let shot = Shot(|| render_playlist_window(&skin, &window, &labels, true));
    assert_golden(
        "playlist_window_mid_page",
        SKIN_LOOKALIKE,
        &shot.render_image(),
    );
}

/// The load-playlist dialog with a typed path: the face the BETA-09 UI half
/// types into (`DialogManager::type_text` on `LoadPlaylist`).
///
/// The body lines mirror what the app assembles for this dialog (the prompt
/// plus the field); the dialog frame itself is the renderer's own choice
/// (S04 open), pinned here at its 300 x 100 client size.
#[test]
fn golden_lookalike_dialog_load_playlist() {
    let skin = builtin_skin();
    let body = ["Playlist file:", r"C:\Music\mix.m3u"];
    let shot = Shot(|| render_dialog(DialogKind::LoadPlaylist, &skin, &body));
    assert_golden("dialog_load_playlist", SKIN_LOOKALIKE, &shot.render_image());
}
