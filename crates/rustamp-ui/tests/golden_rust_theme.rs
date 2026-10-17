// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Golden images for the built-in default Rust-themed skin, so the shipped
//! default can be looked at and compared pixel for pixel.
//!
//! The skin namespace is `rust-theme` (`SKIN_RUST_THEME`), whose goldens are
//! **committed** (`tests/golden/rust-theme/`): they are our own artwork, unlike
//! the `classic-base` namespace, which holds the original skin's references and
//! must never be committed.
//!
//! Write or refresh them with:
//!
//! ```text
//! cargo xtask golden --skin rust-theme --bless
//! ```
//!
//! and review the PNGs before committing. `cargo xtask golden` compares them
//! exactly (tolerance 0) and writes `target/golden-out/rust-theme/<name>.diff.png`
//! on a mismatch.

mod golden_cases;

use golden_cases::{SKIN_RUST_THEME, register_golden_tests, rust_theme_skin};

register_golden_tests!(
    rust_theme_skin,
    SKIN_RUST_THEME,
    golden_rust_theme_main_window,
    golden_rust_theme_equalizer_window,
    golden_rust_theme_playlist_window,
    golden_rust_theme_main_window_playing,
    golden_rust_theme_main_window_double_size,
    golden_rust_theme_equalizer_window_raised_band_and_preamp,
    golden_rust_theme_equalizer_window_curve_and_track_pixels,
    golden_rust_theme_equalizer_window_single_raised_band,
    golden_rust_theme_equalizer_window_extremes,
    golden_rust_theme_equalizer_window_mid_travel,
    golden_rust_theme_equalizer_window_held_buttons,
    golden_rust_theme_equalizer_window_focus_scale_and_provisional_shade,
    golden_rust_theme_playlist_window_selection,
);
