//! The [`Command`] enum: every user or plugin intent that mutates the player.
//!
//! The definitions live in `rustamp-proto` (RFC 0004) and are re-exported here
//! at their original paths, so existing callers do not change.
//!
//! Commands are *requests*; the app/engine applies them and answers with
//! [`Event`](crate::event::Event)s. They are `serde`-serializable so that the
//! plugin host can forward them over IPC. Indices are `u32`; entries are
//! referenced by stable [`EntryId`](crate::playlist::EntryId) where possible.
//!
//! **Re-export rule** (RFC 0008 §3): this module re-exports every public item of
//! the wire types it re-exports, not a hand-picked subset. A re-exported variant
//! drags its payload types with it: [`Command::OpenMedia`],
//! [`Command::OpenJump`] and [`Command::OpenPrefsPage`] name [`OpenMediaKind`],
//! [`JumpTarget`] and [`PrefsPage`], so a consumer that matches on those variants
//! must be able to name them here too. When a later change adds a payload type
//! to a re-exported variant, this list grows in the same change;
//! `tests/command_reexports.rs` stops compiling if it does not.

pub use rustamp_proto::command::{Command, JumpTarget, OpenMediaKind, PrefsPage, SelectOp};
