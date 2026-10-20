//! Top-level playlist reading and writing.
//!
//! [`read_playlist`] is the entry point the task card asks for: it takes the
//! file's bytes and the directory the file lives in, detects the format from
//! the content and returns the entries. The variants add an extension hint
//! (needed to tell `.m3u` from `.m3u8`) or read straight from disk.
//!
//! [`import_playlist_file`] and [`import_playlist_path`] are the *import*
//! side: they do the same detection and return the detected format together
//! with the entries, so a caller that offers "save the playlist" can write it
//! back in the format it came in.

use std::path::Path;

use crate::entry::{LoadedEntry, PlaylistReport};
use crate::error::{PlaylistError, Result};
use crate::format::{PlaylistFormat, detect, looks_like_text, sniff, starts_like_xml};

/// Message used for the formats this crate recognizes but does not implement.
const B4S_UNSUPPORTED: &str = "B4S is not implemented: it is a Winamp-specific XML dialect \
     with no published format description and no free reference implementation available in this \
     repository. A spec is needed; see crates/rustamp-playlist/CONTRACT_NOTES.md";

/// A playlist that was imported: the format it was detected as and the
/// result of reading it.
///
/// The format is part of the result because a caller that writes the playlist
/// back wants to use the format it came in; the readers on their own cannot
/// report it (contract note S06 in `CONTRACT_NOTES.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedPlaylist {
    /// The format the content was detected as.
    pub format: PlaylistFormat,
    /// The surviving entries and the ones that were skipped.
    pub report: PlaylistReport,
}

/// Reads a playlist, detecting the format from the content.
///
/// Returns only surviving valid entries; skipped entries are silently dropped.
/// Use [`read_playlist_report`] to receive both surviving entries and a list of
/// skipped entries with reasons.
pub fn read_playlist(bytes: &[u8], base: &Path) -> Result<Vec<LoadedEntry>> {
    read_playlist_report(bytes, base).map(PlaylistReport::into_entries)
}

/// Reads a playlist, detecting the format from the content, and returns both
/// surviving entries and a list of skipped entries with reasons.
pub fn read_playlist_report(bytes: &[u8], base: &Path) -> Result<PlaylistReport> {
    read_playlist_report_with_hint(bytes, base, None)
}

/// Like [`read_playlist`], with a file path used as a format hint.
pub fn read_playlist_with_hint(
    bytes: &[u8],
    base: &Path,
    path_hint: Option<&Path>,
) -> Result<Vec<LoadedEntry>> {
    read_playlist_report_with_hint(bytes, base, path_hint).map(PlaylistReport::into_entries)
}

/// Like [`read_playlist_report`], with a file path used as a format hint.
pub fn read_playlist_report_with_hint(
    bytes: &[u8],
    base: &Path,
    path_hint: Option<&Path>,
) -> Result<PlaylistReport> {
    let format = detect_or_error(bytes, path_hint)?;
    read_playlist_report_as(format, bytes, base)
}

/// Detects the format or reports the error the readers use for content that is
/// not a playlist.
fn detect_or_error(bytes: &[u8], path_hint: Option<&Path>) -> Result<PlaylistFormat> {
    detect(bytes, path_hint).ok_or_else(|| {
        let extension = path_hint
            .and_then(Path::extension)
            .map(|e| format!(" (extension `.{}`)", e.to_string_lossy()))
            .unwrap_or_default();
        PlaylistError::UnsupportedFormat(format!(
            "unrecognized playlist content{extension}: no M3U, PLS or XSPF marker, and the data \
             is binary, UTF-16, XML with another root, or text behind an extension that is not \
             a playlist format"
        ))
    })
}

/// Reads a playlist from `path`'s bytes: the format hint comes from `path` and
/// relative locations resolve against `path`'s directory.
pub fn read_playlist_path(bytes: &[u8], path: &Path) -> Result<Vec<LoadedEntry>> {
    read_playlist_report_path(bytes, path).map(PlaylistReport::into_entries)
}

/// Reads a playlist from `path`'s bytes, returning both surviving entries and
/// any skipped entries with reasons.
pub fn read_playlist_report_path(bytes: &[u8], path: &Path) -> Result<PlaylistReport> {
    let base = path.parent().unwrap_or_else(|| Path::new(""));
    read_playlist_report_with_hint(bytes, base, Some(path))
}

/// Reads a playlist file from disk.
pub fn load_playlist_file(path: &Path) -> Result<Vec<LoadedEntry>> {
    load_playlist_file_report(path).map(PlaylistReport::into_entries)
}

/// Reads a playlist file from disk, returning surviving entries and skipped entries.
pub fn load_playlist_file_report(path: &Path) -> Result<PlaylistReport> {
    let bytes = std::fs::read(path)?;
    read_playlist_report_path(&bytes, path)
}

/// Imports a playlist file from disk and reports the format it was read as.
///
/// The format is detected from the content and the file name (see
/// [`crate::format::detect`]): a content marker wins over the extension, and
/// the extension only tells `.m3u` from `.m3u8`. Content that is not a
/// playlist at all is an error — never a panic and never a silently empty
/// list.
pub fn import_playlist_file(path: &Path) -> Result<ImportedPlaylist> {
    let bytes = std::fs::read(path)?;
    import_playlist_path(&bytes, path)
}

/// Imports a playlist from `bytes` and reports the format it was read as.
///
/// `path` is the file the bytes came from: its extension is the format hint
/// and its directory is the base that relative locations resolve against.
pub fn import_playlist_path(bytes: &[u8], path: &Path) -> Result<ImportedPlaylist> {
    let format = detect_or_error(bytes, Some(path))?;
    let base = path.parent().unwrap_or_else(|| Path::new(""));
    let report = read_playlist_report_as(format, bytes, base)?;
    Ok(ImportedPlaylist { format, report })
}

/// Reads a playlist with an explicit format, skipping detection.
pub fn read_playlist_as(
    format: PlaylistFormat,
    bytes: &[u8],
    base: &Path,
) -> Result<Vec<LoadedEntry>> {
    read_playlist_report_as(format, bytes, base).map(PlaylistReport::into_entries)
}

/// Reads a playlist with an explicit format, skipping detection, and returns
/// surviving entries plus any skipped entries with reasons.
pub fn read_playlist_report_as(
    format: PlaylistFormat,
    bytes: &[u8],
    base: &Path,
) -> Result<PlaylistReport> {
    if format == PlaylistFormat::B4s {
        return Err(PlaylistError::UnsupportedFormat(B4S_UNSUPPORTED.to_owned()));
    }
    check_claim(format, bytes)?;
    match format {
        PlaylistFormat::M3u => Ok(crate::m3u::read_report(bytes, base)),
        PlaylistFormat::M3u8 => Ok(crate::m3u::read_utf8_report(bytes, base)),
        PlaylistFormat::Pls => Ok(crate::pls::read_report(bytes, base)),
        PlaylistFormat::Xspf => {
            let text = crate::cp1252::decode_text(bytes);
            crate::xspf::validate(&text).map_err(|why| {
                PlaylistError::UnsupportedFormat(format!("not a complete XSPF document: {why}"))
            })?;
            Ok(crate::xspf::read_text_report(&text, base))
        }
        PlaylistFormat::B4s => unreachable!("B4S returned above"),
    }
}

/// Fails when `bytes` visibly are not `format`.
fn check_claim(format: PlaylistFormat, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    let is_xml = format == PlaylistFormat::Xspf;
    if !is_xml && (!looks_like_text(bytes) || starts_like_xml(bytes)) {
        return Err(PlaylistError::UnsupportedFormat(format!(
            "the data is not {format} text (binary, UTF-16 or XML)"
        )));
    }
    if let Some(found) = sniff(bytes) {
        let same = found == format || (found.is_m3u_family() && format.is_m3u_family());
        if !same {
            return Err(PlaylistError::UnsupportedFormat(format!(
                "the content is {found}, not {format}"
            )));
        }
    }
    Ok(())
}

/// Writes entries in `format` and returns the file bytes.
///
/// M3U/M3U8 and PLS use CRLF line endings, XSPF uses LF; everything is UTF-8.
/// B4S is recognized but not implemented and returns an error.
///
/// The result is a **fixed point** of write → read → write: writing the
/// entries the reader returns yields the same bytes, for every entry shape.
///
/// An entry the format cannot store without changing it is a
/// [`PlaylistError::InvalidEntry`] rather than a silently altered file: an
/// empty location, a location with leading or trailing whitespace (readers
/// trim), a location with a line break in M3U/M3U8/PLS, or an M3U/M3U8
/// location starting with `#` (it would read back as a comment). A *title*
/// with a line break is not an error: M3U/PLS write it with the break replaced
/// by a space, and every reader trims titles, so titles with leading or
/// trailing whitespace come back trimmed (documented loss, tested).
pub fn write_playlist(format: PlaylistFormat, entries: &[LoadedEntry]) -> Result<Vec<u8>> {
    if format != PlaylistFormat::B4s {
        check_writable(format, entries)?;
    }
    match format {
        PlaylistFormat::M3u | PlaylistFormat::M3u8 => Ok(crate::m3u::write(entries)),
        PlaylistFormat::Pls => Ok(crate::pls::write(entries)),
        PlaylistFormat::Xspf => Ok(crate::xspf::write(entries)),
        PlaylistFormat::B4s => Err(PlaylistError::UnsupportedFormat(B4S_UNSUPPORTED.to_owned())),
    }
}

/// Rejects entries that `format` would store in a changed form.
fn check_writable(format: PlaylistFormat, entries: &[LoadedEntry]) -> Result<()> {
    let line_based = format != PlaylistFormat::Xspf;
    let m3u_family = format.is_m3u_family();
    for (index, entry) in entries.iter().enumerate() {
        let text = entry.location.to_display_string();
        let problem = if text.trim().is_empty() {
            Some("the location is empty")
        } else if text.trim() != text {
            Some("the location has leading or trailing whitespace")
        } else if line_based && text.contains(['\r', '\n']) {
            Some("the location contains a line break")
        } else if m3u_family && text.starts_with('#') {
            Some("the location starts with `#` and would read back as a comment")
        } else {
            None
        };
        if let Some(problem) = problem {
            return Err(PlaylistError::InvalidEntry(format!(
                "entry {index} cannot be written as {format}: {problem}"
            )));
        }
    }
    Ok(())
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

    // -- the import side ----------------------------------------------------

    #[test]
    fn the_import_reports_the_detected_format() {
        let cases: &[(&[u8], &str, PlaylistFormat)] = &[
            (b"#EXTM3U\r\na.mp3\r\n", "list.m3u", PlaylistFormat::M3u),
            (b"#EXTM3U\r\na.mp3\r\n", "list.m3u8", PlaylistFormat::M3u8),
            (
                b"[playlist]\r\nFile1=a.mp3\r\n",
                "list.m3u",
                PlaylistFormat::Pls,
            ),
            (
                b"<playlist><trackList><track><location>a.mp3</location></track></trackList></playlist>",
                "list.m3u",
                PlaylistFormat::Xspf,
            ),
        ];
        for (bytes, name, expected) in cases {
            let imported = import_playlist_path(bytes, Path::new(name)).expect("import");
            assert_eq!(imported.format, *expected, "content {bytes:?} named {name}");
            assert_eq!(imported.report.entries.len(), 1);
            assert!(imported.report.is_clean());
        }
    }

    #[test]
    fn the_import_resolves_against_the_file_directory() {
        let imported =
            import_playlist_path(b"a.mp3\n", Path::new("C:/Music/Lists/list.m3u")).expect("import");
        assert_eq!(
            imported.report.entries[0].location,
            MediaLocation::File(PathBuf::from("C:/Music/Lists/a.mp3"))
        );
    }

    #[test]
    fn the_import_of_a_broken_file_is_an_error() {
        // Binary data.
        let err = import_playlist_path(&[0x00, 0x01, 0xFF], Path::new("list.m3u"))
            .expect_err("binary data must be refused");
        assert!(matches!(err, PlaylistError::UnsupportedFormat(_)));
        // A truncated XSPF document.
        let err = import_playlist_path(
            b"<playlist><trackList><track><location>a.mp3",
            Path::new("list.xspf"),
        )
        .expect_err("a truncated document must be refused");
        assert!(matches!(err, PlaylistError::UnsupportedFormat(_)));
        // A file name that is not a playlist format and content without a
        // marker.
        let err = import_playlist_path(b"a.mp3\r\n", Path::new("list.mp3"))
            .expect_err("a foreign extension must be refused");
        assert!(matches!(err, PlaylistError::UnsupportedFormat(_)));
    }

    #[test]
    fn the_import_prefers_the_content_over_the_extension() {
        // A PLS file named `.xspf` is a PLS.
        let imported =
            import_playlist_path(b"[playlist]\r\nFile1=a.mp3\r\n", Path::new("list.xspf"))
                .expect("import");
        assert_eq!(imported.format, PlaylistFormat::Pls);
        // An XSPF file named `.m3u` is an XSPF.
        let imported = import_playlist_path(
            b"<playlist><trackList><track><location>a.mp3</location></track></trackList></playlist>",
            Path::new("list.m3u"),
        )
        .expect("import");
        assert_eq!(imported.format, PlaylistFormat::Xspf);
    }

    #[test]
    fn the_import_reports_skipped_entries() {
        let imported = import_playlist_path(
            b"#EXTM3U\r\n#EXTINF:1,Good\r\ngood.mp3\r\n#EXTINF:2,Dangling\r\n",
            Path::new("list.m3u"),
        )
        .expect("import");
        assert_eq!(imported.report.entries.len(), 1);
        assert_eq!(imported.report.skip_count(), 1);
        assert!(!imported.report.is_clean());
    }
}
