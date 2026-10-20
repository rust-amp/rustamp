# Contract notes — rustamp-skin

This crate keeps its public types local to `rustamp-skin`; nothing here needs a
change to `rustamp-core` today. These are proposals for later, recorded as the
task card asks, not requests that block M1.

## 1. A shared RGBA image type

`rustamp-skin::RasterImage` and `rustamp-gfx::Image` both hold an 8-bit RGBA
buffer, because the task card keeps `rustamp-gfx` independent of
`rustamp-skin` ("it takes RGBA buffers and sprite rectangles, not skin paths")
and `rustamp-skin` consuming only `rustamp-core`. `rustamp-ui` will have to
convert between the two (`RasterImage::into_parts` → `Image::from_rgba`).

**Proposal:** if the conversion becomes noise, add one RGBA buffer type to
`rustamp-core` (or a small `rustamp-image` crate) and have both crates use it.
That is a contract change and needs an RFC. Until then the conversion is one
call and the crates stay decoupled.

## 2. Default-skin hand-off

`BuiltinDefaultSkin` generates flat placeholder rectangles at the sizes implied
by the sprite table. It is **not** the shipped default skin: that is the
look-alike skin from the art lane. When it exists, load it with
`Skin::load_with_fallback` / `SkinFallback` so the placeholder is only a test
fixture. `PLACEHOLDER_RGBA` is deliberately obvious.

## 3. Sprite coordinates

`rustamp-skin::webamp::SPRITES` and the default colours are ported from webamp
and are provisional until **S01** (geometry) and **S17** (skin quirks). The
table is isolated in one module so it can be replaced wholesale.

The playlist defaults are no longer provisional: **S17 §5** lists the base
skin's `pledit.txt` and records that a playlist without a `pledit.txt` still
renders green / white / black, so `DEFAULT_PLAYLIST_NORMAL`,
`DEFAULT_PLAYLIST_CURRENT` and `DEFAULT_PLAYLIST_NORMAL_BG` are the built-in
defaults of the classic look. The selected-row background and the font were
**not** observed without the file and keep their webamp values — adopting the
base skin's `#0000C6` would change the rendered playlist and needs the
look-alike golden images in `rustamp-ui` re-blessed.

## 4. What `rustamp-ui` consumes

`Skin::sprite(name)` returns the rectangle plus its `Sheet`; `Skin::sheet(sheet)`
returns the decoded image. The UI never sees a file name. `Skin::substitutions`
and `Skin::warnings` describe anything that was missing or failed to decode.

**The time display's digit sheet** is `Skin::time_digit_sheet()`, never a
guess at the call site. **S17 §3**: when a skin carries `nums_ex.bmp`, the
time display uses it *instead of* `numbers.bmp` and the digit layout changes
with it, so the two sheets are never substituted for one another. The choice
follows the file the *skin* carries — a `nums_ex.bmp` that fails to decode
does not take the display away from the skin's `numbers.bmp`.

The loader no longer reports a sheet-to-sheet substitution: with the layouts
of S17 §3 in force, no sheet can stand in for another, so `Substitute` has
only the two cases the spec names — the built-in default skin and the
documented default values.

## 5. The cursor slots

The cursors a skin may ship are modelled as **slots**
(`rustamp_skin::CursorSlot`): one variant per classic cursor file name
(`normal.cur`, `close.cur`, `posbar.cur`, … — the set the loader asks for,
ported from webamp's `CURSORS` table, MIT). A slot is filled by the file
that carries it, `<stem>.cur` or `<stem>.ani`, matched case-insensitively,
and `Skin::cursor_slot(slot)` hands out that file's **bytes**:

```rust
match skin.cursor_slot(CursorSlot::PosBar) {
    Some(cursor) => /* cursor.bytes, cursor.kind */,
    None => /* the caller falls back to the system cursor */,
}
```

Three decisions are recorded here because they are not measurements:

1. **The bytes are never decoded.** S17 §8 verified that the base skin's
   cursors are ordinary Windows `.cur` files but says nothing about their
   content, and the OS loads them. The crate therefore keeps the file bytes
   untouched and adds no decoder dependency; the `.ani` variant is only
   recognised by its `RIFF` header (`CursorKind::Ani`).
2. **A slot nobody carries is absent, not empty.** The J4 rule (a missing
   cursor falls back to the built-in skin's cursor of the same name) still
   applies first; a name neither the skin nor the fallback carries stays
   absent with no substitution, and the caller falls back to the system
   cursor. `Skin::cursor_slot` therefore returns `Option`, and `None` means
   "this skin has no cursor for that slot" — never "an empty cursor".
3. **The slot names are the classic set, the region mapping is not here.**
   S17 §8 leaves the cursor-to-region mapping open, so the mapping from a
   hit-test result to a slot lives in `rustamp-ui` (`rustamp_ui::cursor`) as
   Rustamp's own choice, documented there.
4. **A directory prefix is ignored**, the way the skin file lookup ignores
   it (`source.rs`): a skin that carries `cursors/close.cur` fills the
   `close` slot, and the skin's own cursor is not shadowed by the fallback's.
   The slot lookup therefore works on the last path component of a name.

`Skin::cursor(name)` (a lookup by file name) is gone: the slot is the one
way to ask, and it also answers for an `.ani` file, which the name lookup
could not.

## 6. The window-shape mask

`rustamp_skin::Region::mask(section, width, height)` turns the polygons of a
`region.txt` section into the **per-pixel coverage** of a window of that size:
a bitmap with one bit per pixel, plus a run-length view
(`RegionMask::spans`) for an application that has to hand the shape to the
system as rectangles. `None` means **no mask** — the section has no polygon,
so the window keeps its rectangle (S17 §7: the base skin's `region.txt` has
every polygon commented out).

Three decisions are recorded here because S17 §7 does not settle them:

1. **The fill rule is this crate's own choice: the non-zero winding rule,
   sampled at the pixel centre.** A pixel is covered when the point
   `(x + 0.5, y + 0.5)` lies inside the polygon, "inside" meaning that the
   winding number around the point is not zero. S17 §7 only says that the
   polygons define the visible area, that the coordinates are the outer pixel
   corners and that the polygons of a section are additive; it does not say
   how a self-intersecting polygon is filled. The rule is exact — the
   rasteriser works in integer arithmetic, every edge crossing being a
   rational number — so a diagonal edge never misplaces a pixel.
2. **The polygons of one section are additive** (S17 §7): the mask is the
   union of their areas, so a polygon wound the other way inside another one
   does not cut a hole. The mask is therefore not a general polygon boolean
   operation, only a union.
3. **A pixel whose centre lies exactly on an edge belongs to the run that
   starts at that edge**: the covered interval of a scanline is half-open,
   `[left, right)`. A diagonal edge through pixel corners therefore splits
   those pixels cleanly instead of covering them twice or not at all.

The mask belongs to one window size: a window that changes its size needs a
new mask. The application applies the mask to the client area; the crate
never calls the system itself.

## 7. The `Font=` line

`PlaylistStyle::font` is an `Option<PlaylistFont>`: the family the skin asks
for, the size in pixels when the file named one, and the two style flags.
(The field was an `Option<String>` before; it now carries the parsed request,
so a consumer does not have to parse the line a second time.)

**What S17 §5 settles:** the key, the family name and the base skin's value
`Font=Arial`. The list is drawn with that real font, not with the `text.bmp`
bitmap font.

**What S17 §5 does not settle:** a size, a style, and "a `Font` name that is
not installed" — the spec lists all three as *not determined*. The syntax
below is therefore this crate's own reading of the classic `Font=Name,size`
form, and the decisions are recorded here:

1. **The value is `Name[,size[,style...]]`.** The first comma-separated token
   is the family name. Every later token is either a size in pixels (a
   decimal number between 1 and 255) or one of the style flags `bold` and
   `italic` (case-insensitive). A token that is none of these is ignored,
   and the first size wins.
2. **A value without a family name is malformed**, and a malformed value
   leaves the built-in default font in place — the same rule a malformed
   colour follows. A skin that names no `Font=` line at all keeps the
   default too.
3. **The size is the size the playlist rows are drawn at.** A skin that names
   no size gets the playlist's own 13 px (S01 §4.2's row pitch); that
   default belongs to `rustamp-ui`, not to this crate, so `PlaylistFont::size`
   stays an `Option`.

The host side of the fallback (a family the machine does not have) lives in
`rustamp-ui`: the requested family, then the base skin's own family, then
the generic sans-serif.

## 8. A malformed `region.txt`

S17 §7 leaves the handling of a malformed `NumPoints`/`PointList` open. The
parser's own rule, and the reason for it:

1. **A count below three is dropped**, and the points that polygon would have
   used are skipped, so the polygons after it keep their own points.
2. **A polygon whose point list ends early keeps the points it has** — a
   polygon with fewer than three points left is dropped.
3. **A count that is not a number at all** (a negative count, a word, a
   number too large to count points) ends the section: the point list can no
   longer be aligned with the counts, so the parser stops there and keeps the
   polygons it has already read.

The third rule exists because the first two cannot cover it: a count whose
size is unknown makes every later polygon ambiguous, and guessing would read
the *next* polygon's points as this one's — a silently wrong window shape.
A shape that is not there is better than a shape that is wrong.

The arithmetic of the parser is saturating, so a count of
`18446744073709551615` is a count, not an overflow.
