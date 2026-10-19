//! `pledit.txt` parsing.
//!
//! The file is a small INI with a single `[Text]` section. It sets the playlist
//! colours and font. Values may be written with or without a leading `#` and
//! may carry extra characters, which are trimmed to the first colour. Missing
//! keys and a missing or empty file keep the documented defaults (ported from
//! webamp; see [`crate::webamp`]).
//!
//! # The `Font=` line
//!
//! S17 §5 settles the key and the family name: the base skin's file says
//! `Font=Arial`, and the playlist list is drawn with that real font rather
//! than with the `text.bmp` bitmap font. The spec settles **nothing** about a
//! size or a style - it lists "a `Font` name that is not installed" as *not
//! determined* - so the size and the style flags follow the classic
//! `Font=Name,size` form and are this crate's own reading.
//!
//! ```text
//! Font=Arial            // the family only
//! Font=Arial,12         // the family and a size in pixels
//! Font=Arial,12,bold    // ... and a bold face
//! Font=Arial,12,italic  // ... and an italic face
//! ```
//!
//! The family name is the first comma-separated token; every later token is
//! either a size in pixels (a decimal number) or one of the style flags
//! `bold` and `italic` (case-insensitive). A token that is none of these is
//! ignored, and the first size wins. A value without a family name is
//! malformed and - like a malformed colour - leaves the built-in default
//! font in place.

use crate::ini::parse_ini;

/// The largest size a `Font=` line may ask for.
///
/// A size outside the range is ignored like any other unusable token, so a
/// skin cannot make the playlist draw an unreadable row.
const MAX_FONT_SIZE: u32 = 255;

/// The font a skin's `pledit.txt` asks the playlist to draw with.
///
/// S17 §5 settles the family name; the size and the style flags are read
/// from the same line in the classic `Font=Name,size` form (see the module
/// docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistFont {
    /// The family name, as the file wrote it.
    pub family: String,
    /// The size in pixels, when the file named one.
    pub size: Option<u32>,
    /// Whether the file asked for a bold face.
    pub bold: bool,
    /// Whether the file asked for an italic face.
    pub italic: bool,
}

impl PlaylistFont {
    /// Parses the value of a `Font=` line.
    ///
    /// The value is `Name[,size[,style...]]`; see the module docs for the
    /// rules. A value without a family name is malformed and yields `None`,
    /// which leaves the caller's default font in place.
    pub fn parse(value: &str) -> Option<PlaylistFont> {
        let mut tokens = value.split(',').map(str::trim);
        let family = tokens.next()?;
        if family.is_empty() {
            return None;
        }
        let mut font = PlaylistFont {
            family: family.to_string(),
            size: None,
            bold: false,
            italic: false,
        };
        for token in tokens {
            if font.size.is_none()
                && !token.is_empty()
                && token.bytes().all(|b| b.is_ascii_digit())
                && let Ok(size) = token.parse::<u32>()
                && (1..=MAX_FONT_SIZE).contains(&size)
            {
                font.size = Some(size);
            } else if token.eq_ignore_ascii_case("bold") {
                font.bold = true;
            } else if token.eq_ignore_ascii_case("italic") {
                font.italic = true;
            }
        }
        Some(font)
    }
}

/// The playlist colours and font from `pledit.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistStyle {
    /// Normal entry text colour.
    pub normal: [u8; 3],
    /// Currently playing entry text colour.
    pub current: [u8; 3],
    /// Normal background colour.
    pub normal_bg: [u8; 3],
    /// Selected entry background colour.
    pub selected_bg: [u8; 3],
    /// Menu-bar foreground colour (`mbFG`), when present.
    pub menu_bar_fg: Option<[u8; 3]>,
    /// Menu-bar background colour (`mbBG`), when present.
    pub menu_bar_bg: Option<[u8; 3]>,
    /// The playlist font, when the file names one.
    pub font: Option<PlaylistFont>,
}

impl Default for PlaylistStyle {
    fn default() -> Self {
        use crate::webamp::{
            DEFAULT_PLAYLIST_CURRENT, DEFAULT_PLAYLIST_FONT, DEFAULT_PLAYLIST_NORMAL,
            DEFAULT_PLAYLIST_NORMAL_BG, DEFAULT_PLAYLIST_SELECTED_BG,
        };
        Self {
            normal: DEFAULT_PLAYLIST_NORMAL,
            current: DEFAULT_PLAYLIST_CURRENT,
            normal_bg: DEFAULT_PLAYLIST_NORMAL_BG,
            selected_bg: DEFAULT_PLAYLIST_SELECTED_BG,
            menu_bar_fg: None,
            menu_bar_bg: None,
            font: Some(PlaylistFont {
                family: DEFAULT_PLAYLIST_FONT.to_string(),
                size: None,
                bold: false,
                italic: false,
            }),
        }
    }
}

impl PlaylistStyle {
    /// Parses `pledit.txt`. A missing, empty or section-less file yields the
    /// defaults.
    pub fn parse(text: &str) -> PlaylistStyle {
        let ini = parse_ini(text);
        let Some(section) = ini.get("text") else {
            return PlaylistStyle::default();
        };

        let mut style = PlaylistStyle::default();
        if let Some(value) = section.get("normal").and_then(|v| parse_color(v)) {
            style.normal = value;
        }
        if let Some(value) = section.get("current").and_then(|v| parse_color(v)) {
            style.current = value;
        }
        if let Some(value) = section.get("normalbg").and_then(|v| parse_color(v)) {
            style.normal_bg = value;
        }
        if let Some(value) = section.get("selectedbg").and_then(|v| parse_color(v)) {
            style.selected_bg = value;
        }
        if let Some(value) = section.get("mbfg").and_then(|v| parse_color(v)) {
            style.menu_bar_fg = Some(value);
        }
        if let Some(value) = section.get("mbbg").and_then(|v| parse_color(v)) {
            style.menu_bar_bg = Some(value);
        }
        if let Some(value) = section.get("font")
            && let Some(font) = PlaylistFont::parse(value)
        {
            style.font = Some(font);
        }
        style
    }
}

/// Parses `#RGB` or `#RRGGBB` (with or without the `#`), trimming to the first
/// colour when the value carries extra characters.
fn parse_color(value: &str) -> Option<[u8; 3]> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let hex_part = value.strip_prefix('#').unwrap_or(value);
    let hex_part = hex_part.trim();
    let digits: String = hex_part
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect();
    match digits.len() {
        6 => Some([
            u8::from_str_radix(&digits[0..2], 16).ok()?,
            u8::from_str_radix(&digits[2..4], 16).ok()?,
            u8::from_str_radix(&digits[4..6], 16).ok()?,
        ]),
        3 => {
            let expand = |c: char| {
                let v = c.to_digit(16)? as u8;
                Some(v * 17)
            };
            let mut chars = digits.chars();
            Some([
                expand(chars.next()?)?,
                expand(chars.next()?)?,
                expand(chars.next()?)?,
            ])
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The font of a parsed style as `(family, size, bold, italic)`.
    fn font_of(style: &PlaylistStyle) -> Option<(&str, Option<u32>, bool, bool)> {
        style
            .font
            .as_ref()
            .map(|font| (font.family.as_str(), font.size, font.bold, font.italic))
    }

    #[test]
    fn defaults_when_empty() {
        assert_eq!(PlaylistStyle::parse(""), PlaylistStyle::default());
    }

    #[test]
    fn defaults_when_section_missing() {
        assert_eq!(
            PlaylistStyle::parse("[Other]\nnormal=#FF0000\n"),
            PlaylistStyle::default()
        );
    }

    #[test]
    fn parses_keys_case_insensitively() {
        let style = PlaylistStyle::parse(
            "[Text]\nNormal=#123456\nCurrent=#FFFFFF\nNormalBG=#000000\nSelectedBG=#0000FF\nFont=Tahoma\n",
        );
        assert_eq!(style.normal, [0x12, 0x34, 0x56]);
        assert_eq!(style.current, [0xFF, 0xFF, 0xFF]);
        assert_eq!(font_of(&style), Some(("Tahoma", None, false, false)));
    }

    /// S17 §5 settles the family name; the size and the style flags follow
    /// the classic `Font=Name,size` form.
    #[test]
    fn font_line_carries_the_family_the_size_and_the_style_flags() {
        let style = PlaylistStyle::parse("[Text]\nFont=Arial,12,bold,italic\n");
        assert_eq!(font_of(&style), Some(("Arial", Some(12), true, true)));
    }

    #[test]
    fn font_line_without_a_size_leaves_the_size_open() {
        let style = PlaylistStyle::parse("[Text]\nFont=Tahoma\n");
        assert_eq!(font_of(&style), Some(("Tahoma", None, false, false)));
    }

    #[test]
    fn font_line_ignores_the_tokens_it_cannot_use() {
        let style = PlaylistStyle::parse("[Text]\nFont=Arial,wide,999,bold\n");
        // "wide" is not a token the format knows and 999 is out of range, so
        // the family and the bold flag are the only things that survive.
        assert_eq!(font_of(&style), Some(("Arial", None, true, false)));
    }

    #[test]
    fn the_first_size_wins() {
        let style = PlaylistStyle::parse("[Text]\nFont=Arial,12,14\n");
        assert_eq!(font_of(&style), Some(("Arial", Some(12), false, false)));
    }

    /// A malformed font line is handled like a malformed colour: the
    /// built-in default stays in place.
    #[test]
    fn a_malformed_font_line_keeps_the_default_font() {
        for value in ["", " ", ",12", ",,,"] {
            let style = PlaylistStyle::parse(&format!("[Text]\nFont={value}\n"));
            assert_eq!(style.font, PlaylistStyle::default().font, "Font={value:?}");
        }
    }

    #[test]
    fn a_missing_font_line_keeps_the_default_font() {
        let style = PlaylistStyle::parse("[Text]\nNormal=#00FF00\n");
        assert_eq!(style.font, PlaylistStyle::default().font);
    }

    #[test]
    fn accepts_values_without_hash_and_extra_characters() {
        let style = PlaylistStyle::parse("[Text]\nnormal=00FF00\ncurrent=#0f0\n");
        assert_eq!(style.normal, [0x00, 0xFF, 0x00]);
        assert_eq!(style.current, [0x00, 0xFF, 0x00]);
    }

    #[test]
    fn keeps_default_for_invalid_colour() {
        let style = PlaylistStyle::parse("[Text]\nnormal=zzz\n");
        assert_eq!(style.normal, PlaylistStyle::default().normal);
    }

    /// S17 §5: the base skin's `pledit.txt` is exactly this file, and a
    /// playlist without a `pledit.txt` still renders green / white / black —
    /// so the three text colours are the built-in defaults of the classic
    /// look. The selected-row background and the font were not observed
    /// without the file, so they keep their own values.
    #[test]
    fn the_base_skin_file_parses_to_the_built_in_defaults() {
        let base_skin = "[Text]\nNormal=#00FF00\nCurrent=#FFFFFF\nNormalBG=#000000\nSelectedBG=#0000C6\nFont=Arial\n";
        let style = PlaylistStyle::parse(base_skin);
        assert_eq!(style.normal, [0x00, 0xFF, 0x00]);
        assert_eq!(style.current, [0xFF, 0xFF, 0xFF]);
        assert_eq!(style.normal_bg, [0x00, 0x00, 0x00]);
        assert_eq!(style.selected_bg, [0x00, 0x00, 0xC6]);
        assert_eq!(font_of(&style), Some(("Arial", None, false, false)));

        // The colours S17 §5 saw without a `pledit.txt` are the defaults.
        let defaults = PlaylistStyle::default();
        assert_eq!(style.normal, defaults.normal);
        assert_eq!(style.current, defaults.current);
        assert_eq!(style.normal_bg, defaults.normal_bg);
    }
}
