// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The media library window's renderer: the list, the selection and the
//! status line, drawn into a frame.
//!
//! [`crate::library::LibraryView`] is the headless model of the media library
//! and [`crate::library_layout::LibraryLayout`] gives the list's columns in
//! text cells. This module draws them: it paints the column titles, the rows
//! at the scroll offset, the selection and the status line.
//!
//! # The library window is a modern window
//!
//! The media library is **not** one of the classic skinned windows: it has an
//! OS title bar, a system font and its own chrome, so no skin sheet, no
//! sprite and no `pledit.txt` colour applies to it. Nothing here reads a
//! skin. The colours are **our own** choice (see the palette below), and the
//! text is rasterized with the same 1-bit system-font path the playlist uses
//! ([`crate::playlist_font`]): every pixel is either opaque ink or
//! transparent, so a band holds exactly the palette's colours and nothing
//! blended.
//!
//! # The geometry
//!
//! The layout gives the columns in **text cells**; the renderer maps a cell
//! to a pixel with its own metrics ([`LIBRARY_CELL_WIDTH`],
//! [`LIBRARY_LINE_HEIGHT`]) and stacks the lines with its own line height.
//! Every column is drawn at its own cell offset and **clipped to its own
//! column box**: a proportional system font would otherwise put every row's
//! artist under a different pixel, and the list would not read as columns.
//!
//! The viewport is split into four bands:
//!
//! * the **header** at the top: the column titles on the panel background;
//! * a one-pixel **separator** under it;
//! * the **rows** in the middle: the rows at the scroll offset, the selected
//!   one on the selection background;
//! * the **status line** at the bottom: the visible range and the total.
//!
//! Everything is clipped to the viewport: a row that does not fit is not
//! drawn, a column that does not fit is cut off, and no pixel outside the
//! viewport is touched.

use rustamp_gfx::{Image, Rect};

use crate::library::LibraryView;
use crate::library_layout::LibraryLayout;

// ---------------------------------------------------------------------------
// The palette (own choice)
// ---------------------------------------------------------------------------

/// The background behind the rows.
pub const LIBRARY_WINDOW_BACKGROUND: [u8; 3] = [32, 32, 32];

/// The background of the header band and of the status band.
pub const LIBRARY_PANEL_BACKGROUND: [u8; 3] = [48, 48, 52];

/// The one-pixel line under the header.
pub const LIBRARY_SEPARATOR: [u8; 3] = [80, 80, 80];

/// The column titles.
pub const LIBRARY_HEADER_TEXT: [u8; 3] = [232, 232, 236];

/// The text of an unselected row.
pub const LIBRARY_ROW_TEXT: [u8; 3] = [208, 208, 208];

/// The background of the selected row.
pub const LIBRARY_SELECTION: [u8; 3] = [38, 79, 120];

/// The text of the selected row.
pub const LIBRARY_SELECTION_TEXT: [u8; 3] = [255, 255, 255];

/// The status line's text.
pub const LIBRARY_STATUS_TEXT: [u8; 3] = [200, 200, 200];

// ---------------------------------------------------------------------------
// The metrics (own choice)
// ---------------------------------------------------------------------------

/// The width of one text cell, in pixels.
///
/// The layout counts its columns in cells; this is the cell's pixel width.
/// The window is therefore `layout.width() * LIBRARY_CELL_WIDTH` pixels wide
/// at the default layout.
pub const LIBRARY_CELL_WIDTH: u32 = 7;

/// The height of one text line, in pixels.
///
/// The system font is rasterized at 13 px; the extra three pixels are the
/// leading between two lines.
pub const LIBRARY_LINE_HEIGHT: u32 = 16;

/// The system font family the library lists are drawn in.
///
/// The rasterizer falls back to Arial and then to the sans-serif family, so a
/// host without this font still draws a readable list.
pub const LIBRARY_FONT_FAMILY: &str = "Segoe UI";

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Draws the library window's content into `frame`, inside `viewport`.
///
/// `viewport` is the part of `frame` the library occupies: the app gives it
/// the whole client area, or the part below a search field it draws itself.
/// The layout gives the columns in text cells, so the app resizes it to the
/// window with `LibraryLayout::resized_to(viewport.width / LIBRARY_CELL_WIDTH)`.
///
/// Nothing outside `viewport` is touched.
pub fn render_library_window(
    frame: &mut Image,
    view: &LibraryView,
    layout: &LibraryLayout,
    viewport: Rect,
) {
    let bands = Bands::new(viewport);
    frame.fill_rect(viewport, opaque(LIBRARY_WINDOW_BACKGROUND));
    frame.fill_rect(bands.header, opaque(LIBRARY_PANEL_BACKGROUND));
    frame.fill_rect(bands.separator, opaque(LIBRARY_SEPARATOR));
    frame.fill_rect(bands.status, opaque(LIBRARY_PANEL_BACKGROUND));

    draw_columns(
        frame,
        layout,
        &layout.header_line(),
        bands.header,
        LIBRARY_HEADER_TEXT,
    );
    draw_rows(frame, view, layout, bands.rows);
    draw_text(
        frame,
        &view.status_line(),
        bands.status,
        LIBRARY_STATUS_TEXT,
    );
}

/// The four bands the viewport is split into.
struct Bands {
    /// The column titles.
    header: Rect,
    /// The one-pixel line under the header.
    separator: Rect,
    /// The rows.
    rows: Rect,
    /// The status line.
    status: Rect,
}

impl Bands {
    /// Splits the viewport into its bands.
    ///
    /// A viewport too short for the header and the status line gives its
    /// height to the header first; the bands never overlap and never leave
    /// the viewport.
    fn new(viewport: Rect) -> Self {
        let header = Rect::new(
            viewport.x,
            viewport.y,
            viewport.width,
            LIBRARY_LINE_HEIGHT.min(viewport.height),
        );
        let separator_y = viewport.y + LIBRARY_LINE_HEIGHT as i32;
        let separator = Rect::new(
            viewport.x,
            separator_y,
            viewport.width,
            u32::from(separator_y < viewport.bottom()),
        );
        let status_height =
            LIBRARY_LINE_HEIGHT.min(viewport.height.saturating_sub(LIBRARY_LINE_HEIGHT + 1));
        let status = Rect::new(
            viewport.x,
            viewport.bottom() - status_height as i32,
            viewport.width,
            status_height,
        );
        let rows_top = separator.y + separator.height as i32;
        let rows = Rect::new(
            viewport.x,
            rows_top,
            viewport.width,
            (status.y - rows_top).max(0) as u32,
        );
        Self {
            header,
            separator,
            rows,
            status,
        }
    }
}

/// Draws the rows at the scroll offset, the selected one on its background.
///
/// Only the rows that fit the band are drawn: the band is the viewport's
/// rows area, and a row below it would lie outside the window.
fn draw_rows(frame: &mut Image, view: &LibraryView, layout: &LibraryLayout, band: Rect) {
    let capacity = (band.height / LIBRARY_LINE_HEIGHT) as usize;
    let selected = view.selected();
    for (index, row) in view.visible_rows().iter().take(capacity).enumerate() {
        let row_band = Rect::new(
            band.x,
            band.y + index as i32 * LIBRARY_LINE_HEIGHT as i32,
            band.width,
            LIBRARY_LINE_HEIGHT,
        );
        let is_selected = Some(row.id) == selected;
        if is_selected {
            frame.fill_rect(row_band, opaque(LIBRARY_SELECTION));
        }
        let colour = if is_selected {
            LIBRARY_SELECTION_TEXT
        } else {
            LIBRARY_ROW_TEXT
        };
        draw_columns(frame, layout, &layout.row_line(row), row_band, colour);
    }
}

/// Draws one layout line: every column at its own cell offset, clipped to
/// its own column box.
///
/// The line is the layout's own text - the column titles for the header, the
/// row's four values for a row - so the renderer never assembles a row of
/// its own.
fn draw_columns(
    frame: &mut Image,
    layout: &LibraryLayout,
    line: &str,
    band: Rect,
    colour: [u8; 3],
) {
    for column in layout.columns() {
        let start = layout.column_x(column) as usize;
        let width = layout.column_width(column) as usize;
        let text = column_text(line, start, width);
        if text.trim().is_empty() {
            continue;
        }
        let x = band.x + start as i32 * LIBRARY_CELL_WIDTH as i32;
        let right = band.x + band.width as i32;
        let column_right = x + width as i32 * LIBRARY_CELL_WIDTH as i32;
        let visible = column_right.min(right);
        if visible <= x {
            continue;
        }
        draw_text(
            frame,
            text,
            Rect::new(x, band.y, (visible - x) as u32, band.height),
            colour,
        );
    }
}

/// Draws `text` at the left edge of `band`, clipped to the band.
fn draw_text(frame: &mut Image, text: &str, band: Rect, colour: [u8; 3]) {
    if text.is_empty() || band.is_empty() {
        return;
    }
    let Some(scratch) =
        crate::playlist_font::render(LIBRARY_FONT_FAMILY, text, band.width, band.height, colour)
    else {
        return;
    };
    frame.blit_alpha(&scratch, scratch.bounds(), band.x, band.y);
}

/// The `width` characters of `line` that start at the character `start`.
///
/// The layout pads every column to its width, so a column's text is a slice
/// of the line: the slice keeps the value and its padding.
fn column_text(line: &str, start: usize, width: usize) -> &str {
    let from = char_index(line, start);
    let to = char_index(line, start + width);
    &line[from..to]
}

/// The byte index of the character at `index`, or the length of the string
/// when it has fewer characters.
fn char_index(line: &str, index: usize) -> usize {
    line.char_indices()
        .nth(index)
        .map(|(offset, _)| offset)
        .unwrap_or(line.len())
}

/// An opaque RGBA colour from an RGB triple.
fn opaque(rgb: [u8; 3]) -> [u8; 4] {
    [rgb[0], rgb[1], rgb[2], 255]
}
