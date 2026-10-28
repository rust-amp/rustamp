// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Test helpers for headless rendering: RGBA images, PNG I/O, pixel
//! comparison and golden-image assertions.
//!
//! UI components implement [`Snapshot`]; tests call [`assert_golden`] (or
//! [`assert_golden_snapshot`]). See `tests/golden/README.md` in the
//! repository for the workflow.
//!
//! [`control`] is the other half of the player's local control channel
//! (`rustamp --control`): a client for the typed JSON-lines named pipe, so a
//! test can drive and inspect the real binary instead of grepping its output.

pub mod control;
mod golden;
mod image;

pub use golden::{
    Diff, GoldenOutcome, Rect, SKIN_CLASSIC_BASE, SKIN_LOOKALIKE, Snapshot, assert_golden,
    assert_golden_snapshot, check_golden, compare, golden_dir_for, testdata_dir, workspace_root,
    write_diff_image,
};
pub use image::{ImageError, RgbaImage};
