// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The media library's headless view model.
//!
//! [`LibraryView`] is the state a media-library window needs: the page of
//! rows the app loaded, the search text, the sort key and direction, the
//! selected row and the scroll offset. Like [`crate::playlist::PlaylistWindow`]
//! it is deliberately **headless** - no `winit`, no `softbuffer`, no clock,
//! no window - and, like every view model in this crate, it is **free of the
//! index**: it never opens a database, never runs a query and never sees an
//! index row.
//!
//! ## How the app drives the view
//!
//! 1. The view says what it needs with a [`LibraryRequest`]: the search term,
//!    the sort column and direction, how many rows to load and the index of
//!    the first one. The request is the *view-owned* part of the index's
//!    query; the app adds the filters it owns (artist, album, genre, path
//!    prefix) and runs the query against the library.
//! 2. The app converts the index's rows into [`LibraryRow`]s and hands them
//!    back with [`LibraryView::apply_page`], together with the total number
//!    of matches and the index of the first row.
//! 3. The view answers what to draw: [`LibraryView::visible_rows`] (the rows
//!    at the scroll offset), [`LibraryView::selected_row`] (the row to play)
//!    and [`LibraryView::status_line`] (the visible range and the total).
//!
//! The request is **scroll-aligned**: its offset is the first visible row, so
//! the rows the app loads are exactly the rows the window shows.
//! [`paging::page_of`] tells the app which screenful the scroll is on, so a
//! page cache can answer the request without a new query.
//!
//! The app keeps the scroll and the loaded window in sync: it scrolls the
//! view, asks for the rows the request names and hands them back with
//! [`LibraryView::apply_page`]. While a query is in flight the view shows the
//! rows of the old window that still lie at the scroll offset, and nothing
//! when the new scroll is beyond them.
//!
//! ## Own choices
//!
//! No spec measures the media library (S20 is not started), so everything
//! below is an own choice, made to mirror the playlist editor's conventions:
//!
//! * The selection is anchored to a **row id**, not to a position. It
//!   therefore survives a page change, a re-sort and a new search, and it
//!   keeps pointing at the same track. The app clears it explicitly when the
//!   result set no longer contains the row.
//! * The scroll offset is an **absolute row index** into the match set, so
//!   the view can ask for the rows it shows without knowing how they were
//!   paged.
//! * A movement that would leave the loaded window is refused: the view
//!   cannot name a row it has not seen. The app loads more rows (it knows
//!   the selection's index and the window's length) and the movement
//!   continues.
//! * A search resets the scroll to the top and keeps the selection, because
//!   the selection belongs to a track and not to a position.

use std::path::PathBuf;

use crate::playlist::ScrollDirection;

/// A column of the library list.
///
/// The columns are the ones the index carries: title, artist, album and the
/// length. The index has no column for artwork or for the file name, so the
/// list has none either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LibraryColumn {
    /// The track title.
    Title,
    /// The track artist.
    Artist,
    /// The album title.
    Album,
    /// The track length.
    Duration,
}

/// The sort keys the library offers.
///
/// The order matches the index's sort keys one to one, so the app maps them
/// without a table: `Id` is the insertion order (the stable row id), the
/// others are the columns. Text columns sort case-insensitively and `NULL`
/// values sort last in both directions; the index breaks every tie by
/// ascending row id, which is what makes its paging stable.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LibrarySort {
    /// Insertion order (the stable row id).
    #[default]
    Id,
    /// Track title.
    Title,
    /// Track artist.
    Artist,
    /// Album title.
    Album,
    /// Track duration.
    Duration,
}

/// One library row, as the view sees it.
///
/// The view keeps the display values only: the id, the path and the four
/// columns the list shows. A missing tag is an empty string (or `None` for
/// the length) - the view never sees the index's metadata struct, and the
/// app never has to translate a missing value twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRow {
    /// The stable row id of the track in the index.
    pub id: i64,
    /// The file path, as the index stored it.
    pub path: PathBuf,
    /// The title; empty when the index has none.
    pub title: String,
    /// The artist; empty when the index has none.
    pub artist: String,
    /// The album; empty when the index has none.
    pub album: String,
    /// The length in milliseconds; `None` when the index has none.
    pub duration_ms: Option<u64>,
}

/// The request the view hands to the app.
///
/// It is the part of a library query the view owns: the free-text search,
/// the sort column and direction, and the window of rows to load. The
/// remaining filters of the index's query (artist, album, genre, path prefix)
/// belong to the app, which merges them into the query it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRequest {
    /// The free-text search term; `None` when the search box is empty or
    /// holds only whitespace.
    pub text: Option<String>,
    /// The sort column.
    pub sort: LibrarySort,
    /// Whether the sort column is reversed.
    pub descending: bool,
    /// How many rows to load.
    pub limit: u32,
    /// The index of the first row to load, in the match set.
    pub offset: u32,
}

/// The media library's headless view model.
///
/// The model holds the page the app loaded, the search text, the sort key
/// and direction, the selected row and the scroll offset. It performs no
/// I/O: [`LibraryView::request`] asks for rows and
/// [`LibraryView::apply_page`] takes them back.
#[derive(Debug, Clone)]
pub struct LibraryView {
    viewport: usize,
    rows: Vec<LibraryRow>,
    total: usize,
    offset: usize,
    scroll: usize,
    text: String,
    sort: LibrarySort,
    descending: bool,
    selected: Option<i64>,
}

impl LibraryView {
    /// An empty view whose window shows `viewport` rows at a time.
    ///
    /// The viewport is the number of rows the window shows; it is also the
    /// number of rows a [`LibraryRequest`] asks for, and the size of a page
    /// in [`paging`].
    pub fn new(viewport: usize) -> Self {
        Self {
            viewport,
            rows: Vec::new(),
            total: 0,
            offset: 0,
            scroll: 0,
            text: String::new(),
            sort: LibrarySort::default(),
            descending: false,
            selected: None,
        }
    }

    /// The number of rows the window shows at a time.
    pub fn viewport(&self) -> usize {
        self.viewport
    }

    /// Sets the number of rows the window shows at a time.
    ///
    /// The scroll offset is clamped to the new range, so the last screenful
    /// of the match set stays full.
    pub fn set_viewport(&mut self, rows: usize) {
        self.viewport = rows;
        self.scroll = self.scroll.min(self.max_scroll());
    }

    /// The loaded rows, in sort order.
    pub fn rows(&self) -> &[LibraryRow] {
        &self.rows
    }

    /// The number of matches for the current filter, ignoring the window.
    ///
    /// This is the total the index reports for a query, so a status bar can
    /// show it without a second query.
    pub fn total(&self) -> usize {
        self.total
    }

    /// The index of the first loaded row in the match set.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// The index of the first visible row in the match set.
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// The current search text, as typed (not trimmed).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The current sort column.
    pub fn sort(&self) -> LibrarySort {
        self.sort
    }

    /// Whether the sort column is reversed.
    pub fn descending(&self) -> bool {
        self.descending
    }

    /// The id of the selected row, if a row is selected.
    ///
    /// The selection is anchored to the row id, so it stays valid when the
    /// page changes and it survives a re-sort.
    pub fn selected(&self) -> Option<i64> {
        self.selected
    }

    /// The selected row, if it is on the loaded page.
    pub fn selected_row(&self) -> Option<&LibraryRow> {
        let id = self.selected?;
        self.rows.iter().find(|row| row.id == id)
    }

    /// The index of the selected row in the loaded page.
    ///
    /// `None` when nothing is selected or when the selected row is not on
    /// the loaded page.
    pub fn selected_index(&self) -> Option<usize> {
        self.selection_index()
    }

    /// The rows the window shows: the loaded rows at the scroll offset, at
    /// most one viewportful.
    ///
    /// Empty while the rows at the scroll offset are not loaded yet - the
    /// view shows what it has, never rows from another position.
    pub fn visible_rows(&self) -> &[LibraryRow] {
        let end = self.offset + self.rows.len();
        if self.scroll < self.offset || self.scroll >= end {
            return &[];
        }
        let from = self.scroll - self.offset;
        let to = (self.scroll + self.viewport).min(end) - self.offset;
        &self.rows[from..to]
    }

    /// Selects the row with the id `id`, if it is on the loaded page.
    ///
    /// Returns whether the row was found. The view scrolls the row into
    /// sight, so a selection made from outside the window is visible.
    pub fn select(&mut self, id: i64) -> bool {
        let Some(index) = self.rows.iter().position(|row| row.id == id) else {
            return false;
        };
        self.selected = Some(id);
        self.reveal(index);
        true
    }

    /// Selects the row at `index` of the loaded page.
    ///
    /// Returns whether the index is inside the loaded page.
    pub fn select_index(&mut self, index: usize) -> bool {
        let Some(row) = self.rows.get(index) else {
            return false;
        };
        let id = row.id;
        self.selected = Some(id);
        self.reveal(index);
        true
    }

    /// Clears the selection.
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Moves the selection one row down.
    ///
    /// With no selection the first visible row is selected, like the
    /// playlist editor does. A selection whose row is not on the loaded page
    /// is re-anchored on the first visible row, because the view cannot move
    /// a row it has not seen. At the last loaded row the movement is
    /// refused: the app loads more rows and the movement continues.
    pub fn move_selection_down(&mut self) {
        let Some(index) = self.selection_index() else {
            if let Some(index) = self.index_of(self.scroll) {
                self.set_selection(index);
            }
            return;
        };
        if index + 1 < self.rows.len() {
            self.set_selection(index + 1);
        }
    }

    /// Moves the selection one row up.
    ///
    /// With no selection the last visible row is selected, like the playlist
    /// editor does. The other rules mirror [`LibraryView::move_selection_down`].
    pub fn move_selection_up(&mut self) {
        let Some(index) = self.selection_index() else {
            if let Some(index) = self.index_of(self.scroll + self.viewport.saturating_sub(1)) {
                self.set_selection(index);
            }
            return;
        };
        if index > 0 {
            self.set_selection(index - 1);
        }
    }

    /// Selects the first row of the loaded page and scrolls it into sight.
    pub fn move_selection_to_first(&mut self) {
        if !self.rows.is_empty() {
            self.set_selection(0);
        }
    }

    /// Selects the last row of the loaded page and scrolls it into sight.
    pub fn move_selection_to_last(&mut self) {
        if let Some(index) = self.rows.len().checked_sub(1) {
            self.set_selection(index);
        }
    }

    /// Sets the free-text search term and scrolls back to the top.
    ///
    /// The selection survives, because it belongs to a track and not to a
    /// position; the app clears it when the new result no longer holds the
    /// row.
    pub fn set_search(&mut self, text: &str) {
        self.text = text.to_string();
        self.scroll = 0;
    }

    /// Sets the sort column and starts the new order ascending.
    ///
    /// The scroll offset goes back to the top, because the rows are in a new
    /// order now.
    pub fn set_sort(&mut self, sort: LibrarySort) {
        self.sort = sort;
        self.descending = false;
        self.scroll = 0;
    }

    /// Toggles the sort: the same column reverses the order, a new column
    /// starts ascending.
    ///
    /// This is the classic column-header behaviour: clicking the column that
    /// is already sorted reverses it, clicking another column sorts by that
    /// one.
    pub fn toggle_sort(&mut self, sort: LibrarySort) {
        if self.sort == sort {
            self.descending = !self.descending;
        } else {
            self.sort = sort;
            self.descending = false;
        }
        self.scroll = 0;
    }

    /// Sets the sort direction without touching the sort column.
    pub fn set_direction(&mut self, descending: bool) {
        self.descending = descending;
    }

    /// Scrolls so that the row at the absolute index `offset` is the first
    /// visible one.
    ///
    /// The offset is clamped to the match set, so the last screenful stays
    /// full.
    pub fn scroll_to(&mut self, offset: usize) {
        self.scroll = offset.min(self.max_scroll());
    }

    /// Scrolls by `delta` rows, clamped to the match set.
    pub fn scroll_by(&mut self, delta: isize) {
        let current = self.scroll as isize;
        let next = current
            .saturating_add(delta)
            .clamp(0, self.max_scroll() as isize);
        self.scroll = next as usize;
    }

    /// Scrolls one screenful up or down.
    ///
    /// One step is one viewport, like the playlist editor's one arrow step
    /// (one page of rows). It is clamped at both ends of the match set.
    pub fn scroll_page(&mut self, direction: ScrollDirection) {
        let step = self.viewport as isize;
        match direction {
            ScrollDirection::Up => self.scroll_by(-step),
            ScrollDirection::Down => self.scroll_by(step),
        }
    }

    /// Hands the view the rows the app loaded.
    ///
    /// `offset` is the index of the first row in the match set, `total` the
    /// number of matches for the current filter. The loaded rows may be any
    /// window: the view shows the ones at the scroll offset and leaves the
    /// rest to the app.
    ///
    /// The window should cover the scroll offset - the app asks for the rows
    /// the request names, so it does. A window that does not cover it leaves
    /// the view empty, because the rows at the scroll offset are not loaded
    /// yet.
    pub fn apply_page(&mut self, rows: Vec<LibraryRow>, total: usize, offset: usize) {
        self.rows = rows;
        self.total = total;
        self.offset = offset;
        self.scroll = self.scroll.min(self.max_scroll());
    }

    /// The request the app should run to fill the view.
    ///
    /// The request is scroll-aligned: its offset is the first visible row,
    /// so the rows the app loads are the rows the window shows. Its limit
    /// is the viewport.
    pub fn request(&self) -> LibraryRequest {
        LibraryRequest {
            text: self.search_term(),
            sort: self.sort,
            descending: self.descending,
            limit: u32::try_from(self.viewport).unwrap_or(u32::MAX),
            offset: u32::try_from(self.scroll).unwrap_or(u32::MAX),
        }
    }

    /// The screenful the scroll is on, counted from zero.
    ///
    /// The app uses it to serve the request from a page cache.
    pub fn page(&self) -> usize {
        paging::page_of(self.scroll, self.viewport)
    }

    /// The number of screenfuls the match set fills.
    pub fn page_count(&self) -> usize {
        paging::page_count(self.total, self.viewport)
    }

    /// The status line: the visible range and the total, e.g. `1-20 of 1234`.
    ///
    /// An empty match set reads `0 of 0`.
    pub fn status_line(&self) -> String {
        if self.total == 0 {
            return "0 of 0".to_string();
        }
        let first = self.scroll + 1;
        let last = (self.scroll + self.viewport).min(self.total);
        format!("{first}-{last} of {}", self.total)
    }

    /// The largest scroll offset: the last screenful of the match set.
    ///
    /// The match set holds `total` rows and the window shows `viewport` of
    /// them, so the last screenful starts at `total - viewport`. A match
    /// set that fits into the window has no scroll at all.
    pub fn max_scroll(&self) -> usize {
        self.total.saturating_sub(self.viewport)
    }

    /// The search term of the request: the trimmed text, or `None`.
    fn search_term(&self) -> Option<String> {
        let trimmed = self.text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    /// The index of the selected row in the loaded page, if it is loaded.
    fn selection_index(&self) -> Option<usize> {
        let id = self.selected?;
        self.rows.iter().position(|row| row.id == id)
    }

    /// The index of the row at the absolute index `absolute`, if it is
    /// loaded.
    fn index_of(&self, absolute: usize) -> Option<usize> {
        if absolute < self.offset {
            return None;
        }
        let index = absolute - self.offset;
        if index < self.rows.len() {
            Some(index)
        } else {
            None
        }
    }

    /// Selects the row at `index` of the loaded page and scrolls it into
    /// sight.
    fn set_selection(&mut self, index: usize) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        self.selected = Some(row.id);
        self.reveal(index);
    }

    /// Scrolls so that the row at `index` of the loaded page is visible.
    fn reveal(&mut self, index: usize) {
        let absolute = self.offset + index;
        if absolute < self.scroll {
            self.scroll_to(absolute);
        } else if absolute >= self.scroll + self.viewport {
            self.scroll_to(absolute + 1 - self.viewport);
        }
    }
}

/// Paging arithmetic over a match set.
///
/// The library is queried window by window, so a view needs to know how many
/// screenfuls a match set fills and which screenful a row is on. A *page* is
/// one screenful of rows - the view's viewport.
pub mod paging {
    /// The number of pages `total` rows fill at `page_size` rows per page.
    ///
    /// An empty match set has no page (`0`), and a page size of `0` has none
    /// either, because nothing can be paged.
    pub fn page_count(total: usize, page_size: usize) -> usize {
        if page_size == 0 {
            return 0;
        }
        total.div_ceil(page_size)
    }

    /// The page that holds the row at the absolute index `row`.
    ///
    /// This is the scroll-to-page rule: the page of a row is its index
    /// divided by the page size. A page size of `0` has no pages, so every
    /// row is reported on page `0`.
    pub fn page_of(row: usize, page_size: usize) -> usize {
        if page_size == 0 {
            return 0;
        }
        row / page_size
    }

    /// The index of the first row of `page` - the inverse of
    /// [`page_of`].
    pub fn page_start(page: usize, page_size: usize) -> usize {
        if page_size == 0 {
            return 0;
        }
        page * page_size
    }
}
