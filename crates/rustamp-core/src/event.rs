//! The [`Event`] enum: notifications published by the player core.
//!
//! The definition lives in `rustamp-proto` (RFC 0004) and is re-exported here
//! at its original path, so existing callers do not change.
//!
//! Events describe *what happened*. UI, platform integrations and plugin
//! bridges subscribe via [`EventBus`](crate::bus::EventBus).

pub use rustamp_proto::event::Event;
