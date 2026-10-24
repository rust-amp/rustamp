//! Resolving playlist entries to [`MediaLocation`]s.
//!
//! The rules (documented because the formats leave them open):
//!
//! * A relative location is joined onto the **playlist file's directory** (the
//!   `base`), never onto the process working directory. `.`/`..` components are
//!   left for the filesystem to resolve.
//! * Both `/` and `\` are accepted as separators everywhere: on Windows `/` is
//!   normalized to `\`, on other platforms `\` is normalized to `/`. A Unix
//!   file whose name really contains a backslash cannot be expressed in a
//!   playlist; that is an accepted limitation.
//! * A location that carries a drive prefix (`C:\x` and also the
//!   drive-relative `C:x`) is kept as it is and never joined onto the base.
//! * `file:` URLs become [`MediaLocation::File`], with percent-decoding, the
//!   `file://localhost/…` form and the `file://server/share` UNC form handled.
//! * Every other `scheme:` URL (`http`, `https`, `ftp`, …) becomes
//!   [`MediaLocation::Url`] and is left byte-for-byte untouched. A single
//!   letter before the colon is a Windows drive, not a scheme.
//! * An empty or whitespace-only location resolves to `None` (the caller skips
//!   the entry).
//!
//! [`Base`] carries either a local directory or a URL, so XSPF's `xml:base`
//! can point at an HTTP directory.

use std::path::{Path, PathBuf};

use rustamp_core::playlist::MediaLocation;

/// The base a relative location is resolved against: a local directory or a
/// URL treated as a directory prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base {
    /// A local directory.
    Dir(PathBuf),
    /// A URL used as a directory prefix (always stored with a trailing `/`).
    Url(String),
}

impl Base {
    /// A local directory base.
    pub fn dir(path: impl Into<PathBuf>) -> Self {
        Base::Dir(path.into())
    }

    /// Resolves an `xml:base`-style specification against this base.
    ///
    /// An empty specification keeps the base. A `file:` URL becomes a
    /// directory base, any other URL becomes a URL base (with a trailing `/`
    /// added so it acts as a directory), and a relative specification is
    /// treated as a sub-directory.
    pub fn resolve_spec(&self, spec: &str) -> Base {
        let spec = spec.trim();
        if spec.is_empty() {
            return self.clone();
        }
        if let Some(scheme) = scheme_of(spec) {
            if scheme.eq_ignore_ascii_case("file") {
                return match file_url_to_path(spec) {
                    Some(path) => Base::Dir(path),
                    None => self.clone(),
                };
            }
            return Base::Url(if spec.ends_with('/') {
                spec.to_owned()
            } else {
                format!("{spec}/")
            });
        }
        match self {
            Base::Dir(dir) => Base::Dir(dir.join(normalize_separators(spec))),
            Base::Url(url) => Base::Url(join_url(url, spec)),
        }
    }

    /// Resolves one raw location string. `None` when it is empty.
    pub fn resolve(&self, raw: &str) -> Option<MediaLocation> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        if let Some(scheme) = scheme_of(raw) {
            if scheme.eq_ignore_ascii_case("file") {
                return file_url_to_path(raw).map(MediaLocation::File);
            }
            return Some(MediaLocation::Url(raw.to_owned()));
        }
        let normalized = normalize_separators(raw);
        let path = Path::new(&normalized);
        // `is_absolute` is false for a Windows drive path on Unix, so drive
        // paths are checked explicitly: a playlist written on Windows must
        // resolve to the same location on every platform.
        if path.is_absolute() || is_drive(&normalized) {
            return Some(MediaLocation::File(path.to_path_buf()));
        }
        match self {
            Base::Dir(dir) => Some(MediaLocation::File(dir.join(path))),
            Base::Url(url) => Some(MediaLocation::Url(join_url(url, raw))),
        }
    }
}

/// Convenience wrapper: resolves `raw` against a local directory.
pub fn resolve_location(raw: &str, dir: &Path) -> Option<MediaLocation> {
    Base::Dir(dir.to_path_buf()).resolve(raw)
}

/// The scheme of a URL-shaped string, if any.
///
/// A single ASCII letter before the colon is a Windows drive (`C:\…`), not a
/// scheme, so it is rejected here.
fn scheme_of(s: &str) -> Option<&str> {
    let colon = s.find(':')?;
    let scheme = &s[..colon];
    if scheme.len() < 2 || !scheme.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }
    if !scheme
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
    {
        return None;
    }
    Some(scheme)
}

/// `true` for `C:` style drive prefixes.
fn is_drive(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// Normalizes path separators: `/` becomes `\` on Windows, `\` becomes `/` on
/// other platforms.
fn normalize_separators(s: &str) -> String {
    if cfg!(windows) {
        s.replace('/', "\\")
    } else {
        s.replace('\\', "/")
    }
}

/// Converts a `file:` URL to a local path.
///
/// Handles `file:///C:/x`, `file:/C:/x`, `file://C:/x`, `file://localhost/C:/x`,
/// `file://server/share/x` (UNC) and `file:/home/x`, with percent-decoding and
/// with any `?query`/`#fragment` dropped.
pub fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let colon = url.find(':')?;
    if !url[..colon].eq_ignore_ascii_case("file") {
        return None;
    }
    let rest = &url[colon + 1..];
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let decoded = percent_decode(rest);
    if decoded.is_empty() {
        return None;
    }

    if let Some(inner) = decoded.strip_prefix('/') {
        if is_drive(inner) {
            return Some(PathBuf::from(normalize_separators(inner)));
        }
        return Some(PathBuf::from(&decoded));
    }
    if is_drive(&decoded) {
        return Some(PathBuf::from(normalize_separators(&decoded)));
    }
    // Authority form: `host/path`.
    let (host, path) = match decoded.find('/') {
        Some(i) => (&decoded[..i], &decoded[i..]),
        None => (decoded.as_str(), ""),
    };
    if host.eq_ignore_ascii_case("localhost") {
        let inner = path.strip_prefix('/').unwrap_or(path);
        if is_drive(inner) {
            return Some(PathBuf::from(normalize_separators(inner)));
        }
        return Some(PathBuf::from(path));
    }
    if host.is_empty() {
        return Some(PathBuf::from(path));
    }
    // A share on another host: UNC on Windows.
    Some(PathBuf::from(normalize_separators(&format!(
        "//{host}{path}"
    ))))
}

/// Converts a local path to a `file:` URL, percent-encoding everything outside
/// the URL path-safe set.
///
/// `C:\Music\a b.mp3` becomes `file:///C:/Music/a%20b.mp3`; a UNC path
/// `\\server\share\x` becomes `file://server/share/x`. The result is the exact
/// inverse of [`file_url_to_path`] for UTF-8 paths.
pub fn path_to_file_url(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.into_owned()
    };
    let encoded = percent_encode_path(&text);
    if let Some(rest) = encoded.strip_prefix("//") {
        format!("file://{rest}")
    } else if encoded.starts_with('/') {
        format!("file://{encoded}")
    } else {
        format!("file:///{encoded}")
    }
}

/// Percent-encodes the bytes that are not allowed in a URL path. `/` and `:`
/// (drive letter) are kept.
fn percent_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if is_path_safe(b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_digit(b >> 4));
            out.push(hex_digit(b & 0x0F));
        }
    }
    out
}

/// Percent-decodes a URL component. Invalid escapes are kept verbatim; the
/// result is decoded as UTF-8 with replacement characters for invalid bytes.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2]))
        {
            out.push(high * 16 + low);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_path_safe(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'/'
                | b':'
                | b'@'
                | b'+'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b','
                | b';'
                | b'='
        )
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + nibble - 10) as char,
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Joins a relative reference onto a URL directory prefix.
fn join_url(base: &str, rel: &str) -> String {
    let rel = rel.trim();
    if rel.is_empty() {
        return base.to_owned();
    }
    if scheme_of(rel).is_some() {
        return rel.to_owned();
    }
    let authority_end = base.find("://").map(|i| i + 3).unwrap_or(0);
    if rel.starts_with('/') {
        let origin_end = base[authority_end..]
            .find('/')
            .map(|i| authority_end + i)
            .unwrap_or(base.len());
        return format!("{}{rel}", &base[..origin_end]);
    }
    if base.ends_with('/') {
        format!("{base}{rel}")
    } else if let Some(i) = base[authority_end..].rfind('/') {
        format!("{}{rel}", &base[..authority_end + i + 1])
    } else {
        format!("{base}/{rel}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(p: &str) -> MediaLocation {
        MediaLocation::File(PathBuf::from(p))
    }

    #[test]
    fn relative_locations_join_the_base() {
        let base = Base::dir("C:/Music/Lists");
        assert_eq!(
            base.resolve("track.mp3"),
            Some(file("C:/Music/Lists/track.mp3"))
        );
        assert_eq!(
            base.resolve("sub\\deep\\track.mp3"),
            Some(file("C:/Music/Lists/sub/deep/track.mp3"))
        );
        assert_eq!(
            base.resolve("sub/deep/track.mp3"),
            Some(file("C:/Music/Lists/sub/deep/track.mp3"))
        );
    }

    #[test]
    fn absolute_paths_and_drives_are_kept() {
        let base = Base::dir("C:/Music/Lists");
        assert_eq!(base.resolve("D:/Other/a.mp3"), Some(file("D:/Other/a.mp3")));
        assert_eq!(
            base.resolve("D:\\Other\\a.mp3"),
            Some(file("D:/Other/a.mp3"))
        );
        // A single letter before ':' is a drive, not a scheme, and a drive
        // prefix is never joined onto the base - not even the drive-relative
        // form `C:a.mp3`.
        assert_eq!(base.resolve("C:a.mp3"), Some(file("C:a.mp3")));
    }

    #[test]
    fn urls_are_left_untouched() {
        let base = Base::dir("C:/Music/Lists");
        assert_eq!(
            base.resolve("http://example.com/stream?x=1"),
            Some(MediaLocation::Url("http://example.com/stream?x=1".into()))
        );
        assert_eq!(
            base.resolve("https://example.com/a b.mp3"),
            Some(MediaLocation::Url("https://example.com/a b.mp3".into()))
        );
        assert_eq!(
            base.resolve("ftp://example.com/a.mp3"),
            Some(MediaLocation::Url("ftp://example.com/a.mp3".into()))
        );
    }

    #[test]
    fn file_urls_become_files() {
        let base = Base::dir("C:/Music/Lists");
        assert_eq!(
            base.resolve("file:///C:/Music/a.mp3"),
            Some(file("C:/Music/a.mp3"))
        );
        assert_eq!(
            base.resolve("file://C:/Music/a.mp3"),
            Some(file("C:/Music/a.mp3"))
        );
        assert_eq!(
            base.resolve("file://localhost/C:/Music/a.mp3"),
            Some(file("C:/Music/a.mp3"))
        );
        assert_eq!(
            base.resolve("file:///C:/Music/a%20b.mp3"),
            Some(file("C:/Music/a b.mp3"))
        );
        assert_eq!(base.resolve("file:///C:/a.mp3?x#y"), Some(file("C:/a.mp3")));
        assert_eq!(
            base.resolve("file://server/share/a.mp3"),
            Some(file("//server/share/a.mp3"))
        );
        assert_eq!(base.resolve("file:"), None);
    }

    #[test]
    fn empty_locations_resolve_to_none() {
        assert_eq!(Base::dir("C:/x").resolve("   "), None);
        assert_eq!(Base::dir("C:/x").resolve(""), None);
    }

    #[test]
    fn file_url_round_trip() {
        for path in [
            "C:/Music/a.mp3",
            "C:/Music/a b.mp3",
            "C:/Music/a#b%c&d.mp3",
            "C:/M\u{00FC}sik/\u{20AC}uro.mp3",
            "//server/share/x y.mp3",
        ] {
            let url = path_to_file_url(Path::new(path));
            let back = file_url_to_path(&url).expect("round trip");
            assert_eq!(back, PathBuf::from(path), "url was {url}");
        }
    }

    #[test]
    fn percent_helpers_are_robust() {
        assert_eq!(percent_decode("%41%zz%2"), "A%zz%2");
        assert_eq!(percent_encode_path("a b"), "a%20b");
        assert_eq!(percent_decode(&percent_encode_path("\u{20AC}")), "\u{20AC}");
    }

    #[test]
    fn xml_base_handling() {
        let dir = Base::dir("C:/Music/Lists");
        assert_eq!(dir.resolve_spec(""), dir);
        assert_eq!(dir.resolve_spec("sub"), Base::dir("C:/Music/Lists/sub"));
        assert_eq!(dir.resolve_spec("sub/"), Base::dir("C:/Music/Lists/sub"));
        assert_eq!(dir.resolve_spec("file:///D:/Share/"), Base::dir("D:/Share"));
        let url = dir.resolve_spec("http://example.com/music");
        assert_eq!(url, Base::Url("http://example.com/music/".into()));
        assert_eq!(
            url.resolve("a.mp3"),
            Some(MediaLocation::Url("http://example.com/music/a.mp3".into()))
        );
        assert_eq!(
            url.resolve("/root.mp3"),
            Some(MediaLocation::Url("http://example.com/root.mp3".into()))
        );
    }

    #[test]
    fn url_joins_without_a_path() {
        assert_eq!(
            join_url("http://example.com/", "a.mp3"),
            "http://example.com/a.mp3"
        );
        assert_eq!(
            join_url("http://example.com/dir/", "a.mp3"),
            "http://example.com/dir/a.mp3"
        );
        assert_eq!(
            join_url("http://example.com/dir/", "http://other/b.mp3"),
            "http://other/b.mp3"
        );
    }
}
