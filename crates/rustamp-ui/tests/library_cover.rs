// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless tests of the media library's cover column.
//!
//! Every test paints into an in-memory frame and reads the pixels back: no
//! window, no skin, no clock, no database and no file. The covers are
//! synthetic pictures built in the test, so the tests assert the
//! **invariants** - where a cell is, what fills it and what stays outside
//! it - and never the exact shape of a glyph.

use std::path::PathBuf;

use rustamp_gfx::{Image, Rect};
use rustamp_ui::library::{LibraryColumn, LibraryRow, LibraryView};
use rustamp_ui::library_layout::{LIBRARY_COVER_WIDTH, LibraryLayout};
use rustamp_ui::library_render::{
    CoverRef, LIBRARY_CELL_WIDTH, LIBRARY_LINE_HEIGHT, LIBRARY_ROW_TEXT, LIBRARY_SELECTION,
    LIBRARY_WINDOW_BACKGROUND, LibraryChrome, cover_cell, render_library_window, text_column_x,
};

/// A red, fully opaque pixel.
const RED: [u8; 4] = [255, 0, 0, 255];

/// A blue, fully opaque pixel.
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// A transparent pixel.
const CLEAR: [u8; 4] = [0, 0, 0, 0];

/// An opaque RGBA colour from an RGB triple.
fn rgba(rgb: [u8; 3]) -> [u8; 4] {
    [rgb[0], rgb[1], rgb[2], 255]
}

/// A row with the display values the layout needs.
fn row(id: i64) -> LibraryRow {
    LibraryRow {
        id,
        path: PathBuf::from(format!("C:/music/track{id}.flac")),
        title: "Title".to_string(),
        artist: "Artist".to_string(),
        album: "Album".to_string(),
        duration_ms: Some(215_000),
    }
}

/// A view that holds `rows` and knows `total` matches.
fn view(rows: Vec<LibraryRow>, total: usize) -> LibraryView {
    let mut view = LibraryView::new(rows.len());
    view.apply_page(rows, total, 0);
    view
}

/// A cover of one colour: the raw RGBA bytes of a picture.
fn cover(width: u32, height: u32, colour: [u8; 4]) -> Vec<u8> {
    colour.repeat((width * height) as usize)
}

/// The pixels of a cover whose left half is blue and whose right half is
/// red.
fn half_cover(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::new();
    for _ in 0..height {
        for x in 0..width {
            let colour = if x < width / 2 { BLUE } else { RED };
            pixels.extend_from_slice(&colour);
        }
    }
    pixels
}

/// A borrowed view of a picture of `width` x `height` pixels.
fn cover_ref(width: u32, height: u32, pixels: &[u8]) -> CoverRef<'_> {
    CoverRef {
        width,
        height,
        pixels,
    }
}

/// The first x in `rect` that holds `colour`, or `None`.
fn first_ink_x(frame: &Image, rect: Rect, colour: [u8; 4]) -> Option<i32> {
    (rect.x..rect.right())
        .find(|x| (rect.y..rect.bottom()).any(|y| frame.pixel(*x, y) == Some(colour)))
}

/// The number of pixels of `colour` in `rect`.
fn count(frame: &Image, rect: Rect, colour: [u8; 4]) -> usize {
    let mut found = 0;
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if frame.pixel(x, y) == Some(colour) {
                found += 1;
            }
        }
    }
    found
}

#[test]
fn every_row_has_a_cover_cell_of_the_same_width() {
    let view = view(vec![row(1), row(2), row(3)], 3);
    let viewport = Rect::new(0, 0, 700, 100);
    let chrome = LibraryChrome::new(&view, viewport);

    for index in 0..3 {
        let cell = cover_cell(chrome.rows, index);
        assert_eq!(cell.x, chrome.rows.x, "the cell leads the row");
        assert_eq!(cell.width, LIBRARY_COVER_WIDTH, "the cell is the column");
        assert_eq!(cell.height, LIBRARY_LINE_HEIGHT, "the cell is one line");
        assert_eq!(
            cell.y,
            chrome.rows.y + index as i32 * LIBRARY_LINE_HEIGHT as i32,
            "the cell of the row at {index}"
        );
    }
}

#[test]
fn a_cover_is_drawn_into_the_cell_of_its_row() {
    let view = view(vec![row(1), row(2)], 2);
    let viewport = Rect::new(0, 0, 700, 100);
    let chrome = LibraryChrome::new(&view, viewport);
    let red = cover(8, 8, RED);
    let covers = vec![Some(cover_ref(8, 8, &red)), None];

    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(
        &mut frame,
        &view,
        &LibraryLayout::default(),
        viewport,
        false,
        &covers,
    );

    // The whole cell of the first row is the cover: the picture fills it.
    let cell = cover_cell(chrome.rows, 0);
    assert_eq!(
        count(&frame, cell, RED),
        (cell.width * cell.height) as usize,
        "the cover fills its cell"
    );

    // The second row has no cover: its cell keeps the row's background.
    let empty = cover_cell(chrome.rows, 1);
    assert_eq!(
        count(&frame, empty, rgba(LIBRARY_WINDOW_BACKGROUND)),
        (empty.width * empty.height) as usize,
        "the cell of a row without a cover is empty"
    );
}

#[test]
fn a_cover_stays_inside_its_cell() {
    let view = view(vec![row(1), row(2)], 2);
    let viewport = Rect::new(0, 0, 700, 100);
    let chrome = LibraryChrome::new(&view, viewport);
    let red = cover(8, 8, RED);
    let covers = vec![Some(cover_ref(8, 8, &red)), None];

    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(
        &mut frame,
        &view,
        &LibraryLayout::default(),
        viewport,
        false,
        &covers,
    );

    let cell = cover_cell(chrome.rows, 0);
    // Right of the cell: the row's text, never the cover.
    for y in cell.y..cell.bottom() {
        assert_ne!(
            frame.pixel(cell.right(), y),
            Some(RED),
            "the cover reached past its cell at {y}"
        );
    }
    // Below the cell: the next row's cell, which has no cover.
    for x in cell.x..cell.right() {
        assert_ne!(
            frame.pixel(x, cell.bottom()),
            Some(RED),
            "the cover reached into the next row at {x}"
        );
    }
    // Above the cell: the header.
    for x in cell.x..cell.right() {
        assert_ne!(
            frame.pixel(x, cell.y - 1),
            Some(RED),
            "the cover reached into the header at {x}"
        );
    }
}

#[test]
fn a_cover_is_scaled_to_the_cell_and_cropped_at_its_edges() {
    let view = view(vec![row(1)], 1);
    let viewport = Rect::new(0, 0, 700, 100);
    let chrome = LibraryChrome::new(&view, viewport);
    // A wide cover: the cell is narrower, so the picture is cropped at the
    // left and at the right edge.
    let wide = half_cover(64, 16);
    let covers = vec![Some(cover_ref(64, 16, &wide))];

    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(
        &mut frame,
        &view,
        &LibraryLayout::default(),
        viewport,
        false,
        &covers,
    );

    let cell = cover_cell(chrome.rows, 0);
    // The cover's middle is blue and its right half is red; the cell shows
    // the middle 40 columns of the 64, so the colours change in the middle
    // of the cell.
    let middle = cell.x + cell.width as i32 / 2;
    assert_eq!(
        frame.pixel(middle - 1, cell.y),
        Some(BLUE),
        "the left half of the cover is blue"
    );
    assert_eq!(
        frame.pixel(middle, cell.y),
        Some(RED),
        "the right half of the cover is red"
    );
}

#[test]
fn a_transparent_cover_leaves_the_background_of_the_cell() {
    let view = view(vec![row(1)], 1);
    let viewport = Rect::new(0, 0, 700, 100);
    let chrome = LibraryChrome::new(&view, viewport);
    let clear = cover(8, 8, CLEAR);
    let covers = vec![Some(cover_ref(8, 8, &clear))];

    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(
        &mut frame,
        &view,
        &LibraryLayout::default(),
        viewport,
        false,
        &covers,
    );

    let cell = cover_cell(chrome.rows, 0);
    assert_eq!(
        count(&frame, cell, rgba(LIBRARY_WINDOW_BACKGROUND)),
        (cell.width * cell.height) as usize,
        "a transparent cover lets the background through"
    );
}

#[test]
fn the_text_columns_start_exactly_behind_the_cover_column() {
    let layout = LibraryLayout::default();
    let list_x = 12;
    assert_eq!(
        text_column_x(list_x, &layout, LibraryColumn::Title),
        list_x + LIBRARY_COVER_WIDTH as i32,
        "the title stands behind the cover column"
    );
    assert_eq!(
        text_column_x(list_x, &layout, LibraryColumn::Artist),
        list_x + LIBRARY_COVER_WIDTH as i32 + 41 * LIBRARY_CELL_WIDTH as i32,
        "the artist stands one title and one gap further"
    );

    // The pixels agree with the geometry: the row's text starts behind the
    // cover column and inside the title's own cell.
    let view = view(vec![row(1)], 1);
    let viewport = Rect::new(0, 0, 700, 100);
    let chrome = LibraryChrome::new(&view, viewport);
    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);

    let band = Rect::new(
        chrome.rows.x,
        chrome.rows.y,
        chrome.rows.width,
        LIBRARY_LINE_HEIGHT,
    );
    let first = first_ink_x(&frame, band, rgba(LIBRARY_ROW_TEXT)).expect("the row is drawn");
    assert!(
        first >= chrome.rows.x + LIBRARY_COVER_WIDTH as i32,
        "the text starts behind the cover column, not at {first}"
    );
    assert!(
        first < chrome.rows.x + LIBRARY_COVER_WIDTH as i32 + LIBRARY_CELL_WIDTH as i32,
        "the text starts in the title's own cell, not at {first}"
    );
}

#[test]
fn the_header_and_the_rows_line_up_behind_the_cover_column() {
    let view = view(vec![row(1)], 1);
    let layout = LibraryLayout::default();
    let viewport = Rect::new(0, 0, 700, 100);
    let chrome = LibraryChrome::new(&view, viewport);
    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);

    // The header's cover cell is empty: the column holds pictures, not
    // text, so nothing is drawn in the header's slice of the column.
    let header_cell = Rect::new(
        chrome.header.x,
        chrome.header.y,
        LIBRARY_COVER_WIDTH,
        chrome.header.height,
    );
    assert_eq!(
        count(&frame, header_cell, rgba(LIBRARY_ROW_TEXT)),
        0,
        "the header's cover cell holds no row text"
    );

    // The title of the header and the title of the row stand at the same
    // place.
    let title_x = text_column_x(chrome.rows.x, &layout, LibraryColumn::Title);
    let header_ink = first_ink_x(
        &frame,
        Rect::new(
            title_x,
            chrome.header.y,
            LIBRARY_CELL_WIDTH,
            chrome.header.height,
        ),
        rgba(LIBRARY_ROW_TEXT),
    );
    assert!(
        header_ink.is_none(),
        "the header is not drawn in the row's colour"
    );
    let row_ink = first_ink_x(
        &frame,
        Rect::new(
            title_x,
            chrome.rows.y,
            LIBRARY_CELL_WIDTH,
            LIBRARY_LINE_HEIGHT,
        ),
        rgba(LIBRARY_ROW_TEXT),
    );
    assert!(row_ink.is_some(), "the row's title is drawn in its cell");
}

#[test]
fn the_cover_of_a_selected_row_is_drawn_on_the_selection() {
    let mut view = view(vec![row(1), row(2)], 2);
    assert!(view.select(2), "the row is on the loaded page");
    let viewport = Rect::new(0, 0, 700, 100);
    let chrome = LibraryChrome::new(&view, viewport);
    let red = cover(8, 8, RED);
    let covers = vec![None, Some(cover_ref(8, 8, &red))];

    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(
        &mut frame,
        &view,
        &LibraryLayout::default(),
        viewport,
        false,
        &covers,
    );

    // The unselected row keeps the window's background, the selected row
    // the selection's, and the cover of the selected row stands on it.
    let plain = cover_cell(chrome.rows, 0);
    assert_eq!(
        count(&frame, plain, rgba(LIBRARY_WINDOW_BACKGROUND)),
        (plain.width * plain.height) as usize,
        "the unselected row keeps the background"
    );
    let selected = cover_cell(chrome.rows, 1);
    assert_eq!(
        count(&frame, selected, rgba(LIBRARY_SELECTION)),
        0,
        "the cover stands on the selection"
    );
    assert_eq!(
        count(&frame, selected, RED),
        (selected.width * selected.height) as usize,
        "the cover fills the cell of the selected row"
    );
}

#[test]
fn nothing_of_a_cover_is_drawn_outside_the_viewport() {
    let view = view(vec![row(1)], 1);
    let viewport = Rect::new(10, 20, 200, 60);
    let red = cover(64, 64, RED);
    let covers = vec![Some(cover_ref(64, 64, &red))];

    let mut frame = Image::filled(400, 200, [7, 11, 13, 255]);
    render_library_window(
        &mut frame,
        &view,
        &LibraryLayout::default(),
        viewport,
        false,
        &covers,
    );

    for y in 0..200 {
        for x in 0..400 {
            let inside = (viewport.x..viewport.right()).contains(&x)
                && (viewport.y..viewport.bottom()).contains(&y);
            if !inside {
                assert_eq!(
                    frame.pixel(x, y),
                    Some([7, 11, 13, 255]),
                    "the renderer touched {x},{y} outside the viewport"
                );
            }
        }
    }
}

#[test]
fn a_cover_that_breaks_its_own_size_is_drawn_as_no_cover() {
    let view = view(vec![row(1)], 1);
    let viewport = Rect::new(0, 0, 700, 100);
    let chrome = LibraryChrome::new(&view, viewport);
    // The pixels do not match the size: the cover is broken.
    let broken = RED.to_vec();
    let covers = vec![Some(cover_ref(8, 8, &broken))];

    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(
        &mut frame,
        &view,
        &LibraryLayout::default(),
        viewport,
        false,
        &covers,
    );

    let cell = cover_cell(chrome.rows, 0);
    assert_eq!(
        count(&frame, cell, rgba(LIBRARY_WINDOW_BACKGROUND)),
        (cell.width * cell.height) as usize,
        "a broken cover leaves the cell empty"
    );
}
