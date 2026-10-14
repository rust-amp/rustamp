//! The [`Command`] enum: every user or plugin intent that mutates the player.
//!
//! The definitions live in `rustamp-proto` (RFC 0004) and are re-exported here
//! at their original paths, so existing callers do not change.
//!
//! Commands are *requests*; the app/engine applies them and answers with
//! [`Event`](crate::event::Event)s. They are `serde`-serializable so that the
//! plugin host can forward them over IPC. Indices are `u32`; entries are
//! referenced by stable [`EntryId`](crate::playlist::EntryId) where possible.

pub use rustamp_proto::command::{Command, SelectOp};
