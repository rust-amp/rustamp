//! Playlist formats: the [`PlaylistFormat`] enum, extension mapping and
//! content sniffing.
//!
//! Detection is deliberately two-step and deterministic:
//!
//! 1. **Content markers** decide first. They are strong signals that a file is
//!    not what its extension says: an XML root (`<playlist` → XSPF, `<winamp`
//!    → B4S), a `[playlist]` section or a `FileN=` key (PLS), `#EXTM3U` (M3U).
//! 2. Otherwise the **file name extension** decides between the two M3U
//!    encodings (`.m3u` vs `.m3u8`).
//! 3. Text that does not start an XML document is treated as M3U, so M3U
//!    content in a file named `.pls` still parses.
//! 4. Anything else is **rejected**: binary or UTF-16 data, XML with an unknown
//!    root, marker-less text behind an unknown or XML extension. Only empty
//!    input falls back to the extension's format.
//!
//! The marker search covers the **whole file**, not just its first kilobyte:
//! a PLS file with a long comment preamble, or an XSPF document with a long
//! XML comment in front of its root element, is still recognized. The markers
//! are ASCII, so they are searched for in the raw bytes. That makes the
//! search independent of the file's encoding and needs no second decode of
//! the file.
//!
//! See [`detect`] for the exact order.

use std::path::Path;

/// A playlist container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaylistFormat {
    /// `#EXTM3U` style text file, historically Windows-1252 on Windows.
    M3u,
    /// The same text format, UTF-8 by definition.
    M3u8,
    /// `[playlist]` / `FileN=` / `TitleN=` / `LengthN=`.
    Pls,
    /// XML shareable playlist format.
    Xspf,
    /// Winamp's own XML playlist. **Not implemented**, see
    /// [`PlaylistFormat::is_supported`].
    B4s,
}

impl PlaylistFormat {
    /// Every format this crate knows about.
    pub const ALL: [PlaylistFormat; 5] = [
        PlaylistFormat::M3u,
        PlaylistFormat::M3u8,
        PlaylistFormat::Pls,
        PlaylistFormat::Xspf,
        PlaylistFormat::B4s,
    ];

    /// The conventional file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            PlaylistFormat::M3u => "m3u",
            PlaylistFormat::M3u8 => "m3u8",
            PlaylistFormat::Pls => "pls",
            PlaylistFormat::Xspf => "xspf",
            PlaylistFormat::B4s => "b4s",
        }
    }

    /// Whether this crate can read and write the format.
    ///
    /// B4S is recognized (so callers can report a useful message) but not
    /// implemented: it is a Winamp-specific XML dialect with no published
    /// format description and no free reference implementation available in
    /// this repository. See `CONTRACT_NOTES.md`.
    pub fn is_supported(self) -> bool {
        !matches!(self, PlaylistFormat::B4s)
    }

    /// Whether the format is one of the two M3U encodings.
    ///
    /// M3U and M3U8 have the same syntax and differ only in the encoding they
    /// are read with, so a caller that groups the two (a file dialog filter, a
    /// "plain list" check) asks for this instead of matching on both.
    pub fn is_m3u_family(self) -> bool {
        matches!(self, PlaylistFormat::M3u | PlaylistFormat::M3u8)
    }

    /// Maps a file extension (with or without a leading dot, any case) to a
    /// format.
    pub fn from_extension(ext: &str) -> Option<PlaylistFormat> {
        let ext = ext.strip_prefix('.').unwrap_or(ext);
        match ext.to_ascii_lowercase().as_str() {
            "m3u" => Some(PlaylistFormat::M3u),
            "m3u8" => Some(PlaylistFormat::M3u8),
            "pls" => Some(PlaylistFormat::Pls),
            "xspf" => Some(PlaylistFormat::Xspf),
            "b4s" => Some(PlaylistFormat::B4s),
            _ => None,
        }
    }

    /// Maps a path's extension to a format.
    pub fn from_path(path: &Path) -> Option<PlaylistFormat> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(PlaylistFormat::from_extension)
    }
}

impl std::fmt::Display for PlaylistFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.extension())
    }
}

/// Number of leading bytes examined by [`looks_like_text`].
const TEXT_SAMPLE_LIMIT: usize = 4096;

/// Looks for a content marker that identifies the format regardless of the
/// file name. Returns `None` when the content carries no marker.
///
/// Markers, in the order they are tested:
/// * an XML root element (`<winamp…` → B4S, `<playlist…` → XSPF),
/// * a `[playlist]` section line or a `FileN=` key (PLS),
/// * `#EXTM3U` (M3U).
///
/// The whole file is searched, so a marker behind a long preamble is found
/// too. An XML document is decided by its root element alone: another root
/// means "not a playlist format I know", not "look further".
pub fn sniff(bytes: &[u8]) -> Option<PlaylistFormat> {
    let bytes = strip_utf8_bom(bytes);

    if starts_like_xml(bytes) {
        let root = xml_root(bytes)?;
        if root.len() >= 6 && root[..6].eq_ignore_ascii_case(b"winamp") {
            return Some(PlaylistFormat::B4s);
        }
        if root.len() >= 8 && root[..8].eq_ignore_ascii_case(b"playlist") {
            return Some(PlaylistFormat::Xspf);
        }
        // Some other XML document: not a playlist format we know.
        return None;
    }

    for line in lines(bytes) {
        let line = trim_ascii(line);
        if line.is_empty() {
            continue;
        }
        if line.eq_ignore_ascii_case(b"[playlist]") {
            return Some(PlaylistFormat::Pls);
        }
        if is_pls_key_line(line) {
            return Some(PlaylistFormat::Pls);
        }
        if line.len() >= 7 && line[..7].eq_ignore_ascii_case(b"#EXTM3U") {
            return Some(PlaylistFormat::M3u);
        }
    }
    None
}

/// Detects the format from the content, with an optional file path used as a
/// hint (for the extension).
///
/// The precedence, in order:
///
/// 1. a **content marker** (see [`sniff`]) wins, even over a contradicting
///    extension; `#EXTM3U` is refined to M3U8 by a `.m3u8` extension,
/// 2. empty input: the extension's format (so an empty `.xspf` still selects
///    XSPF), or `None` without a hint,
/// 3. input that is not text (a NUL byte, many control characters — this
///    includes UTF-16, which is not decoded) is **rejected** (`None`), whatever
///    the extension says,
/// 4. input that starts an XML document but has no known root (`<html>`, a
///    truncated `<?xml …?>`) is **rejected**,
/// 5. a path with an extension that is **not** a playlist extension (`.mp3`,
///    `.txt`, …) and no content marker is **rejected**; a path without any
///    extension, or no path at all, is not,
/// 6. remaining marker-less text: an XML format hint (`.xspf`, `.b4s`) is
///    **rejected** because XML cannot be a bare list; `.m3u8` selects M3U8;
///    everything else is treated as M3U (the universal fallback for a bare
///    list of paths, e.g. M3U content in a file named `.pls`).
pub fn detect(bytes: &[u8], path_hint: Option<&Path>) -> Option<PlaylistFormat> {
    let hint = path_hint.and_then(PlaylistFormat::from_path);
    if let Some(format) = sniff(bytes) {
        return Some(match (format, hint) {
            (PlaylistFormat::M3u, Some(PlaylistFormat::M3u8)) => PlaylistFormat::M3u8,
            _ => format,
        });
    }
    if bytes.is_empty() {
        return hint;
    }
    if !looks_like_text(bytes) || starts_like_xml(bytes) {
        return None;
    }
    let unknown_extension =
        hint.is_none() && path_hint.is_some_and(|path| path.extension().is_some());
    if unknown_extension {
        return None;
    }
    match hint {
        Some(PlaylistFormat::Xspf) | Some(PlaylistFormat::B4s) => None,
        Some(PlaylistFormat::M3u8) => Some(PlaylistFormat::M3u8),
        _ => Some(PlaylistFormat::M3u),
    }
}

/// Whether the bytes open an XML construct (after a BOM and whitespace).
pub(crate) fn starts_like_xml(bytes: &[u8]) -> bool {
    let bytes = strip_utf8_bom(bytes);
    let mut rest = bytes;
    while let Some((&first, tail)) = rest.split_first() {
        if first == b'<' {
            return true;
        }
        if !is_ascii_space(first) {
            return false;
        }
        rest = tail;
    }
    false
}

/// Whether the first few KiB look like text rather than binary.
///
/// True when there is no NUL byte and control characters other than tab, CR
/// and LF make up less than 10 % of the sample. This keeps a binary blob from
/// being "parsed" as a list of file names.
pub fn looks_like_text(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let sample = &bytes[..bytes.len().min(TEXT_SAMPLE_LIMIT)];
    let mut controls = 0usize;
    for &b in sample {
        match b {
            0x00 => return false,
            0x09 | 0x0A | 0x0D => {}
            0x01..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0x7F => controls += 1,
            _ => {}
        }
    }
    controls * 10 < sample.len()
}

/// Strips a leading UTF-8 byte order mark.
fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes)
}

/// Whether the byte is ASCII whitespace.
fn is_ascii_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C)
}

/// Trims ASCII whitespace from both ends.
fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !is_ascii_space(*b))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !is_ascii_space(*b))
        .map_or(start, |i| i + 1);
    &bytes[start..end]
}

/// The lines of `bytes`, split on LF, CR and CRLF.
fn lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split(|&b| b == b'\n' || b == b'\r')
}

/// `true` for a `FileN=` line (case-insensitive, `N` decimal).
fn is_pls_key_line(line: &[u8]) -> bool {
    let Some(separator) = line.iter().position(|&b| b == b'=') else {
        return false;
    };
    let key = trim_ascii(&line[..separator]);
    if key.len() <= 4 || !key[..4].eq_ignore_ascii_case(b"file") {
        return false;
    }
    key[4..].iter().all(u8::is_ascii_digit)
}

/// The root element name of an XML document, skipping the declaration,
/// comments and the DOCTYPE. `None` when no element can be found.
fn xml_root(bytes: &[u8]) -> Option<&[u8]> {
    let mut rest = trim_ascii(bytes);
    // A handful of prologue constructs is enough; anything else means the file
    // is malformed and we bail out.
    for _ in 0..8 {
        if rest.starts_with(b"<?") {
            let end = find(rest, b"?>")?;
            rest = trim_ascii(&rest[end + 2..]);
        } else if rest.starts_with(b"<!--") {
            let end = find(rest, b"-->")?;
            rest = trim_ascii(&rest[end + 3..]);
        } else if rest.starts_with(b"<!") {
            let end = find(rest, b">")?;
            rest = trim_ascii(&rest[end + 1..]);
        } else {
            break;
        }
    }
    let after = rest.strip_prefix(b"<")?;
    let name = element_name(trim_ascii(after));
    (!name.is_empty()).then_some(name)
}

/// The element name at the start of `s` (up to whitespace, `/` or `>`).
fn element_name(s: &[u8]) -> &[u8] {
    let end = s
        .iter()
        .position(|&c| is_ascii_space(c) || c == b'/' || c == b'>')
        .unwrap_or(s.len());
    &s[..end]
}

/// The offset of the first occurrence of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn extension_mapping_is_case_insensitive() {
        assert_eq!(
            PlaylistFormat::from_extension("M3U"),
            Some(PlaylistFormat::M3u)
        );
        assert_eq!(
            PlaylistFormat::from_extension(".M3u8"),
            Some(PlaylistFormat::M3u8)
        );
        assert_eq!(
            PlaylistFormat::from_extension("pls"),
            Some(PlaylistFormat::Pls)
        );
        assert_eq!(
            PlaylistFormat::from_extension("xspf"),
            Some(PlaylistFormat::Xspf)
        );
        assert_eq!(
            PlaylistFormat::from_extension("b4s"),
            Some(PlaylistFormat::B4s)
        );
        assert_eq!(PlaylistFormat::from_extension("mp3"), None);
        assert_eq!(
            PlaylistFormat::from_path(&PathBuf::from("C:/a/b.XSPF")),
            Some(PlaylistFormat::Xspf)
        );
    }

    #[test]
    fn sniffs_content_markers() {
        assert_eq!(
            sniff(b"#EXTM3U\r\n#EXTINF:1,x\r\na.mp3\r\n"),
            Some(PlaylistFormat::M3u)
        );
        assert_eq!(
            sniff(b"[playlist]\r\nFile1=a.mp3\r\n"),
            Some(PlaylistFormat::Pls)
        );
        assert_eq!(
            sniff(b"NumberOfEntries=1\nFile1=a.mp3\n"),
            Some(PlaylistFormat::Pls)
        );
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?>\n<playlist version=\"1\">"),
            Some(PlaylistFormat::Xspf)
        );
        assert_eq!(sniff(b"<WinampXML><playlist>"), Some(PlaylistFormat::B4s));
        assert_eq!(sniff(b"a.mp3\nb.mp3\n"), None);
        assert_eq!(sniff(b"<html><body>no</body></html>"), None);
    }

    #[test]
    fn content_marker_beats_a_wrong_extension() {
        let xspf = b"<?xml version=\"1.0\"?><playlist version=\"1\"></playlist>";
        assert_eq!(
            detect(xspf, Some(Path::new("misnamed.m3u"))),
            Some(PlaylistFormat::Xspf)
        );
    }

    #[test]
    fn extension_refines_m3u_to_m3u8() {
        assert_eq!(
            detect(b"#EXTM3U\r\na.mp3\r\n", Some(Path::new("p.m3u8"))),
            Some(PlaylistFormat::M3u8)
        );
        assert_eq!(
            detect(b"#EXTM3U\r\na.mp3\r\n", Some(Path::new("p.m3u"))),
            Some(PlaylistFormat::M3u)
        );
    }

    #[test]
    fn plain_text_without_marker_falls_back_to_m3u() {
        assert_eq!(detect(b"a.mp3\nb.mp3\n", None), Some(PlaylistFormat::M3u));
        assert_eq!(
            detect(b"a.mp3\n", Some(Path::new("misnamed.pls"))),
            Some(PlaylistFormat::M3u)
        );
    }

    #[test]
    fn binary_blob_is_not_a_playlist() {
        let mut blob = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        blob.extend((0u16..4096).map(|i| (i % 251) as u8));
        assert_eq!(detect(&blob, None), None);
        assert_eq!(detect(&blob, Some(Path::new("no-extension"))), None);
        assert!(!looks_like_text(&blob));
    }

    #[test]
    fn empty_input_is_not_a_playlist() {
        assert_eq!(detect(b"", None), None);
        assert_eq!(detect(b"", Some(Path::new("x.mp3"))), None);
    }

    #[test]
    fn extension_alone_still_selects_a_known_format() {
        assert_eq!(
            detect(b"", Some(Path::new("x.m3u"))),
            Some(PlaylistFormat::M3u)
        );
    }

    #[test]
    fn non_ascii_keys_do_not_panic() {
        assert!(!is_pls_key_line("\u{20AC}\u{00E4}=x".as_bytes()));
        assert!(is_pls_key_line(b"FILE7=C:\\a.mp3"));
        assert!(!is_pls_key_line(b"Filex=1"));
    }

    // -- the marker search covers the whole file ----------------------------

    #[test]
    fn a_marker_behind_a_long_preamble_is_found() {
        // A PLS file whose section header is far beyond the first kilobyte.
        let mut pls = Vec::new();
        for i in 0..500 {
            pls.extend_from_slice(format!("; comment {i}\r\n").as_bytes());
        }
        pls.extend_from_slice(b"[playlist]\r\nFile1=a.mp3\r\n");
        assert_eq!(sniff(&pls), Some(PlaylistFormat::Pls));
        assert_eq!(
            detect(&pls, Some(Path::new("list.pls"))),
            Some(PlaylistFormat::Pls)
        );

        // The same for an M3U header.
        let mut m3u = Vec::new();
        for i in 0..500 {
            m3u.extend_from_slice(format!("# filler {i}\r\n").as_bytes());
        }
        m3u.extend_from_slice(b"#EXTM3U\r\na.mp3\r\n");
        assert_eq!(sniff(&m3u), Some(PlaylistFormat::M3u));
    }

    #[test]
    fn a_long_xml_preamble_does_not_hide_the_root() {
        let mut xspf = b"<!--".to_vec();
        xspf.extend(std::iter::repeat_n(b'x', 5000));
        xspf.extend_from_slice(b"--><playlist><trackList/></playlist>");
        assert_eq!(sniff(&xspf), Some(PlaylistFormat::Xspf));
    }

    #[test]
    fn a_comment_is_not_a_marker() {
        // A commented-out section header must not turn an M3U into a PLS.
        let m3u = b"#EXTM3U\r\n# [playlist] of mine\r\n#File1=a.mp3\r\na.mp3\r\n";
        assert_eq!(sniff(m3u), Some(PlaylistFormat::M3u));
    }

    // -- detection table ----------------------------------------------------

    /// (content, file name, expected format)
    const CASES: &[(&[u8], &str, Option<PlaylistFormat>)] = &[
        // M3U, M3U8: the extension tells the two encodings apart.
        (
            b"#EXTM3U\r\na.mp3\r\n",
            "list.m3u",
            Some(PlaylistFormat::M3u),
        ),
        (
            b"#EXTM3U\r\na.mp3\r\n",
            "list.m3u8",
            Some(PlaylistFormat::M3u8),
        ),
        (
            b"#EXTM3U\r\na.mp3\r\n",
            "list.txt",
            Some(PlaylistFormat::M3u),
        ),
        // PLS, wrong extension: the content wins.
        (
            b"[playlist]\r\nFile1=a.mp3\r\n",
            "list.m3u",
            Some(PlaylistFormat::Pls),
        ),
        (
            b"[playlist]\r\nFile1=a.mp3\r\n",
            "list.m3u8",
            Some(PlaylistFormat::Pls),
        ),
        // XSPF, wrong extension: the content wins.
        (
            b"<playlist><trackList/></playlist>",
            "list.m3u",
            Some(PlaylistFormat::Xspf),
        ),
        (
            b"<playlist><trackList/></playlist>",
            "list.pls",
            Some(PlaylistFormat::Xspf),
        ),
        // B4S is recognized.
        (
            b"<WinampXML><playlist/></WinampXML>",
            "list.b4s",
            Some(PlaylistFormat::B4s),
        ),
        // A bare list of paths is M3U, whatever the extension says.
        (b"a.mp3\r\nb.mp3\r\n", "list.m3u", Some(PlaylistFormat::M3u)),
        (b"a.mp3\r\nb.mp3\r\n", "list.pls", Some(PlaylistFormat::M3u)),
        (
            b"a.mp3\r\nb.mp3\r\n",
            "list.m3u8",
            Some(PlaylistFormat::M3u8),
        ),
        // An unknown extension without a marker is refused.
        (b"a.mp3\r\nb.mp3\r\n", "list.mp3", None),
        // XML that is not a playlist is refused.
        (b"<html><body>no</body></html>", "list.xspf", None),
        (b"<?xml version=\"1.0\"?>", "list.xspf", None),
        // Binary data is refused.
        (&[0x00, 0x01, 0x02, 0x03, 0xFF, 0xFE], "list.m3u", None),
    ];

    #[test]
    fn the_detection_table_holds() {
        for (content, name, expected) in CASES {
            assert_eq!(
                detect(content, Some(Path::new(name))),
                *expected,
                "content {content:?} named {name}"
            );
        }
    }

    #[test]
    fn an_empty_file_falls_back_to_the_extension() {
        assert_eq!(
            detect(b"", Some(Path::new("list.xspf"))),
            Some(PlaylistFormat::Xspf)
        );
        assert_eq!(
            detect(b"", Some(Path::new("list.pls"))),
            Some(PlaylistFormat::Pls)
        );
        assert_eq!(detect(b"", None), None);
    }

    #[test]
    fn a_utf8_bom_does_not_hide_the_marker() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"#EXTM3U\r\na.mp3\r\n");
        assert_eq!(sniff(&bytes), Some(PlaylistFormat::M3u));
    }
}
