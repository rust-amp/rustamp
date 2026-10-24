// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless tests of the media library's renderer.
//!
//! Every test paints into an in-memory frame and reads the pixels back: no
//! window, no skin, no clock, no database. The text is the host's system
//! font, so the tests assert the **invariants** - which band holds which
//! colour, where a text starts, what is clipped - and never the exact shape
//! of a glyph.

use std::path::PathBuf;

use rustamp_gfx::{Image, Rect};
use rustamp_ui::library::{LibraryColumn, LibraryRow, LibraryView};
use rustamp_ui::library_layout::LibraryLayout;
use rustamp_ui::library_render::{
    LIBRARY_CELL_WIDTH, LIBRARY_HEADER_TEXT, LIBRARY_LINE_HEIGHT, LIBRARY_PANEL_BACKGROUND,
    LIBRARY_ROW_TEXT, LIBRARY_SCROLLBAR_WIDTH, LIBRARY_SELECTION, LIBRARY_SELECTION_TEXT,
    LIBRARY_STATUS_TEXT, LIBRARY_WINDOW_BACKGROUND, render_library_window,
};

/// The height of the search field's band: the text as typed.
const SEARCH_HEIGHT: i32 = LIBRARY_LINE_HEIGHT as i32;

/// The height of the header band: the column titles.
const HEADER_HEIGHT: i32 = LIBRARY_LINE_HEIGHT as i32;

/// The height of the line under the header.
const SEPARATOR_HEIGHT: i32 = 1;

/// An opaque RGBA colour from an RGB triple.
fn rgba(rgb: [u8; 3]) -> [u8; 4] {
    [rgb[0], rgb[1], rgb[2], 255]
}

/// A row with the display values the layout needs.
fn row(id: i64, title: &str, artist: &str, album: &str) -> LibraryRow {
    LibraryRow {
        id,
        path: PathBuf::from(format!("C:/music/track{id}.flac")),
        title: title.to_string(),
        artist: artist.to_string(),
        album: album.to_string(),
        duration_ms: Some(215_000),
    }
}

/// A view that holds `rows` and knows `total` matches.
fn view(rows: Vec<LibraryRow>, total: usize) -> LibraryView {
    let mut view = LibraryView::new(rows.len());
    view.apply_page(rows, total, 0);
    view
}

/// The header band of a viewport: below the search field.
fn header_band(viewport: Rect) -> Rect {
    Rect::new(
        viewport.x,
        viewport.y + SEARCH_HEIGHT,
        viewport.width,
        LIBRARY_LINE_HEIGHT,
    )
}

/// The band of the row at `index`: the list is narrowed by the scrollbar.
fn row_band(viewport: Rect, index: i32) -> Rect {
    Rect::new(
        viewport.x,
        viewport.y
            + SEARCH_HEIGHT
            + HEADER_HEIGHT
            + SEPARATOR_HEIGHT
            + index * LIBRARY_LINE_HEIGHT as i32,
        viewport.width - LIBRARY_SCROLLBAR_WIDTH,
        LIBRARY_LINE_HEIGHT,
    )
}

/// The status band: the last line of the viewport.
fn status_band(viewport: Rect) -> Rect {
    Rect::new(
        viewport.x,
        viewport.bottom() - LIBRARY_LINE_HEIGHT as i32,
        viewport.width,
        LIBRARY_LINE_HEIGHT,
    )
}

/// The rows area of a viewport: between the separator and the status band.
fn rows_area(viewport: Rect) -> Rect {
    Rect::new(
        viewport.x,
        viewport.y + SEARCH_HEIGHT + HEADER_HEIGHT + SEPARATOR_HEIGHT,
        viewport.width - LIBRARY_SCROLLBAR_WIDTH,
        (viewport.height as i32
            - SEARCH_HEIGHT
            - HEADER_HEIGHT
            - SEPARATOR_HEIGHT
            - LIBRARY_LINE_HEIGHT as i32)
            .max(0) as u32,
    )
}

/// The number of pixels of `colour` in `rect`.
fn count(frame: &Image, rect: Rect, colour: [u8; 3]) -> usize {
    let colour = rgba(colour);
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

/// The first x in `rect` that holds `colour`, or `None`.
fn first_ink_x(frame: &Image, rect: Rect, colour: [u8; 3]) -> Option<i32> {
    let colour = rgba(colour);
    (rect.x..rect.right())
        .find(|x| (rect.y..rect.bottom()).any(|y| frame.pixel(*x, y) == Some(colour)))
}

/// The last x in `rect` that holds `colour`, or `None`.
fn last_ink_x(frame: &Image, rect: Rect, colour: [u8; 3]) -> Option<i32> {
    let colour = rgba(colour);
    (rect.x..rect.right())
        .rev()
        .find(|x| (rect.y..rect.bottom()).any(|y| frame.pixel(*x, y) == Some(colour)))
}

/// The ink profile of `rect`: for every x, whether the band holds a pixel of
/// `colour` in that column.
fn ink_profile(frame: &Image, rect: Rect, colour: [u8; 3]) -> Vec<bool> {
    let colour = rgba(colour);
    (rect.x..rect.right())
        .map(|x| (rect.y..rect.bottom()).any(|y| frame.pixel(x, y) == Some(colour)))
        .collect()
}

/// Asserts that every pixel of `rect` is one of the two given colours.
fn assert_only_colours(frame: &Image, rect: Rect, first: [u8; 3], second: [u8; 3]) {
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            let pixel = frame.pixel(x, y).expect("the pixel is inside the frame");
            assert!(
                pixel == rgba(first) || pixel == rgba(second),
                "unexpected pixel {pixel:?} at {x},{y}"
            );
        }
    }
}

#[test]
fn the_viewport_is_split_into_the_header_the_rows_and_the_status_line() {
    let view = view(
        vec![
            row(1, "One", "Artist", "Album"),
            row(2, "Two", "Artist", "Album"),
        ],
        2,
    );
    let layout = LibraryLayout::default();
    let viewport = Rect::new(0, 0, 700, 100);
    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);

    // The header holds the column titles, and only the panel background and
    // the titles' own colour.
    let header = header_band(viewport);
    assert!(
        count(&frame, header, LIBRARY_HEADER_TEXT) > 0,
        "the column titles are drawn"
    );
    assert_only_colours(
        &frame,
        header,
        LIBRARY_PANEL_BACKGROUND,
        LIBRARY_HEADER_TEXT,
    );

    // The rows hold the row text on the window background.
    for index in 0..2 {
        let band = row_band(viewport, index);
        assert!(
            count(&frame, band, LIBRARY_ROW_TEXT) > 0,
            "row {index} is drawn"
        );
        assert_only_colours(&frame, band, LIBRARY_WINDOW_BACKGROUND, LIBRARY_ROW_TEXT);
    }

    // The status line is the last line of the viewport, and the status text
    // is nowhere else.
    let status = status_band(viewport);
    assert_eq!(status.bottom(), viewport.bottom());
    assert!(
        count(&frame, status, LIBRARY_STATUS_TEXT) > 0,
        "the status line is drawn"
    );
    let above = Rect::new(
        viewport.x,
        viewport.y,
        viewport.width,
        viewport.height - LIBRARY_LINE_HEIGHT,
    );
    assert_eq!(
        count(&frame, above, LIBRARY_STATUS_TEXT),
        0,
        "the status text belongs to the status band alone"
    );
}

#[test]
fn the_selected_row_is_highlighted_and_the_others_are_not() {
    let mut view = view(
        vec![
            row(1, "One", "Artist", "Album"),
            row(2, "Two", "Artist", "Album"),
            row(3, "Three", "Artist", "Album"),
        ],
        3,
    );
    assert!(view.select(2), "the row is on the loaded page");
    let layout = LibraryLayout::default();
    let viewport = Rect::new(0, 0, 700, 100);
    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);

    let selected = row_band(viewport, 1);
    assert!(
        count(&frame, selected, LIBRARY_SELECTION) > 0,
        "the selected row carries the selection background"
    );
    assert!(
        count(&frame, selected, LIBRARY_SELECTION_TEXT) > 0,
        "the selected row is drawn in the selection's text colour"
    );
    for index in [0, 2] {
        let band = row_band(viewport, index);
        assert_eq!(
            count(&frame, band, LIBRARY_SELECTION),
            0,
            "row {index} is not highlighted"
        );
        assert_eq!(
            count(&frame, band, LIBRARY_SELECTION_TEXT),
            0,
            "row {index} is not drawn in the selection's colour"
        );
    }
}

#[test]
fn the_columns_line_up_with_the_header() {
    // A row whose values are the column titles must draw the same pixels as
    // the header: the columns are placed by the layout's cell offsets, not by
    // the font's proportions.
    let view = view(vec![row(1, "Title", "Artist", "Album")], 1);
    let layout = LibraryLayout::default();
    let viewport = Rect::new(0, 0, 700, 100);
    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);

    let upto = layout.column_x(LibraryColumn::Duration) * LIBRARY_CELL_WIDTH;
    let header = ink_profile(
        &frame,
        Rect::new(
            viewport.x,
            header_band(viewport).y,
            upto,
            LIBRARY_LINE_HEIGHT,
        ),
        LIBRARY_HEADER_TEXT,
    );
    let row = ink_profile(
        &frame,
        Rect::new(
            viewport.x,
            row_band(viewport, 0).y,
            upto,
            LIBRARY_LINE_HEIGHT,
        ),
        LIBRARY_ROW_TEXT,
    );
    assert!(header.iter().any(|ink| *ink), "the header is drawn");
    assert_eq!(
        header, row,
        "a row that reads like the header draws the same pixels"
    );
}

#[test]
fn a_row_that_does_not_fit_the_viewport_is_not_drawn() {
    let rows: Vec<LibraryRow> = (1..=5)
        .map(|id| row(id, &format!("Track {id}"), "Artist", "Album"))
        .collect();
    let view = view(rows, 5);
    let layout = LibraryLayout::default();

    // A viewport with room for two rows: the search field, the header, the
    // separator, two rows and the status line.
    let short = Rect::new(0, 0, 700, LIBRARY_LINE_HEIGHT * 5 + 1);
    let mut frame = Image::filled(700, short.height, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, short, false, &[]);
    assert!(
        count(&frame, row_band(short, 0), LIBRARY_ROW_TEXT) > 0,
        "the first row is drawn"
    );
    assert!(
        count(&frame, row_band(short, 1), LIBRARY_ROW_TEXT) > 0,
        "the second row is drawn"
    );
    let area = rows_area(short);
    for y in area.bottom()..short.bottom() {
        for x in short.x..short.right() {
            assert_ne!(
                frame.pixel(x, y),
                Some(rgba(LIBRARY_ROW_TEXT)),
                "a row text at {x},{y} lies below the rows area"
            );
        }
    }

    // A taller viewport does draw the third row: the short one did not leave
    // it out for good.
    let tall = Rect::new(0, 0, 700, LIBRARY_LINE_HEIGHT * 8 + 1);
    let mut frame = Image::filled(700, tall.height, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, tall, false, &[]);
    assert!(
        count(&frame, row_band(tall, 2), LIBRARY_ROW_TEXT) > 0,
        "the third row is drawn when the viewport has room for it"
    );
}

#[test]
fn nothing_is_drawn_outside_the_viewport() {
    let view = view(vec![row(1, "One", "Artist", "Album")], 1);
    let layout = LibraryLayout::default();
    let viewport = Rect::new(10, 20, 200, 60);
    let mut frame = Image::filled(400, 200, [7, 11, 13, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);

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
fn the_status_line_starts_at_the_left_edge_of_the_viewport() {
    let view = view(
        vec![
            row(1, "One", "Artist", "Album"),
            row(2, "Two", "Artist", "Album"),
        ],
        1234,
    );
    let layout = LibraryLayout::default();
    let viewport = Rect::new(0, 0, 400, 100);
    let mut frame = Image::filled(400, 100, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);

    let status = status_band(viewport);
    let first = first_ink_x(&frame, status, LIBRARY_STATUS_TEXT).expect("the status line is drawn");
    assert!(
        (viewport.x..viewport.x + 4).contains(&first),
        "the status text starts at the left edge, not at {first}"
    );
    let last = last_ink_x(&frame, status, LIBRARY_STATUS_TEXT).expect("the status line is drawn");
    assert!(
        last < viewport.x + 200,
        "the status text is left aligned, it ends at {last}"
    );
}

#[test]
fn the_scan_status_is_drawn_in_the_status_band() {
    let mut view = view(
        vec![
            row(1, "One", "Artist", "Album"),
            row(2, "Two", "Artist", "Album"),
        ],
        1234,
    );
    let layout = LibraryLayout::default();
    let viewport = Rect::new(0, 0, 400, 100);
    let status = status_band(viewport);

    // The range line alone.
    let mut frame = Image::filled(400, 100, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);
    let range = ink_profile(&frame, status, LIBRARY_STATUS_TEXT);

    // The scan's text takes the band's place.
    view.set_scan_status("scanning: 120 seen, 30 added, 5 updated, 2 skipped");
    let mut frame = Image::filled(400, 100, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);

    assert!(
        count(&frame, status, LIBRARY_STATUS_TEXT) > 0,
        "the scan's text is drawn"
    );
    assert_ne!(
        ink_profile(&frame, status, LIBRARY_STATUS_TEXT),
        range,
        "the scan's text replaces the range line"
    );
    assert_only_colours(
        &frame,
        status,
        LIBRARY_PANEL_BACKGROUND,
        LIBRARY_STATUS_TEXT,
    );

    // The text belongs to the status band alone.
    let above = Rect::new(
        viewport.x,
        viewport.y,
        viewport.width,
        viewport.height - LIBRARY_LINE_HEIGHT,
    );
    assert_eq!(
        count(&frame, above, LIBRARY_STATUS_TEXT),
        0,
        "the scan's text stays in the status band"
    );

    // It stands where the range stands: at the left edge of the band.
    let first = first_ink_x(&frame, status, LIBRARY_STATUS_TEXT).expect("the band holds ink");
    assert!(
        (viewport.x..viewport.x + 4).contains(&first),
        "the scan's text starts at the left edge, not at {first}"
    );
}

#[test]
fn a_long_scan_status_is_clipped_to_the_status_band() {
    let mut view = view(vec![row(1, "One", "Artist", "Album")], 1234);
    view.set_scan_status(&"scanning: 120 seen, ".repeat(40));
    let layout = LibraryLayout::default();
    let viewport = Rect::new(0, 0, 200, 100);
    let status = status_band(viewport);
    let mut frame = Image::filled(200, 100, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);

    assert!(
        count(&frame, status, LIBRARY_STATUS_TEXT) > 0,
        "the scan's text is drawn"
    );

    // The text is cut off at the band's edge, not shortened before it.
    let last = last_ink_x(&frame, status, LIBRARY_STATUS_TEXT).expect("the band holds ink");
    assert!(
        last >= status.right() - 2 * LIBRARY_CELL_WIDTH as i32,
        "the text reaches the band's edge, it ends at {last}"
    );

    // Nothing of it reaches into the rows above the band.
    let above = Rect::new(
        viewport.x,
        viewport.y,
        viewport.width,
        viewport.height - LIBRARY_LINE_HEIGHT,
    );
    assert_eq!(
        count(&frame, above, LIBRARY_STATUS_TEXT),
        0,
        "the scan's text stays in the status band"
    );
}

#[test]
fn a_viewport_that_is_too_short_shows_the_header_and_nothing_else() {
    let view = view(
        vec![
            row(1, "One", "Artist", "Album"),
            row(2, "Two", "Artist", "Album"),
        ],
        2,
    );
    let layout = LibraryLayout::default();
    let viewport = Rect::new(0, 0, 700, 10);
    let mut frame = Image::filled(700, 10, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);

    assert!(
        count(&frame, Rect::new(0, 0, 700, 10), LIBRARY_HEADER_TEXT) > 0,
        "the header still fits"
    );
    assert_eq!(
        count(&frame, Rect::new(0, 0, 700, 10), LIBRARY_ROW_TEXT),
        0,
        "no row fits into ten pixels"
    );
    assert_eq!(
        count(&frame, Rect::new(0, 0, 700, 10), LIBRARY_STATUS_TEXT),
        0,
        "the status band does not fit either"
    );
}
