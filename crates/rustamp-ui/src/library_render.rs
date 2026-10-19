// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The media library window's renderer: the chrome, the list, the selection
//! and the status line, drawn into a frame.
//!
//! [`crate::library::LibraryView`] is the headless model of the media library
//! and [`crate::library_layout::LibraryLayout`] gives the list's columns in
//! text cells. This module draws them: it paints the search field, the
//! column titles, the rows at the scroll offset, the selection, the
//! scrollbar and the status line.
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
//! The viewport is split into five bands:
//!
//! * the **search field** at the top: the text as typed, a caret while the
//!   field has the focus and a hint while it is empty;
//! * the **header** below it: the column titles on the panel background;
//! * a one-pixel **separator** under the header;
//! * the **rows** in the middle: the rows at the scroll offset, the selected
//!   one on the selection background, and the **scrollbar** in a column on
//!   their right;
//! * the **status line** at the bottom: the visible range and the total.
//!
//! Everything is clipped to the viewport: a row that does not fit is not
//! drawn, a column that does not fit is cut off, and no pixel outside the
//! viewport is touched.
//!
//! # The cover column
//!
//! The list starts with a **cover column**: a fixed-width column that
//! holds the album art of the row ([`LIBRARY_COVER_WIDTH`] pixels). The
//! cover column is not a text column, so it is not part of the layout's
//! cell grid; the renderer shifts the text columns past it and draws the
//! cover into the cell of the row ([`cover_cell`]).
//!
//! The renderer takes the covers of the rows it draws as a slice - one
//! entry per visible row, [`Option`] because a track may have no cover. A
//! cover is a **borrowed** view of a picture ([`CoverRef`]): the renderer
//! borrows the bytes and never owns them, so the caller hands over the
//! picture it already has without copying it. A row without a cover keeps
//! its cell's background; the cell is only painted when there is a picture
//! for it.
//!
//! The picture **fills** its cell: it is scaled with nearest-neighbour
//! sampling by the larger of the two cell-to-cover ratios, so the aspect
//! ratio is kept and the cell is covered, and the part of the picture that
//! does not fit is cropped away at the cell's edges. The scaled picture is
//! blitted into the cell with its own alpha, so a transparent part of a
//! cover lets the cell's background through. Nothing of a cover is ever
//! drawn outside its cell.
//!
//! The cover column's **header cell** - the header band's slice of the
//! column - is empty: the column holds pictures, not text.
//!
//! # The chrome (own choice)
//!
//! No spec measures the media library (S20 is not started), so the chrome is
//! our own choice. It follows two rules:
//!
//! * **The geometry is a pure function of the viewport and the view.**
//!   [`LibraryChrome::new`] computes every rectangle from the two of them
//!   and from nothing else - no clock, no window, no mutable state. The app
//!   can therefore hit-test against the rectangles it drew, and the same
//!   viewport always gives the same chrome.
//! * **The window is the list.** The search field is a convenience above the
//!   list, so a window that is too short for everything gives its height to
//!   the header first and to the search field last.
//!
//! The **search field** is one text line at the top of the window, the full
//! width. It shows [`LibraryView::text`], the text as it was typed; while
//! the field is empty it shows a hint ([`LIBRARY_SEARCH_PLACEHOLDER`]) in a
//! dimmer colour, and while the field has the focus it shows a caret behind
//! the text. The caret stands at the width the text takes in the system
//! font, so it follows the text and not a cell grid.
//!
//! The **scrollbar** is a column on the right of the rows, as wide as
//! [`LIBRARY_SCROLLBAR_WIDTH`]. The rows band is narrowed by it, so no row
//! text ever runs under it. The **thumb** is as tall as the share of the
//! match set the window shows (`visible / total`): it fills the whole track
//! when everything fits, it is proportional when there is more to scroll,
//! and it is gone when there is nothing at all. Its place in the track is
//! the scroll's place in the range (`scroll / max_scroll`).
//!
//! A click in the track asks for the scroll it sits at:
//! [`scroll_for_click`] maps the click's y to a scroll offset, and the app
//! scrolls the view to it.

use rustamp_gfx::{Image, Rect};

use crate::library::{LibraryColumn, LibraryView};
use crate::library_layout::{LIBRARY_COVER_WIDTH, LibraryLayout};

// ---------------------------------------------------------------------------
// The cover picture
// ---------------------------------------------------------------------------

/// A borrowed view of a cover picture.
///
/// The renderer does not need to own the picture, it needs to know its size
/// and to read its pixels. The view borrows the bytes, so the caller can
/// hand over a decoded cover - the artwork cache's, for instance - without
/// copying it.
///
/// The pixels are 8-bit non-premultiplied RGBA, row-major, the first row on
/// top: the same layout [`Image`] uses, so a caller that already has a
/// picture can lend its bytes as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverRef<'a> {
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The RGBA bytes: `width * height * 4` of them.
    pub pixels: &'a [u8],
}

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

/// The background of the search field's band.
pub const LIBRARY_SEARCH_BACKGROUND: [u8; 3] = [40, 40, 44];

/// The text typed into the search field.
pub const LIBRARY_SEARCH_TEXT: [u8; 3] = [232, 232, 236];

/// The hint the empty search field shows.
pub const LIBRARY_SEARCH_HINT: [u8; 3] = [128, 128, 134];

/// The caret of the focused search field.
pub const LIBRARY_SEARCH_CARET: [u8; 3] = [255, 255, 255];

/// The scrollbar's track.
pub const LIBRARY_SCROLLBAR_TRACK: [u8; 3] = [24, 24, 28];

/// The scrollbar's thumb.
pub const LIBRARY_SCROLLBAR_THUMB: [u8; 3] = [112, 112, 118];

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

/// The width of the scrollbar, in pixels.
pub const LIBRARY_SCROLLBAR_WIDTH: u32 = 12;

/// The narrowest the scrollbar's thumb may become, in pixels.
///
/// A thumb that is thinner than this cannot be grabbed, so it keeps this
/// height even when the match set is much longer than the window.
pub const LIBRARY_MIN_THUMB_HEIGHT: u32 = 8;

/// The width of the caret in the search field, in pixels.
pub const LIBRARY_CARET_WIDTH: u32 = 2;

/// The padding between the search field's left edge and its text, in pixels.
///
/// The padding is one text cell, so the text starts where a column of the
/// list would start.
pub const LIBRARY_SEARCH_PADDING: u32 = LIBRARY_CELL_WIDTH;

/// The hint the search field shows while it is empty.
pub const LIBRARY_SEARCH_PLACEHOLDER: &str = "Search the library";

/// The system font family the library lists are drawn in.
///
/// The rasterizer falls back to Arial and then to the sans-serif family, so a
/// host without this font still draws a readable list.
pub const LIBRARY_FONT_FAMILY: &str = "Segoe UI";

// ---------------------------------------------------------------------------
// The geometry
// ---------------------------------------------------------------------------

/// The geometry of the media library window.
///
/// Every rectangle is a band of the window or a part of the scrollbar, and
/// every one of them lies inside the viewport. The geometry is a pure
/// function of the viewport and the view ([`LibraryChrome::new`]), so the
/// app can map a click or a key to the band it belongs to.
///
/// The bands are, from the top: the search field, the header, the
/// separator, the rows and the status line. The scrollbar is a column on
/// the right of the rows, and the rows band is narrowed by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryChrome {
    /// The search field's band: the top of the window, the full width.
    pub search: Rect,
    /// The column titles' band.
    pub header: Rect,
    /// The one-pixel line under the header.
    pub separator: Rect,
    /// The rows' band: the list, without the scrollbar.
    pub rows: Rect,
    /// The status line's band: the bottom of the window, the full width.
    pub status: Rect,
    /// The scrollbar's track: the right-hand column of the rows' band.
    pub scrollbar: Rect,
    /// The scrollbar's thumb: the part of the track that shows the scroll.
    pub thumb: Rect,
}

impl LibraryChrome {
    /// The chrome of a library window with the given viewport.
    ///
    /// The viewport is the part of the frame the window occupies. The
    /// geometry depends on the viewport and on the view alone: the view
    /// says how many rows the window shows, how many rows there are and
    /// where the scroll is, and the window says how much room it has.
    ///
    /// A window that is too short for all its bands gives the room to the
    /// header first and to the search field last: the list is the window.
    pub fn new(view: &LibraryView, viewport: Rect) -> Self {
        let header_height = LIBRARY_LINE_HEIGHT.min(viewport.height);
        let search_height = LIBRARY_LINE_HEIGHT.min(viewport.height - header_height);
        let bands_height = search_height + header_height;
        let separator_height = u32::from(viewport.height > bands_height);
        let free = viewport.height - bands_height - separator_height;
        let status_height = LIBRARY_LINE_HEIGHT.min(free);
        let rows_height = free - status_height;

        let search_y = viewport.y;
        let header_y = search_y + search_height as i32;
        let separator_y = header_y + header_height as i32;
        let rows_y = separator_y + separator_height as i32;
        let status_y = viewport.bottom() - status_height as i32;

        let scrollbar_width = LIBRARY_SCROLLBAR_WIDTH.min(viewport.width);
        let rows_width = viewport.width - scrollbar_width;
        let rows = Rect::new(viewport.x, rows_y, rows_width, rows_height);
        let scrollbar = Rect::new(
            viewport.x + rows_width as i32,
            rows_y,
            scrollbar_width,
            rows_height,
        );
        let thumb = thumb_rect(view, scrollbar);

        Self {
            search: Rect::new(viewport.x, search_y, viewport.width, search_height),
            header: Rect::new(viewport.x, header_y, viewport.width, header_height),
            separator: Rect::new(viewport.x, separator_y, viewport.width, separator_height),
            rows,
            status: Rect::new(viewport.x, status_y, viewport.width, status_height),
            scrollbar,
            thumb,
        }
    }

    /// The number of text cells the rows' band holds.
    ///
    /// The cover column takes [`LIBRARY_COVER_WIDTH`] pixels of the band,
    /// so the text columns have the rest of it. The app sizes the layout
    /// with it: `layout.resized_to(chrome.text_cells())`.
    pub fn text_cells(&self) -> u32 {
        self.rows.width.saturating_sub(LIBRARY_COVER_WIDTH) / LIBRARY_CELL_WIDTH
    }
}

/// The cover cell of the row at `index` in the rows' band.
///
/// The cell is the cover column's slice of the row: [`LIBRARY_COVER_WIDTH`]
/// pixels wide and one line high. The header's cover cell is the same
/// column in the header's band, and it is empty.
pub fn cover_cell(band: Rect, index: usize) -> Rect {
    let y = band.y.saturating_add(
        i32::try_from(index)
            .unwrap_or(i32::MAX)
            .saturating_mul(LIBRARY_LINE_HEIGHT as i32),
    );
    Rect::new(band.x, y, LIBRARY_COVER_WIDTH, LIBRARY_LINE_HEIGHT)
}

/// The pixel x of a text column in the list.
///
/// The cover column leads the list, so the text columns begin after it: the
/// title stands at [`LIBRARY_COVER_WIDTH`], the next column one text cell
/// further. The offset is the same for the header and for the rows, so the
/// two line up.
pub fn text_column_x(list_x: i32, layout: &LibraryLayout, column: LibraryColumn) -> i32 {
    list_x
        .saturating_add(LIBRARY_COVER_WIDTH as i32)
        .saturating_add(
            i32::try_from(layout.column_x(column))
                .unwrap_or(i32::MAX)
                .saturating_mul(LIBRARY_CELL_WIDTH as i32),
        )
}

/// The scrollbar's thumb for a view.
///
/// The thumb is as tall as the share of the match set the window shows: it
/// fills the track when everything fits, it is proportional to
/// `visible / total` when there is more to scroll, and it is gone when
/// there is nothing at all. Its place in the track is the scroll's place in
/// the range: `scroll / max_scroll`.
fn thumb_rect(view: &LibraryView, track: Rect) -> Rect {
    let total = view.total();
    if total == 0 || track.height == 0 {
        return Rect::new(track.x, track.y, track.width, 0);
    }
    let visible = view.viewport();
    let height = if total <= visible {
        track.height
    } else {
        let proportional = track.height as usize * visible / total;
        proportional
            .max(LIBRARY_MIN_THUMB_HEIGHT as usize)
            .min(track.height as usize) as u32
    };
    let travel = track.height - height;
    let max_scroll = view.max_scroll();
    let offset = (travel as usize * view.scroll())
        .checked_div(max_scroll)
        .unwrap_or(0)
        .min(travel as usize);
    Rect::new(track.x, track.y + offset as i32, track.width, height)
}

/// The scroll offset a click at `y` in the scrollbar asks for.
///
/// The click's place in the track is the place in the scroll range: a click
/// at the top of the track asks for the first row, a click at its bottom
/// for the last screenful, and a click in between for the offset in
/// between. A click outside the track asks for the nearest end.
///
/// The app scrolls the view to the offset and runs the view's request
/// again; a match set that fits into the window has no scroll at all.
pub fn scroll_for_click(view: &LibraryView, chrome: &LibraryChrome, y: i32) -> usize {
    let max_scroll = view.max_scroll();
    let track = chrome.scrollbar;
    if max_scroll == 0 || track.height == 0 {
        return 0;
    }
    // The last pixel of the track is the last screenful.
    let span = (track.height - 1).max(1) as i64;
    let offset = (y - track.y).clamp(0, span as i32) as i64;
    ((offset * max_scroll as i64 + span / 2) / span) as usize
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Draws the library window's content into `frame`, inside `viewport`.
///
/// `viewport` is the part of `frame` the library occupies: the app gives it
/// the whole client area. The layout gives the columns in text cells, so
/// the app resizes it to the list with
/// `LibraryLayout::resized_to(chrome.text_cells())`.
///
/// `search_focused` says whether the search field has the focus: a focused
/// field shows a caret behind the text.
///
/// `covers` holds the cover of every visible row, in the order of
/// [`LibraryView::visible_rows`]: the first entry belongs to the first
/// visible row, the second to the second, and so on. A row whose cover is
/// missing - or a slice that is shorter than the visible rows - is drawn
/// without a cover, and its cell keeps the row's background.
///
/// A cover is a borrowed view of a picture ([`CoverRef`]): the caller lends
/// the bytes it already has - the artwork cache's cover, for instance - and
/// the renderer neither owns nor copies them.
///
/// The chrome the renderer draws is the one
/// [`LibraryChrome::new`](LibraryChrome::new) computes from the same
/// viewport and the same view, so the app can hit-test against it and get
/// the bands that were drawn.
///
/// Nothing outside `viewport` is touched.
pub fn render_library_window(
    frame: &mut Image,
    view: &LibraryView,
    layout: &LibraryLayout,
    viewport: Rect,
    search_focused: bool,
    covers: &[Option<CoverRef<'_>>],
) {
    let chrome = LibraryChrome::new(view, viewport);
    frame.fill_rect(viewport, opaque(LIBRARY_WINDOW_BACKGROUND));
    frame.fill_rect(chrome.header, opaque(LIBRARY_PANEL_BACKGROUND));
    frame.fill_rect(chrome.separator, opaque(LIBRARY_SEPARATOR));
    frame.fill_rect(chrome.status, opaque(LIBRARY_PANEL_BACKGROUND));

    draw_columns(
        frame,
        layout,
        &layout.header_line(),
        chrome.header,
        LIBRARY_HEADER_TEXT,
    );
    draw_rows(frame, view, layout, chrome.rows, covers);
    draw_text(
        frame,
        &view.status_line(),
        chrome.status,
        LIBRARY_STATUS_TEXT,
    );
    draw_search(frame, view, &chrome, search_focused);
    draw_scrollbar(frame, &chrome);
}

/// Draws the rows at the scroll offset, the selected one on its background.
///
/// Only the rows that fit the band are drawn: the band is the viewport's
/// rows area, and a row below it would lie outside the window. Every row
/// that has a cover gets it in the cover column's cell of that row.
fn draw_rows(
    frame: &mut Image,
    view: &LibraryView,
    layout: &LibraryLayout,
    band: Rect,
    covers: &[Option<CoverRef<'_>>],
) {
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
        if let Some(cover) = covers.get(index).and_then(|cover| *cover) {
            draw_cover(frame, cover, cover_cell(band, index));
        }
        let colour = if is_selected {
            LIBRARY_SELECTION_TEXT
        } else {
            LIBRARY_ROW_TEXT
        };
        draw_columns(frame, layout, &layout.row_line(row), row_band, colour);
    }
}

/// Draws a cover into the cell of its row.
///
/// The picture is scaled to the cell and clipped to it, so nothing of the
/// cover is drawn outside the cell it belongs to.
fn draw_cover(frame: &mut Image, cover: CoverRef<'_>, cell: Rect) {
    let Some(scaled) = scaled_cover(cover, cell) else {
        return;
    };
    frame.blit_alpha(&scaled, scaled.bounds(), cell.x, cell.y);
}

/// The cover scaled into the cell, as an image.
///
/// The picture **fills** the cell: it is scaled by the larger of the two
/// cell-to-cover ratios, so the aspect ratio is kept and the cell is
/// covered, and the part of the picture that does not fit is cropped away
/// at the cell's edges. The sampling is nearest-neighbour: every pixel of
/// the cell takes the pixel of the cover nearest to it.
///
/// The cover's pixels are already in the [`Image`] layout - the same
/// non-premultiplied RGBA, row by row - so the picture is read in place:
/// there is no second image type and no copy of the whole cover.
///
/// `None` when the cell is empty or when the cover does not match its own
/// size: a broken cover is drawn as no cover at all.
fn scaled_cover(cover: CoverRef<'_>, cell: Rect) -> Option<Image> {
    let (cover_width, cover_height) = (cover.width, cover.height);
    if cover_width == 0 || cover_height == 0 || cell.is_empty() {
        return None;
    }
    let expected = u64::from(cover_width)
        .checked_mul(u64::from(cover_height))
        .and_then(|pixels| pixels.checked_mul(4));
    if expected != Some(cover.pixels.len() as u64) {
        // A cover whose pixels do not match its size is broken: it is drawn
        // as no cover.
        return None;
    }

    // The scale that covers the cell: the larger of the two ratios, as a
    // fraction. `num / den` is that factor.
    let num = (u128::from(cell.width) * u128::from(cover_height))
        .max(u128::from(cell.height) * u128::from(cover_width));
    let den = u128::from(cover_width) * u128::from(cover_height);
    let scaled_width = (u128::from(cover_width) * num).div_ceil(den);
    let scaled_height = (u128::from(cover_height) * num).div_ceil(den);
    // The scaled picture is at least as large as the cell, so it overflows
    // on one axis; the overflow is the part that is cropped away.
    let offset_x = (scaled_width - u128::from(cell.width)) / 2;
    let offset_y = (scaled_height - u128::from(cell.height)) / 2;

    let mut scaled = Image::new(cell.width, cell.height);
    for y in 0..cell.height {
        let source_y = (offset_y + u128::from(y)) * u128::from(cover_height) / scaled_height;
        for x in 0..cell.width {
            let source_x = (offset_x + u128::from(x)) * u128::from(cover_width) / scaled_width;
            let index = (source_y as usize * cover_width as usize + source_x as usize) * 4;
            scaled.set_pixel(
                x as i32,
                y as i32,
                [
                    cover.pixels[index],
                    cover.pixels[index + 1],
                    cover.pixels[index + 2],
                    cover.pixels[index + 3],
                ],
            );
        }
    }
    Some(scaled)
}

/// Draws the search field: the text as typed, the hint while the field is
/// empty and the caret while the field has the focus.
fn draw_search(frame: &mut Image, view: &LibraryView, chrome: &LibraryChrome, focused: bool) {
    let band = chrome.search;
    if band.is_empty() {
        return;
    }
    frame.fill_rect(band, opaque(LIBRARY_SEARCH_BACKGROUND));
    let field = Rect::new(
        band.x + LIBRARY_SEARCH_PADDING as i32,
        band.y,
        band.width.saturating_sub(LIBRARY_SEARCH_PADDING),
        band.height,
    );
    let text = view.text();
    if text.is_empty() {
        draw_text(
            frame,
            LIBRARY_SEARCH_PLACEHOLDER,
            field,
            LIBRARY_SEARCH_HINT,
        );
    } else {
        draw_text(frame, text, field, LIBRARY_SEARCH_TEXT);
    }
    if focused {
        draw_caret(frame, text, field, band);
    }
}

/// Draws the caret of the focused search field.
///
/// The caret stands behind the text, at the width the text takes in the
/// system font; an empty field has its caret at the field's left edge. A
/// text that is longer than the field is clipped, and its caret stands at
/// the field's right edge.
fn draw_caret(frame: &mut Image, text: &str, field: Rect, band: Rect) {
    let width = crate::playlist_font::text_width(LIBRARY_FONT_FAMILY, text).unwrap_or(0);
    let caret_x = field
        .x
        .saturating_add(width as i32)
        .min(band.right() - LIBRARY_CARET_WIDTH as i32);
    if caret_x < field.x {
        return;
    }
    let height = band.height.saturating_sub(4);
    if height == 0 {
        return;
    }
    let y = band.y + ((band.height - height) / 2) as i32;
    frame.fill_rect(
        Rect::new(caret_x, y, LIBRARY_CARET_WIDTH, height),
        opaque(LIBRARY_SEARCH_CARET),
    );
}

/// Draws the scrollbar: the track and the thumb.
fn draw_scrollbar(frame: &mut Image, chrome: &LibraryChrome) {
    frame.fill_rect(chrome.scrollbar, opaque(LIBRARY_SCROLLBAR_TRACK));
    frame.fill_rect(chrome.thumb, opaque(LIBRARY_SCROLLBAR_THUMB));
}

/// Draws one layout line: every column at its own cell offset, clipped to
/// its own column box.
///
/// The line is the layout's own text - the column titles for the header, the
/// row's four values for a row - so the renderer never assembles a row of
/// its own. The columns start behind the cover column, so the line stands
/// at the same place in the header and in every row.
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
        let x = text_column_x(band.x, layout, column);
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
