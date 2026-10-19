# Media library window chrome (M5)

## Scope

`rustamp-ui` gains the **window chrome** of the media library: the search
field and the vertical scrollbar, with their geometry and their drawing. The
view model (`library.rs`) and the column layout (`library_layout.rs`) stay
as they are; the model only gains a public `max_scroll()`.

`rustamp-app`, `rustamp-library` and `rustamp-skin` are untouched. No new
dependency: the text goes through the crate's own 1-bit system-font path
(`playlist_font`).

Files:

- `crates/rustamp-ui/src/library_render.rs` - the chrome geometry, the
  palette, the metrics and the drawing.
- `crates/rustamp-ui/src/playlist_font.rs` - `text_width()`, the advance
  width of a text in the metrics `render()` uses (the caret needs it).
- `crates/rustamp-ui/tests/library_chrome.rs` - the headless tests (9).

## The geometry

`LibraryChrome::new(&view, viewport)` is the whole contract: a **pure
function of the viewport and the view**. No clock, no window, no mutable
state - the same viewport and the same view always give the same rectangles,
so the app can hit-test against the bands it drew.

The viewport is split from the top:

| Band | Height | Content |
| --- | --- | --- |
| `search` | one text line | the text as typed, a hint while it is empty, a caret while it is focused |
| `header` | one text line | the column titles |
| `separator` | one pixel | the line under the header |
| `rows` | the rest | the rows at the scroll offset, the selection |
| `status` | one text line | the visible range and the total |

A window that is too short for all of them gives the room to the **header
first** and to the **search field last**: the list is the window, the search
field is a convenience above it.

The **scrollbar** is a column on the right of the rows,
`LIBRARY_SCROLLBAR_WIDTH` (12 px) wide, spanning the rows' band. The rows'
band is narrowed by it, so no row text ever runs under it.

### The thumb

| Case | The thumb |
| --- | --- |
| `total == 0` | none - there is nothing to scroll |
| `total <= visible` | the whole track - everything fits |
| `total > visible` | `track.height * visible / total`, at least `LIBRARY_MIN_THUMB_HEIGHT` (8 px) |

Its place in the track is the scroll's place in the range:
`track.y + (track.height - thumb.height) * scroll / max_scroll`.

`visible` is `LibraryView::viewport()` - the number of rows the window shows.
The app keeps it in step with the window's height; the scroll range is the
model's own (`LibraryView::max_scroll()` = `total - viewport`), so the thumb
and the scroll always agree.

### The click

`scroll_for_click(&view, &chrome, y)` maps a click's y in the track to a
scroll offset: the click's place in the track is the place in the scroll
range. A click at the top of the track asks for the first row, a click at its
bottom for the last screenful, a click in between for the offset in between;
a click outside the track asks for the nearest end.

## The API

```rust
let chrome = LibraryChrome::new(&view, viewport);
let layout = layout.resized_to(chrome.rows.width / LIBRARY_CELL_WIDTH);
render_library_window(&mut frame, &view, &layout, viewport, search_focused);
```

| Item | Meaning |
| --- | --- |
| `LibraryChrome::new(view, viewport)` | the window's geometry |
| `chrome.search` | the search field's band: the click target for the focus |
| `chrome.header` / `chrome.separator` / `chrome.status` | the other bands |
| `chrome.rows` | the list's band, without the scrollbar |
| `chrome.scrollbar` | the scrollbar's track: the click target for the scroll |
| `chrome.thumb` | the thumb, as it is drawn |
| `scroll_for_click(view, chrome, y)` | the scroll offset a click asks for |
| `render_library_window(frame, view, layout, viewport, search_focused)` | draws the whole window |

New constants: `LIBRARY_SEARCH_BACKGROUND`, `LIBRARY_SEARCH_TEXT`,
`LIBRARY_SEARCH_HINT`, `LIBRARY_SEARCH_CARET`, `LIBRARY_SEARCH_PADDING`,
`LIBRARY_SEARCH_PLACEHOLDER`, `LIBRARY_SCROLLBAR_TRACK`,
`LIBRARY_SCROLLBAR_THUMB`, `LIBRARY_SCROLLBAR_WIDTH`,
`LIBRARY_MIN_THUMB_HEIGHT`, `LIBRARY_CARET_WIDTH`.

## What the app must wire

1. **Draw.** `render_library_window(&mut frame, &view, &layout, viewport,
   focused)`. The viewport is the whole client area of the library window.
2. **Size the layout** to the list, not to the window:
   `layout.resized_to(chrome.rows.width / LIBRARY_CELL_WIDTH)`.
3. **Keep the view's viewport in step** with the window's height:
   `view.set_viewport(chrome.rows.height / LIBRARY_LINE_HEIGHT)`. The thumb
   and the scroll range follow it.
4. **Own the focus.** The renderer draws the caret; the app decides when the
   field has the focus and routes the keys to `view.set_search()` while it
   has.
5. **Map the clicks.** A click in `chrome.search` focuses the field; a click
   in `chrome.scrollbar` scrolls to `scroll_for_click(&view, &chrome, y)` and
   runs the view's request again.
6. **The text is clipped** to the field: a search text longer than the field
   is cut off, and its caret stands at the field's right edge. The app may
   want to scroll the text itself; the renderer does not.

## Tests (9, all headless)

`crates/rustamp-ui/tests/library_chrome.rs`:

- the search field is the band above the header, and the bands do not
  overlap,
- the field shows the text as typed, the hint while it is empty, and no
  caret while it has no focus,
- the caret of a focused field stands behind the text (right of its last
  ink pixel, within a cell), and the caret of an empty field stands where
  the text would start,
- the thumb is gone for an empty match set, fills the track when everything
  fits, is the share `visible / total` when there is more to scroll, and
  keeps its narrowest size for a very long match set,
- the thumb stands where the scroll is,
- a click in the track asks for the scroll it sits at (top, middle, bottom,
  and outside the track),
- the scrollbar is a column beside the rows, and the list does not reach
  into it,
- the chrome paints nothing outside the viewport.

## Remaining limits

- The geometry is an own choice: S20 (the media library) is not started, so
  the band's heights, the scrollbar's width and the thumb's rule are ours.
- The caret follows the **proportional** system font, not the text cell
  grid: it stands where the glyphs end, which is not always a cell border.
- The search field does not scroll its text: a long text is clipped.
- The scrollbar has no arrows and no page step: a click jumps to the place
  it sits at.
