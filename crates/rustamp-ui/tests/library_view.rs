// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless tests of the media-library view model and of the library list's
//! layout.
//!
//! Nothing here opens a window, a dialog or a database: the view model is
//! pure state, and the layout is text.

use std::ops::Range;
use std::path::PathBuf;

use rustamp_ui::ScrollDirection;
use rustamp_ui::library::paging;
use rustamp_ui::library::{LibraryColumn, LibraryRequest, LibraryRow, LibrarySort, LibraryView};
use rustamp_ui::library_layout::{LIBRARY_MIN_TITLE_WIDTH, LibraryLayout, format_duration};

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

/// The rows of the ids `ids`.
fn rows(ids: Range<i64>) -> Vec<LibraryRow> {
    ids.map(row).collect()
}

#[test]
fn the_search_updates_the_request_and_scrolls_back_to_the_top() {
    let mut view = LibraryView::new(20);
    view.apply_page(rows(0..20), 100, 0);
    view.scroll_to(40);

    view.set_search("  beatles  ");
    assert_eq!(view.text(), "  beatles  ", "the search box keeps its text");

    let request: LibraryRequest = view.request();
    assert_eq!(
        request.text.as_deref(),
        Some("beatles"),
        "the request carries the trimmed term"
    );
    assert_eq!(view.scroll(), 0, "a search starts at the top");
    assert_eq!(request.offset, 0);

    view.set_search("   ");
    assert_eq!(view.request().text, None, "whitespace is no filter");
}

#[test]
fn sorting_toggles_the_direction_of_the_same_column() {
    let mut view = LibraryView::new(20);
    assert_eq!(view.sort(), LibrarySort::Id);
    assert!(!view.descending());

    view.toggle_sort(LibrarySort::Title);
    assert_eq!(view.sort(), LibrarySort::Title);
    assert!(!view.descending(), "a new column starts ascending");

    view.toggle_sort(LibrarySort::Title);
    assert!(view.descending(), "the same column reverses the order");

    view.toggle_sort(LibrarySort::Artist);
    assert_eq!(view.sort(), LibrarySort::Artist);
    assert!(!view.descending(), "another column starts ascending again");

    view.set_direction(true);
    assert!(view.descending());

    let request = view.request();
    assert_eq!(request.sort, LibrarySort::Artist);
    assert!(request.descending);
}

#[test]
fn a_new_sort_starts_at_the_top() {
    let mut view = LibraryView::new(20);
    view.apply_page(rows(0..20), 100, 0);
    view.scroll_to(40);
    view.set_sort(LibrarySort::Album);
    assert_eq!(view.scroll(), 0, "the rows are in a new order now");
}

#[test]
fn the_selection_moves_within_the_loaded_page() {
    let mut view = LibraryView::new(20);
    view.apply_page(rows(0..20), 100, 0);
    assert_eq!(view.selected(), None);

    view.move_selection_down();
    assert_eq!(view.selected(), Some(0), "no selection takes the first row");
    view.move_selection_down();
    assert_eq!(view.selected(), Some(1));
    view.move_selection_up();
    assert_eq!(view.selected(), Some(0));
    view.move_selection_up();
    assert_eq!(view.selected(), Some(0), "the top of the page stops it");

    view.move_selection_to_last();
    assert_eq!(view.selected(), Some(19));
    view.move_selection_down();
    assert_eq!(view.selected(), Some(19), "the last loaded row stops it");
    view.move_selection_to_first();
    assert_eq!(view.selected(), Some(0));
}

#[test]
fn moving_up_without_a_selection_takes_the_last_visible_row() {
    let mut view = LibraryView::new(20);
    view.apply_page(rows(0..20), 100, 0);
    view.move_selection_up();
    assert_eq!(view.selected(), Some(19));
}

#[test]
fn the_selection_survives_a_page_change() {
    let mut view = LibraryView::new(20);
    view.apply_page(rows(0..20), 100, 0);
    view.select(5);
    assert_eq!(view.selected_index(), Some(5));

    // The user scrolls on; the app loads the page the scroll is on.
    view.scroll_to(20);
    view.apply_page(rows(20..40), 100, 20);
    assert_eq!(view.selected(), Some(5));
    assert_eq!(view.selected_index(), None, "the row is not on this page");
    assert!(view.selected_row().is_none());
    assert_eq!(view.visible_rows()[0].id, 20, "the page is on the screen");

    // The page with the row comes back: the selection is on it again.
    view.scroll_to(0);
    view.apply_page(rows(0..20), 100, 0);
    assert_eq!(view.selected_index(), Some(5));
    assert_eq!(view.selected_row().map(|row| row.id), Some(5));

    // A re-sort keeps it as well: the selection belongs to a track.
    view.toggle_sort(LibrarySort::Title);
    assert_eq!(view.selected(), Some(5));
}

#[test]
fn a_movement_re_anchors_a_selection_that_is_not_on_the_page() {
    let mut view = LibraryView::new(20);
    view.apply_page(rows(0..20), 100, 0);
    view.select(5);
    view.scroll_to(20);
    view.apply_page(rows(20..40), 100, 20);
    // The selected row is not loaded, so the movement cannot step from it.
    view.move_selection_down();
    assert_eq!(
        view.selected(),
        Some(20),
        "it lands on the first visible row"
    );
}

#[test]
fn paging_counts_the_pages_and_maps_a_row_to_its_page() {
    assert_eq!(paging::page_count(0, 20), 0);
    assert_eq!(paging::page_count(1, 20), 1);
    assert_eq!(paging::page_count(20, 20), 1);
    assert_eq!(paging::page_count(21, 20), 2);
    assert_eq!(paging::page_count(100, 20), 5);
    assert_eq!(paging::page_count(100, 0), 0, "nothing can be paged");

    assert_eq!(paging::page_of(0, 20), 0);
    assert_eq!(paging::page_of(19, 20), 0);
    assert_eq!(paging::page_of(20, 20), 1);
    assert_eq!(paging::page_of(99, 20), 4);
    assert_eq!(paging::page_of(7, 0), 0);

    assert_eq!(paging::page_start(0, 20), 0);
    assert_eq!(paging::page_start(3, 20), 60);
    assert_eq!(paging::page_start(2, 0), 0);

    // The rule and its inverse agree on every row.
    for row_index in 0..100 {
        let page = paging::page_of(row_index, 20);
        assert_eq!(paging::page_start(page, 20) + 20, row_index / 20 * 20 + 20);
        assert!(row_index >= paging::page_start(page, 20));
        assert!(row_index < paging::page_start(page, 20) + 20);
    }
}

#[test]
fn the_view_reports_the_page_the_scroll_is_on() {
    let mut view = LibraryView::new(20);
    view.apply_page(rows(0..20), 100, 0);
    assert_eq!(view.page_count(), 5);
    assert_eq!(view.page(), 0);
    view.scroll_to(45);
    assert_eq!(view.page(), 2);
    assert_eq!(paging::page_start(view.page(), 20), 40);
}

#[test]
fn the_total_is_displayed() {
    let mut view = LibraryView::new(20);
    view.apply_page(rows(0..20), 1234, 0);
    assert_eq!(view.total(), 1234);
    assert_eq!(view.status_line(), "1-20 of 1234");

    view.scroll_to(1214);
    assert_eq!(view.status_line(), "1215-1234 of 1234");

    // Fewer matches than the viewport: the range ends at the last row.
    view.apply_page(rows(0..5), 5, 0);
    assert_eq!(view.status_line(), "1-5 of 5");

    // Nothing at all.
    view.apply_page(Vec::new(), 0, 0);
    assert_eq!(view.status_line(), "0 of 0");
    assert_eq!(view.total(), 0);
}

#[test]
fn the_scroll_follows_the_selection() {
    let mut view = LibraryView::new(10);
    view.apply_page(rows(0..50), 100, 0);

    view.select(10);
    view.move_selection_down();
    assert_eq!(view.selected(), Some(11));
    assert_eq!(view.scroll(), 2, "the view follows the selection down");

    view.scroll_to(5);
    view.select(5);
    view.move_selection_up();
    assert_eq!(view.selected(), Some(4));
    assert_eq!(view.scroll(), 4, "the view follows the selection up");
}

#[test]
fn the_scroll_stays_inside_the_match_set() {
    let mut view = LibraryView::new(20);
    view.apply_page(rows(0..20), 100, 0);

    view.scroll_to(1000);
    assert_eq!(view.scroll(), 80, "the last screenful stays full");
    view.scroll_by(-5);
    assert_eq!(view.scroll(), 75);
    view.scroll_by(-1000);
    assert_eq!(view.scroll(), 0);
    view.scroll_by(1000);
    assert_eq!(view.scroll(), 80);

    view.scroll_page(ScrollDirection::Down);
    assert_eq!(view.scroll(), 80, "one screenful down, clamped");
    view.scroll_to(0);
    view.scroll_page(ScrollDirection::Up);
    assert_eq!(view.scroll(), 0, "one screenful up, clamped");
    view.scroll_page(ScrollDirection::Down);
    assert_eq!(view.scroll(), 20);
}

#[test]
fn the_visible_rows_are_the_loaded_window_at_the_scroll() {
    let mut view = LibraryView::new(3);
    view.apply_page(rows(0..20), 100, 0);
    view.scroll_to(10);
    view.apply_page(rows(10..20), 100, 10);
    assert_eq!(view.visible_rows().len(), 3);
    assert_eq!(view.visible_rows()[0].id, 10);

    view.scroll_to(12);
    assert_eq!(view.visible_rows()[0].id, 12);
    assert_eq!(view.visible_rows().len(), 3);

    // The rows at the scroll offset are not loaded: the view shows nothing
    // rather than rows from another position.
    view.apply_page(rows(0..10), 100, 0);
    view.scroll_to(50);
    assert!(view.visible_rows().is_empty());
}

#[test]
fn the_request_asks_for_the_rows_the_window_shows() {
    let mut view = LibraryView::new(20);
    view.apply_page(rows(0..20), 100, 0);
    view.scroll_to(40);
    let request = view.request();
    assert_eq!(request.limit, 20);
    assert_eq!(request.offset, 40);
    assert_eq!(request.text, None);
    assert_eq!(request.sort, LibrarySort::Id);
    assert!(!request.descending);
}

#[test]
fn the_layout_renders_the_four_columns() {
    let layout = LibraryLayout::default();
    assert_eq!(
        layout.columns(),
        [
            LibraryColumn::Title,
            LibraryColumn::Artist,
            LibraryColumn::Album,
            LibraryColumn::Duration
        ]
    );

    let mut entry = row(1);
    entry.title = "Title".to_string();
    entry.artist = "Artist".to_string();
    entry.album = "Album".to_string();
    entry.duration_ms = Some(215_000);

    let line = layout.row_line(&entry);
    assert_eq!(line.chars().count(), layout.width() as usize);
    assert!(line.starts_with("Title"));
    assert!(line.trim_end().ends_with("3:35"));

    // Every column starts at its own offset.
    assert_eq!(layout.column_x(LibraryColumn::Title), 0);
    assert_eq!(layout.column_x(LibraryColumn::Artist), 41);
    assert_eq!(layout.column_x(LibraryColumn::Album), 66);
    assert_eq!(layout.column_x(LibraryColumn::Duration), 91);
    let cells: Vec<char> = line.chars().collect();
    assert_eq!(&cells[41..47], &['A', 'r', 't', 'i', 's', 't']);
    assert_eq!(&cells[66..71], &['A', 'l', 'b', 'u', 'm']);
    assert_eq!(&cells[91..96], &['3', ':', '3', '5', ' ']);

    // The header carries the column names in the same places.
    let header = layout.header_line();
    assert_eq!(header.chars().count(), layout.width() as usize);
    let header_cells: Vec<char> = header.chars().collect();
    assert_eq!(&header_cells[0..5], &['T', 'i', 't', 'l', 'e']);
    assert_eq!(&header_cells[41..47], &['A', 'r', 't', 'i', 's', 't']);
}

#[test]
fn a_long_value_is_truncated_with_an_ellipsis() {
    let layout = LibraryLayout::default();
    let mut entry = row(1);
    entry.title = "x".repeat(100);

    let line = layout.row_line(&entry);
    assert_eq!(line.chars().count(), layout.width() as usize);
    let cells: Vec<char> = line.chars().collect();
    assert!(cells[..39].iter().all(|cell| *cell == 'x'));
    assert_eq!(cells[39], '…');
}

#[test]
fn the_title_column_takes_the_slack_of_a_wider_line() {
    let layout = LibraryLayout::default();
    let wide = layout.resized_to(120);
    assert_eq!(wide.width(), 120);
    assert_eq!(
        wide.column_width(LibraryColumn::Title),
        120 - (24 + 24 + 8 + 3)
    );
    assert_eq!(wide.column_width(LibraryColumn::Artist), 24);
    assert_eq!(wide.column_width(LibraryColumn::Duration), 8);

    // Too narrow for the fixed columns: the title keeps its minimum and the
    // line stays wider than asked for.
    let narrow = layout.resized_to(10);
    assert_eq!(
        narrow.column_width(LibraryColumn::Title),
        LIBRARY_MIN_TITLE_WIDTH
    );
    assert!(narrow.width() > 10);
}

#[test]
fn durations_are_formatted_as_minutes_and_seconds() {
    assert_eq!(format_duration(None), "");
    assert_eq!(format_duration(Some(0)), "0:00");
    assert_eq!(format_duration(Some(215_000)), "3:35");
    assert_eq!(format_duration(Some(3_599_000)), "59:59");
    assert_eq!(format_duration(Some(3_600_000)), "1:00:00");
    assert_eq!(format_duration(Some(3_661_000)), "1:01:01");
}
