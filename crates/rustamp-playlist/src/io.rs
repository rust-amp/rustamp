//! Top-level playlist reading and writing.
//!
//! [`read_playlist`] is the entry point the task card asks for: it takes the
//! file's bytes and the directory the file lives in, detects the format from
//! the content and returns the entries. The variants add an extension hint
//! (needed to tell `.m3u` from `.m3u8`) or read straight from disk.

use std::path::Path;

use crate::entry::LoadedEntry;
use crate::error::{PlaylistError, Result};
use crate::format::{PlaylistFormat, detect};

/// Message used for the formats this crate recognizes but does not implement.
const B4S_UNSUPPORTED: &str = "B4S is not implemented: it is a Winamp-specific XML dialect \
     with no published format description and no free reference implementation available in this \
     repository. A spec is needed; see crates/rustamp-playlist/CONTRACT_NOTES.md";

/// Reads a playlist, detecting the format from the content.
///
/// `base` is the directory relative locations resolve against — in practice
/// the parent directory of the playlist file. The process working directory is
/// never consulted.
///
/// Because there is no file name, `.m3u` and `.m3u8` cannot be told apart
/// here; both decode UTF-8 first and fall back to Windows-1252, which only
/// differs for a `.m3u8` that contains invalid UTF-8. Use
/// [`read_playlist_path`] when the file name is known.
pub fn read_playlist(bytes: &[u8], base: &Path) -> Result<Vec<LoadedEntry>> {
    read_playlist_with_hint(bytes, base, None)
}

/// Like [`read_playlist`], with a file path used as a format hint.
///
/// `base` stays the resolution directory; only `path_hint`'s extension is
/// consulted.
pub fn read_playlist_with_hint(
    bytes: &[u8],
    base: &Path,
    path_hint: Option<&Path>,
) -> Result<Vec<LoadedEntry>> {
    let format = detect(bytes, path_hint).ok_or_else(|| {
        PlaylistError::UnsupportedFormat(
            "unrecognized playlist content: no M3U, PLS or XSPF marker and not text".to_owned(),
        )
    })?;
    read_playlist_as(format, bytes, base)
}

/// Reads a playlist from `path`'s bytes: the format hint comes from `path` and
/// relative locations resolve against `path`'s directory.
pub fn read_playlist_path(bytes: &[u8], path: &Path) -> Result<Vec<LoadedEntry>> {
    let base = path.parent().unwrap_or_else(|| Path::new(""));
    read_playlist_with_hint(bytes, base, Some(path))
}

/// Reads a playlist file from disk.
pub fn load_playlist_file(path: &Path) -> Result<Vec<LoadedEntry>> {
    let bytes = std::fs::read(path)?;
    read_playlist_path(&bytes, path)
}

/// Reads a playlist with an explicit format, skipping detection.
pub fn read_playlist_as(
    format: PlaylistFormat,
    bytes: &[u8],
    base: &Path,
) -> Result<Vec<LoadedEntry>> {
    match format {
        PlaylistFormat::M3u => Ok(crate::m3u::read(bytes, base)),
        PlaylistFormat::M3u8 => Ok(crate::m3u::read_utf8(bytes, base)),
        PlaylistFormat::Pls => Ok(crate::pls::read(bytes, base)),
        PlaylistFormat::Xspf => Ok(crate::xspf::read(bytes, base)),
        PlaylistFormat::B4s => Err(PlaylistError::UnsupportedFormat(B4S_UNSUPPORTED.to_owned())),
    }
}

/// Writes entries in `format` and returns the file bytes.
///
/// M3U/M3U8 and PLS use CRLF line endings, XSPF uses LF; everything is UTF-8.
/// B4S is recognized but not implemented and returns an error.
pub fn write_playlist(format: PlaylistFormat, entries: &[LoadedEntry]) -> Result<Vec<u8>> {
    match format {
        PlaylistFormat::M3u | PlaylistFormat::M3u8 => Ok(crate::m3u::write(entries)),
        PlaylistFormat::Pls => Ok(crate::pls::write(entries)),
        PlaylistFormat::Xspf => Ok(crate::xspf::write(entries)),
        PlaylistFormat::B4s => Err(PlaylistError::UnsupportedFormat(B4S_UNSUPPORTED.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rustamp_core::playlist::MediaLocation;

    use super::*;

    fn base() -> PathBuf {
        PathBuf::from("C:/Music/Lists")
    }

    #[test]
    fn detection_drives_the_reader() {
        let m3u = read_playlist(b"#EXTM3U\r\n#EXTINF:1,A\r\na.mp3\r\n", &base()).unwrap();
        assert_eq!(m3u[0].title.as_deref(), Some("A"));

        let pls = read_playlist(b"[playlist]\r\nFile1=a.mp3\r\n", &base()).unwrap();
        assert_eq!(pls.len(), 1);

        let xspf = read_playlist(
            b"<playlist><track><location>a.mp3</location></track></playlist>",
            &base(),
        )
        .unwrap();
        assert_eq!(xspf.len(), 1);
    }

    #[test]
    fn a_hint_refines_m3u_to_m3u8() {
        let mut bytes = b"#EXTM3U\r\n#EXTINF:1,M".to_vec();
        bytes.push(0xFF);
        bytes.extend_from_slice(b"\r\na.mp3\r\n");
        let without = read_playlist(&bytes, &base()).unwrap();
        let with = read_playlist_with_hint(&bytes, &base(), Some(Path::new("p.m3u8"))).unwrap();
        assert_ne!(without[0].title, with[0].title);
        assert_eq!(with[0].title.as_deref(), Some("M\u{FFFD}"));
    }

    #[test]
    fn unknown_bytes_are_an_error() {
        let mut blob = vec![0u8; 512];
        blob.extend_from_slice(b"\x89PNG");
        assert!(matches!(
            read_playlist(&blob, &base()),
            Err(PlaylistError::UnsupportedFormat(_))
        ));
    }

    #[test]
    fn b4s_is_recognized_but_rejected() {
        let bytes = b"<WinampXML><playlist><entry/></playlist></WinampXML>";
        assert!(matches!(
            read_playlist(bytes, &base()),
            Err(PlaylistError::UnsupportedFormat(_))
        ));
        assert!(matches!(
            write_playlist(PlaylistFormat::B4s, &[]),
            Err(PlaylistError::UnsupportedFormat(_))
        ));
        assert!(!PlaylistFormat::B4s.is_supported());
    }

    #[test]
    fn path_based_reading_uses_the_parent_directory() {
        let entries = read_playlist_path(b"a.mp3\n", Path::new("C:/Music/Lists/list.m3u")).unwrap();
        assert_eq!(
            entries[0].location,
            MediaLocation::File(PathBuf::from("C:/Music/Lists/a.mp3"))
        );
    }

    #[test]
    fn error_converts_to_the_core_error() {
        let err = PlaylistError::UnsupportedFormat("x".into());
        assert!(matches!(
            rustamp_core::Error::from(err),
            rustamp_core::Error::UnsupportedFormat(_)
        ));
        assert_eq!(
            PlaylistError::Io(std::io::Error::other("boom")).to_string(),
            "playlist I/O error: boom"
        );
    }
}
