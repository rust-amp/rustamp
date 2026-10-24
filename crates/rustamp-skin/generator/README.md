# Rustamp Rust-Themed Built-in Skin Generator

This tool procedurally generates the Rust-themed built-in default skin sprite
sheets, cursors and text files for Rustamp. It is the generator behind
`crates/rustamp-skin/src/rust_theme.rs`.

## Clean-Room Design Principles

In compliance with `CLEANROOM.md` and `AGENTS.md`:
- **No copied pixels or artwork**: No original Winamp skin assets or extracted copies were read, fetched, or copied.
- **100% procedural**: Built using purely mathematical and geometric primitives (lines, bevels, gradients, 7-segment digital LED font, custom 5x6 bitmap font, and geometric icon shapes).
- **No external dependencies**: Uses standard Python 3 with built-in `struct` and `math` modules to produce standard uncompressed 24-bit Windows BMP files, valid Windows `.cur` files (32-bit RGBA DIB + 1-bit AND mask), and UTF-8 text files.
- **Exact specification alignment**: Every sheet matches the exact sprite rectangles and dimensions established in `specs/S01-window-geometry.md`, `specs/S02-main-window-behaviour.md`, `specs/S17-skin-loading-quirks.md`, and `crates/rustamp-skin/src/webamp.rs`.

## Generated Files

Running the script writes 33 files into `crates/rustamp-skin/assets/rust-theme/`,
one per entry of `RUST_THEME_GENERATORS` in `generate_skin.py`:

| File | Kind | Dimensions | Purpose |
|---|---|---|---|
| `main.bmp` | BMP 24-bit | 275 × 116 | Main window dark chassis, LCD display cutout, slider grooves, transport button well |
| `titlebar.bmp` | BMP 24-bit | 344 × 87 | Focused/unfocused title bars, shade title bars, Easter egg title bars, window buttons, clutter bar and lit O/A/I/D/V buttons |
| `cbuttons.bmp` | BMP 24-bit | 136 × 36 | Transport buttons (Previous, Play, Pause, Stop, Next, Eject) normal and pressed |
| `shufrep.bmp` | BMP 24-bit | 92 × 85 | Shuffle and Repeat toggle buttons across 4 states, EQ and Playlist window toggles |
| `posbar.bmp` | BMP 24-bit | 307 × 10 | Track position slider track and thumb knobs (resting and dragged) |
| `volume.bmp` | BMP 24-bit | 68 × 433 | 28-frame volume meter wedge bar graph and thumb knobs |
| `balance.bmp` | BMP 24-bit | 68 × 433 | 28-frame center-zero stereo balance meter and thumb knobs |
| `monoster.bmp` | BMP 24-bit | 58 × 24 | Mono and Stereo display badges (lit and dim states) |
| `playpaus.bmp` | BMP 24-bit | 42 × 9 | Transport status indicators (Play, Pause, Stop, Decoder activity) |
| `numbers.bmp` | BMP 24-bit | 99 × 13 | 7-segment ember-orange digital time font (digits 0–9, minus sign, blank) |
| `nums_ex.bmp` | BMP 24-bit | 108 × 13 | Extended 12-cell 7-segment digit set with full blank and minus cells |
| `text.bmp` | BMP 24-bit | 155 × 74 | 31 × 3 grid of 5 × 6 character cells in four colorway stripes |
| `pledit.bmp` | BMP 24-bit | 280 × 186 | Playlist editor title bar, side rails, scrollbar, and bottom command buttons |
| `eqmain.bmp` | BMP 24-bit | 275 × 315 | Equalizer window chrome, band sliders, Rust gear motif, and graph |
| `eq_ex.bmp` | BMP 24-bit | 275 × 82 | Shaded equalizer title bars, mini slider controls and buttons |
| `gen.bmp` | BMP 24-bit | 194 × 109 | Generic window frame components with etched gear-teeth border motif |
| `genex.bmp` | BMP 24-bit | 130 × 75 | Generic window extension parts (sliders, checkboxes, decorative tabs) |
| `mb.bmp` | BMP 24-bit | 234 × 119 | Minibrowser window chassis, titlebar, navigation buttons, address bar well |
| `video.bmp` | BMP 24-bit | 234 × 119 | Video window chassis, titlebar, screen frame with gear watermark, controls |
| `normal.cur` … `pvscroll.cur` | CUR 32 × 32 | 11 files | Standard arrow, playlist, close, titlebar-move, position, volume/balance, EQ-slider, EQ-title, resize, playlist-titlebar and scrollbar cursors |
| `pledit.txt` | Text (INI) | — | Playlist styling: Normal/Current/NormalBG/SelectedBG colours and font |
| `region.txt` | Text (INI) | — | Window region boundaries for `[Normal]`, `[WindowShade]`, `[Equalizer]` |
| `viscolor.txt` | Text (CSV) | — | 24-colour furnace/ember analyser gradient |

## Static Dimension Check

`main()` ends with `verify_static_dimensions(RUST_THEME_DIR)`, which asserts —
via the `EXPECTED_DIMENSIONS` table — that every generated file exists and
that every BMP decodes to its spec width/height and every CUR is a single
32 × 32 image. A mismatch fails the script loudly.

Note: this check runs when the script is executed manually. There is no
`build.rs` in `crates/rustamp-skin`, so it does **not** run at build time
(an earlier task card said otherwise; that claim was wrong).

## The Lookalike Assets Are Frozen

`crates/rustamp-skin/assets/lookalike/` still holds the 16 BMPs of the
earlier lookalike skin (task card J1), now selectable via
`BuiltinLookalikeSkin`. The current `main()` does **not** regenerate them:
it writes only to `RUST_THEME_DIR`, and the `LOOKALIKE_DIR` constant is dead
(it is kept as a pointer to where those frozen assets live). Do not expect
re-running the script to reproduce the lookalike assets byte for byte — the
lookalike generators were replaced by the Rust-theme generators.

## Usage

To regenerate all Rust-theme assets:

```powershell
python crates/rustamp-skin/generator/generate_skin.py
```
