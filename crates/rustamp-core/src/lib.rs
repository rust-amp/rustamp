//! # rustamp-core
//!
//! The **contract crate** of Rustamp. It contains the domain types, the
//! [`Command`](command::Command) / [`Event`](event::Event) enums, the
//! [`PlayerState`](state::PlayerState) snapshot, the audio traits
//! ([`Decoder`](audio::Decoder), [`OutputDevice`](audio::OutputDevice),
//! [`DspStage`](audio::DspStage)), the playlist model, metadata, typed
//! configuration, the in-process message bus and the error type.
//!
//! This crate depends only on the permissive `rustamp-proto` crate (RFC 0004),
//! which holds the wire types it re-exports, and is kept small so it also
//! builds for `i686-pc-windows-msvc`. Its public API is frozen by
//! `docs/rfc/0001-core-contracts.md`: changes require a new RFC.
//!
//! Conventions used throughout:
//! * Positions and durations on the wire (commands, events, state) are `u64`
//!   milliseconds; the audio traits use [`std::time::Duration`].
//! * Indices that cross crate or process boundaries are `u32` (never `usize`),
//!   so wire types are identical on x64 and i686.
//! * Volume is `u8` in `0..=255` (Winamp style), balance is `i8` in `-127..=127`.
//! * Audio is interleaved `f32` in `[-1.0, 1.0]`.
//!
//! Enable the `mock` feature for [`mock`] test doubles.

pub mod audio;
pub mod bus;
pub mod command;
pub mod config;
pub mod error;
pub mod event;
pub mod meta;
pub mod playlist;
pub mod state;
pub mod vis;

#[cfg(feature = "mock")]
pub mod mock;

pub use error::{Error, Result};
