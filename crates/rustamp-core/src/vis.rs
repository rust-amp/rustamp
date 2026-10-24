//! Visualization data tap contract.
//!
//! The definitions live in `rustamp-proto` (RFC 0004) and are re-exported here
//! at their original paths, so existing callers do not change.
//!
//! The audio engine publishes [`VisFrame`]s (a recent window of audio plus a
//! spectrum) to the visualization consumers. The exact Winamp derivation of
//! plugin-facing data - 8-bit scaling of the waveform, FFT size/window,
//! spectrum bin mapping and scaling, refresh cadence - is defined by spec
//! **S08**, and the conversion lives in `rustamp-vis`. Until then the fields
//! here carry *neutral* data: waveform as `f32` in `[-1, 1]`, spectrum as
//! linear magnitude normalised so a full-scale sine is about `1.0`.
//! Do not bake S08 assumptions into other crates.

pub use rustamp_proto::vis::{VIS_CHANNELS, VIS_SAMPLES, VisFrame};
