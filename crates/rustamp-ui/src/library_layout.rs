// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The media library window's list layout: the columns, their widths and the
//! row text.
//!
//! # The library window is a modern window
//!
//! The media library is **not** one of the classic skinned windows. It has
//! an OS title bar, a system font and its own chrome, so none of the skin
//! sheets apply to it: no `MAIN.BMP`, no `PLEDIT.BMP`, no `pledit.txt`
//! colours, no sprite rectangles. Nothing in this module reads a skin, and
//! nothing in it will: the library window is not skinned.
//!
//! The layout is therefore given in **text cells** and in **text lines**. A
//! cell is one character of the window's font; the renderer maps a cell to a
//! pixel with its own font metrics and stacks the lines with its own line
//! height. That keeps the layout free of any pixel claim, which the
//! unmeasured library window could not justify anyway.
//!
//! The **text** columns are the ones the index carries: title, artist,
//! album and the length. The index has no column for the file name, so the
//! list has none either.
//!
//! # The cover column
//!
//! In front of the text columns is the **cover column**: a fixed-width
//! column that holds the album art of the row. It is not one of the index's
//! columns and it cannot be sorted, so it is not part of
//! [`LIBRARY_COLUMNS`] and it has no entry in [`LIBRARY_COLUMN_TITLES`].
//!
//! The cover column is measured in **pixels** and not in text cells: a
//! picture has no font metrics, so a cell is the wrong unit for it. Its
//! width is [`LIBRARY_COVER_WIDTH`], and the renderer shifts the text
//! columns past it - the cell offsets below are relative to the text area
//! that follows the cover column.

use crate::library::{LibraryColumn, LibraryRow};

/// The columns of the library list, in display order.
pub const LIBRARY_COLUMNS: [LibraryColumn; 4] = [
    LibraryColumn::Title,
    LibraryColumn::Artist,
    LibraryColumn::Album,
    LibraryColumn::Duration,
];

/// The default column widths, in text cells, in display order.
///
/// The title takes the most room because it is the column that is truncated
/// most often; the length is narrow because it is a `m:ss` value.
pub const LIBRARY_COLUMN_WIDTHS: [u32; 4] = [40, 24, 24, 8];

/// The column header labels, in display order.
pub const LIBRARY_COLUMN_TITLES: [&str; 4] = ["Title", "Artist", "Album", "Length"];

/// The gap between two columns, in text cells.
pub const LIBRARY_COLUMN_GAP: u32 = 1;

/// The width of the cover column, in pixels.
///
/// The width is **our own choice**: no spec measures the media library
/// (S20 is not started). Forty pixels is wide enough for the thumbnail the
/// artwork cache keeps - at most 64 pixels on its longest edge - and
/// narrow enough to leave the text columns their room.
pub const LIBRARY_COVER_WIDTH: u32 = 40;

/// The narrowest the title column may become when a line is resized.
pub const LIBRARY_MIN_TITLE_WIDTH: u32 = 8;

/// The layout of the library list: the width of every column, in text cells.
///
/// The default layout is [`LIBRARY_COLUMN_WIDTHS`]. A wider window keeps
/// the three right-hand columns and gives the slack to the title, which is
/// what [`LibraryLayout::resized_to`] does.
///
/// The widths cover the **text** columns only. The cover column in front of
/// them is a fixed number of pixels wide ([`LIBRARY_COVER_WIDTH`]) and is
/// not part of the widths: the renderer adds it to every cell offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryLayout {
    widths: [u32; 4],
}

impl Default for LibraryLayout {
    fn default() -> Self {
        Self {
            widths: LIBRARY_COLUMN_WIDTHS,
        }
    }
}

impl LibraryLayout {
    /// A layout with the given column widths, in display order.
    pub fn new(widths: [u32; 4]) -> Self {
        Self { widths }
    }

    /// The column widths, in display order.
    pub fn widths(&self) -> [u32; 4] {
        self.widths
    }

    /// The columns, in display order.
    pub fn columns(&self) -> [LibraryColumn; 4] {
        LIBRARY_COLUMNS
    }

    /// The width of one column, in text cells.
    pub fn column_width(&self, column: LibraryColumn) -> u32 {
        self.widths[column_index(column)]
    }

    /// The cell offset of a column's first cell.
    ///
    /// The offset is relative to the text area that follows the cover
    /// column; the renderer adds the cover column's width to it.
    pub fn column_x(&self, column: LibraryColumn) -> u32 {
        let index = column_index(column);
        let mut x = 0;
        for width in self.widths.iter().take(index) {
            x += width + LIBRARY_COLUMN_GAP;
        }
        x
    }

    /// The width of a whole line, in text cells.
    pub fn width(&self) -> u32 {
        let mut width = 0;
        for (i, column_width) in self.widths.iter().enumerate() {
            width += column_width;
            if i + 1 < self.widths.len() {
                width += LIBRARY_COLUMN_GAP;
            }
        }
        width
    }

    /// A layout for a line `width` cells wide.
    ///
    /// The title column takes the slack, the other three keep their width.
    /// A line too narrow for the other three columns and the minimum title
    /// width is not honoured: the title keeps [`LIBRARY_MIN_TITLE_WIDTH`]
    /// and the line stays wider than asked for.
    pub fn resized_to(&self, width: u32) -> LibraryLayout {
        let gaps = LIBRARY_COLUMN_GAP * (LIBRARY_COLUMNS.len() as u32 - 1);
        let fixed: u32 = self.widths[1..].iter().sum::<u32>() + gaps;
        let mut widths = self.widths;
        widths[0] = width.saturating_sub(fixed).max(LIBRARY_MIN_TITLE_WIDTH);
        LibraryLayout { widths }
    }

    /// The header line: the column titles, padded to the column widths.
    pub fn header_line(&self) -> String {
        self.line(&[
            LIBRARY_COLUMN_TITLES[0],
            LIBRARY_COLUMN_TITLES[1],
            LIBRARY_COLUMN_TITLES[2],
            LIBRARY_COLUMN_TITLES[3],
        ])
    }

    /// The text of one row: the four columns, padded to the column widths.
    ///
    /// A value that does not fit its column is truncated and marked with an
    /// ellipsis; a value that fits is padded with spaces, so every line has
    /// the same width.
    pub fn row_line(&self, row: &LibraryRow) -> String {
        let duration = format_duration(row.duration_ms);
        self.line(&[&row.title, &row.artist, &row.album, &duration])
    }

    /// One line of four column values, padded and separated.
    fn line(&self, values: &[&str; 4]) -> String {
        let mut line = String::new();
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                line.push_str(&" ".repeat(LIBRARY_COLUMN_GAP as usize));
            }
            line.push_str(&fit(value, self.widths[index]));
        }
        line
    }
}

/// The index of a column in the display order.
fn column_index(column: LibraryColumn) -> usize {
    match column {
        LibraryColumn::Title => 0,
        LibraryColumn::Artist => 1,
        LibraryColumn::Album => 2,
        LibraryColumn::Duration => 3,
    }
}

/// Fits `text` into `width` cells: padded with spaces, or truncated with an
/// ellipsis.
fn fit(text: &str, width: u32) -> String {
    let width = width as usize;
    let count = text.chars().count();
    if count <= width {
        let mut fitted = String::from(text);
        fitted.push_str(&" ".repeat(width - count));
        fitted
    } else if width == 0 {
        String::new()
    } else {
        let mut fitted: String = text.chars().take(width - 1).collect();
        fitted.push('…');
        fitted
    }
}

/// Formats a length in milliseconds as `m:ss`, or as `h:mm:ss` from an hour
/// on.
///
/// A length the index does not know formats as an empty string, so the
/// column stays blank instead of showing a placeholder.
pub fn format_duration(duration_ms: Option<u64>) -> String {
    let Some(duration_ms) = duration_ms else {
        return String::new();
    };
    let seconds = duration_ms / 1000;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let remainder = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{remainder:02}")
    } else {
        format!("{minutes}:{remainder:02}")
    }
}
