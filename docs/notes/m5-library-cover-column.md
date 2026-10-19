# Media library cover column (M5)

## Scope

`rustamp-ui` gains a **cover column** in the media library's list: a
fixed-width column in front of the text columns that shows the album art of
the row. The view model (`library.rs`) and the artwork cache
(`rustamp-library`) stay as they are; the renderer only draws what the app
hands it.

`rustamp-app`, `rustamp-library` and `rustamp-skin` are untouched, and
`rustamp-ui` does **not** depend on `rustamp-library`: the interface is a
renderer, the library is storage, and the dependency would drag `rusqlite`
(bundled SQLite) and `image` into every interface build. The renderer
therefore takes a **borrowed view** of a picture ([`CoverRef`]) and the app
converts the artwork cache's `CoverImage` into it.

No new dependency at all: the cover is scaled by hand, with the crate's own
`rustamp-gfx` image as the only picture type. `Cargo.lock` and
`THIRD_PARTY.md` are unchanged.

Files:

- `crates/rustamp-ui/src/library_layout.rs` - the cover column's width.
- `crates/rustamp-ui/src/library_render.rs` - the cell's geometry, the
  scaling and the blit.
- `crates/rustamp-ui/tests/library_cover.rs` - the headless tests (10).

## The column

| Item | Value |
| --- | --- |
| `LIBRARY_COVER_WIDTH` | 40 px (own choice) |
| Unit | pixels, not text cells |
| Place | in front of the text columns |
| Header cell | empty |
| Cell of a row | `LIBRARY_COVER_WIDTH` px wide, one line high |

The width is **our own choice**: no spec measures the media library (S20 is
not started). Forty pixels is wide enough for the thumbnail the artwork
cache keeps - at most 64 px on its longest edge - and narrow enough to
leave the text columns their room.

The cover column is measured in **pixels** and not in text cells, because a
picture has no font metrics. It is therefore not part of `LIBRARY_COLUMNS`
(which are the index's sortable columns) and not part of the layout's cell
widths. The renderer adds its width to every cell offset, so the text
columns stand behind it in the header and in every row alike.

The **header cell** - the header band's slice of the column - is empty: the
column holds pictures, not text.

## The blit

The renderer takes the covers of the rows it draws as a slice - one entry
per visible row, `Option` because a track may have no cover. A cover is a
**borrowed view** of a picture:

```rust
pub struct CoverRef<'a> {
    pub width: u32,
    pub height: u32,
    pub pixels: &'a [u8],
}

pub fn render_library_window(
    frame: &mut Image,
    view: &LibraryView,
    layout: &LibraryLayout,
    viewport: Rect,
    search_focused: bool,
    covers: &[Option<CoverRef<'_>>],
)
```

`covers[i]` belongs to the i-th row of `view.visible_rows()`. A row whose
cover is missing - or a slice that is shorter than the visible rows - is
drawn without a cover, and its cell keeps the row's background.

The view borrows the bytes, so the caller hands over the picture it already
has **without copying it**: the artwork cache's `CoverImage` becomes a
`CoverRef` by lending its three fields.

The picture **fills** its cell:

* the scale is the **larger** of the two cell-to-cover ratios, so the
  aspect ratio is kept and the cell is covered;
* the sampling is **nearest-neighbour**: every pixel of the cell takes the
  pixel of the cover nearest to it;
* the part of the picture that does not fit is **cropped** away at the
  cell's edges (the crop is centred);
* the scaled picture is blitted with its own alpha, so a transparent part
  of a cover lets the cell's background through;
* nothing of a cover is ever drawn outside its cell.

A cover whose pixels do not match its own size is broken and is drawn as no
cover at all.

The borrowed bytes are already in the `rustamp-gfx` `Image` layout - the
same non-premultiplied RGBA, row by row - so the picture is read in place:
there is no second image type and no copy of the whole thumbnail. Only the
scaled picture (one cell, at most 40x16 px) is materialised.

## The API

| Item | Meaning |
| --- | --- |
| `LIBRARY_COVER_WIDTH` | the cover column's width, in pixels |
| `CoverRef` | a borrowed view of a cover picture |
| `cover_cell(band, index)` | the cell of the row at `index` in the rows' band |
| `text_column_x(list_x, layout, column)` | the pixel x of a text column |
| `LibraryChrome::text_cells()` | the text cells the rows' band holds |

## What the app must wire

1. **Draw.**
   `render_library_window(&mut frame, &view, &layout, viewport, focused, &covers)`.
2. **Size the layout** to the list, not to the window:
   `layout.resized_to(chrome.text_cells())`. The cover column takes its
   pixels out of the band, so the text columns are narrower than the band.
3. **Fetch the covers.** For every visible row, ask the artwork cache for
   the cover of the row's path (`cache.get(&row.path)`) and hand the
   answers to the renderer in the order of `view.visible_rows()`. The
   `Arc<CoverImage>`s must outlive the call.
4. **Convert the cover into a view.** The conversion is a borrow of the
   three fields, not a copy:

   ```rust
   let covers: Vec<Option<CoverRef<'_>>> = view
       .visible_rows()
       .iter()
       .map(|row| {
           cache.get(&row.path).ok().flatten().map(|cover| CoverRef {
               width: cover.width,
               height: cover.height,
               pixels: &cover.pixels,
           })
       })
       .collect();
   ```

   Nothing is copied per frame: the renderer reads the thumbnail's bytes
   where they already lie.
5. **Keep the view's viewport in step** with the window's height:
   `view.set_viewport(chrome.rows.height / LIBRARY_LINE_HEIGHT)`.

## Tests (10, all headless)

`crates/rustamp-ui/tests/library_cover.rs`. The covers are synthetic
pictures built as raw RGBA bytes in the test, so the suite needs neither a
file nor a database - and no crate the interface does not depend on.

- every row has a cover cell of the same width, at the row's own place,
- a cover is drawn into the cell of its row and fills it,
- a row without a cover keeps its cell's background,
- a cover stays inside its cell (right, below and above),
- a cover is scaled to the cell and cropped at its edges,
- a transparent cover leaves the background of the cell,
- the text columns start exactly behind the cover column, in the geometry
  and in the pixels,
- the header's cover cell is empty and the header and the rows line up,
- the cover of a selected row is drawn on the selection,
- nothing of a cover is drawn outside the viewport,
- a cover that breaks its own size is drawn as no cover.

## Remaining limits

- The crop is **centred**, so a square cover shows its middle band in a
  16 px high row. A row that is taller would show more of the picture.
- The picture is scaled for the **cell**, not for the row's height: a
  cover column of its own height would need a row pitch of its own.
- The covers are fetched per frame by the app; the renderer does not
  cache them.
- The cover column cannot be sorted and is not a column of the index.
