//! Error type for playlist reading and writing.
//!
//! The readers themselves are lenient: malformed input never fails, it is
//! skipped with a documented rule (see the module docs of [`crate::m3u`],
//! [`crate::pls`] and [`crate::xspf`]). Only two things can go wrong: the
//! underlying file I/O, and a format that is recognized but not supported.
//!
//! `Display`/`Error` are implemented by hand so this crate needs no dependency
//! beyond `rustamp-core`.

use std::fmt;

/// Errors raised while reading or writing a playlist.
#[derive(Debug)]
pub enum PlaylistError {
    /// The playlist file could not be read from or written to disk.
    Io(std::io::Error),
    /// The content is not a playlist we can read (unknown bytes), or the format
    /// is recognized but not implemented yet (B4S).
    UnsupportedFormat(String),
}

/// Result alias using [`PlaylistError`].
pub type Result<T, E = PlaylistError> = std::result::Result<T, E>;

impl fmt::Display for PlaylistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaylistError::Io(e) => write!(f, "playlist I/O error: {e}"),
            PlaylistError::UnsupportedFormat(msg) => write!(f, "unsupported playlist: {msg}"),
        }
    }
}

impl std::error::Error for PlaylistError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PlaylistError::Io(e) => Some(e),
            PlaylistError::UnsupportedFormat(_) => None,
        }
    }
}

impl From<std::io::Error> for PlaylistError {
    fn from(value: std::io::Error) -> Self {
        PlaylistError::Io(value)
    }
}

impl From<PlaylistError> for rustamp_core::Error {
    fn from(value: PlaylistError) -> Self {
        match value {
            PlaylistError::Io(e) => rustamp_core::Error::Io(e),
            PlaylistError::UnsupportedFormat(msg) => rustamp_core::Error::UnsupportedFormat(msg),
        }
    }
}
