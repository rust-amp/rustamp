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

`Skin::cursor(name)` (a lookup by file name) is gone: the slot is the one
way to ask, and it also answers for an `.ani` file, which the name lookup
could not.
