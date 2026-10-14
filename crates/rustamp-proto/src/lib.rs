//! # rustamp-proto
//!
//! The **wire types** that cross the process boundary between `rustamp.exe`
//! (x64) and `rustamp-plughost32.exe` (i686). They were extracted out of
//! `rustamp-core` by RFC 0004 so that the plugin-facing crates can be
//! permissively licensed; `rustamp-core` re-exports every item at its original
//! path (`rustamp_core::vis::VisFrame`, `rustamp_core::command::Command`, ...),
//! so no caller changes.
//!
//! Licensed **`MIT OR Apache-2.0`**: see `LICENSE-MIT` and `LICENSE-APACHE` at
//! the repository root. This crate contains only Rustamp's own code and has no
//! internal dependencies.
//!
//! Nothing here encodes anything by itself: the `postcard` encoding and the
//! length-prefixed framing live in `rustamp-ipc`. The `serde` derives are part
//! of the frozen wire format (protocol version 1); changing them changes the
//! wire and needs a new RFC.
//!
//! Conventions kept from `rustamp-core`:
//! * Positions and durations on the wire are `u64` milliseconds.
//! * Indices that cross crate or process boundaries are `u32` (never `usize`),
//!   so wire types are identical on x64 and i686.
//! * Volume is `u8` in `0..=255` (Winamp style), balance is `i8` in `-127..=127`.

pub mod command;
pub mod error;
pub mod event;
pub mod meta;
pub mod playlist;
pub mod state;
pub mod vis;
