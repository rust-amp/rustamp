---
Spec ID: S-01
Title: Window geometry, sprite rectangles and pixel layout deltas
Sources consulted (category only): dirty room: binary behaviour capture; dirty room: sandboxed binary behaviour capture (rounds 3, 3b); dirty room: shipped skin assets (test input); dirty room: synthetic test skin (own bitmap, round 2); free: webamp (cross-check only)
Author agent: R-agent (dirty room, capture session 2026-10-14); dirty-room capture agents (round-2 sessions 2026-10-15); dirty-room capture agent (round-3 sandbox session 2026-10-16); dirty-room capture agent (round-3b sandbox session 2026-10-16)
Auditor: Spec Auditor (dirty room)
Audit date: 2026-10-14; round-2 audits 2026-10-15 (three passes); round-3 audit 2026-10-16 (one pass), verdict PASS WITH CORRECTIONS; round-3b audit 2026-10-16 (one pass), verdict PASS WITH CORRECTIONS
Export run (round 3): sandbox measurement run 20261016-003014 (Windows Sandbox capture host; frozen job s01r2-open.json, 136/136 steps ok)
Export run (round 3b): sandbox measurement run 20261016-013304 (Windows Sandbox capture host; frozen job s01r2-right-edges.json, 117/117 steps ok)
Export run (held states): sandbox measurement run 20261016-050228 (Windows Sandbox capture host; frozen job held-states.json, 69/69 steps ok)
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561, extracted from winamp295.exe SHA-256 ef1423999cbefd4f2a3a98acee2b194a4260268cb4517abda9f12963e866b7e5)
Status: audited
---

# S01 — Window geometry, sprite rectangles and pixel layout deltas

Scope: the exact pixel geometry of the three classic windows and of every classic skin
sheet, and where each sprite is drawn. Everything marked **verified** was measured from
renders of the real 2.95 binary or read from the skin files. Everything marked
**provisional** is a candidate that this session did not confirm against 2.95; treat
provisional rows as unconfirmed and do not rely on them. Rows and passages added by the
round-2 sessions of 2026-10-15 are marked **(round 2)**; rows and passages added or
changed by the round-3 session of 2026-10-16 are marked **(round 3)**; rows and
passages added by the round-3b session of 2026-10-16 are marked **(round 3b)**; rows and
passages added or changed by the held-states run of 2026-10-16 are marked **(held-states
run)**.

Measurement method (for provenance): a screenshot of a window is compared pixel by pixel
against a rectangle of a skin sheet; a rectangle is accepted only when the mean
per-channel difference over the rectangle is exactly zero. Positions are window-relative
(x to the right, y down, origin at the window's client top-left). The round-2 measurements
use the same comparison and add one controlled input for the playlist tile pitch: a
self-made synthetic skin whose `PLEDIT.BMP` gives every sheet column a distinct colour, so
a render column names the sheet column it came from (see §4.1).

(round 3) The round-3 facts come from the export run named in the header (a Windows
Sandbox capture host, frozen job `s01r2-open.json`, 136/136 steps ok). On that host every
capture asserts the 275x116 landmark (invariant region 0,49,275,67, zero differing
pixels), so its coordinate space is comparable with the recorded evidence. The guest was
launched with the 60-entry reference playlist; it lists all 60 entries although their
files are not mapped into the guest, so the listed content is the playlist, not the media.
The window settles 15.1 s after a playlist launch, and **13** independent fresh-launch
state captures are byte-identical to one another (and the smoke twins are byte-identical
and 0-pixel-identical to the landmark region). The round-3 host recorded `dpi=144 (150%)`,
a 3050x1668 screen and a per-monitor-v2 process; the round-2 sessions recorded `240 (250%)`
on their own machine. These are **different machines**, so a row measured in one states
which environment it is from, and no single DPI figure applies to the whole spec.

(round 3b) The round-3b facts come from the round-3b export run named in the header
(a Windows Sandbox capture host, frozen job `s01r2-right-edges.json`, 117/117 steps ok;
landmark gate 0,49,275,67, 0 of 18425 pixels differing). The host is the same environment
as the round-3 export run (`dpi=144 (150%)`, a 3050x1668 screen, a per-monitor-v2
process), so its coordinate space is comparable with the recorded evidence. The guest was
launched with the same 60-entry reference playlist (its files are not mapped into the
guest); after the settle the playlist view is at the bottom, showing rows 57–60. The
smoke twins are byte-identical (`43c90f1e335b38b7…`) and differ by 0 of 31900 px. The run
reproduces its own fresh state **21** times under one SHA-256 (`c0ee422e1144…`), its
selection state **14** times (`baefcc984dc5…`) and the REM-removal state **10** times
(`6ac754b2c11a…`), so a "no change" result in this round is a byte identity, not a
threshold.

(held-states run) The held-state facts below come from the held-states export run named in
the header (a Windows Sandbox capture host, frozen job `held-states.json`, 69/69 steps ok;
landmark gate 0,49,275,67, 0 of 18425 pixels differing; the same environment as the round-3
and round-3b runs, `dpi=144 (150%)`, a 3050x1668 screen, a per-monitor-v2 process). The
guest was launched with the same 60-entry reference playlist. These captures settle the
equalizer's four held rects and the Presets held question, and the volume/balance/preamp
laws of §3.1 are refined by them; the ones that touch this spec are restated below and the
still-open bounds stay open. Rows and passages added or changed by this run are marked
**(held-states run)**.
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
| Title bar, window focused | eqmain.bmp | 0,134,275,14 | 0,0 | verified (round 2) |
| Shade-mode title bar, not focused | eq_ex.bmp | 0,15,275,14 | 0,0 | provisional |
| Shade-mode title bar, focused | eq_ex.bmp | 0,0,275,14 | 0,0 | provisional |
| On button | eqmain.bmp | 10,119,25,12 | 14,18 | verified (round 2; 25 px wide, not 26) |
| On button, engaged | eqmain.bmp | 69,119,25,12 | 14,18 | verified (round 2) |
| On button, held, not engaged | eqmain.bmp | 128,119,25,12 | 14,18 | verified (round 2; (held-states run) confirms (14,18,25,12), 300/300 px) |
| On button, held and engaged | eqmain.bmp | 187,119,25,12 | 14,18 | verified (round 2; (held-states run) confirms (14,18,25,12), 300/300 px) |
| Auto button | eqmain.bmp | 36,119,32,12 | 40,18 | verified (round 2) |
| Auto button, engaged | eqmain.bmp | 95,119,32,12 | 40,18 | verified (round 2) |
| Auto button, held, not engaged | eqmain.bmp | 153,119,33,12 | 39,18 | verified (round 2; (held-states run) corrects the held window footprint to (39,18,33,12)) |
| Auto button, held and engaged | eqmain.bmp | 212,119,33,12 | 39,18 | verified (round 2; (held-states run) corrects the held window footprint to (39,18,33,12)) |
| Presets button | eqmain.bmp | 224,164,44,12 | 217,18 | verified (round 2; (held-states run) 528/528 px of the 44x12 footprint change on a lone down and back on release; reproduces `s01r2_eq_presets_pressed_eq.png` at 0 px; the 534-px whole-window figure is a post-release difference = 528 + 6 Auto cell) |
| Presets button, engaged | eqmain.bmp | 224,176,44,12 | 217,18 | verified (round 2) |
| Response graph background | eqmain.bmp | 0,294,113,19 | 86,17 | verified with an explained residual (round 2; see §3.1) |
| Slider track area | eqmain.bmp | 13,164,209,129 | 13,17 | provisional; refuted as a single blit (round 2) |
| Slider thumb, resting | eqmain.bmp | 0,164,11,11 | preamp (22,63) and bands (79,63)+18k at the flat value | verified (round 2) |
| Slider thumb, dragged | eqmain.bmp | 0,176,11,11 | follows the drag (see §3.1) | verified (round 2) |
| Graph colour scale | eqmain.bmp | 115,294,1,19 | — (never blitted) | verified as a colour scale only (round 2; see §3.1) |
| Band label row | — | — | — | see below |

The On and Auto rows above are the four held states the earlier S01 lacked: held
not-engaged and held-and-engaged are separate sprites per button, and the four rects are
all drawn (round 2). The On button's drawn region is **25 px** wide, not S01's earlier 26
px; the 26th sheet column is the separator before the Auto button and is not drawn at the
On button's window position. Auto is 32 px wide for the resting/engaged rows, and its held
footprint is (39,18,33,12): the held rows extend one sheet column to the left, into the
separator before the Auto body, whose 12 px at x=39 change under a hold. (Round 2;
(held-states run) corrects the held Auto window footprint from (40,18,32,12) to
(39,18,33,12).)

The slider thumb row is corrected by round 2: the drawn thumb is `eqmain.bmp`
**(0,164,11,11)**. S01's earlier `(13,229,11,11)` is **not** a thumb; it matches the empty
slider-track cell (the untiled track tile) at the flat setting and was mislabelled.

Window-focus note (round 2): `eqmain.bmp` carries the focused and unfocused title bar as
a 14-row pair at y = 134 and y = 149. The focused variant is **(0,134)** and the unfocused
variant is **(0,149)**; both are verified. The pair is the window's own active/inactive
render, driven by the window's activation messages. S01's earlier "which of the two is
drawn when the window has focus was not resolved" is closed by round 2.

Band slider positions (window-relative, from the rendered window and the free-source
layout, consistent with the printed labels). x is the **thumb top-left**; y is the thumb
top-left in the flat setting:

| Slider | x | y |
|---|---|---|
| Preamp | 22 | 63 |
| 60 Hz | 79 | 63 |
| 170 Hz | 97 | 63 |
| 310 Hz | 115 | 63 |
| 600 Hz | 133 | 63 |
| 1 kHz | 151 | 63 |
| 3 kHz | 169 | 63 |
| 6 kHz | 187 | 63 |
| 12 kHz | 205 | 63 |
| 14 kHz | 223 | 63 |
| 16 kHz | 241 | 63 |

Band spacing is 18 px. y = 63 is the thumb top-left at the flat setting (saved value 31);
at the top of travel (saved value 0) it is 38 and at the bottom (saved value 63) it is 89
(§3.1). Round 2 corrected the x column: S01's earlier preamp x = 21 and bands
x = 78 + 18k are the **track-slot** origin, one pixel left of the drawn thumb's top-left;
the drawn columns are 22 and 79 + 18k. The printed band labels in the base skin are, left
to right: `60`, `170`, `310`, `600`, `1K`, `3K`, `6K`, `12K`, `14K`, `16K`; the axis
labels are `+12db`, `+0db`, `-12db`. These are part of the sheet, not sprites, and their
centres (y ≈ 39, 68 and 98) remain printed-sheet estimates.

### 3.1 Equalizer interior — round-2 measurements

The round-2 sessions (2026-10-15) re-measured the equalizer interior against the base
skin's `eqmain.bmp` and produced the corrections marked "(round 2)" in §3. The facts that
are not sprite rows are collected here.

- **Slider origin.** The column origin is the thumb sprite's top-left pixel: the preamp
  column is window x = 22 and the ten bands are x = 79 + 18k (18 px pitch). The earlier
  S01 values x = 21 and x = 78 + 18k are the track-slot origin, one pixel to the left,
  and are not the drawn thumb's position.
- **Groove.** The vertical groove under a slider matches sheet column 14 (and identically
  column 15) of `eqmain.bmp`, at sheet y = window y + 191. Verified for the band columns
  and for the preamp column, excluding the rows the thumb covers.
- **Thumb-y mapping.** The resting thumb's top-left y is 38 at saved value 0 (top of
  travel), 63 at saved value 31 (flat, 0 dB) and 89 at saved value 63 (bottom of travel).
  The mapping is linear; the ≈0.81 px-per-unit step follows from these three points.
- **Dragged thumb.** While a slider is dragged, the thumb is drawn from `eqmain.bmp`
  (0,176,11,11); it reverts to the resting sprite (0,164,11,11) on release. The dragged
  sprite's top-left follows the drag (measured offset: the drag target's y minus 6).
- **Graph colour scale.** `eqmain.bmp` (115,294,1,19) is a vertical colour scale — 19
  rows carrying 12 distinct colours — and is **never blitted as a sprite** (it matches no
  graph column in any captured state). Its rows are the colours the response curve and
  the slider body are drawn in: the flat curve is rows 7–9, the minimum-value curve row 0
  and the maximum-value curve row 18; the slider body at saved value 0 equals row 0 and
  at saved value 63 equals row 18. This strip may be stated only as a colour scale; a
  causal "used as the source" reading is not asserted. Which graph feature uses which
  ramp row is S07's.
- **Graph background.** The row `eqmain.bmp` (0,294,113,19) at window (86,17) is verified
  with an explained residual: the ignored pixels are exactly the pixels where the
  flat-state render differs from the sheet (the flat response curve plus the horizontal
  reference line), taken from the flat capture. A non-flat state has a different residual
  set, so the mask is the flat-state one.
- **Slider body colouring.** Below the thumb, a band's slider body is coloured by a
  band-value scale (red at the top of travel through orange and yellow to green at the
  bottom), and the fill is constant from the thumb to the bottom of the track. The
  endpoints equal the colour scale's row 0 (value 0) and row 18 (value 63); the exact
  value-to-colour function and the full track decomposition are S07's.
- **Saved settings.** The equalizer's saved state lives in `winamp.ini`: the keys
  `preamp` (the preamp value, 0–63), `eq_data` (ten comma-separated band values, 0–63),
  `autoload_eq` and `use_eq`. The setter used to produce the captured values is a direct
  edit of that file while the binary is not running, followed by a restart. Writing
  `preamp`/`eq_data` at 0, 31 and 63 reproduces the captured minimum, flat and maximum
  states with zero differing pixels; `autoload_eq` toggles exactly the AUTO indicator
  cell at client (44–46, 22–23). The ON indicator's cell at client (18–20, 22–23) is
  associated with `use_eq` in the same file, but that link was not independently
  re-verified (provisional; carried as an open item **(round 3)**, §7.22).
- **(round 3) Fresh-launch reproducibility.** A fresh launch with `eq_data=31` (x10) and
  `preamp=31` renders the flat equalizer identical to the recorded flat capture
  `s01r2d_o8_flat_eq.png` except for **6 of 31900 pixels**, all inside the AUTO indicator
  cell at client (44–46, 22–23) — the `autoload_eq` marker of the bullet above, i.e. a
  state difference, not a geometry difference.
- **(round 3) Preamp drag — negative result.** A fresh real drag of the preamp thumb from
  its flat centre (27,68) to above the track top (27,30), clamped to maximum, does **not**
  reproduce the recorded real-drag capture `s01r2d_realinput_eq_ondrag_after_eq.png`:
  **95 of 31900 pixels** differ, all in the preamp column (bounds (18,22)-(32,50)). The
  cause is not established; this is a provenance caveat on that one recorded capture and
  is carried as an open item (§7.20), not as a reproduction. Both drags clamp to the
  maximum (the recorded drag target was (27,45)), so the residual is not a different
  final value.

- **(held-states run) Equalizer held rects.** Posted as a lone `WM_LBUTTONDOWN` (0x201)
  with no `WM_LBUTTONUP`, so the control stays held while the capture runs (the earlier
  runs released the button in a `finally` before capturing; see the method note below):
  the On button's held window footprint is **(14,18,25,12)**, 300/300 px changed,
  measured in both the engaged and the not-engaged held state, and it matches both
  recorded round-2 held captures at 0 px inside the footprint; the Auto button's held
  window footprint is **(39,18,33,12)**, 395/396 px changed, in both engagement
  states. The Auto value supersedes the round-2 working value (40,18,32,12), which
  was one column short on the left (12 px at x=39); the render is identical to the
  recorded captures at 0 px, so it was a width error in the footprint, not a different
  sprite. Engagement contributes 6 px only: On at (20,24)-(22,25) and Auto at
  (45,24)-(47,25). Both held rects reproduce the recorded round-2 held captures at
  0 px inside the footprint.
- **(held-states run) The 300 / 401 / 534-px figures were post-release differences,
  not held sprites.** Decoded from the same comparison (recorded held baseline minus a
  frame taken after the release):
  - On held: 300 px, bounds (14,18)-(38,29) = the On sprite absent from the post-release
    frame; nothing else changes.
  - Auto held: 401 px, bounds (18,18)-(71,29) = 395 (the Auto sprite at the corrected
    33-px width) + 6 (the On indicator cell at the round-2 position).
  - Presets held: 534 px, bounds (44,18)-(260,29) = 528 (the real held sprite, a 44x12
    footprint at (217,18)) + 6 (the Auto cell). The recorded capture
    `s01r2_eq_presets_pressed_eq.png` is a genuine held sprite: 528 of 528 px of the
    footprint change on a lone down and the same 528 change back on the release, and the
    held run reproduces it at 0 px. Against its own resting equalizer the earlier
    run differs by 0 px inside (217,18,44,12).
  The release-in-`finally` behaviour (method note below) explains all three: they
  were measured after the button was already up.
- **(held-states run) Calibration.** A whole-window verdict against a fresh state is
  dominated by a session floor of 1435 px confined to rows 4-11 (the title band); rows
  30-115 differ by exactly 0 in every held-versus-baseline comparison. A whole-window
  count of roughly 1400-2000 px is therefore session noise, not a sprite: read the
  bounding box and its row profile, never the total. (The earlier prediction of
  1700-2000 px folded the round-2 sessions' own 6-px indicator-cell difference into
  the floor.)
- **(held-states run) Why the earlier claim could not be tested.** A real-mouse held
  state cannot be captured with this harness: `Invoke-RealMouseAction` releases the
  button in a `finally` before `Invoke-Capture.ps1` captures, so any hold-time effect
  (a held sprite, a slider's `VOLUME: n%` hint) is already gone when the frame is
  taken; two earlier runs failed on exactly this. The fix was to post a lone
  `WM_LBUTTONDOWN` (0x201) with no `WM_LBUTTONUP` through the `send-message`
  primitive, leaving the control held while the capture runs. Steps and captures:
  `on_down_*`/`on_held_*` (00-09), `auto_down_*`/`auto_held_*` (10-19) and
  `presets_down`/`presets_held`/`presets_up` (64-66).
---

## 4. Playlist editor (275 x 116)

Sheets used: `pledit.bmp`, plus a real font for the list text.

| Element | Sheet | Rect | At | State |
|---|---|---|---|---|
| Bottom bar, left block | pledit.bmp | 0,72,125,38 | 0,78 | verified |
| Bottom bar, right block | pledit.bmp | 126,72,150,38 | 125,78 | verified (small residual) |
| Top-left corner | pledit.bmp | 0,0,25,20 | 0,0 | verified (round 2) |
| Title bar strip (100 px) | pledit.bmp | 26,y,100,20 | x = 87 only, blitted once | verified (round 2) |
| Top tile | pledit.bmp | 127,y,25,20 | tiled at 25 px pitch, phase anchored (see §4.1) | verified (round 2) |
| Top-right corner | pledit.bmp | 153,0,25,20 | 250,0 | verified (round 2) |
| List background / side rails | pledit.bmp | 0,42,12,29 and 31,42,20,29 | tiled | left rail verified (round 2); right rail provisional |
| Close button | pledit.bmp | 52,42,9,9 | 264,3 | provisional |
| Shade button | pledit.bmp | 62,42,9,9 | 254,3 | provisional |
| Shade title bar | pledit.bmp | 72,42,25,14 etc. | tiled | provisional |
| Add / Remove / Select / Misc buttons | pledit.bmp | y = 86, each ≈ 22–25 px wide | window y 92–109, x ≈ 11 / 43 / 72 / 101 | verified (round 2; refutes the y = 111 rows) |
| List options button | pledit.bmp | 229,86,24,18 | 228,92 | verified (round 2) |
| Scroll handle | pledit.bmp | 52,53,7,17 | (260, y) with y = 20 / 40 / 60 | verified (round 2) |
| Scroll arrows (up above down) | pledit.bmp | 261,72,8,10 | 260,78 (fixed) | verified (round 2); no pressed sprite |

The playlist's own title-bar layout is assembled from tiles rather than one sprite. Round
2 measured the composition on the base skin and on a self-made periodic-tile skin (see
§4.1): a 25 px left corner, a tiled band, the 100 px strip blitted once, a second tiled
band and a 25 px right corner, above a 38 px bottom bar split 125/150. The tile
boundaries and the strip position are now measured, so the earlier "treat the whole top
half as provisional" no longer applies to them; the earlier "tiled from x = 125" reading
is refuted.

Text: the playlist list is drawn with a **real font**, not the bitmap font of §2.2. The
colour comes from `pledit.txt`: the entry text rendered in the base skin is exactly the
`Normal` colour `#00FF00`, with a darker shade of the same hue behind it.

Selection: with a row selected, the selected row's background is drawn in the
`SelectedBG` colour from `pledit.txt` over the list area (verified by diffing the
playlist capture with and without a selection; the changed pixels are confined to the
selected row band).

### 4.1 Playlist upper half — round-2 measurements

The round-2 sessions measured the playlist's upper half against `pledit.bmp`, and the
tile pitch against a **self-made synthetic skin** used as a controlled input: a skin
whose `PLEDIT.BMP` gives every sheet column a distinct colour, so each render column
names the sheet column it came from. The skin is our own bitmap, not reference material.

- **Title-band composition and tile pitch.** The tile is **25 px** wide, source
  `PLEDIT.BMP` (127,y,25,20). Reading the title band left to right: the left corner
  (0,y,25,20) at render x 0–24; a tiled band at render x 25–86; the 100 px strip
  (26,y,100,20) blitted exactly once at render x 87–186; a second tiled band at render
  x 187–249; the right corner (153,y,25,20) at render x 250–274. Each tiled band's first
  piece is partial — 12 px on the left band, 13 px on the right band — so the tiling is
  anchored to the adjacent fixed piece (the strip at 87, the right corner at 250) and the
  leftover at the band's outer end is a fresh tile from the band edge. This is not a
  uniform tiling anchored at the window edge (which would put boundaries at 25, 50, 75).
  S01's "tiled from x = 125" is refuted, as is its "strip (26,0,100,20) at (25,0)" (the
  strip is at 87 only).
- **Focus variants.** The focused title band draws the sheet rows 0–19 and the unfocused
  band the rows 21–40 (corner `(0,y,25,20)`, tile `(127,y,25,20)`, strip
  `(26,y,100,20)` at the same positions in both). The pair is the window's own
  active/inactive render, driven by the window's activation messages.
- **Side rails.** The left rail (0,42,12,29) tiles from window y = 20. The right rail
  (31,42,20,29) does not tile cleanly from y = 20 in the captures and stays provisional.
- **Scrollbar.** The handle is `PLEDIT.BMP` (52,53,7,17), drawn at (260, y): y = 20 at the
  scroll top, 40 at the middle and 60 at the bottom for the captured list, which shows
  four rows at 13 px pitch, i.e. 40 px of travel. A dragged handle is the same sprite
  moved (no separate dragged sprite). An offset rect (53,55,7,16) at (261, y+2) also
  matches because the sprite's border is uniform; the two are the same drawn handle. The
  arrow pair is `PLEDIT.BMP` (261,72,8,10), fixed at (260,78) in every captured state,
  and has **no pressed sprite** (a held arrow differs only in an unrelated text element).
  The arrow step size was **open** in round 2 and is **measured in round 3** as one page of
  four rows per click (round 3; see §4.2 and §7.14).
- **Bottom-bar buttons.** The button sprites come from `PLEDIT.BMP` **y = 86** and are
  drawn at **window y = 92–109**. This refutes S01's y = 111 source rows and its window
  y = 86 positions. The x origins are ambiguous by ±3 px because the leading columns are
  uniform; the drawn buttons are at window x ≈ 11, 43, 72, 101 and 228 (LIST OPTS is at
  sheet x = 229, one pixel right of the others' alignment), widths ≈ 22–25 px. No
  bottom-bar button draws a pressed sprite, under real holds as well as posted ones.
- **Bottom-bar hit areas.** Established by probing with posted client-relative clicks at
  window y ≈ 92: ADD ≈ x 14–35, MISC ≈ x 102–121 and LIST OPTS ≈ x 232–251. ADD and
  LIST OPTS open a file dialog and MISC opens a popup menu; the probe's positive points
  are the dialog/menu windows it opened. **(round 3)** The three posted positives
  reproduce: (24,92) opens the ADD `#32770` file dialog, (112,92) the MISC `#32768` popup
  menu and (242,92) the LIST OPTS `#32770` file dialog. REM and SEL **left edges** are now
  measured (round 3, §4.2); their **right edges** stay open (§7.12). Whether a **real**
  click fires these buttons was **open** in round 2 and is **resolved in round 3**
  (§4.2, §7.13).

### 4.2 Playlist editor — round-3 measurements

The round-3 sandbox session (2026-10-16) re-measured the playlist editor's bottom-bar
interaction with posted and with real input; every capture asserts the 275x116 landmark
(the export run named in the header). The facts below are **(round 3)**, except where
tagged **(round 3b)**; the round-3b facts come from the round-3b export run named in the
header, on the same host environment.

- **The `WinampVis` child window at y = 92.** At window y = 92 the client region
  **x 52–122** resolves to a child window of class `WinampVis` whose root ancestor is the
  playlist editor. The **left edge** is bracketed to **{51, 52}** (x=50 is `Winamp PE`,
  x=52 is `WinampVis`); the **right edge** is bracketed only to **{123, 124, 125}** —
  x=123 and x=125 were not probed and x=124 resolves to the `#32768` MISC popup menu that
  an earlier posted MISC click left open, so the child window may extend under it. x ≥ 126
  is the playlist window itself. The measured coverage is therefore **52–122**; the exact
  right edge stays open (§7.17). **(round 3b)** The right edge is now measured: in a
  menu-free launch (one fresh launch, no MISC click, a descending 1-px grid at y=92 over
  x = 126…122) the pre-flight guard `class=` changes **between x=124 and x=123** — 126/125/124
  read `Winamp PE` (hitIsTarget=True), 123/122 read `WinampVis` (hitIsTarget=False), and
  no guard returned `#32768`. All five captures are byte-identical to the fresh state
  (`c0ee422e1144…`, 10765 bytes, 0 px), so no click in the group changed the playlist
  window. The coverage at y=92 is therefore **x 52–123**, and the export run's `#32768`
  reading at x=124 is thereby confirmed as the masking MISC menu, not the child window.
  (Scope of that 0-px statement: only the MISC `#32768` menu is absent from a playlist
  `PrintWindow`; the REM/SEL click-#1 dropdown **is** rendered into it — §7.21.) The
  **left** edge (51 vs 52) is untouched by that launch and stays open (§7.17).
- **Posted clicks fire the bottom bar.** A posted client-relative click at (24,92) opens
  the ADD `#32770` file dialog, at (112,92) the MISC `#32768` popup menu and at (242,92)
  the LIST OPTS `#32770` file dialog. Which of the two clicks of a positive probe fired is
  not separable; the posted path is proven.
- **Real clicks fire the same five buttons.** With real input (`SetCursorPos` plus the
  legacy `mouse_event`), ADD at (23,92) and at (23,100) opens the `#32770` file dialog;
  MISC at (110,92) and (110,100) opens the `#32768` menu; LIST OPTS at (242,92) opens the
  `#32770` file dialog; REM at (46,92) removes the selected entry (the list renumbers;
  4863 differing pixels against the selection state) and SEL at (74,92) selects all
  visible rows (10673 differing pixels). The **second** real click is the one that fires —
  the first real click on a non-foreground window is consumed by activation. **On REM and
  SEL the first real click additionally opens a four-item dropdown menu** over the bottom
  bar (REM: `REM Misc / REM ALL / CROP / REM SEL`; SEL: `INV SEL / SEL ZERO / SEL ALL /
  …`); the posted grid clicks open no such menu. The nature of that menu is open (§7.21).
  **(round 3b)** The same design at y=100 for REM, SEL and LIST OPTS is measured in the
  round-3b run (the bullet below): the second real click reaches the same end states as
  at y=92 (byte-identical), and the first real click at y=100 opens no dropdown — a
  measured difference from y=92, mechanism open (§7.21).
- **Posted grid context.** On a fresh launch with the list at the bottom and its first
  visible row selected, a posted click at y = 92 acts at **x = 46** (REM) and **x = 74**
  (SEL) and at none of the other 34 probed points (x = 8, 12, 36–44, 48–72, 76–102, 130,
  150, 190, 226, 258). The two positives changed the window (x=46 removed the selected
  entry; x=74 then selected all visible rows); every other probed point was a no-op.
  **Qualification:** this holds in the cumulative grid launch; its even-indexed clicks are
  weakened by the documented "every second probe fails to fire" artefact, so a negative at
  an even-indexed point cannot be read as "no hit" — the same reason the REM/SEL
  right-edge probes stay open (§7.12). **(round 3b)** The right-edge candidates were
  re-probed under this same context with the double click: REM acts at 47–55 (edge above
  55, open), and SEL's 75–83 remain 0 px with the edge open on the delivery-control
  ground (§7.12).

- **Context qualification of the left edges.** The x=46 and x=74 positives are the *grid*
  context, not the first bottom-bar click of a fresh launch; a fresh-launch candidate
  click can be swallowed by activation. Whether the same x acts as the first click of a
  fresh launch stays open (§7.12). **(round 3b)** Measured by the round-3b discriminator
  (the bullet below): a single posted click at (46,92) or (74,92) as the first bottom-bar
  click of a fresh launch produces no visible change (0 px, immediate and delayed); the
  qualification stands, **argued, not demonstrated** — the discriminator settles the
  timing question only, and the export's left edges stand unchanged (§7.12).
- **Scrollbar arrow step.** One click on a scrollbar arrow scrolls the playlist by **one
  page = four rows** (the four visible rows). Up and down are symmetric: the first visible
  row reads 25→29→33→29 (down, down, up), and eight up-clicks from the bottom page
  (rows 57–60) land exactly on rows 25–28. This closes the round-2 conflict (§7.14) in
  favour of one page per click and refutes the "one line per press" reading; the
  "one **hold** per arrow" (auto-repeat) half is not measured.
- **The left-edge positives are click-driven, not timed; the export's left edges stand
  unchanged (round 3b).** In the round-3b discriminator, a fresh launch settles, the first
  visible row is selected by one posted click at (60,28) (2918 px, bbox (12,23)-(254,93)),
  and then **one** posted click at (46,92) or at (74,92) — the first bottom-bar click of
  the launch — is captured immediately and again a few seconds later (4 s later for REM,
  2 s for SEL; the harness's own step latency). Against the group's select capture,
  **both** post-click captures are **0 px** and byte-identical to it
  (`baefcc984dc5…`): captures `003`/`004` for REM and `007`/`008` for SEL. The change is
  in neither capture, so the timed hypothesis is **refuted** in the immediate window, and
  the same refutation holds across the run: the ten right-edge launches each post the
  five context gap clicks at x = 36–44 before the candidate (select → candidate click
  19–20 s), and **all 50 gap steps** are 0 px, byte-identical to their group's select
  capture (all 50 re-measured). A settled launch does not drift. **What the discriminator
  does not show:** the probed click is the launch's *second posted* click (after the
  select), so under the documented per-point "every second probe fails to fire" artefact a
  swallowed click and a miss are indistinguishable in that group; it settles the timing
  question only. The export's left-edge positives therefore **stand unchanged** — x=46
  (3475 px) and x=74 (a further 9091 px) in the grid's cumulative context — and the
  context qualification stays **argued, not demonstrated**: what would demonstrate it is
  the double-click design at x=46 and x=74 with a delivery control in the same launch
  (§7.12).
- **REM's right edge: all of 47–55 act, the edge is above 55 (round 3b).** Each candidate
  runs in its own launch: fresh launch, settle, the selection at (60,28), the five context
  gap clicks at x = 36–44 (all 0 px), then the candidate x **clicked twice** at y=92. For
  x = 47, 49, 51, 53 and 55 the first click changes the window by **3475 px** against the
  group's select capture (bbox (12,23)-(254,93)) and the second click adds nothing
  (0 px): the change is the REM removal itself — the selected entry is removed and the
  list renumbers (rows 57–60 → 57–59, no selection). All ten candidate captures are
  byte-identical to the frozen grid positive `008_o5b_grid_x046_pe.png` (SHA-256
  `6ac754b2c11a…`, 10550 bytes), i.e. the same pixel set as the x=46 positive; the
  candidates therefore act **even under the `WinampVis` child**, whose guard the
  pre-flight reads at 53 and 55 (`WinampVis`, hitIsTarget=False), while 47/49/51 read
  `Winamp PE` (hitIsTarget=True). The REM right edge is therefore **above 55** and stays
  open (56 and up were not probed in this design). The frozen run's one-click right-edge
  captures (steps `093`…`120`) are all byte-identical to their select captures: the
  one-click design was uninformative there, and the grid-context double-click design
  recovers the positives.
- **SEL's right edge: 75–83 act nothing, the last acting x stays 74 (round 3b).** Under
  the same design, x = 75, 77, 79, 81 and 83 change nothing on either click: all ten
  captures are byte-identical to their group's select capture (`baefcc984dc5…`), and the
  pre-flight guard reads `WinampVis` (hitIsTarget=False) at all five points. The last x
  that acts stays **74** (the frozen grid positive, a further 9091 px). **Qualification:**
  the SEL candidate launches carry **no delivery control**, so a genuine miss and a
  swallowed click cannot be separated there — that is the ground on which the edge stays
  open, not an invisible-effect hypothesis. The supported reading is that the posted hit
  area at y=92 ends at x=74 and 75–83 are genuine misses; the alternative "the action is
  invisible" reading is out, because the REM/SEL click-#1 dropdown **is** rendered in a
  playlist capture (below; §7.21) and posted clicks do not open it at all, so it offers
  no mechanism.
- **Real clicks at y=100 reach the same end states as y=92 (round 3b).** With real input
  (`SetCursorPos` plus the legacy `mouse_event`, screen-pixel coordinates), two real
  clicks at each button, then a post-click state capture. REM at (46,100): the first
  click is **1388 px** confined to the title band (bbox (6,4)-(271,19) — rows 4–10,
  1387 px, x 6–271, plus row 19, 1 px, x 257), the band's focused/unfocused render, i.e.
  window activation only, no list change; the second click is **3475 px** (the REM
  removal, bbox (12,23)-(254,93)); the post-click capture adds nothing. Capture `102` is
  byte-identical to the frozen y=92 real-click capture `081_o5real_rem_click2_pe.png`
  (SHA-256 `0a4355ddaa86…`, 10532 bytes): the second real click at y=100 lands in exactly
  the same playlist state as at y=92. The select-to-second-click total of 4863 px
  decomposes as 1388 + 3475, so these are the same measurements as the frozen run's
  numbers, not new ones. SEL at (74,100): first click 1388 px (title band), second click
  **9285 px** (all visible rows selected, bbox (12,35)-(254,73)), post-click 0 px; `107`
  and `108` are byte-identical to `086_o5real_sel_click2_pe.png` (`81823df19180…`,
  10406 bytes), select-to-second-click 10673 = 1388 + 9285. LIST OPTS at (242,100): the
  playlist list never changes, and step `112` captures the `#32770` dialog
  **'Load Playlist'** at client **1503×778** — the positive control for real-mouse
  delivery at y=100. Pre-flight guards: (46,100) `Winamp PE` hitIsTarget=True,
  (74,100) `WinampVis` hitIsTarget=False, (242,100) `Winamp PE` hitIsTarget=True.
  **Measured difference from y=92:** at y=100 the first real click opens **no** four-item
  dropdown — the 1388-px set stops at row 19, while the frozen y=92 click-#1 pairs carry
  the rendered dropdown block on top of the title band (export `080` vs `079`: +1428 px
  at x 40–64, rows 32–99; `085` vs `084`: +954 px at x 69–93, rows 50–99; totals 2819 and
  2345 px, each the 1388-px title band plus the dropdown block). That is a measured
  difference between the two y values; the mechanism is not established (§7.21).

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
   the title bar body sprite; which pixels change while a button is held **are resolved by
   S02 §7**, which measures the held cells (menu (0,9), minimize (9,9), close (18,9), shade (9,18)),
   closing this open question.
2. **Which title bar is "focused".** Resolved for the main window (focused variant
   (27, 0), unfocused (27, 15)) and by round 2 for the equalizer ((0, 134) focused,
   (0, 149) unfocused) and the playlist (focused title band = sheet rows 0–19, unfocused
   = rows 21–40). The focused/unfocused pair is the window's own active/inactive render,
   driven by the window's activation messages.
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
7. **Equalizer internals.** Round 2 resolved the slider origin, the groove, the thumb
   rects and the thumb-y mapping, the On/Auto held sprites and the graph background, and
   it recorded the graph colour scale (§3.1). Still open: the **full slider-track
   single-blit tiling** — the free-source (13,164,209,129) at (13,17) does not match as
   one blit, and the decomposition belongs to S07 — and the exact `eq_ex.bmp` shade
   layout, which stays provisional.
8. **Playlist top half.** Round 2 resolved the tile pitch (25 px), the title-band
   composition, the strip position and the bottom-bar button source rows and window
   positions (§4.1). **(round 3)** Round 3 measured the scrollbar arrow step (one page of
   four rows, §7.14) and the REM/SEL left edges (§7.12). **(round 3b)** Round 3b measured
   the REM right-edge bracket (47–55 act, edge above 55) and kept the SEL right edge open
   at 74-vs-75 on the delivery-control ground (§7.12). Still open: the right rail's
   tiling, the REM right edge above 55, and the SEL right edge (§7.12).
9. **`gen.bmp` / `genex.bmp`** sprite tables are out of scope here (S11).
10. The `Winamp MB` and `Winamp Video` window sheets (`mb.bmp`, `video.bmp`) were not
    analysed.
11. **Horizontal graph strip (round 2).** `eqmain.bmp` (0,314,113,1) is a single solid
    colour, and a uniform colour matches anywhere, so its use as a drawn sprite cannot be
    shown from a render. It stays open. Needs: a state where that colour appears as a
    distinctive line that cannot be a coincidence, or source evidence.
12. **Playlist REM and SEL hit areas (round 2; partially closed in rounds 3 and 3b).**
    **(round 3)** The **left edges** are measured in the grid context: REM acts at x=46
    and SEL at x=74 (the drawn bodies span 43–63 and 72–92). The **right edges** stay
    **open**: the round-3 one-launch-per-candidate design clicks each candidate only
    once, and a swallowed click cannot be told from a miss, so its all-zero results are
    uninformative. Needs: one launch per candidate x above 46 (REM) and above 74 (SEL),
    **clicked twice** (or a launch with a known-positive click before the candidate),
    capturing the post-click list; the discriminator captures are the grid positives at
    x=46 (REM) and x=74 (SEL). Also open: whether x=46/74 act as the *first* bottom-bar
    click of a fresh launch — the grid is a cumulative launch, and the same run can
    settle it. **(round 3)** **(round 3b)** The round-3b run performed the double-click
    design this item asked for. **REM:** x = 47, 49, 51, 53 and 55 all act — each
    candidate's first click changes the window by 3475 px (the REM-removal state,
    byte-identical to the grid positive `6ac754b2c11a…`) and the second adds nothing; 53
    and 55 act even under the `WinampVis` child (hitIsTarget=False). The REM right edge
    is therefore **above 55** and open (56 and up unprobed in this design). **SEL:**
    x = 75, 77, 79, 81 and 83 act nothing (both clicks 0 px, byte-identical to the
    group's select state); the last acting x stays **74**. **Left edges:** the grid
    positives stand unchanged (x=46 = 3475 px, x=74 = a further 9091 px, in the grid's
    cumulative context) and are click-driven, not timed — the round-3b discriminator
    refutes the timing hypothesis (0 px over 4 s of delayed capture and over 19–20 s of
    the 50 gap clicks) — but the context qualification stands **argued, not
    demonstrated**, because the discriminator's single click is the launch's second
    posted click and cannot separate a swallowed click from a miss. Still open, with the
    capture each needs: (i) the **REM right edge above 55** — the same design at
    x = 57, 59, 61, … until a 0-px pair (one launch per candidate); (ii) the **SEL edge
    74-vs-75** — the same double-click design at 75/77/83 **plus a delivery control in
    the same launch** (a known-positive bottom-bar click, or real-mouse), so the
    swallowed-click objection is closed rather than argued; the supported reading is that
    the posted hit area at y=92 ends at x=74; (iii) **x=46/74 as the first bottom-bar
    click of a fresh launch** — the group-B design at x=46 and x=74, candidate clicked
    twice, **plus a delivery control in the same launch**; that capture is also what
    would demonstrate the context qualification. **(round 3b)**
13. **Does a real click fire the playlist bottom-bar buttons? (round 2) — resolved in
    round 3.** **(round 3)** Yes: real input fires all five buttons — ADD at (23,92) and
    (23,100) opens the ADD dialog, MISC at (110,92) and (110,100) opens the MISC menu,
    LIST OPTS at (242,92) opens the LIST OPTS dialog, REM at (46,92) removes the selected
    entry and SEL at (74,92) selects all visible rows; the **second** real click is the one
    that fires, and on REM and SEL the first real click opens a four-item dropdown menu
    (§4.2). Still open: real clicks at **y=100** for REM, SEL and LIST OPTS (round 3
    measured y=100 only for ADD and MISC) — §7.18. **(round 3)** **(round 3b)** The
    y=100 half is resolved: real clicks at (46,100), (74,100) and (242,100) behave like
    the y=92 ones — the second real click removes the selected entry (REM, 3475 px),
    selects all visible rows (SEL, 9285 px) or leaves the list unchanged while the
    `#32770` 'Load Playlist' dialog opens (LIST OPTS, client 1503×778); the end states
    are byte-identical to the frozen y=92 captures (`0a4355ddaa86…`, `81823df19180…`);
    the first real click at y=100 is the title band's focused/unfocused render
    (1388 px) and opens no dropdown (§4.2, §7.18). **(round 3b)**
14. **Playlist scrollbar arrow step size (round 2) — resolved in round 3.** **(round 3)**
    One click scrolls **one page of four rows** (the four visible rows); up and down are
    symmetric (the first visible row reads 25→29→33→29), and eight up-clicks from the
    bottom page (rows 57–60) land on rows 25–28. The "one line per press" reading is
    refuted. The "one **hold** per arrow" (auto-repeat) half is not measured and stays
    open. **(round 3)**
15. **Full slider-track single-blit tiling (round 2).** The track is not one blit; the
    per-band body colouring is captured, but the tiling decomposition is S07's.
16. **The round-2 original captures' saved-state setter record (round 2).** The EQ values
    0/31/63 are reproducible by editing `winamp.ini` and restarting (§3.1), but the
    original record of which script or key produced the round-2 `min`/`flat`/`max`
    captures is not in any capture record. Needs: a manifest sentence, or the three rows
    marked provisional.
17. **`WinampVis` child window edges (round 3; right edge resolved in round 3b).** At
    window y = 92 the class `WinampVis` covers client x 52–122; the left edge is
    bracketed to {51, 52} and the right edge only to {123, 124, 125} (x=124 was masked
    by an open `#32768` MISC menu, x=123/125 unprobed). Needs: one launch with a 1-px
    grid over x=51 and over x=123–125 **with no MISC click open** (do not click x ≥ 102
    in the same launch), recording the guard `class=` per point. **(round 3)**
    **(round 3b)** The right edge is **resolved**: in a menu-free launch (one fresh
    launch, no MISC click, a descending 1-px grid at y=92) the guard `class=` changes
    between x=124 and x=123 (126/125/124 read `Winamp PE`, 123/122 read `WinampVis`, no
    guard returned `#32768`), and all five captures are byte-identical to the fresh
    state, so the coverage at y=92 is **x 52–123** and the export run's `#32768` at x=124
    is confirmed as the masking menu, not the child window. Only the **left** edge
    (51 vs 52) stays open. Needs: one launch with an ascending 1-px grid over x=51/52,
    no MISC click open, guard `class=` per point. **(round 3b)**
18. **Real clicks at y=100 for REM, SEL and LIST OPTS (round 3) — resolved in round 3b.**
    The round-3 real-click run measured y=100 only for ADD and MISC; REM, SEL and LIST
    OPTS were measured at y=92 only. **(round 3b)** Resolved by the round-3b run
    (steps `100`–`112`): the second real click at (46,100) removes the selected entry
    (3475 px, byte-identical `0a4355ddaa86…` to the frozen y=92 capture `081`), the
    second real click at (74,100) selects all visible rows (9285 px, byte-identical
    `81823df19180…` to `086`), and the clicks at (242,100) leave the list unchanged
    while step `112` captures the `#32770` dialog 'Load Playlist' (client 1503×778) —
    the positive control for real-mouse delivery at y=100. The first real click at y=100
    is the title band's focused/unfocused render (1388 px, bbox (6,4)-(271,19)) and
    opens no dropdown — a measured difference from the y=92 first click (mechanism open,
    §7.21). The full measurements are in the round-3b bullet of §4.2. **(round 3b)**
19. **MISC's posted hit area reaching x=122 (round 3, provisional).** The grid pre-flights
    at x=104…122 all read `WinampVis` (no menu) and the x=124 pre-flight reads `#32768`;
    the last click before it is x=122, so the menu was opened by the x=122 click and MISC's
    *posted* hit area reaches x=122. The evidence is that single guard line (the menu is
    not rendered in any playlist capture), so the reading is provisional. Needs: one launch
    that posts (122,92) and then captures the `#32768` menu window directly, or records the
    guard line at x=123/124. **(round 3)** **(round 3b)** Stays provisional. The
    round-3b run adds no `#32768` capture and no guard line after a 122 click; it adds
    the corroboration that x=124 in a menu-free descending grid reads `Winamp PE`,
    confirming the frozen run's `#32768` at x=124 as a masking menu, and the observation
    that the x=122 click changed nothing observable in the playlist capture (only the
    MISC menu is invisible to it). The inference keeps its single-line-of-evidence
    status. **(round 3b)**
20. **The 95-px preamp residual (round 3).** A fresh real preamp-to-max drag does not
    reproduce the recorded real-drag capture `s01r2d_realinput_eq_ondrag_after_eq.png`
    (95 of 31900 pixels differ, in the preamp column, bounds (18,22)-(32,50)); the cause is
    not established. Needs: a manifest sentence naming the recorded capture's exact drag
    path and state, or a fresh capture of the recorded state for comparison. **(round 3)**
    **(round 3b)** Untouched by the round-3b run; stays open by owner decision — it closes
    by a manifest sentence naming the recorded capture's exact drag path and state, or by
    a fresh capture of that state; the round-3b pixels cannot decide it. **(round 3b)**
21. **The nature of the REM/SEL click-#1 dropdown menu (round 3).** The first real click on
    REM and on SEL opens a four-item menu over the bottom bar; whether it is the button's
    own dropdown or another window is not established. Needs: one capture of the menu
    window opened by the real click #1 (the playlist render shows it; a `#32768`-class
    capture would name it). **(round 3)** **(round 3b)** The menu **is** rendered in a
    playlist `PrintWindow`: in the export run `080` differs from `079` by a 1428-px block
    at x 40–64, rows 32–99, and `085` differs from `084` by a 954-px block at x 69–93,
    rows 50–99; the items are legible (`REM MISC / REM ALL / CROP / REM SEL`,
    `INV SEL / SEL ZERO / SEL ALL`). What is missing is therefore only the menu
    window's name/class. Also measured (round 3b): the real click #1 at y=100 opens
    **no** dropdown (the 1388-px title-band set stops at row 19) — a measured difference
    between y=92 and y=100, mechanism not established. Needs (restated): a `#32768`-class
    capture (or a guard line) of the menu opened by the real click #1 at (46,92)/(74,92) —
    the menu is rendered, only its name/class is missing. **(round 3b)**
22. **The ON-indicator `use_eq` link (round 2; unchanged).** §3.1 records the ON
    indicator's cell at client (18–20, 22–23) as associated with `use_eq`, but the link was
    not independently re-verified (both round-2 autoload captures had `use_eq=1`). Needs:
    one restart with `use_eq` toggled and nothing else changed, then a capture. **(round 3)**
