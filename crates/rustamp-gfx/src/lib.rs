// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-gfx
//!
//! The CPU pixel compositor behind Rustamp's skin rendering.
//!
//! The crate is deliberately independent of `rustamp-skin`: it operates on
//! already-decoded [`Image`] buffers and sprite [`Rect`]angles, never on skin
//! paths or archives. That keeps the compositor usable by any window, the
//! plugin host's `gen.bmp` frames and the test harness alike.
//!
//! Two building blocks:
//!
//! * [`Image`] — an 8-bit RGBA canvas with clipping blits, alpha and colour-key
//!   compositing, fills and exact integer scaling. No operation panics on an
//!   out-of-bounds rectangle; it clips instead.
//! * [`font`] — bitmap fonts driven by sprite cells: [`BitmapFont`] for
//!   `text.bmp` and [`NumberFont`] for the `numbers`/`nums_ex` time display.
//!
//! Codes against the frozen core contract in [`rustamp_core`]; see
//! `docs/rfc/0001-core-contracts.md`.

#![forbid(unsafe_code)]

pub mod font;
pub mod image;

pub use font::{BitmapFont, GlyphMap, NumberFont};
pub use image::{GfxError, Image, Rect};
