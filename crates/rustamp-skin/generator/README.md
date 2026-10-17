# Rustamp Default Lookalike Skin Generator

This tool procedurally generates the default lookalike skin sprite sheets for Rustamp.

## Clean-Room Design Principles

In compliance with `CLEANROOM.md` and `AGENTS.md`:
- **No copied pixels or artwork**: No original Winamp skin assets or extracted copies were read, fetched, or copied.
- **100% procedural**: Built using purely mathematical and geometric primitives (lines, bevels, gradients, 7-segment digital LED font, custom 5x6 bitmap font, and geometric icon shapes).
- **No external dependencies**: Uses standard Python 3 with built-in `struct` and `math` modules to produce standard uncompressed 24-bit Windows BMP files.
- **Exact specification alignment**: Every sheet matches the exact sprite rectangles and dimensions established in `specs/S01-window-geometry.md`, `specs/S02-main-window-behaviour.md`, and `crates/rustamp-skin/src/webamp.rs`.

## Generated Sheets

The generator outputs 16 classic `.bmp` sprite sheets into `crates/rustamp-skin/assets/lookalike/`:

| Sheet | Dimensions | Purpose |
|---|---|---|
| `main.bmp` | 275 × 116 | Main window dark metallic chrome chassis and LCD display background |
| `titlebar.bmp` | 350 × 87 | Focused/unfocused title bars, shade mode, window buttons, and clutter bar |
| `cbuttons.bmp` | 136 × 36 | Transport buttons (Previous, Play, Pause, Stop, Next, Eject) normal and pressed |
| `shufrep.bmp` | 92 × 85 | Shuffle and Repeat toggle buttons, EQ and Playlist window toggles |
| `posbar.bmp` | 307 × 10 | Track position slider track and metallic thumb knobs (resting and dragged) |
| `volume.bmp` | 68 × 433 | 28-frame volume meter wedge bar graph and thumb knobs |
| `balance.bmp` | 47 × 433 | 28-frame center-zero stereo balance meter and thumb knobs |
| `monoster.bmp` | 56 × 24 | Mono and Stereo display badges (lit and dim states) |
| `playpaus.bmp` | 42 × 9 | Transport status indicators (Play, Pause, Stop, Decoder activity) |
| `numbers.bmp` | 99 × 13 | 7-segment green LED digital time font (digits 0–9, minus sign, blank) |
| `nums_ex.bmp` | 108 × 13 | Extended 12-cell 7-segment digit set with full blank and minus cells |
| `text.bmp` | 155 × 74 | 31 × 3 grid of 5 × 6 character cells (four colorway stripes) |
| `pledit.bmp` | 280 × 186 | Playlist editor title bar, side rails, scrollbar, and bottom command buttons |
| `eqmain.bmp` | 275 × 315 | Equalizer window chrome, band sliders, response graph, and buttons |
| `eq_ex.bmp` | 275 × 56 | Shaded equalizer title bar and mini slider controls |
| `gen.bmp` | 178 × 86 | Generic window frame components (corners, top tiles, side rails) |

## Usage

To regenerate all sprite sheets:

```powershell
python crates/rustamp-skin/generator/generate_skin.py
```
