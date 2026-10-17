// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-dsp
//!
//! Winamp-style 10-band equalizer and preamp (spec S07), resampler and
//! limiter, as DspStage implementations.
//!
//! Codes against the frozen contracts in [`rustamp_core`]; see
//! `docs/rfc/0001-core-contracts.md`. The equalizer is [`eq::Equalizer`] and
//! the look-ahead limiter is [`limiter::Limiter`] (disabled by default, so the
//! pipeline stays bit-transparent until a caller enables it). The resampler is
//! **not** here: it changes the frame count, which a `DspStage` cannot, so it
//! lives in `rustamp-audio`'s converter.

#![warn(missing_docs)]

pub mod eq;
pub mod limiter;

pub use eq::{BAND_CENTERS_HZ, BAND_Q, EqControl, Equalizer};
pub use limiter::{Limiter, LimiterControl};
