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
//! default main window at double size, the flat equalizer, a raised equalizer,
//! the empty playlist window and the playlist window with a selection.

use rustamp_core::command::SelectOp;
use rustamp_core::playlist::EntryId;
use rustamp_skin::{BuiltinDefaultSkin, MemorySource, Skin};
use rustamp_testkit::{SKIN_LOOKALIKE, Snapshot, assert_golden};
use rustamp_ui::{
    EqualizerLayout, EqualizerState, MainWindowLayout, MainWindowState, MonoStereo, PlaylistWindow,
    TransportState, frame_size, render_equalizer_window, render_main_window,
    render_playlist_window,
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
/// The response *curve* is not drawn by the renderer (S07 §7.3/§7.4 leave it
/// open), so this pins the graph background and the thumbs over it, not a
/// curve.
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
