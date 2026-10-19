// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless tests of the media library window's chrome: the search field
//! and the scrollbar.
//!
//! Every test paints into an in-memory frame and reads the pixels back: no
//! window, no skin, no clock, no database. The text is the host's system
//! font, so the tests assert the **invariants** - which band holds which
//! colour, where the caret stands, how big the thumb is - and never the
//! exact shape of a glyph.

use std::path::PathBuf;

use rustamp_gfx::{Image, Rect};
use rustamp_ui::library::{LibraryRow, LibraryView};
use rustamp_ui::library_layout::LibraryLayout;
use rustamp_ui::library_render::{
    LIBRARY_CELL_WIDTH, LIBRARY_LINE_HEIGHT, LIBRARY_SCROLLBAR_THUMB, LIBRARY_SCROLLBAR_TRACK,
    LIBRARY_SEARCH_CARET, LIBRARY_SEARCH_HINT, LIBRARY_SEARCH_PADDING, LIBRARY_SEARCH_TEXT,
    LibraryChrome, render_library_window, scroll_for_click,
};

/// A row with the display values the layout needs.
fn row(id: i64) -> LibraryRow {
    LibraryRow {
        id,
        path: PathBuf::from(format!("C:/music/track{id}.flac")),
        title: format!("Track {id}"),
        artist: "Artist".to_string(),
        album: "Album".to_string(),
        duration_ms: Some(215_000),
    }
}

/// A view that holds `rows` and knows `total` matches.
fn make_view(rows: Vec<LibraryRow>, total: usize) -> LibraryView {
    let mut view = LibraryView::new(rows.len());
    view.apply_page(rows, total, 0);
    view
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

/// An opaque RGBA colour from an RGB triple.
fn rgba(rgb: [u8; 3]) -> [u8; 4] {
    [rgb[0], rgb[1], rgb[2], 255]
}

#[test]
fn the_search_field_is_the_band_above_the_header() {
    let view = make_view(vec![row(1)], 1);
    let viewport = Rect::new(0, 0, 700, 100);
    let chrome = LibraryChrome::new(&view, viewport);

    assert_eq!(
        chrome.search,
        Rect::new(0, 0, 700, LIBRARY_LINE_HEIGHT),
        "the field is the top line of the window"
    );
    assert_eq!(
        chrome.header.y,
        chrome.search.bottom(),
        "the header follows the field"
    );
    assert_eq!(chrome.header.height, LIBRARY_LINE_HEIGHT);
    assert_eq!(
        chrome.search.width, viewport.width,
        "the field is the full width of the window"
    );
    assert!(
        chrome.search.bottom() <= chrome.header.y
            && chrome.header.bottom() <= chrome.separator.y
            && chrome.separator.bottom() <= chrome.rows.y
            && chrome.rows.bottom() <= chrome.status.y,
        "the bands do not overlap and stay in order"
    );
}

#[test]
fn the_search_field_shows_the_text_the_hint_and_the_caret() {
    let mut view = make_view(vec![], 0);
    let layout = LibraryLayout::default();
    let viewport = Rect::new(0, 0, 700, 100);
    let chrome = LibraryChrome::new(&view, viewport);
    let field = chrome.search;

    // An empty field shows the hint and no text.
    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);
    assert!(
        count(&frame, field, LIBRARY_SEARCH_HINT) > 0,
        "the hint is drawn while the field is empty"
    );
    assert_eq!(
        count(&frame, field, LIBRARY_SEARCH_TEXT),
        0,
        "there is no text to draw"
    );
    assert_eq!(
        count(&frame, field, LIBRARY_SEARCH_CARET),
        0,
        "an unfocused field has no caret"
    );

    // A field with text shows the text and no hint.
    view.set_search("hello");
    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);
    assert!(
        count(&frame, field, LIBRARY_SEARCH_TEXT) > 0,
        "the text as typed is drawn"
    );
    assert_eq!(
        count(&frame, field, LIBRARY_SEARCH_HINT),
        0,
        "the hint is gone once there is text"
    );
}

#[test]
fn the_caret_stands_behind_the_text_of_the_focused_field() {
    let mut view = make_view(vec![], 0);
    view.set_search("hello");
    let layout = LibraryLayout::default();
    let viewport = Rect::new(0, 0, 700, 100);
    let chrome = LibraryChrome::new(&view, viewport);

    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, true, &[]);
    let field = chrome.search;
    let last_text = last_ink_x(&frame, field, LIBRARY_SEARCH_TEXT).expect("the text is drawn");
    let caret = first_ink_x(&frame, field, LIBRARY_SEARCH_CARET).expect("the caret is drawn");
    assert!(
        caret > last_text,
        "the caret stands behind the text, not inside it"
    );
    assert!(
        caret - last_text <= LIBRARY_CELL_WIDTH as i32,
        "the caret is right behind the text, it is {caret} and the text ends at {last_text}"
    );
    assert!(
        caret >= field.x + LIBRARY_SEARCH_PADDING as i32,
        "the caret is inside the field"
    );
    assert!(caret < field.right(), "the caret is inside the field");
}

#[test]
fn the_caret_of_an_empty_field_stands_at_its_left_edge() {
    let view = make_view(vec![], 0);
    let layout = LibraryLayout::default();
    let viewport = Rect::new(0, 0, 700, 100);
    let chrome = LibraryChrome::new(&view, viewport);

    let mut frame = Image::filled(700, 100, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, true, &[]);
    let caret = first_ink_x(&frame, chrome.search, LIBRARY_SEARCH_CARET)
        .expect("the caret is drawn while the field is focused");
    assert_eq!(
        caret,
        chrome.search.x + LIBRARY_SEARCH_PADDING as i32,
        "the caret stands where the text would start"
    );
}

#[test]
fn the_thumb_is_as_tall_as_the_share_of_the_match_set_the_window_shows() {
    let viewport = Rect::new(0, 0, 700, 200);

    // Nothing at all: there is no thumb to show.
    let view = make_view(vec![], 0);
    let chrome = LibraryChrome::new(&view, viewport);
    assert_eq!(chrome.thumb.height, 0, "there is nothing to scroll");
    assert_eq!(
        chrome.scrollbar.height, chrome.rows.height,
        "the track spans the rows"
    );
    assert_eq!(chrome.scrollbar.width, rustamp_ui::LIBRARY_SCROLLBAR_WIDTH);

    // Fewer matches than the window shows: the thumb fills the track.
    let mut view = LibraryView::new(10);
    view.apply_page((1..=3).map(row).collect(), 3, 0);
    let chrome = LibraryChrome::new(&view, viewport);
    assert_eq!(
        chrome.thumb, chrome.scrollbar,
        "the thumb fills the track when everything fits"
    );

    // Exactly as many matches as the window shows: the thumb fills the track.
    let view = make_view((1..=5).map(row).collect(), 5);
    let chrome = LibraryChrome::new(&view, viewport);
    assert_eq!(
        chrome.thumb, chrome.scrollbar,
        "the thumb fills the track when everything fits"
    );

    // Much more to scroll: the thumb is the share the window shows.
    let view = make_view((1..=10).map(row).collect(), 100);
    let chrome = LibraryChrome::new(&view, viewport);
    let track = chrome.scrollbar;
    assert_eq!(chrome.thumb.width, track.width);
    assert_eq!(
        chrome.thumb.height,
        track.height * 10 / 100,
        "the thumb is the share of the match set the window shows"
    );
    assert_eq!(chrome.thumb.y, track.y, "the thumb is at the top");
    assert_eq!(
        chrome.thumb.bottom(),
        track.y + chrome.thumb.height as i32,
        "the thumb hangs at the top of the track"
    );
    assert!(
        chrome.thumb.height < track.height,
        "the thumb is smaller than the track"
    );

    // A match set that is far too long: the thumb keeps its narrowest size.
    let view = make_view((1..=10).map(row).collect(), 10_000);
    let chrome = LibraryChrome::new(&view, viewport);
    assert_eq!(
        chrome.thumb.height,
        rustamp_ui::LIBRARY_MIN_THUMB_HEIGHT,
        "the thumb stays grabbable"
    );
}

#[test]
fn the_thumb_stands_where_the_scroll_is() {
    let viewport = Rect::new(0, 0, 700, 200);
    let mut view = make_view((1..=10).map(row).collect(), 100);
    let track = LibraryChrome::new(&view, viewport).scrollbar;

    // The scroll's place in the range is the thumb's place in the track.
    view.scroll_to(45);
    let chrome = LibraryChrome::new(&view, viewport);
    let travel = track.height - chrome.thumb.height;
    assert_eq!(
        chrome.thumb.y,
        track.y + travel as i32 * 45 / 90,
        "the thumb is where the scroll is"
    );

    // The last screenful puts the thumb at the bottom of the track.
    view.scroll_to(90);
    let chrome = LibraryChrome::new(&view, viewport);
    assert_eq!(
        chrome.thumb.bottom(),
        track.bottom(),
        "the thumb is at the bottom of the track"
    );
}

#[test]
fn a_click_in_the_track_asks_for_the_scroll_it_sits_at() {
    let viewport = Rect::new(0, 0, 700, 200);
    let view = make_view((1..=10).map(row).collect(), 100);
    let chrome = LibraryChrome::new(&view, viewport);
    let track = chrome.scrollbar;

    assert_eq!(
        scroll_for_click(&view, &chrome, track.y),
        0,
        "the top of the track is the first row"
    );
    assert_eq!(
        scroll_for_click(&view, &chrome, track.bottom() - 1),
        90,
        "the bottom of the track is the last screenful"
    );
    assert_eq!(
        scroll_for_click(&view, &chrome, track.y + ((track.height - 1) / 2) as i32),
        45,
        "the middle of the track is the middle of the range"
    );
    assert_eq!(
        scroll_for_click(&view, &chrome, track.y - 10),
        0,
        "a click above the track asks for the first row"
    );
    assert_eq!(
        scroll_for_click(&view, &chrome, track.bottom() + 10),
        90,
        "a click below the track asks for the last screenful"
    );

    // A match set that fits into the window has no scroll at all.
    let view = make_view((1..=5).map(row).collect(), 5);
    let chrome = LibraryChrome::new(&view, viewport);
    assert_eq!(scroll_for_click(&view, &chrome, chrome.scrollbar.y), 0);
    assert_eq!(
        scroll_for_click(&view, &chrome, chrome.scrollbar.bottom() - 1),
        0
    );
}

#[test]
fn the_scrollbar_is_a_column_beside_the_rows() {
    let view = make_view((1..=10).map(row).collect(), 100);
    let layout = LibraryLayout::default();
    let viewport = Rect::new(0, 0, 700, 200);
    let chrome = LibraryChrome::new(&view, viewport);
    let mut frame = Image::filled(700, 200, [0, 0, 0, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, false, &[]);

    assert_eq!(
        chrome.rows.right(),
        chrome.scrollbar.x,
        "the list does not reach into the scrollbar's column"
    );
    assert!(
        count(&frame, chrome.scrollbar, LIBRARY_SCROLLBAR_TRACK) > 0,
        "the track is drawn"
    );
    assert!(
        count(&frame, chrome.thumb, LIBRARY_SCROLLBAR_THUMB) > 0,
        "the thumb is drawn"
    );
}

#[test]
fn the_chrome_paints_nothing_outside_the_viewport() {
    let mut view = make_view((1..=10).map(row).collect(), 100);
    view.set_search("hello");
    let layout = LibraryLayout::default();
    let viewport = Rect::new(10, 20, 300, 200);
    let mut frame = Image::filled(400, 300, [7, 11, 13, 255]);
    render_library_window(&mut frame, &view, &layout, viewport, true, &[]);

    for y in 0..300 {
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
