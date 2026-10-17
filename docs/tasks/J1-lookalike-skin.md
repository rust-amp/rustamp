# Task card J1 — Default Lookalike Skin

## Overview

Rustamp is a clean-room Winamp 2.95 reimplementation in Rust. Previously, the shipped default skin was a flat magenta placeholder (`PLACEHOLDER_RGBA` `[0xFF, 0x00, 0xFF, 0xFF]`), which blocked beta-readiness (BETA-02 in `docs/tasks/BETA-readiness.md`).

Task J1 delivers a newly-drawn lookalike skin with classic dark player chrome, 3D buttons, sliders, LED displays, equalizer, playlist, and a legible bitmap font.

## 1. Clean-Room Provenance & Constraints

In strict adherence to `CLEANROOM.md` and `AGENTS.md`:
- No original Winamp skin files, extracted assets, or research repository data (`D:\code\rustamp-research`, `D:\code\rustamp-refbin`) were read, viewed, copied, or fetched.
- All dimensions, layout coordinates, and sprite boundaries were sourced exclusively from clean specifications (`specs/S01-window-geometry.md`, `specs/S02-main-window-behaviour.md`, `specs/S05-docking.md`, `specs/S06-playlist.md`, `specs/S17-skin-loading-quirks.md`) and the audited clean sprite table in `crates/rustamp-skin/src/webamp.rs`.
- All artwork is 100% procedurally generated from scratch using mathematical and geometric drawing primitives.
- No external dependencies, libraries, or asset downloads were added.

## 2. Specification Audit & Required Assets

The built-in default skin loader expects 16 BMP sprite sheets plus standard playlist and visualizer color defaults:

| Sheet | File Name | Size (px) | Verified Sprites / Contents |
|---|---|---|---|
| `Main` | `main.bmp` | 275 × 116 | `MAIN_WINDOW_BACKGROUND` (0, 0, 275, 116); main chassis, LCD display well, slider and button slots |
| `Titlebar` | `titlebar.bmp` | 350 × 87 | `MAIN_TITLE_BAR` (27, 15, 275, 14), `MAIN_TITLE_BAR_SELECTED` (27, 0, 275, 14), `MAIN_SHADE_BACKGROUND` (27, 42, 275, 14), `MAIN_SHADE_BACKGROUND_SELECTED` (27, 29, 275, 14), `MAIN_EASTER_EGG_TITLE_BAR` (27, 72, 275, 14), `MAIN_EASTER_EGG_TITLE_BAR_SELECTED` (27, 57, 275, 14), window buttons (Options, Min, Shade, Close) normal & depressed, clutter bar background (304, 0, 8, 43), clutter bar lit buttons (O, A, I, D, V) at (308..340, 47..78), shade position slider parts |
| `Cbuttons` | `cbuttons.bmp` | 136 × 36 | Transport buttons (Previous, Play, Pause, Stop, Next, Eject) normal at y=0, active/pressed at y=18 (22×18 each, eject 22×16) |
| `Shufrep` | `shufrep.bmp` | 92 × 85 | Repeat button (0, 0, 28, 15), Shuffle button (28, 0, 47, 15) across 4 states (normal, depressed, selected, selected+depressed); EQ button (0, 61, 23, 12), Playlist button (23, 61, 23, 12) normal, selected, depressed, depressed+selected |
| `Posbar` | `posbar.bmp` | 307 × 10 | `MAIN_POSITION_SLIDER_BACKGROUND` track (0, 0, 248, 10), `MAIN_POSITION_SLIDER_THUMB` resting (246, 0, 29, 10), `MAIN_POSITION_SLIDER_THUMB_SELECTED` dragged (278, 0, 29, 10) |
| `Volume` | `volume.bmp` | 68 × 433 | `MAIN_VOLUME_BACKGROUND` (0, 0, 68, 420; 28 frames of 68×15), `MAIN_VOLUME_THUMB` (15, 420, 14, 11), `MAIN_VOLUME_THUMB_SELECTED` (0, 420, 14, 11) |
| `Balance` | `balance.bmp` | 47 × 433 | `MAIN_BALANCE_BACKGROUND` (9, 0, 38, 420; 28 frames of 38×15), `MAIN_BALANCE_THUMB` (15, 420, 14, 11), `MAIN_BALANCE_THUMB_ACTIVE` (0, 420, 14, 11) |
| `Monoster` | `monoster.bmp` | 56 × 24 | `MAIN_STEREO_SELECTED` (0, 0, 29, 12), `MAIN_STEREO` (0, 12, 29, 12), `MAIN_MONO_SELECTED` (29, 0, 27, 12), `MAIN_MONO` (29, 12, 27, 12) |
| `Playpaus` | `playpaus.bmp` | 42 × 9 | `MAIN_PLAYING_INDICATOR` (0, 0, 9, 9), `MAIN_PAUSED_INDICATOR` (9, 0, 9, 9), `MAIN_STOPPED_INDICATOR` (18, 0, 9, 9), `MAIN_WORKING_INDICATOR` (36, 0, 3, 9), `MAIN_NOT_WORKING_INDICATOR` (39, 0, 3, 9) |
| `Numbers` | `numbers.bmp` | 99 × 13 | 11 cells of 9×13 (digits 0–9, filler/blank at 90, 0), minus sign (20, 6, 5, 1), no-minus placeholder (9, 6, 5, 1) |
| `NumsEx` | `nums_ex.bmp` | 108 × 13 | 12 cells of 9×13 (digits 0–9, blank at 90, 0, minus cell at 99, 0) |
| `Text` | `text.bmp` | 155 × 74 | 31 columns × 3 rows of 5×6 glyph cells (letters, numerals, punctuation, specials), 4 vertical colorway stripes at y=0, 18, 38, 56 |
| `Pledit` | `pledit.bmp` | 280 × 186 | Title bar corners (0, 0/21, 25, 20), strip (26, 0/21, 100, 20), top tile (127, 0/21, 25, 20), right corner (153, 0/21, 25, 20); left rail (0, 42, 12, 29), right rail (31, 42, 20, 29); bottom blocks (0, 72, 125, 38) and (126, 72, 150, 38); scroll handle (52, 53, 8, 18) and arrows (261, 72, 8, 10); bottom buttons (ADD, REM, SEL, MISC, LIST OPTS) |
| `EqMain` | `eqmain.bmp` | 275 × 315 | `EQ_WINDOW_BACKGROUND` (0, 0, 275, 116), title bars (0, 134/149, 275, 14), slider track area (13, 164, 209, 129), thumbs (0, 164/176, 11, 11), buttons (ON, AUTO, PRESETS), graph background (0, 294, 113, 19), graph scale (115, 294, 1, 19), preamp line (0, 314, 113, 1) |
| `EqEx` | `eq_ex.bmp` | 275 × 56 | Shaded equalizer title bars focused (0, 0, 275, 14) and unfocused (0, 15, 275, 14), mini slider thumbs and window buttons |
| `Gen` | `gen.bmp` | 178 × 86 | Generic window frame components (corners, top tiles, side rails, bottom bars) |

## 3. Procedural Generator

The generator is located at `crates/rustamp-skin/generator/generate_skin.py` with documentation in `crates/rustamp-skin/generator/README.md`.
It uses pure Python standard library (`struct`, `math`) to construct valid, uncompressed 24-bit BMP files directly.

Features drawn procedurally:
- **Chassis Texture**: Alternating horizontal scanline micro-striations over a dark metallic palette.
- **3D Bevels**: Raised bevels for outer borders and resting buttons; sunken bevels for the LCD display well, sliders, and active buttons.
- **Transport Icons**: Crisp geometric vector shapes (`|<<`, `>`, `||`, `■`, `>>|`, `▲/_`).
- **7-Segment LED Font**: 9×13 digit cells with bright active segments and subtle unlit ghost segments.
- **Marquee Font**: High-contrast, legible 5×6 bitmap font covering all uppercase letters, digits, punctuation, and Scandinavian diacritics.
- **Multi-Frame Sliders**: 28 discrete LED wedge frames for volume and balance meters.

## 4. Wiring

> **Superseded by task card J2 (Rust theme).** The built-in default is the Rust
> theme now; the look-alike is selectable. The bullets record how J1 was wired
> when it landed; the *Now* notes give the current code. *(W11 docs sweep.)*

- **Asset Embedding**: The generated BMP files are placed in `crates/rustamp-skin/assets/lookalike/*.bmp` and embedded into `crates/rustamp-skin/src/lookalike.rs` using `include_bytes!`.
- **Builtin Skin Implementation (as of J1)**: `BuiltinDefaultSkin::sheet_image(sheet)` in `crates/rustamp-skin/src/skin.rs` delegated to `lookalike::sheet_image(sheet)` which decodes the embedded BMP data using Rustamp's internal `bmp::decode`. **Now** it delegates to `rust_theme::sheet_image(sheet)` (`crates/rustamp-skin/src/skin.rs:105-116`).
- **Fallback Integrity**: `SkinFallback` supplies sheets when any individual sheet is missing from a loaded skin, fulfilling S17 §2. The default fallback is now `BuiltinDefaultSkin` (the Rust theme); `BuiltinLookalikeSkin` supplies the look-alike sheets (`crates/rustamp-skin/src/skin.rs:132-143`).
- **App Integration (as of J1)**: `rustamp_app::skin::builtin()` loaded the look-alike on first run. **Now** it loads `BuiltinDefaultSkin::source()`, i.e. the Rust theme, on first run or when `--skin` is omitted (`crates/rustamp-app/src/skin.rs:85-100`; `choose(None, None) == SkinChoice::Builtin`, lines 30-36).

## 5. Own Choices

Where specifications left color, texture, or aesthetic details unspecified, the following clean-room design choices were made:

1. **Chassis Color Palette**:
   - Base chassis: Dark metallic charcoal `RGB(40, 42, 48)` with alternating `RGB(38, 40, 46)` and `RGB(44, 46, 52)` horizontal striations to recreate the classic Hi-Fi brushed aluminum look.
   - Raised bevel highlight: `RGB(85, 90, 100)`; shadow: `RGB(15, 16, 20)`.
   - LCD display well: Deep obsidian `RGB(8, 10, 12)` with sunken bevel shadow `RGB(14, 16, 18)` and highlight `RGB(70, 75, 85)`.

2. **Title Bar Gradients**:
   - Focused title bar: Deep metallic sapphire/navy gradient from `RGB(20, 35, 90)` on the left to `RGB(35, 55, 130)` on the right with a fine highlight stripe and embossed chrome "RUSTAMP" title lettering.
   - Unfocused title bar: Muted dark slate / steel grey gradient from `RGB(45, 48, 55)` to `RGB(30, 32, 38)`.

3. **Button Styling**:
   - Resting buttons: Gunmetal grey `RGB(55, 60, 68)` with raised 3D lighting.
   - Pressed buttons: Sunken bevel with darkened face `RGB(42, 46, 52)` and 1px downward-right shift.
   - Button icons: Clean silver/white `RGB(200, 215, 230)`.

4. **Digital LED Time Display**:
   - Active segments: Bright glowing phosphor green `RGB(0, 255, 60)`.
   - Inactive segments: Faint dithered ghost segments `RGB(12, 35, 18)` on `RGB(5, 8, 8)` background to mimic a vacuum fluorescent or LCD display.

5. **5×6 Text Font Design**:
   - Custom 1px-stroke font designed specifically for readability in a 5×6 cell on a black background (`RGB(0, 255, 60)`).
   - Distinct letterforms for confusing pairs (e.g., `O` vs `0`, `1` vs `I`, `8` vs `B`).

6. **Volume & Balance Meter Wedges**:
   - Volume: 28-frame progressive wedge transitioning from green (`RGB(0, 240, 50)`) through yellow (`RGB(250, 220, 0)`) to red (`RGB(255, 40, 30)`).
   - Balance: 28-frame centered L/R meter centered at frame 14.

7. **Equalizer & Playlist Chrome**:
   - Styled to match the main chassis, including frequency markers (`60`, `170`, `310`, `600`, `1K`, `3K`, `6K`, `12K`, `14K`, `16K`), dB scale markings (`+12db`, `0db`, `-12db`), and functional button labels (`ON`, `AUTO`, `PRESETS`, `ADD`, `REM`, `SEL`, `MISC`, `OPTS`).

## 6. Verification

Tests added in `crates/rustamp-skin/tests/lookalike.rs`:
- `builtin_skin_loads_successfully`: Verifies all 16 sheets exist and load with non-zero dimensions.
- `every_sprite_exists_with_expected_dimensions`: Validates every sprite in `webamp::SPRITES` fits within its sheet bounds.
- `no_sprite_is_placeholder_magenta_colour`: Verifies that no sprite rectangle is a flat magenta (`[0xFF, 0x00, 0xFF, 0xFF]`) placeholder.
- `text_font_renders_legible_glyphs`: Verifies that every non-space glyph in `TEXT_GLYPHS` has positive ink pixel coverage and is not a solid block.
