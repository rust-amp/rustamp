# Task card J2 — Complete Rust-Themed Built-in Skin

## Overview

Task J2 delivers a complete, newly-drawn **Rust-themed** built-in default skin for the Rustamp project (a clean-room Winamp 2.95 reimplementation in Rust) and proves its dimensions are pixel-exact against both specifications and the reference fixture.

The skin replaces the earlier J1 lookalike as the default out-of-the-box appearance, while keeping the J1 lookalike accessible via `--skin lookalike` or `--skin builtin:lookalike`.

## 1. Clean-Room Provenance & Binding Legal Rule

In strict adherence to `CLEANROOM.md`, `AGENTS.md`, and project legal constraints:
- **Every pixel is 100% our own.** No original Winamp skin artwork, extracted assets, or research repository data (`D:\code\rustamp-research`, `D:\code\rustamp-refbin`) were viewed, decompiled, copied, or referenced.
- All artwork, glyphs, cursors, bevels, textures, and motifs are procedurally generated from scratch using pure mathematical primitives in `crates/rustamp-skin/generator/generate_skin.py`.
- **Dimensions-only fixture use:** The test fixture `D:\code\rustamp-testskins\base-2.91.wsz` was inspected exclusively to compare entry names, sheet widths and heights, and cursor sizes. Never was any pixel, color value, or graphical layout extracted from it.

## 2. Completeness (32 Classic Entries + Extended Sheet)

The original Winamp base skin contains exactly 32 entries (18 BMPs, 11 CURs, 3 TXTs). Rustamp now generates all 32 entries plus the optional 19th sheet (`nums_ex.bmp`):

| Entry Name | Type | Size (px) | Citation / Provenance | Verified Sprites / Contents |
|---|---|---|---|---|
| `main.bmp` | BMP (24bpp) | 275 × 116 | S01 §5, line 680 | Main chassis, LCD display cutout, slider grooves, transport button wells |
| `titlebar.bmp` | BMP (24bpp) | 344 × 87 | S01 §5, line 681 | Focused & unfocused titlebars, shade titlebars, Easter egg titlebars, window buttons, clutter bar & lit O/A/I/D/V buttons |
| `cbuttons.bmp` | BMP (24bpp) | 136 × 36 | S01 §5, line 682 | Transport buttons (Prev, Play, Pause, Stop, Next, Eject) normal and pressed |
| `shufrep.bmp` | BMP (24bpp) | 92 × 85 | S01 §5, line 683 | Repeat & Shuffle across 4 states; EQ & Playlist toggle buttons across 4 states |
| `posbar.bmp` | BMP (24bpp) | 307 × 10 | S01 §5, line 684 | Position slider track (248×10), resting thumb (246, 0, 29, 10), dragged thumb (278, 0, 29, 10) |
| `volume.bmp` | BMP (24bpp) | 68 × 433 | S01 §5, line 685 | 28 frames of 68×15 volume track LED wedges + resting/selected thumbs at y=420 |
| `balance.bmp` | BMP (24bpp) | 68 × 433 | S01 §5, line 686 | 28 frames of 38×15 balance track + resting/selected thumbs at y=420 |
| `monoster.bmp` | BMP (24bpp) | 58 × 24 | S01 §5, line 687 | Stereo selected (29×12) & dim (29×12), Mono selected (27×12) & dim (27×12) |
| `playpaus.bmp` | BMP (24bpp) | 42 × 9 | S01 §5, line 688 | Playing (9×9), Paused (9×9), Stopped (9×9), Working (3×9), Idle (3×9) indicators |
| `numbers.bmp` | BMP (24bpp) | 99 × 13 | S01 §5, line 689 | 11 cells of 9×13 7-segment digits (0–9, blank) and minus sign |
| `nums_ex.bmp` | BMP (24bpp) | 108 × 13 | S01 §2.3, webamp.rs | 12 cells of 9×13 (0–9, blank, minus sign) |
| `text.bmp` | BMP (24bpp) | 155 × 74 | S01 §5, line 690 | 4 colorway stripes of 31×3 grid of 5×6 font glyph cells |
| `pledit.bmp` | BMP (24bpp) | 280 × 186 | S01 §5, line 691 | Playlist titlebar corners & strip, side rails, scrollbar track, handle & arrows, bottom blocks & buttons (ADD, REM, SEL, MISC, OPTS) |
| `eqmain.bmp` | BMP (24bpp) | 275 × 315 | S01 §5, line 692 | EQ chassis (275×116), titlebars, slider tracks, thumbs, ON/AUTO/PRESETS buttons, graph background with Rust gear motif |
| `eq_ex.bmp` | BMP (24bpp) | 275 × 82 | S01 §5, line 693 | Shaded EQ titlebars focused & unfocused, mini slider thumbs and buttons |
| `gen.bmp` | BMP (24bpp) | 194 × 109 | S01 §5, line 694 | Generic window frame components (top border corners, center tiles with etched gear teeth, side rails, bottom bars) |
| `genex.bmp` | BMP (24bpp) | 130 × 75 | S01 §5, line 695 | Generic window extension parts (sliders, checkboxes, radio buttons, decorative tabs) |
| `mb.bmp` | BMP (24bpp) | 234 × 119 | S01 §5, line 696 | Minibrowser window chassis, titlebar, navigation buttons, address bar well, content frame |
| `video.bmp` | BMP (24bpp) | 234 × 119 | S01 §5, line 697 | Video window chassis, titlebar, video screen frame with gear watermark, playback controls |
| `normal.cur` | CUR (Windows) | 32 × 32 | S17 §8, fixture | Standard arrow pointer with Rust orange spine accent, hotspot (0, 0) |
| `pnormal.cur` | CUR (Windows) | 32 × 32 | S17 §8, fixture | Playlist normal arrow with playlist document lines icon in Rust orange, hotspot (0, 0) |
| `close.cur` | CUR (Windows) | 32 × 32 | S17 §8, fixture | Close button targeting cursor with orange 'X' marker, hotspot (4, 4) |
| `titlebar.cur` | CUR (Windows) | 32 × 32 | S17 §8, fixture | 4-way move cursor with Rust orange directional arrowheads, hotspot (8, 8) |
| `posbar.cur` | CUR (Windows) | 32 × 32 | S17 §8, fixture | Horizontal position slider adjustment cursor with bidirectional orange arrows, hotspot (8, 8) |
| `volbal.cur` | CUR (Windows) | 32 × 32 | S17 §8, fixture | Volume/balance horizontal adjust cursor with volume arc ticks, hotspot (8, 8) |
| `eqslid.cur` | CUR (Windows) | 32 × 32 | S17 §8, fixture | Equalizer vertical slider adjustment cursor with up/down orange arrows, hotspot (8, 8) |
| `eqtitle.cur` | CUR (Windows) | 32 × 32 | S17 §8, fixture | Equalizer titlebar drag cursor with mini EQ graph icon, hotspot (8, 8) |
| `psize.cur` | CUR (Windows) | 32 × 32 | S17 §8, fixture | Playlist diagonal window resize cursor (NW-SE arrows) in Rust orange, hotspot (8, 8) |
| `ptbar.cur` | CUR (Windows) | 32 × 32 | S17 §8, fixture | Playlist titlebar drag cursor with mini playlist lines icon, hotspot (8, 8) |
| `pvscroll.cur` | CUR (Windows) | 32 × 32 | S17 §8, fixture | Playlist vertical scrollbar cursor with scroll thumb grip, hotspot (8, 8) |
| `pledit.txt` | Text (INI) | 91 bytes | S17 §5, fixture | Playlist styling: Normal text `#DF7B38`, Current `#FFFFFF`, NormalBG `#141517`, SelectedBG `#6A2610`, Font `Arial` |
| `region.txt` | Text (INI) | 189 bytes | S17 §7, fixture | Window region boundaries for `[Normal]`, `[WindowShade]`, and `[Equalizer]` |
| `viscolor.txt` | Text (CSV) | 265 bytes | S17 §6, fixture | 24-color furnace/ember analyzer gradient: obsidian background, dark charcoal grid, 16-step fire spectrum, 5 scope lines, blazing peak dots |

## 3. The Rust Theme Design & Palette Choices

Where specifications left aesthetic details to implementation, the following clean-room design choices were made:

1. **Near-Black Industrial Chassis:**
   - Base metal: Charcoal obsidian `RGB(23, 24, 28)` (`#17181C`) with subtle alternating horizontal micro-striations (`RGB(20, 21, 24)` and `RGB(26, 27, 32)`).
   - Raised 3D bevels: Highlight `RGB(62, 66, 73)` (`#3E4249`), Shadow `RGB(13, 14, 16)` (`#0D0E10`).
   - Sunken 3D bevels: Shadow `RGB(10, 11, 13)` (`#0A0B0D`), Highlight `RGB(50, 54, 60)` (`#32363C`).
   - LCD display wells: Deep obsidian `RGB(11, 12, 14)` (`#0B0C0E`).

2. **Rust Orange Accents:**
   - Signature Rust Orange: `RGB(229, 90, 28)` (`#E55A1C`) and glowing active `RGB(255, 107, 43)` (`#FF6B2B`).
   - Searing amber highlight: `RGB(255, 160, 92)` (`#FFA05C`).
   - Deep burnished rust for shadows and pressed states: `RGB(158, 51, 22)` (`#9E3316`).
   - Play button: Glowing Rust orange triangle icon in resting state, deepened flame when depressed.
   - Position slider thumb: Features vertical grip lines in glowing Rust orange.
   - Volume wedge meter: 28-frame progressive wedge transitioning from warm golden amber (`RGB(230, 160, 20)`) to fiery Rust orange (`RGB(255, 107, 43)`) to blazing flame red (`RGB(229, 57, 28)`).
   - Active toggles: Repeat, Shuffle, Equalizer, Playlist, Stereo, Mono glow in vivid Rust orange when active.

3. **Digital Displays & Fonts:**
   - 7-Segment time display: Bright glowing ember orange `RGB(255, 122, 41)` (`#FF7A29`) with faint warm ghost segments `RGB(48, 22, 13)` (`#30160D`) on obsidian background.
   - Text font (`text.bmp`): 4 distinct colorway stripes:
     - Stripe 0 (y=0): Off-white `RGB(240, 242, 245)` on black.
     - Stripe 1 (y=18): Glowing Rust orange `RGB(255, 122, 41)` on dark charcoal.
     - Stripe 2 (y=38): Warm amber `RGB(255, 179, 102)` on black.
     - Stripe 3 (y=56): Pure crisp white `RGB(255, 255, 255)` on black.

4. **Rust Motifs:**
   - **Equalizer Graph Area (`eqmain.bmp` at (0, 294, 113, 19)):**
     A clean procedural Rust cog / gear silhouette centered in the analyzer background (center at (56, 303), outer radius 8 px, inner radius 5 px, center axle hole 2 px, with 8 gear teeth). Rendered in warm burnished rust tones (`RGB(140, 59, 24)` and `RGB(74, 34, 20)`), providing an iconic Rust watermark without obstructing visualization readings.
   - **Generic Window Header (`gen.bmp`):**
     Top center tiles feature an etched industrial gear tooth border pattern in burnished bronze/rust along the upper frame.
   - **Minibrowser & Video Windows:**
     Matching chassis with gear watermarks embedded into viewport frames.

5. **Windows Cursors:**
   - 11 valid Windows `.cur` files constructed using 32-bit RGBA DIB format with 1-bit transparency AND masks.
   - Hand-crafted cursor shapes matching classic functional requirements with distinct Rust orange arrows and accents.

## 4. Verification

The skin and its dimensions are verified in multiple independent ways:

1. **Static Generator Check:**
   `generate_skin.py::verify_static_dimensions` verifies all 30 graphical sheets and cursors against `EXPECTED_DIMENSIONS` at build time, failing loudly on any dimension discrepancy.
2. **Static Test Suite:**
   `crates/rustamp-skin/tests/rust_theme.rs::static_sheet_dimensions_match_spec_exact` decodes all 19 sheets and asserts their pixel width and height against `SPEC_SHEET_SIZES`.
3. **Cursor & Text Verification:**
   `cursor_dimensions_and_formats_are_valid` asserts each of the 11 cursors has valid 32×32 dimensions, exact hotspot coordinates, and valid `.cur` headers.
   `text_files_parse_results_match_rust_theme` validates parse results of `pledit.txt`, `viscolor.txt`, and `region.txt`.
4. **Fixture Comparison (`RUSTAMP_TESTSKINS`):**
   `compare_sheet_dimensions_against_fixture_when_env_set` checks every sheet and cursor in `base-2.91.wsz` against the shipped Rust theme. It compares **dimensions only** (never pixels), and skips cleanly with an informative note when `RUSTAMP_TESTSKINS` is unset.

## 5. Wiring as Built-in Default

- **Crate `rustamp-skin`:**
  - `rust_theme` module embeds all 33 generated assets and provides `source()` and `sheet_image()`.
  - `BuiltinDefaultSkin` now delegates to `rust_theme::sheet_image()` with dark metallic fallback.
  - `BuiltinLookalikeSkin` delegates to `lookalike::sheet_image()`.
- **Crate `rustamp-app`:**
  - `builtin()` loads the complete Rust-themed skin via `BuiltinDefaultSkin::source()`.
  - `lookalike()` loads the J1 lookalike skin via `BuiltinLookalikeSkin::source()`.
  - `choose()` routes unconfigured runs to `SkinChoice::Builtin` (Rust theme), while `--skin lookalike` or `--skin builtin:lookalike` selects the lookalike skin.
