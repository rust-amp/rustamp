//! Error type for skin loading and decoding.

/// Errors raised while opening, reading or decoding a skin.
#[derive(Debug, thiserror::Error)]
pub enum SkinError {
    /// Filesystem or archive I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// The ZIP container could not be read.
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// The skin path does not exist or is not a supported container.
    #[error("not a skin source: {0}")]
    NotASkin(String),
    /// A BMP could not be decoded.
    #[error("bmp decode error in {name}: {reason}")]
    Bmp {
        /// The entry that failed to decode.
        name: String,
        /// Why it failed.
        reason: String,
    },
    /// An archive entry tried to escape the archive root.
    #[error("unsafe path in archive: {0}")]
    UnsafePath(String),
    /// The skin contains no readable files at all.
    #[error("empty skin: {0}")]
    Empty(String),
}

/// Result alias using [`SkinError`].
pub type Result<T, E = SkinError> = std::result::Result<T, E>;

impl From<SkinError> for rustamp_core::Error {
    fn from(value: SkinError) -> Self {
        match value {
            SkinError::Io(e) => rustamp_core::Error::Io(e),
            other => rustamp_core::Error::Other(other.to_string()),
        }
    }
}
