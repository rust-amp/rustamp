//! `viscolor.txt` parsing.
//!
//! The file holds up to 24 analyser colours, one per line, as `r,g,b` with
//! comma or space separators. Every line may carry a trailing comment
//! introduced by `//` (S17 §6), which is cut off before the numbers are read.
//! Lines that do not start with a number (a comment on its own, a note from
//! the skin author) are ignored and do not consume a slot. Missing lines keep
//! the documented defaults, which are the base skin's own list (S17 §6, see
//! [`crate::webamp`]).

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
    // S17 §6: every entry may carry a trailing comment introduced by `//`.
    // The comment is cut off first, so a comment that follows the last number
    // directly (`0,0,0//black`) cannot swallow that number. The base skin
    // writes `0,0,0, // color 0 = black`, so the comma before the comment is
    // part of the line and simply ends the value list.
    let line = line.split("//").next().unwrap_or("");
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

    /// S17 §6: every entry may carry a trailing comment introduced by `//`.
    #[test]
    fn reads_a_trailing_comment() {
        // The base skin writes the comment after a comma.
        let colors = parse_viscolors("0,0,0, // color 0 = black\n24,33,41, // grey\n");
        assert_eq!(colors[0], [0, 0, 0]);
        assert_eq!(colors[1], [24, 33, 41]);
    }

    /// S17 §6: the comment may follow the last number directly, without a
    /// separating space or comma.
    #[test]
    fn a_comment_may_touch_the_last_number() {
        let colors = parse_viscolors("38,15,58//0\n1,2,3//1\n");
        assert_eq!(colors[0], [38, 15, 58]);
        assert_eq!(colors[1], [1, 2, 3]);
    }

    /// A line the comment swallows must not shift the slots of the lines
    /// after it.
    #[test]
    fn a_commented_entry_does_not_shift_the_slots() {
        let colors = parse_viscolors("1,2,3//first\n4,5,6//second\n7,8,9//third\n");
        assert_eq!(colors[0], [1, 2, 3]);
        assert_eq!(colors[1], [4, 5, 6]);
        assert_eq!(colors[2], [7, 8, 9]);
    }

    /// S17 §6: the base skin's `viscolor.txt` writes its comments after a
    /// comma and names the first three slots.
    #[test]
    fn reads_the_base_skin_file() {
        let colors = parse_viscolors(
            "0,0,0, // color 0 = black\n24,33,41, // color 1 = grey for dots\n239,49,16, // color 2 = top of spec\n",
        );
        assert_eq!(colors[0], [0, 0, 0]);
        assert_eq!(colors[1], [24, 33, 41]);
        assert_eq!(colors[2], [239, 49, 16]);
    }

    /// S17 §6: the base skin's file lists 24 entries, and the built-in
    /// defaults are exactly that list.
    #[test]
    fn the_base_skin_file_parses_to_the_built_in_defaults() {
        let base_skin = "\
0,0,0, // color 0 = black
24,33,41, // color 1 = grey for dots
239,49,16, // color 2 = top of spec
206,41,16, // 3
214,90,0, // 4
214,102,0, // 5
214,115,0, // 6
198,123,8, // 7
222,165,24, // 8
214,181,33, // 9
189,222,41, // 10
148,222,33, // 11
41,206,16, // 12
50,190,16, // 13
57,181,16, // 14
49,156,8, // 15
41,148,0, // 16
24,132,8, // 17
255,255,255, // 18 = osc 1
214,214,222, // 19 = osc 2
181,189,189, // 20 = osc 3
160,170,175, // 21 = osc 4
148,156,165, // 22 = osc 4
150, 150, 150, // 23 = analyzer peak dots
";
        let colors = parse_viscolors(base_skin);
        assert_eq!(colors, DEFAULT_VISCOLORS);
    }
}
