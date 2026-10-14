//! `pledit.txt` parsing.
//!
//! The file is a small INI with a single `[Text]` section. It sets the playlist
//! colours and font. Values may be written with or without a leading `#` and
//! may carry extra characters, which are trimmed to the first colour. Missing
//! keys and a missing or empty file keep the documented defaults (ported from
//! webamp; see [`crate::webamp`]).

use crate::ini::parse_ini;

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
    /// The playlist font, when present.
    pub font: Option<String>,
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
            font: Some(DEFAULT_PLAYLIST_FONT.to_string()),
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
        if let Some(value) = section.get("font") {
            if !value.is_empty() {
                style.font = Some(value.clone());
            }
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
        assert_eq!(style.font.as_deref(), Some("Tahoma"));
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
}
