// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-dsp
//!
//! Winamp-style 10-band equalizer and preamp (spec S07), resampler and
//! limiter, as DspStage implementations.
//!
//! Codes against the frozen contracts in [`rustamp_core`]; see
//! `docs/rfc/0001-core-contracts.md`. The equalizer is [`eq::Equalizer`]; the
//! resampler and the limiter are not implemented yet.

#![warn(missing_docs)]

pub mod eq;

pub use eq::{BAND_CENTERS_HZ, BAND_Q, EqControl, Equalizer};
