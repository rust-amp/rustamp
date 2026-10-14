//! `viscolor.txt` parsing.
//!
//! The file holds up to 24 analyser colours, one per line, as `r,g,b` with
//! comma or space separators. Lines that do not start with a number (comments)
//! are ignored. Missing lines keep the documented defaults, which are ported
//! from webamp's base skin (see [`crate::webamp`]).

use crate::webamp::DEFAULT_VISCOLORS;

/// Parses `viscolor.txt`, starting from the documented defaults.
pub fn parse_viscolors(text: &str) -> [[u8; 3]; 24] {
    let mut colors = DEFAULT_VISCOLORS;
    let mut index = 0;
    for line in text.lines() {
        if index >= colors.len() {
            break;
        }
        // Only lines that parse as a colour consume a slot, so comment lines
        // do not shift the colour positions.
        if let Some(rgb) = parse_color_line(line) {
            colors[index] = rgb;
            index += 1;
        }
    }
    colors
}

fn parse_color_line(line: &str) -> Option<[u8; 3]> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let mut values = [0u8; 3];
    let mut count = 0;
    for token in trimmed.split([',', ' ', '\t']) {
        if token.is_empty() {
            continue;
        }
        let Ok(number) = token.parse::<u32>() else {
            break;
        };
        values[count] = number.min(255) as u8;
        count += 1;
        if count == 3 {
            break;
        }
    }
    (count == 3).then_some(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comma_and_space_separated() {
        let colors = parse_viscolors("1,2,3\n4 5 6\n; comment\n7,8,9\n");
        assert_eq!(colors[0], [1, 2, 3]);
        assert_eq!(colors[1], [4, 5, 6]);
        assert_eq!(colors[2], [7, 8, 9]);
        // Untouched entries keep their defaults.
        assert_eq!(colors[3], DEFAULT_VISCOLORS[3]);
    }

    #[test]
    fn defaults_when_empty() {
        assert_eq!(parse_viscolors(""), DEFAULT_VISCOLORS);
    }

    #[test]
    fn ignores_comment_lines() {
        let colors = parse_viscolors("// leading comment\n255,255,255\n");
        // The comment line does not consume an index; the colour lands at 0.
        assert_eq!(colors[0], [255, 255, 255]);
    }
}
