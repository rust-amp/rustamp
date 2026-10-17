// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-audio
//!
//! The audio engine: decode thread -> lock-free ring -> output callback, with
//! transport, gapless playback, volume/balance and the visualization tap.
//!
//! ```no_run
//! use rustamp_audio::{Engine, EngineConfig};
//! use rustamp_core::bus::EventBus;
//! use rustamp_output::default_output;
//! # fn main() -> rustamp_core::Result<()> {
//! let engine = Engine::new(
//!     rustamp_decoders::default_registry(),
//!     default_output(),
//!     EventBus::new(),
//!     EngineConfig::default(),
//! )?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Pipeline
//!
//! ```text
//! decode thread:  Decoder -> Equalizer (S07) -> [DspStage]* -> Limiter -> FormatConverter -> rtrb::Producer
//! output thread:  rtrb::Consumer -> volume/balance -> OutputDevice
//! caller thread:  Engine (commands, events, audio clock, vis tap)
//! ```
//!
//! The ring is a lock-free SPSC buffer; the render callback fills the whole
//! device buffer, writes silence on underrun and neither allocates nor locks
//! nor logs. Position comes from [`rustamp_core::audio::OutputDevice::played_frames`].
//!
//! Modules:
//!
//! * [`engine`] — [`Engine`], transport, events, the audio clock.
//! * [`decode`] — the decode thread and the [`TrackSource`] queue.
//! * [`ring`] — the ring and the seek/flush handshake.
//! * [`convert`] — engine format -> device format (rate and channels).
//! * [`gain`] — the single place where volume/balance become gains.
//! * [`vistap`] — [`VisTap`], the 2 x 576 waveform tap (spectrum: spec S08).
//!
//! See `docs/rfc/0002-audio-contracts.md` for the `rustamp-core` changes this
//! crate needs.

#![warn(missing_docs)]

pub mod convert;
pub mod decode;
pub mod engine;
pub mod gain;
pub mod resident;
pub mod ring;
pub mod vistap;

pub use convert::FormatConverter;
pub use decode::TrackSource;
pub use engine::{Engine, EngineConfig};
#[cfg(windows)]
pub use resident::ProcessDecoder;
pub use resident::{HostDecoder, ResidentConfig, ResidentFactory};
pub use ring::{GainState, RingSync};
pub use vistap::VisTap;
