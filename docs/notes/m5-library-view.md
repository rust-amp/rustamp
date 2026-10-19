# Media library view model (M5)

## Scope

`rustamp-ui` gains the **headless** state a media-library window needs: the
loaded page of rows, the search text, the sort key and direction, the
selected row and the scroll offset. It is a pure model - no `winit`, no
`softbuffer`, no clock, no window - and it never touches SQLite: it asks
for rows and takes them back.

`rustamp-library` and `rustamp-app` are untouched. The view model has no
dependency on the index crate, so the UI crate stays free of `rusqlite`
(and of the bundled SQLite).

Files:

- `crates/rustamp-ui/src/library.rs` - the model and the paging helper.
- `crates/rustamp-ui/src/library_layout.rs` - the renderer-facing layout.
- `crates/rustamp-ui/tests/library_view.rs` - the headless tests (18).

## The API

### `LibraryView`

State and accessors:

| Item | Meaning |
| --- | --- |
| `rows()` | the loaded page of rows, in sort order |
| `total()` | the number of matches for the current filter (the index's `TrackPage::total`) |
| `offset()` | the index of the first loaded row in the match set |
| `scroll()` | the index of the first visible row in the match set |
| `text()` | the search text as typed |
| `sort()` / `descending()` | the sort column and its direction |
| `selected()` | the id of the selected row |
| `selected_row()` / `selected_index()` | the selected row / its index in the loaded page |
| `visible_rows()` | the rows at the scroll offset, at most one viewportful |
| `status_line()` | the visible range and the total, e.g. `1-20 of 1234` |
| `page()` / `page_count()` | the screenful the scroll is on / how many there are |

Mutators:

| Item | Meaning |
| --- | --- |
| `new(viewport)` / `set_viewport(n)` | how many rows the window shows |
| `apply_page(rows, total, offset)` | the app hands over the rows it loaded |
| `request()` | the query-shaped request the app should run |
| `set_search(text)` | the search term; scrolls back to the top |
| `set_sort(key)` / `toggle_sort(key)` / `set_direction(desc)` | the sort |
| `select(id)` / `select_index(i)` / `clear_selection()` | the selection |
| `move_selection_up/down()` / `move_selection_to_first/last()` | the selection moves |
| `scroll_to(offset)` / `scroll_by(delta)` / `scroll_page(direction)` | the scroll |

### `LibraryRequest`

The **view-owned** part of an index query:

```rust
pub struct LibraryRequest {
    pub text: Option<String>,   // the trimmed search term; None when empty
    pub sort: LibrarySort,      // Id | Title | Artist | Album | Duration
    pub descending: bool,
    pub limit: u32,             // the viewport
    pub offset: u32,            // the first visible row
}
```

The request is **scroll-aligned**: its offset is the first visible row, so
the rows the app loads are exactly the rows the window shows. The remaining
filters of the index's query (artist, album, genre, path prefix) belong to
the app, which merges them into the query it runs.

### `LibraryRow`

The row shape the view sees - the display values only:

```rust
pub struct LibraryRow {
    pub id: i64,                 // the stable row id of the index
    pub path: PathBuf,           // the file path, as the index stored it
    pub title: String,           // empty when the index has none
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<u64>, // None when the index has none
}
```

The app converts an index row into this shape:

```rust
fn to_view_row(row: &TrackRow) -> LibraryRow {
    LibraryRow {
        id: row.id,
        path: row.path.clone(),
        title: row.metadata.title.clone().unwrap_or_default(),
        artist: row.metadata.artist.clone().unwrap_or_default(),
        album: row.metadata.album.clone().unwrap_or_default(),
        duration_ms: row.metadata.duration_ms,
    }
}
```

### `paging`

```rust
paging::page_count(total, page_size)  // how many screenfuls a match set fills
paging::page_of(row, page_size)        // the scroll-to-page rule
paging::page_start(page, page_size)    // the inverse of page_of
```

A *page* is one screenful of rows (the view's viewport). `page_count` is
`ceil(total / page_size)`; an empty match set has no page. The rule is
`row / page_size`, and `page_start` is its inverse. The app uses it to
serve a request from a page cache.

## The layout

The library window is a **modern** window, not a skinned one: it has an OS
title bar, a system font and its own chrome, so no skin sheet, no sprite and
no `pledit.txt` colour applies to it. The layout is therefore given in
**text cells** and **text lines**; the renderer maps a cell to a pixel with
its own font metrics.

Columns, in display order, with the default widths in cells:

| Column | Width | Content |
| --- | --- | --- |
| `Title` | 40 | the track title |
| `Artist` | 24 | the track artist |
| `Album` | 24 | the album title |
| `Duration` | 8 | the length as `m:ss` (or `h:mm:ss` from an hour) |

The columns are separated by one cell, so the default line is 99 cells wide.
`LibraryLayout::resized_to(width)` gives the slack to the title column and
keeps the other three; the title never becomes narrower than 8 cells.

`LibraryLayout::row_line(&row)` renders one row: every value is padded to
its column width, and a value that does not fit is truncated and marked
with an ellipsis (`…`). `LibraryLayout::header_line()` renders the column
titles (`Title`, `Artist`, `Album`, `Length`) in the same places. The
length of a track the index does not know is an empty cell, not a
placeholder.

## What the app must do

1. **Own the index.** The app holds the `rustamp-library` connection and
   runs the query. The view never sees it.
2. **Answer the request.** Run the index's query with the request's `text`,
   `sort`, `descending`, `limit` and `offset`, plus the filters the app
   owns, and convert the rows with the mapping above.
3. **Hand the rows back.** `view.apply_page(rows, total, offset)` with the
   total of the matches and the index of the first row. The window the app
   loads should cover the scroll offset: the request names exactly those
   rows.
4. **Draw.** `view.visible_rows()` for the rows, `LibraryLayout::row_line`
   for the text, `view.status_line()` for the status bar, and
   `view.selected_row()` for the row to play.
5. **React to the view.** After a search (`set_search`), a sort
   (`toggle_sort`) or a scroll (`scroll_to`, `scroll_page`), run the
   request again. The view scrolls back to the top on a search and on a new
   sort, so the request's offset is `0` in both cases.
6. **Load more rows at the edge.** The view cannot move a selection past
   the last loaded row: it cannot name a row it has not seen. When the
   selection is on the last row of the loaded page, the app loads the next
   rows and the movement continues.
7. **Keep the selection.** The selection is anchored to a row id, so it
   survives a page change, a re-sort and a search. The app clears it
   explicitly when the result set no longer holds the row.

The keyboard and the mouse are the app's: the view answers
`move_selection_up/down`, `move_selection_to_first/last`, `select_index`
and the scroll methods, and the app maps the keys and the clicks onto them.
The playlist's conventions are the model: a movement with no selection
takes the first (down) or the last (up) visible row, and the view follows
the selection so it stays on the screen.

## Tests (18, all headless)

`crates/rustamp-ui/tests/library_view.rs`:

- the search updates the request and scrolls back to the top,
- sorting toggles the direction of the same column, a new column starts
  ascending, a new sort starts at the top,
- the selection moves within the loaded page, stops at its edges, and
  re-anchors on the first visible row when its row is not loaded,
- the selection survives a page change and a re-sort,
- the paging boundaries (0, 1, an exact multiple, a partial page, a page
  size of 0) and the scroll-to-page rule,
- the total is reported in the status line (`1-20 of 1234`, `0 of 0`),
- the scroll follows the selection and stays inside the match set,
- the visible rows are the loaded window at the scroll,
- the request asks for the rows the window shows,
- the layout renders the four columns at their offsets, truncates a long
  value with an ellipsis, gives the slack to the title column and formats
  the durations.

## Remaining limits

- The view knows no index row: the conversion (title, artist, album,
  duration) is the app's, and the view has no column for the file name, the
  genre or the year - the layout shows four columns only.
- The scroll offset is absolute, so a jump over a long match set needs the
  rows of the new position; the app answers the request for the rows at
  the scroll offset, and a page cache keeps the rest cheap.
- The window's own chrome (the search box, the column headers, the status
  bar) is not modelled: the layout gives the column headers and the row
  lines, the rest belongs to the window.
- Nothing here is measured: S20 (the media library) is not started, so the
  column set, the widths and the scroll rules are own choices, made to
  mirror the playlist editor's conventions.
