//! # rustamp-decoders
//!
//! Concrete [`Decoder`](rustamp_core::audio::Decoder) implementations
//! registered into the core
//! [`DecoderRegistry`](rustamp_core::audio::DecoderRegistry).
//!
//! * [`symphonia_dec`]: pure-Rust decoding of MP3/MP2/MP1, FLAC, Ogg Vorbis,
//!   WAV/PCM, AIFF, CAF, MP4/M4A (AAC, ALAC), raw ADTS AAC and Matroska audio.
//! * Later FFI decoders (libopenmpt, game-music-emu, ...) are added in
//!   [`default_registry`].
//!
//! See `CONTRACT_NOTES.md` for behaviour the core contract leaves open.

use std::sync::Arc;

use rustamp_core::audio::DecoderRegistry;

pub mod symphonia_dec;

pub use symphonia_dec::{SymphoniaDecoder, SymphoniaDecoderFactory};

/// Registry with every decoder this crate provides, in priority order.
///
/// Register future FFI decoders here; on equal probe confidence the earlier
/// registration wins (RFC 0013).
pub fn default_registry() -> DecoderRegistry {
    let mut reg = DecoderRegistry::new();
    reg.register(Arc::new(SymphoniaDecoderFactory::new()));
    reg
}
