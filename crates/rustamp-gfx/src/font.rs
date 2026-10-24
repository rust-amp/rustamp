//! Bitmap fonts driven by sprite sheets.
//!
//! The classic skin ships two kinds of font sheets:
//!
//! * `text.bmp` — a grid of small fixed-size glyph cells, one cell per
//!   character. [`BitmapFont`] maps a `char` to a grid cell and stamps the
//!   cells.
//! * `numbers.bmp` / `nums_ex.bmp` — the large time-display digits.
//!   [`NumberFont`] stamps a fixed-width field and supports a dedicated "no
//!   digit" cell for the leading positions.
//!
//! Neither type knows about the skin file layout: both take an already-decoded
//! [`Image`] plus the cell rectangles. The classic cell coordinates live in the
//! skin crate's ported sprite tables (see `rustamp-skin`).

use crate::image::{Image, Rect};

/// A character-to-cell map for a fixed-cell grid font such as `text.bmp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphMap {
    cell_width: u32,
    cell_height: u32,
    entries: Vec<(char, u32, u32)>,
}

impl GlyphMap {
    /// Creates an empty map with the given cell size.
    pub fn new(cell_width: u32, cell_height: u32) -> Self {
        Self {
            cell_width,
            cell_height,
            entries: Vec::new(),
        }
    }

    /// Adds a glyph at grid column `col`, row `row` (builder style).
    pub fn with(mut self, ch: char, col: u32, row: u32) -> Self {
        self.entries.push((ch, col, row));
        self
    }

    /// Adds a glyph at grid column `col`, row `row`.
    pub fn insert(&mut self, ch: char, col: u32, row: u32) {
        self.entries.push((ch, col, row));
    }

    /// The grid cell size in pixels.
    pub const fn cell_size(&self) -> (u32, u32) {
        (self.cell_width, self.cell_height)
    }

    /// Looks up the grid cell for `ch`.
    pub fn cell(&self, ch: char) -> Option<(u32, u32)> {
        self.entries
            .iter()
            .find(|(c, _, _)| *c == ch)
            .map(|(_, col, row)| (*col, *row))
    }
}

/// A fixed-cell bitmap font such as `text.bmp`.
#[derive(Debug, Clone)]
pub struct BitmapFont {
    image: Image,
    glyphs: GlyphMap,
    transparent: Option<[u8; 3]>,
    spacing: u32,
}

impl BitmapFont {
    /// Creates a font from a decoded sheet and a glyph map.
    ///
    /// `transparent` is the colour key that is skipped when stamping a cell
    /// (the classic sheets store their background as an opaque colour); `None`
    /// stamps the cell opaquely. `spacing` is added after every cell.
    pub fn new(image: Image, glyphs: GlyphMap, transparent: Option<[u8; 3]>, spacing: u32) -> Self {
        Self {
            image,
            glyphs,
            transparent,
            spacing,
        }
    }

    /// The advance of a single glyph: cell width plus spacing.
    pub fn advance(&self) -> u32 {
        self.glyphs.cell_width + self.spacing
    }

    /// The cell rectangle for `ch`, clipped to the sheet.
    pub fn glyph_rect(&self, ch: char) -> Option<Rect> {
        let (col, row) = self.glyphs.cell(ch)?;
        Some(Rect::new(
            (col * self.glyphs.cell_width) as i32,
            (row * self.glyphs.cell_height) as i32,
            self.glyphs.cell_width,
            self.glyphs.cell_height,
        ))
    }

    /// The pixel width a string occupies when rendered.
    pub fn width_of(&self, text: &str) -> u32 {
        let n = text.chars().count() as u32;
        if n == 0 {
            return 0;
        }
        n * self.advance() - self.spacing
    }

    /// Stamps `text` with its top-left corner at `(x, y)` and returns the x
    /// coordinate just past the last glyph.
    ///
    /// Characters without a glyph are skipped but still advance the cursor.
    pub fn render_text(&self, dst: &mut Image, x: i32, y: i32, text: &str) -> i32 {
        let mut cursor = x;
        for ch in text.chars() {
            if let Some(rect) = self.glyph_rect(ch) {
                match self.transparent {
                    Some(key) => dst.blit_keyed(&self.image, rect, cursor, y, key),
                    None => dst.blit_rect(&self.image, rect, cursor, y),
                }
            }
            cursor = cursor.saturating_add(self.advance() as i32);
        }
        cursor
    }
}

/// A fixed-width digit field such as `numbers.bmp` / `nums_ex.bmp`.
#[derive(Debug, Clone)]
pub struct NumberFont {
    image: Image,
    digits: [Rect; 10],
    blank: Option<Rect>,
    transparent: Option<[u8; 3]>,
    advance: i32,
}

impl NumberFont {
    /// Creates a digit font. `digits` are the cell rectangles for `0..=9` in
    /// order. `blank` is the "no digit" cell drawn for an empty position
    /// (`None` draws nothing). `advance` is the x distance between positions.
    pub fn new(
        image: Image,
        digits: [Rect; 10],
        blank: Option<Rect>,
        transparent: Option<[u8; 3]>,
        advance: i32,
    ) -> Self {
        Self {
            image,
            digits,
            blank,
            transparent,
            advance,
        }
    }

    /// The x distance between two digit positions.
    pub const fn advance(&self) -> i32 {
        self.advance
    }

    /// The cell for a digit value (`0..=9`), clipped to nothing if out of range.
    pub fn digit_rect(&self, digit: u8) -> Option<Rect> {
        self.digits.get(digit as usize).copied()
    }

    /// Stamps one cell at `(x, y)`: `Some(d)` draws the digit, `None` draws the
    /// "no digit" cell when one is configured.
    pub fn stamp_cell(&self, dst: &mut Image, x: i32, y: i32, cell: Option<u8>) {
        let rect = match cell {
            Some(d) => self.digit_rect(d),
            None => self.blank,
        };
        let Some(rect) = rect else {
            return;
        };
        match self.transparent {
            Some(key) => dst.blit_keyed(&self.image, rect, x, y, key),
            None => dst.blit_rect(&self.image, rect, x, y),
        }
    }

    /// Stamps a field of cells left to right and returns the x just past it.
    pub fn render_cells(&self, dst: &mut Image, x: i32, y: i32, cells: &[Option<u8>]) -> i32 {
        let mut cursor = x;
        for &cell in cells {
            self.stamp_cell(dst, cursor, y, cell);
            cursor = cursor.saturating_add(self.advance);
        }
        cursor
    }

    /// Stamps `value` right-aligned in a field of at least `min_digits` cells;
    /// shorter numbers use the "no digit" cell for the leading positions.
    pub fn render_number(
        &self,
        dst: &mut Image,
        x: i32,
        y: i32,
        value: u64,
        min_digits: usize,
    ) -> i32 {
        let text = value.to_string();
        let mut cells: Vec<Option<u8>> = Vec::new();
        for _ in 0..min_digits.saturating_sub(text.len()) {
            cells.push(None);
        }
        for byte in text.bytes() {
            if byte.is_ascii_digit() {
                cells.push(Some(byte - b'0'));
            } else {
                cells.push(None);
            }
        }
        self.render_cells(dst, x, y, &cells)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a 2-cell font sheet: cell 0 is red, cell 1 is green.
    fn two_cell_sheet() -> Image {
        let mut sheet = Image::filled(4, 2, [0, 0, 0, 255]);
        // cell (0,0) -> red, cell (1,0) -> green
        let mut red = Image::filled(2, 2, [255, 0, 0, 255]);
        red.set_pixel(0, 0, [0, 0, 0, 255]); // mark the colour key corner
        sheet.blit(&red, 0, 0);
        let green = Image::filled(2, 2, [0, 255, 0, 255]);
        sheet.blit(&green, 2, 0);
        sheet
    }

    #[test]
    fn render_text_matches_expected_rgba() {
        let font = BitmapFont::new(
            two_cell_sheet(),
            GlyphMap::new(2, 2).with('A', 0, 0).with('B', 1, 0),
            Some([0, 0, 0]),
            0,
        );
        let mut dst = Image::filled(4, 2, [9, 9, 9, 255]);
        font.render_text(&mut dst, 0, 0, "AB");

        let mut expected = Image::new(4, 2);
        expected.set_pixel(0, 0, [9, 9, 9, 255]); // keyed-out corner keeps dst
        expected.set_pixel(1, 0, [255, 0, 0, 255]);
        expected.set_pixel(0, 1, [255, 0, 0, 255]);
        expected.set_pixel(1, 1, [255, 0, 0, 255]);
        expected.set_pixel(2, 0, [0, 255, 0, 255]);
        expected.set_pixel(3, 0, [0, 255, 0, 255]);
        expected.set_pixel(2, 1, [0, 255, 0, 255]);
        expected.set_pixel(3, 1, [0, 255, 0, 255]);

        assert_eq!(dst.as_rgba(), expected.as_rgba());
    }

    #[test]
    fn render_text_advances_with_spacing() {
        let font = BitmapFont::new(
            two_cell_sheet(),
            GlyphMap::new(2, 2).with('A', 0, 0),
            Some([0, 0, 0]),
            1,
        );
        assert_eq!(font.advance(), 3);
        assert_eq!(font.width_of("AAA"), 8);
        let mut dst = Image::new(10, 2);
        assert_eq!(font.render_text(&mut dst, 0, 0, "AA"), 6);
    }

    #[test]
    fn number_font_draws_blank_cell_for_short_field() {
        // A 1x1-per-cell sheet: digit 1 green (x=0), blank blue (x=1), digit 2
        // red (x=2).
        let mut sheet = Image::filled(3, 1, [0, 0, 0, 255]);
        sheet.set_pixel(0, 0, [0, 255, 0, 255]); // digit "1"
        sheet.set_pixel(1, 0, [0, 0, 255, 255]); // blank / no digit
        sheet.set_pixel(2, 0, [255, 0, 0, 255]); // digit "2"
        let mut digits = [Rect::new(0, 0, 1, 1); 10];
        digits[1] = Rect::new(0, 0, 1, 1);
        digits[2] = Rect::new(2, 0, 1, 1);
        let font = NumberFont::new(sheet, digits, Some(Rect::new(1, 0, 1, 1)), None, 1);
        let mut dst = Image::filled(4, 1, [0, 0, 0, 255]);
        // "12" in a 4-cell field: two blank cells then 1, 2.
        font.render_cells(&mut dst, 0, 0, &[None, None, Some(1), Some(2)]);
        assert_eq!(dst.pixel(0, 0), Some([0, 0, 255, 255]));
        assert_eq!(dst.pixel(1, 0), Some([0, 0, 255, 255]));
        assert_eq!(dst.pixel(2, 0), Some([0, 255, 0, 255]));
        assert_eq!(dst.pixel(3, 0), Some([255, 0, 0, 255]));
    }

    #[test]
    fn render_number_uses_blank_for_leading_positions() {
        let mut sheet = Image::filled(2, 1, [0, 0, 0, 255]);
        sheet.set_pixel(0, 0, [255, 255, 0, 255]); // digit "1"
        sheet.set_pixel(1, 0, [0, 0, 255, 255]); // blank
        let mut digits = [Rect::new(0, 0, 1, 1); 10];
        digits[1] = Rect::new(0, 0, 1, 1);
        let font = NumberFont::new(sheet, digits, Some(Rect::new(1, 0, 1, 1)), None, 1);
        let mut dst = Image::new(3, 1);
        font.render_number(&mut dst, 0, 0, 1, 3);
        assert_eq!(dst.pixel(0, 0), Some([0, 0, 255, 255])); // blank
        assert_eq!(dst.pixel(1, 0), Some([0, 0, 255, 255])); // blank
        assert_eq!(dst.pixel(2, 0), Some([255, 255, 0, 255])); // digit 1
    }
}
