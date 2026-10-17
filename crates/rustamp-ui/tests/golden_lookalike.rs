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

use rustamp_skin::{BuiltinDefaultSkin, MemorySource, Skin};
use rustamp_testkit::{SKIN_LOOKALIKE, Snapshot, assert_golden};
use rustamp_ui::{
    EqualizerLayout, EqualizerState, MainWindowLayout, MainWindowState, PlaylistWindow,
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
