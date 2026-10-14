//! Player state snapshot types.
//!
//! The definitions live in `rustamp-proto` (RFC 0004) and are re-exported here
//! at their original paths, so existing callers do not change.

pub use rustamp_proto::state::{
    EQ_BANDS, EqState, PlayerState, StreamInfo, TransportState, WindowId,
};
