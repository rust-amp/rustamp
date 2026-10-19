// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-dsp
//!
//! Winamp-style 10-band equalizer and preamp (spec S07), resampler and
//! limiter, as DspStage implementations.
//!
//! Codes against the frozen contracts in [`rustamp_core`]; see
//! `docs/rfc/0001-core-contracts.md`. The equalizer is [`eq::Equalizer`] and
//! the look-ahead true-peak limiter is [`limiter::Limiter`], which the engine
//! enables by default (owner decision, 2026-10-17): what the reference player
//! does with overs is unmeasured, so that default is our own choice and it
//! costs the look-ahead latency. A stage on its own is disabled, so a caller
//! that builds one directly still gets a bit-exact bypass — except that a
//! directly built [`Limiter`] defaults to enabled and must be switched off
//! explicitly. The output fade is [`fade::Fade`], an equal-power gain ramp
//! that the engine runs in the render callback; spec S18 does not exist, so
//! its law and its rules are our own choices. The resampler is **not** here:
//! it changes the frame count, which a `DspStage` cannot, so it lives in
//! `rustamp-audio`'s converter.

#![warn(missing_docs)]

pub mod eq;
pub mod fade;
pub mod limiter;

pub use eq::{BAND_CENTERS_HZ, BAND_Q, EqControl, Equalizer};
pub use fade::{Fade, FadeControl};
pub use limiter::{Limiter, LimiterControl};

/// Signal-domain input policy shared by the stages: a non-finite sample can
/// never be processed honestly, so it becomes silence. Every finite value —
/// including subnormals and DC — passes through unchanged; subnormal *history*
/// is flushed inside the stages instead (see `eq`), and the resampled path
/// flushes subnormal outputs in `rustamp-audio`'s converter.
pub(crate) fn sanitize_sample(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}
