// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Golden images for the lookalike skin, so the classic look can be compared
//! pixel for pixel.
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

mod golden_cases;

use golden_cases::{SKIN_LOOKALIKE, lookalike_skin, register_golden_tests};

#[test]
fn golden_lookalike_visualization() {
    golden_cases::case_visualization(&lookalike_skin(), SKIN_LOOKALIKE);
}

register_golden_tests!(
    lookalike_skin,
    SKIN_LOOKALIKE,
    golden_lookalike_main_window,
    golden_lookalike_equalizer_window,
    golden_lookalike_playlist_window,
    golden_lookalike_main_window_playing,
    golden_lookalike_main_window_double_size,
    golden_lookalike_equalizer_window_raised_band_and_preamp,
    golden_lookalike_equalizer_window_curve_and_track_pixels,
    golden_lookalike_equalizer_window_single_raised_band,
    golden_lookalike_equalizer_window_extremes,
    golden_lookalike_equalizer_window_mid_travel,
    golden_lookalike_equalizer_window_held_buttons,
    golden_lookalike_equalizer_window_focus_scale_and_provisional_shade,
    golden_lookalike_playlist_window_selection,
    golden_lookalike_playlist_window_mid_page,
    golden_lookalike_dialog_load_playlist,
);
