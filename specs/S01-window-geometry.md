---
Spec ID: S-01
Title: Window geometry, sprite rectangles and pixel layout deltas
Sources consulted (category only): dirty room: binary behaviour capture; dirty room: shipped skin assets (test input); free: webamp (cross-check only)
Author agent: R-agent (dirty room, capture session 2026-10-14)
Auditor: Spec Auditor (dirty room)
Audit date: 2026-10-14
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561, extracted from winamp295.exe SHA-256 ef1423999cbefd4f2a3a98acee2b194a4260268cb4517abda9f12963e866b7e5)
Status: audited
---

# S01 — Window geometry, sprite rectangles and pixel layout deltas

Scope: the exact pixel geometry of the three classic windows and of every classic skin
sheet, and where each sprite is drawn. Everything marked **verified** was measured from
renders of the real 2.95 binary or read from the skin files. Everything marked
**provisional** is a candidate that this session did not confirm against 2.95; treat
provisional rows as unconfirmed and do not rely on them.

Measurement method (for provenance): a screenshot of a window is compared pixel by pixel
against a rectangle of a skin sheet; a rectangle is accepted only when the mean
per-channel difference over the rectangle is exactly zero. Positions are window-relative
(x to the right, y down, origin at the window's client top-left).

All numbers below are for the **original Winamp base skin** (the `.wsz` that ships with
webamp as `base-2.91`), loaded into a sandboxed 2.95. Pixel tables are skin data, not
Winamp data: a different skin will have different pixels but the same rectangles.

---

## 1. Window geometry

Measured from window rectangles at run time and from the saved window state file.

| Window | Window class | Default size (px) | Windowshade size | Double-size size |
|---|---|---|---|---|
| Main | `Winamp v1.x` | 275 x 116 | 275 x 14 | 550 x 232 |
| Equalizer | `Winamp EQ` | 275 x 116 | 275 x 14 | 550 x 232 |
| Playlist editor | `Winamp PE` | 275 x 116 | 275 x 14 | unchanged (275 x 116) |
| Minibrowser | `Winamp MB` | 350 x 348 | n/a | n/a |
| Video | `Winamp Video` | 275 x 232 | n/a | n/a |

Verified: the main and EQ windows are both 275 x 116 at 1x and both exactly 2x in
double-size mode (550 x 232); the playlist editor is **not** affected by double-size mode.
The main, EQ and playlist sizes were read from running windows. The minibrowser and video
sizes are the defaults recorded in the saved state file and were **not** confirmed on a
running window in this session (provisional).

Notes that follow from the measurements:

- Every window is a plain rectangle. With the base skin there is **no** masked shape,
  because the base skin's `region.txt` has every polygon commented out. A skin that does
  define a region changes the visible outline only; the window rectangle stays as above.
- Double-size is a whole-window 2x scale: the double-size render is an exact
  pixel-doubling of the 1x render (verified on the titlebar band, mean difference 0.0 for
  every sampling phase). There is no separate double-size sprite set in the base skin.
- Windowshade height is 14 px for main, EQ and playlist. The playlist remembers a
  separate shade height in its saved state (`pe_height_ws`), which is 0 in the default
  state; the drawn shade window is 14 px tall regardless.
- The three windows stack by default with no gaps: main at (26, 29), EQ at (26, 145),
  playlist at (26, 261). Each is exactly one main-window height (116 px) below the
  previous one. The minibrowser's default position is (301, 29), i.e. one main-window
  width (275 px) to the right of the main window; the video window defaults to the EQ
  position (26, 145).
- The saved state file records the geometry as: `wx`, `wy`, `eq_wx`, `eq_wy`, `pe_wx`,
  `pe_wy`, `pe_width`, `pe_height`, `pe_height_ws`, `mb_wx`, `mb_wy`, `mb_width`,
  `mb_height`, `video_wx`, `video_wy`, `video_width`, `video_height`, plus the open
  flags `mw_open`, `eq_open`, `pe_open`, `mb_open`, `video_open`, and the mode flags
  `windowshade`, `dsize`, `eq_ws`. Defaults measured on a fresh install: `wx=26 wy=29
  eq_wx=26 eq_wy=145 pe_wx=26 pe_wy=261 pe_width=275 pe_height=116 pe_height_ws=0
  mb_wx=301 mb_wy=29 mb_width=350 mb_height=348 video_wx=26 video_wy=145
  video_width=275 video_height=232`, all open flags 1 except `video_open=0`.

---

## 2. Main window (275 x 116)

Sheets used: `main.bmp`, `titlebar.bmp`, `cbuttons.bmp`, `shufrep.bmp`, `posbar.bmp`,
`volume.bmp`, `balance.bmp`, `monoster.bmp`, `playpaus.bmp`, `numbers.bmp`, `text.bmp`.

### 2.1 Sprite table

`sheet` is the file name; `rect` is x, y, width, height inside the sheet; `at` is the
window position where the sprite is blitted. Rows marked verified matched with zero
difference.

| Element | Sheet | Rect | At | State |
|---|---|---|---|---|
| Window background | main.bmp | 0,0,275,116 | 0,0 | verified |
| Title bar, window focused | titlebar.bmp | 27,0,275,14 | 0,0 | verified |
| Title bar, window not focused | titlebar.bmp | 27,15,275,14 | 0,0 | verified |
| Shade-mode title bar, focused | titlebar.bmp | 27,29,275,14 | 0,0 | provisional |
| Shade-mode title bar, not focused | titlebar.bmp | 27,42,275,14 | 0,0 | provisional |
| Easter-egg title bar, focused | titlebar.bmp | 27,57,275,14 | 0,0 | provisional |
| Easter-egg title bar, not focused | titlebar.bmp | 27,72,275,14 | 0,0 | provisional |
| Clutter bar background | titlebar.bmp | 304,0,8,43 | 10,22 | verified |
| Clutter bar, letter 1 lit | titlebar.bmp | 308,47,4,6 | 14,25 | provisional (never captured lit) |
| Clutter bar, letter 2 lit | titlebar.bmp | 316,55,4,6 | 14,33 | verified |
| Clutter bar, letter 3 lit | titlebar.bmp | 324,63,3,6 | 14,41 | verified |
| Clutter bar, letter 4 lit | titlebar.bmp | 332,71,4,6 | 14,49 | verified (also via double-size) |
| Clutter bar, letter 5 lit | titlebar.bmp | 340,78,4,6 | 14,56 | verified |
| Previous | cbuttons.bmp | 0,0,22,18 | 16,88 | verified |
| Play | cbuttons.bmp | 23,0,22,18 | 39,88 | verified |
| Pause | cbuttons.bmp | 46,0,22,18 | 62,88 | verified |
| Stop | cbuttons.bmp | 69,0,22,18 | 85,88 | verified |
| Next | cbuttons.bmp | 92,0,22,18 | 108,88 | verified |
| Eject | cbuttons.bmp | 114,0,22,16 | 136,89 | verified |
| All six, pressed | cbuttons.bmp | same x, y = 18 | same | provisional |
| Shuffle button | shufrep.bmp | 28,0,47,15 | 164,89 | verified |
| Shuffle, lit | shufrep.bmp | 28,30,47,15 | 164,89 | verified |
| Repeat button | shufrep.bmp | 0,0,28,15 | 210,89 | verified |
| Repeat, lit | shufrep.bmp | 0,30,28,15 | 210,89 | verified |
| EQ toggle button | shufrep.bmp | 0,61,23,12 | 219,58 | verified |
| EQ toggle, window open | shufrep.bmp | 0,73,23,12 | 219,58 | verified |
| Playlist toggle button | shufrep.bmp | 23,61,23,12 | 242,58 | verified |
| Playlist toggle, window open | shufrep.bmp | 23,73,23,12 | 242,58 | verified |
| Position bar track | posbar.bmp | 0,0,248,10 | 16,72 | verified |
| Position bar thumb | posbar.bmp | 248,0,29,10 | 16,72 at position 0 | provisional (composite, small residual) |
| Position bar thumb, dragged | posbar.bmp | 278,0,29,10 | as above | provisional |
| Volume track | volume.bmp | see open question 4 | 107,57 | provisional |
| Balance track | balance.bmp | see open question 4 | 177,57 | provisional |
| Time digit 1 (tens of minutes) | numbers.bmp | 9d,0,9,13 | 48,26 | verified |
| Time digit 2 (minutes) | numbers.bmp | 9d,0,9,13 | 60,26 | verified |
| Time digit 3 (tens of seconds) | numbers.bmp | 9d,0,9,13 | 78,26 | verified |
| Time digit 4 (seconds) | numbers.bmp | 9d,0,9,13 | 90,26 | verified |
| Remaining-time minus sign | numbers.bmp | 20,6,5,1 | 38,32 | verified |
| Play indicator | playpaus.bmp | 0,0,9,9 | 26,28 | verified |
| Pause indicator | playpaus.bmp | 9,0,9,9 | 26,28 | verified |
| Stop indicator | playpaus.bmp | 18,0,9,9 | 26,28 | verified |
| Decoder-active indicator | playpaus.bmp | 36,0,3,9 | 24,28 | verified |
| Stereo, lit | monoster.bmp | 0,0,29,12 | 239,41 | verified |
| Stereo, dim | monoster.bmp | 0,12,29,12 | 239,41 | verified |
| Mono, lit | monoster.bmp | 29,0,27,12 | 212,41 | verified |
| Mono, dim | monoster.bmp | 29,12,27,12 | 212,41 | verified |

`d` above is the digit value 0–9, so the digit cells are at sheet x = 0, 9, 18, … 81.

### 2.2 Bitmap-font text (text.bmp)

- `text.bmp` is 155 x 74 px, 8-bit palettised, with exactly three palette entries:
  an ink colour, a background colour and one more. In the base skin the ink is a mid
  green and the glyphs are drawn in that ink colour as-is; the rest of the sheet is not
  drawn.
- The glyph grid is **31 columns x 3 rows of 5 x 6 px cells** (155 = 31 x 5, and three
  glyph rows of 6 px = 18 rows per font copy).
- The same 18-row font is stored **four times**, at sheet y = 0, 18, 38 and 56. The four
  copies are pixel-identical in shape; only the colour of the non-ink pixels differs
  (black for y = 0 and 38, the third palette colour for y = 18 and 56). Measured on the
  rendered main window: a glyph taken from any of the four copies matches the render
  exactly, so the choice of copy is not observable.
- Glyph order, by (row, column), column 0 at the left:

| Row | Columns and characters |
|---|---|
| 0 | 0–25 = `A`–`Z`; 26 = `"`; 27 = `@`; 28–29 unused; 30 = space |
| 1 | 0–9 = `0`–`9`; 10 = ellipsis; 11 = `.`; 12 = `:`; 13 = `(`; 14 = `)`; 15 = `-`; 16 = `'`; 17 = `!`; 18 = `_`; 19 = `+`; 20 = `\`; 21 = `/`; 22 = `[`; 23 = `]`; 24 = `^`; 25 = `&`; 26 = `%`; 27 = `,`; 28 = `=`; 29 = `$`; 30 = `#` |
| 2 | 0 = `Å`; 1 = `Ö`; 2 = `Ä`; 3 = `?`; 4 = `*`; 5–30 unused |

  The font has **no lowercase**: lowercase letters are rendered with the uppercase
  glyphs.
- Verified placements of the font in the main window:
  - Track title marquee: the A–Z glyph row (row 0) of the font, first cell at window
    (111, 27), 5 px per character cell, glyph height 6 px.
  - `kbps` numeral: the digit glyph row (row 1), first cell at window (111, 43), 5 px
    per cell.
  - `kHz` numeral: the digit glyph row (row 1), first cell at window (156, 43), 5 px
    per cell.
  - The static words `kbps` and `kHz` are part of `main.bmp`, not the font.
- The playlist editor does **not** use this font: its entries are drawn with a real
  font (see §4).

### 2.3 Numeric digits (numbers.bmp) and the nums_ex fallback

- `numbers.bmp` is 99 x 13 px, 8-bit palettised, RLE8-compressed, and holds **11 cells of
  9 x 13 px**: cells 0–9 are the digits `0`–`9`, cell 10 is a filler cell.
- The digit glyphs are a fixed 7-segment-style shape: the "on" segments are solid, the
  "off" segments are drawn as a 50 % dither, which is why a digit cell is not a solid
  block of one colour.
- The minus sign is a **5 x 1** sprite inside the sheet at (20, 6). A second 5 x 1 region
  at (9, 6) is the dithered "no minus" placeholder.
- Time display cell positions (verified, see §2.1): x = 48, 60, 78, 90 at y = 26. Note
  the 6 px gap between the minutes pair and the seconds pair (48/60 then 78/90): the
  colon that appears between them is part of `main.bmp`, not a sprite.
- `nums_ex.bmp` is an optional replacement. When it is present in the skin, the time
  display uses it instead of `numbers.bmp` and the digit layout changes (the digits move
  to different x positions and the cell pitch changes). See S17 for the fallback rule.
  The exact `nums_ex` cell positions are **not** in this spec.

### 2.4 Webamp deltas in the main window

| Webamp value | Measured on 2.95 | Comment |
|---|---|---|
| `cbuttons` transport buttons 23 px wide | 22 px wide, pitch 23 (buttons at x = 0, 23, 46, 69, 92) | 23 px covers one button plus the separator column |
| `MAIN_EJECT_BUTTON` 22 x 16 at (114, 0) | correct | matches, including the 1 px lower y (89) |
| `MAIN_NEXT_BUTTON_ACTIVE` 22 x 18 at (92, 18) | 22 x 18 at (92, 18) | consistent |
| `MAIN_PLAYING_INDICATOR` 9 x 9 at (0, 0) | correct | matches |
| `MAIN_NOT_WORKING_INDICATOR` (36, 0) / `MAIN_WORKING_INDICATOR` (39, 0) | reversed: the decoder-active sprite is **(36, 0, 3, 9)** drawn at (24, 28); (39, 0, 3, 9) is the idle one | webamp has the two swapped |
| `MAIN_CLUTTER_BAR_BUTTON_*_SELECTED` are 8 px wide, 7–8 px tall | the drawn letter sprites are 4 x 6 px (3 x 6 for letter 3), on a diagonal in the sheet | webamp's rectangles include transparent padding |
| `MAIN_CLUTTER_BAR_BACKGROUND` (304, 0, 8, 43) | correct | matches |
| `MAIN_TITLE_BAR` (27, 15) / `MAIN_TITLE_BAR_SELECTED` (27, 0) | correct | (27, 0) is the focused variant, (27, 15) the unfocused one |
| `NUMBERS` `NO_MINUS_SIGN` (9, 6, 5, 1) / `MINUS_SIGN` (20, 6, 5, 1) | correct | the minus is the solid 5 x 1 region at (20, 6) |
| `MAIN_OPTIONS/MINIMIZE/SHADE/CLOSE` drawn from 9 x 9 sprites at (0, 0), (9, 0), (0, 18), (18, 0) | 2.95 draws the window buttons from the **title bar body sprite itself** (rect 27,0,275,14 for the focused band): replacing the sheet cells at (0,0)-(27,36) changes nothing, replacing the corresponding cells inside the body sprite changes the on-screen buttons | the base skin carries the window-button graphics inside its title bar sprite; 2.95 does not use the 9 x 9 cells at x = 0-26 (see open question 1) |
| `MAIN_VOLUME_BACKGROUND` (0, 0, 68, 420) | the sheet is 68 x 433: 28 frames of 15 rows (y = 0…419) plus a 13-row thumb strip (y = 420…432) | frame count matches; see open questions |
| `MAIN_VOLUME_THUMB` (15, 422, 14, 11) | the thumb strip begins at y = 420, not 422 | off by 2 |
| `MAIN_BALANCE_BACKGROUND` (9, 0, 38, 420) | 28 frames of 15 rows, same as volume | consistent |
| `MAIN_POSITION_SLIDER_THUMB` (248, 0, 29, 10) | the thumb graphic in the sheet starts at x = 246–248; the measured best fit is 2 px to the left of webamp's value | small offset, not resolved |
| `#kbps`, `#khz` drawn with `numbers.bmp` | drawn with **text.bmp** (5 x 6 glyphs) | webamp uses the wrong font here |

---

## 3. Equalizer window (275 x 116)

Sheets used: `eqmain.bmp`, `eq_ex.bmp`.

| Element | Sheet | Rect | At | State |
|---|---|---|---|---|
| Window background | eqmain.bmp | 0,0,275,116 | 0,0 | verified (as the base layer) |
| Title bar, window not focused | eqmain.bmp | 0,149,275,14 | 0,0 | verified |
| Title bar, window focused | eqmain.bmp | 0,134,275,14 | 0,0 | provisional |
| Shade-mode title bar, not focused | eq_ex.bmp | 0,15,275,14 | 0,0 | provisional |
| Shade-mode title bar, focused | eq_ex.bmp | 0,0,275,14 | 0,0 | provisional |
| On button | eqmain.bmp | 10,119,26,12 | 14,18 | verified |
| On button, engaged | eqmain.bmp | 69,119,26,12 | 14,18 | provisional |
| Auto button | eqmain.bmp | 36,119,32,12 | 40,18 | verified |
| Auto button, engaged | eqmain.bmp | 95,119,32,12 | 40,18 | provisional |
| Presets button | eqmain.bmp | 224,164,44,12 | 217,18 | verified |
| Presets button, engaged | eqmain.bmp | 224,176,44,12 | 217,18 | provisional |
| Response graph background | eqmain.bmp | 0,294,113,19 | 86,17 | provisional (composite with the drawn curve) |
| Slider track area | eqmain.bmp | 13,164,209,129 | 13,17 | provisional |
| Slider thumb | eqmain.bmp | 13,229,11,11 | preamp (21,38), bands (78,38)+18k | verified for the preamp at 0 dB; the per-band track graphics are tiled |
| Band label row | — | — | — | see below |

Window-focus note: as with the main window, `eqmain.bmp` carries a focused and an
unfocused title bar as a 14-row pair. Which of (0,134) and (0,149) is drawn when the
window has focus was **not** resolved in this session; the render observed in the capture
set used (0,149). Treat the pair as "the two 14-row title bars at y = 134 and y = 149"
and resolve the focus mapping before use.

Band slider positions (window-relative, from the rendered window and the free-source
layout, consistent with the printed labels):

| Slider | x | y |
|---|---|---|
| Preamp | 21 | 38 |
| 60 Hz | 78 | 38 |
| 170 Hz | 96 | 38 |
| 310 Hz | 114 | 38 |
| 600 Hz | 132 | 38 |
| 1 kHz | 150 | 38 |
| 3 kHz | 168 | 38 |
| 6 kHz | 186 | 38 |
| 12 kHz | 204 | 38 |
| 14 kHz | 222 | 38 |
| 16 kHz | 240 | 38 |

Band spacing is 18 px. The printed band labels in the base skin are, left to right:
`60`, `170`, `310`, `600`, `1K`, `3K`, `6K`, `12K`, `14K`, `16K`; the axis labels are
`+12db`, `+0db`, `-12db`. These are part of the sheet, not sprites.

---

## 4. Playlist editor (275 x 116)

Sheets used: `pledit.bmp`, plus a real font for the list text.

| Element | Sheet | Rect | At | State |
|---|---|---|---|---|
| Bottom bar, left block | pledit.bmp | 0,72,125,38 | 0,78 | verified |
| Bottom bar, right block | pledit.bmp | 126,72,150,38 | 125,78 | verified (small residual) |
| Top-left corner | pledit.bmp | 0,0,25,20 | 0,0 | provisional |
| Title bar strip | pledit.bmp | 26,0,100,20 | 25,0 | provisional |
| Top tile | pledit.bmp | 127,0,25,20 | tiled from x = 125 | provisional |
| Top-right corner | pledit.bmp | 153,0,25,20 | 250,0 | provisional |
| List background / side rails | pledit.bmp | 0,42,12,29 and 31,42,20,29 | tiled | provisional |
| Close button | pledit.bmp | 52,42,9,9 | 264,3 | provisional |
| Shade button | pledit.bmp | 62,42,9,9 | 254,3 | provisional |
| Shade title bar | pledit.bmp | 72,42,25,14 etc. | tiled | provisional |
| Add / Remove / Select / Misc buttons | pledit.bmp | 0,111 / 54,111 / 104,111 / 154,111, each 22 x 18 | 14,86 / 43,86 / 72,86 / 101,86 | provisional |
| List options button | pledit.bmp | 204,111,22,18 | 231,86 | provisional |

The playlist's own title-bar layout is assembled from tiles rather than one sprite; the
free-source layout model (a 25 px left corner, a 100 px title strip, a tiled middle, a
25 px right corner, then a 38 px bottom bar split 125/150) reproduces the rendered window
in the parts that were checked, but this session did **not** verify the tile boundaries
pixel-exactly. Treat the whole top half as provisional.

Text: the playlist list is drawn with a **real font**, not the bitmap font of §2.2. The
colour comes from `pledit.txt`: the entry text rendered in the base skin is exactly the
`Normal` colour `#00FF00`, with a darker shade of the same hue behind it.

Selection: with a row selected, the selected row's background is drawn in the
`SelectedBG` colour from `pledit.txt` over the list area (verified by diffing the
playlist capture with and without a selection; the changed pixels are confined to the
selected row band).

---

## 5. Sheet dimensions and encodings (base skin)

Measured directly from the base skin files. This is the set of sheets a loader must
accept; sizes are the base skin's, not a requirement of the format.

| Sheet | Size (px) | Bit depth | Encoding |
|---|---|---|---|
| `main.bmp` | 275 x 116 | 8 | RLE8 |
| `titlebar.bmp` | 344 x 87 | 8 | uncompressed |
| `cbuttons.bmp` | 136 x 36 | 8 | RLE8 |
| `shufrep.bmp` | 92 x 85 | 8 | RLE8 |
| `posbar.bmp` | 307 x 10 | 8 | RLE8 |
| `volume.bmp` | 68 x 433 | 8 | uncompressed |
| `balance.bmp` | 68 x 433 | 8 | RLE8 |
| `monoster.bmp` | 58 x 24 | 8 | RLE8 |
| `playpaus.bmp` | 42 x 9 | 8 | uncompressed |
| `numbers.bmp` | 99 x 13 | 8 | RLE8 |
| `text.bmp` | 155 x 74 | 8 | RLE8 |
| `pledit.bmp` | 280 x 186 | 8 | uncompressed |
| `eqmain.bmp` | 275 x 315 | 8 | RLE8 |
| `eq_ex.bmp` | 275 x 82 | 8 | uncompressed |
| `gen.bmp` | 194 x 109 | 8 | RLE8 |
| `genex.bmp` | 130 x 75 | 8 | uncompressed |
| `mb.bmp` | 234 x 119 | 8 | uncompressed |
| `video.bmp` | 234 x 119 | 8 | uncompressed |

**A loader must support RLE8-compressed BMPs**: the base skin mixes RLE8 and
uncompressed sheets, so an uncompressed-only loader cannot render the original skin.
(Confirmed by decoding every sheet above both ways and comparing against an independent
decoder: identical results.)

`gen.bmp` / `genex.bmp` are the generic-plugin window sheets; their sprite table belongs
to S11 and is **not** in this spec.

---

## 6. Compositing: the sheets are drawn opaquely

Verified: 2.95 copies the classic sheets onto the window **opaquely**. There is **no**
per-sheet transparency colour key. Tested by replacing a region that is drawn on screen —
the play button in `cbuttons.bmp` and a band of the title bar in `titlebar.bmp` — with the
colour a key would use, and separately with palette index 0: in every case the replacement
colour appears in the render unchanged. The clutter-bar block of `titlebar.bmp`
(304,0,8,43) is reproduced in the window pixel for pixel, filler colours included, and the
rendered main window contains no pixel of the pale cyan `(0,198,255)` that several sheets
use as filler.

Consequence for a loader: nothing has to be masked. Each sprite carries its own
background, and the parts of a sheet that are not blitted (the filler colour around the
sprites) are never read. The pale cyan is filler in unblitted areas, not a key.

---

## 7. Open questions

1. **Window-button pressed states.** §2.4 and §6 establish that the buttons are part of
   the title bar body sprite, but the pressed variant of each of the four buttons was not
   captured; which pixels change while a button is held is open.
2. **Which title bar is "focused".** For the main window the focused variant was
   identified by text brightness ((27, 0) is white text, (27, 15) is grey). For the EQ the
   pair is (0, 134) and (0, 149) and the mapping was not resolved. The playlist's
   equivalent pair is not listed at all.
3. **Transparency.** No sheet in the base skin relies on a colour key (§6); whether any
   third-party skin does, and what 2.95 does with it, was not found.
4. **Volume and balance frames.** The sheets have 28 frames of 15 rows plus a 13-row
   thumb strip, but which frame corresponds to which slider value, and which 13 of the 15
   rows are drawn, was not determined. The drawn track is 68 x 13 at (107, 57) for volume
   and 38 x 13 at (177, 57) for balance. At value 0 the volume thumb's left edge sits at
   window x ≈ 108 (measured from the render; the sprite rect is 14 px wide per the free
   source, so the exact blit origin is still open).
5. **Position-bar thumb rect.** The measured best fit for the thumb sprite is 2 px left of
   the free-source value; a skin with a distinctive thumb would settle the exact rect.
6. **`nums_ex.bmp` cell layout** when it is present (positions, pitch, minus-sign cell).
7. **Equalizer internals.** The slider track tiling, the thumb rect per band, the graph
   background rect and the exact `eq_ex.bmp` shade layout are provisional.
8. **Playlist top half.** Tile boundaries and the title-bar button rects are provisional.
9. **`gen.bmp` / `genex.bmp`** sprite tables are out of scope here (S11).
10. The `Winamp MB` and `Winamp Video` window sheets (`mb.bmp`, `video.bmp`) were not
    analysed.
