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

## 4. What `rustamp-ui` consumes

`Skin::sprite(name)` returns the rectangle plus its `Sheet`; `Skin::sheet(sheet)`
returns the decoded image. The UI never sees a file name. `Skin::substitutions`
and `Skin::warnings` describe anything that was missing or failed to decode.
