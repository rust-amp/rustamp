//! # rustamp-decoders
//!
//! Concrete [`Decoder`](rustamp_core::audio::Decoder) implementations
//! registered into the core
//! [`DecoderRegistry`](rustamp_core::audio::DecoderRegistry).
//!
//! * [`symphonia_dec`]: pure-Rust decoding of MP3/MP2/MP1, FLAC, Ogg Vorbis,
//!   Ogg Opus, WAV/PCM, AIFF, CAF, MP4/M4A (AAC, ALAC), raw ADTS AAC and
//!   Matroska audio.
//! * [`opus_dec`]: the Opus codec for the symphonia pipeline. Symphonia ships
//!   no Opus codec, so this crate brings its own (see the module docs).
//! * [`wave_dec`]: a dependency-free RIFF/WAVE PCM decoder (the classic Winamp
//!   `in_wave` format family). It overlaps symphonia on purpose - the same
//!   extensions (`wav`, `wave`) and the same content signature (`RIFF`/`WAVE`)
//!   at the same confidence - so RFC 0013's selection rule has two real
//!   claimants in [`default_registry`] instead of a test double.
//! * [`http_dec`]: a decoder over an `http://` stream (S19). The request it
//!   sends and the failures it names are in [`http_source`].
//! * Later FFI decoders (libopenmpt, game-music-emu, ...) are added in
//!   [`default_registry`].
//!
//! See `CONTRACT_NOTES.md` for behaviour the core contract leaves open.

use std::sync::Arc;

use rustamp_core::audio::DecoderRegistry;

pub mod http_dec;
pub mod http_source;
pub mod opus_dec;
pub mod symphonia_dec;
pub mod wave_dec;

mod aiff_header;
mod flac_header;
mod mp4_header;
mod mpeg_header;
mod ogg_header;

#[cfg(test)]
mod probe_fuzz;

pub use http_dec::HttpDecoder;
pub use http_source::HttpError;
pub use symphonia_dec::{SymphoniaDecoder, SymphoniaDecoderFactory};
pub use wave_dec::{WaveDecoder, WaveDecoderFactory};

/// Registry with every decoder this crate provides, in priority order.
///
/// Registration order *is* the RFC 0013 tie-break, so it is part of the
/// contract and not an accident of the call order here:
///
/// 1. [`SymphoniaDecoderFactory`] stays first. For the media both factories
///    claim (a `RIFF`/`WAVE` signature or a `.wav`/`.wave` extension) it is the
///    more complete decoder - it also reads ADPCM and other WAV codecs - so the
///    default behaviour for WAV is unchanged by the second factory.
/// 2. [`WaveDecoderFactory`] is the second claimant and the fallback that does
///    not route the most basic format through the symphonia dependency.
///
/// Register future FFI decoders here as well; on equal probe confidence the
/// earlier registration wins (RFC 0013).
pub fn default_registry() -> DecoderRegistry {
    let mut reg = DecoderRegistry::new();
    reg.register(Arc::new(SymphoniaDecoderFactory::new()));
    reg.register(Arc::new(WaveDecoderFactory::new()));
    reg
}
