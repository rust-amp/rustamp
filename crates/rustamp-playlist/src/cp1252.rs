//! Text decoding: UTF-8 first, hand-written Windows-1252 as the fallback.
//!
//! `.m3u8` files are UTF-8 by definition, but `.m3u` files in the wild are very
//! often Windows-1252 (or a byte-compatible single-byte code page such as
//! CP-1251 for Cyrillic, which decodes to the wrong letters but never fails).
//! The rule implemented here is the one every player uses in practice:
//!
//! 1. If the bytes are valid UTF-8 (after an optional UTF-8 BOM), decode them
//!    as UTF-8.
//! 2. Otherwise decode every byte through the 256-entry table below.
//!
//! Decoding is total: no input can panic, and no byte is lost. The five byte
//! values that Windows-1252 leaves undefined (`0x81`, `0x8D`, `0x8F`, `0x90`,
//! `0x9D`) map to U+FFFD REPLACEMENT CHARACTER.

/// The full 256-entry Windows-1252 table, written out by hand.
///
/// `0x00..=0x7F` are ASCII, `0xA0..=0xFF` are Latin-1, and `0x80..=0x9F` are
/// the Windows-specific block. The five undefined slots of the code page hold
/// U+FFFD.
pub const CP1252: [char; 256] = [
    // 0x00
    '\u{0000}', '\u{0001}', '\u{0002}', '\u{0003}', '\u{0004}', '\u{0005}', '\u{0006}', '\u{0007}',
    '\u{0008}', '\u{0009}', '\u{000A}', '\u{000B}', '\u{000C}', '\u{000D}', '\u{000E}', '\u{000F}',
    // 0x10
    '\u{0010}', '\u{0011}', '\u{0012}', '\u{0013}', '\u{0014}', '\u{0015}', '\u{0016}', '\u{0017}',
    '\u{0018}', '\u{0019}', '\u{001A}', '\u{001B}', '\u{001C}', '\u{001D}', '\u{001E}', '\u{001F}',
    // 0x20
    ' ', '!', '"', '#', '$', '%', '&', '\'', '(', ')', '*', '+', ',', '-', '.', '/',
    // 0x30
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ':', ';', '<', '=', '>', '?',
    // 0x40
    '@', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O',
    // 0x50
    'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '[', '\\', ']', '^', '_',
    // 0x60
    '`', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o',
    // 0x70
    'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', '{', '|', '}', '~', '\u{007F}',
    // 0x80
    '\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    // 0x88
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}', '\u{017D}', '\u{FFFD}',
    // 0x90
    '\u{FFFD}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    // 0x98
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}',
    // 0xA0
    '\u{00A0}', '\u{00A1}', '\u{00A2}', '\u{00A3}', '\u{00A4}', '\u{00A5}', '\u{00A6}', '\u{00A7}',
    '\u{00A8}', '\u{00A9}', '\u{00AA}', '\u{00AB}', '\u{00AC}', '\u{00AD}', '\u{00AE}', '\u{00AF}',
    // 0xB0
    '\u{00B0}', '\u{00B1}', '\u{00B2}', '\u{00B3}', '\u{00B4}', '\u{00B5}', '\u{00B6}', '\u{00B7}',
    '\u{00B8}', '\u{00B9}', '\u{00BA}', '\u{00BB}', '\u{00BC}', '\u{00BD}', '\u{00BE}', '\u{00BF}',
    // 0xC0
    '\u{00C0}', '\u{00C1}', '\u{00C2}', '\u{00C3}', '\u{00C4}', '\u{00C5}', '\u{00C6}', '\u{00C7}',
    '\u{00C8}', '\u{00C9}', '\u{00CA}', '\u{00CB}', '\u{00CC}', '\u{00CD}', '\u{00CE}', '\u{00CF}',
    // 0xD0
    '\u{00D0}', '\u{00D1}', '\u{00D2}', '\u{00D3}', '\u{00D4}', '\u{00D5}', '\u{00D6}', '\u{00D7}',
    '\u{00D8}', '\u{00D9}', '\u{00DA}', '\u{00DB}', '\u{00DC}', '\u{00DD}', '\u{00DE}', '\u{00DF}',
    // 0xE0
    '\u{00E0}', '\u{00E1}', '\u{00E2}', '\u{00E3}', '\u{00E4}', '\u{00E5}', '\u{00E6}', '\u{00E7}',
    '\u{00E8}', '\u{00E9}', '\u{00EA}', '\u{00EB}', '\u{00EC}', '\u{00ED}', '\u{00EE}', '\u{00EF}',
    // 0xF0
    '\u{00F0}', '\u{00F1}', '\u{00F2}', '\u{00F3}', '\u{00F4}', '\u{00F5}', '\u{00F6}', '\u{00F7}',
    '\u{00F8}', '\u{00F9}', '\u{00FA}', '\u{00FB}', '\u{00FC}', '\u{00FD}', '\u{00FE}', '\u{00FF}',
];

/// Decodes bytes as Windows-1252. Total: every byte maps to exactly one char.
pub fn decode_cp1252(bytes: &[u8]) -> String {
    bytes.iter().map(|b| CP1252[*b as usize]).collect()
}

/// Decodes playlist text: UTF-8 when the bytes are valid UTF-8, otherwise
/// Windows-1252.
///
/// A leading UTF-8 BOM is removed. A UTF-16 BOM is *not* handled; such a file
/// falls through to Windows-1252 and decodes to garbage (but never panics).
/// This is a deliberate scope decision of task card E1; UTF-16 playlists are a
/// candidate for a later spec request.
pub fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => decode_cp1252(bytes),
    }
}

/// Decodes bytes as UTF-8, replacing invalid sequences with U+FFFD.
///
/// Used for `.m3u8`, which is UTF-8 by definition: a broken byte is a
/// replacement character rather than a reason to fall back to Windows-1252.
pub fn decode_utf8_lossy(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_latin1_round_trip_through_the_table() {
        assert_eq!(decode_cp1252(b"Hello, World!"), "Hello, World!");
        assert_eq!(CP1252[0x41], 'A');
        assert_eq!(CP1252[0xFF], '\u{00FF}');
        assert_eq!(CP1252[0xA0], '\u{00A0}');
    }

    #[test]
    fn windows_block_is_mapped() {
        assert_eq!(
            decode_cp1252(&[0x80, 0x93, 0x94, 0x85, 0xE9]),
            "\u{20AC}\u{201C}\u{201D}\u{2026}\u{00E9}"
        );
    }

    #[test]
    fn undefined_slots_become_replacement_characters() {
        for b in [0x81u8, 0x8D, 0x8F, 0x90, 0x9D] {
            assert_eq!(CP1252[b as usize], '\u{FFFD}');
        }
    }

    #[test]
    fn decode_text_prefers_utf8_and_strips_a_bom() {
        assert_eq!(decode_text("\u{00E4}pfel".as_bytes()), "\u{00E4}pfel");
        assert_eq!(decode_text(b"\xEF\xBB\xBFabc"), "abc");
        // A lone 0xE4 is not valid UTF-8: it must fall back to CP1252 ('ä').
        assert_eq!(decode_text(&[0xE4, b'p']), "\u{00E4}p");
    }

    #[test]
    fn decode_text_never_fails_on_random_bytes() {
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        for _ in 0..1000 {
            let mut bytes = Vec::new();
            for _ in 0..32 {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                bytes.push((state >> 33) as u8);
            }
            let text = decode_text(&bytes);
            assert!(!text.is_empty());
        }
    }
}
