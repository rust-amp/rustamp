---
Spec ID: S-17
Title: Skin-loading quirks (missing files, fallback, nums_ex, case and folders, pledit.txt, viscolor.txt, region.txt, BMP encodings, cursors)
Sources consulted (category only): dirty room: binary behaviour capture; dirty room: shipped skin assets (test input); free: webamp (cross-check only)
Author agent: R-agent (dirty room, capture session 2026-10-14)
Auditor: Spec Auditor (dirty room)
Audit date: 2026-10-14
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561, extracted from winamp295.exe SHA-256 ef1423999cbefd4f2a3a98acee2b194a4260268cb4517abda9f12963e866b7e5)
Status: audited
---

# S17 — Skin-loading quirks

Everything marked **verified** was produced by loading a purpose-built skin into the
sandboxed 2.95 and looking at the result. Items marked **not determined** were not tested
in this session; they are listed so that nobody mistakes silence for a negative result.

Terminology: a **skin** is either a `.wsz` file (a ZIP archive) or an unpacked directory.
The base skin used for the reference measurements is the original Winamp base skin
(`base-2.91`), which contains 32 entries.

---

## 1. Skin container and lookup

- **Verified:** a `.wsz` file is a plain ZIP archive; the base skin is one.
- **Verified:** an **unpacked directory** works as a skin. Placing the base skin's files
  in a subdirectory of the skin folder and selecting that subdirectory by name renders
  the same window as the `.wsz`.
- **Verified (by construction):** skin file lookup is **case-insensitive**. The base skin
  stores its sheets with uppercase names, while the program's internal name list uses
  lowercase; the base skin still loads and renders completely. A loader must fold case.
- **Not determined:** whether a directory *inside* a `.wsz` is treated as a skin, and
  whether a `.wsz` may carry a subdirectory prefix on its entries.
- **Not determined:** behaviour with **duplicate entries** of the same name inside one
  ZIP (which one wins). Not tested.
- **Not determined:** whether a file whose name matches but whose extension case differs
  (`.BMP` vs `.bmp`) is found; implied by case-insensitivity but not tested directly.

## 2. Missing files and fallback

- **Verified:** deleting `main.bmp` **and** `titlebar.bmp` from an otherwise complete skin
  directory does not stop the window from rendering: the window still draws, and the
  missing elements come from the built-in default skin compiled into the program. The
  substituted title bar is visibly *brighter* than the base skin's, which shows the
  built-in default skin is **not** pixel-identical to the `base-2.91` skin.
- **Verified:** the fallback is per element, not all-or-nothing: the rest of the skin
  (clutter bar, controls) still came from the modified directory.
- **Not determined:** the exact per-file fallback matrix (which missing sheet falls back
  to which built-in sheet, and whether any sheet's absence is fatal). Only the
  `main.bmp` + `titlebar.bmp` case was exercised.
- **Not determined:** what happens when the *whole* skin is invalid (a `.wsz` that is not
  a ZIP, or a directory with no sheets at all).
- **Not determined:** whether a missing `pledit.txt` / `viscolor.txt` / `region.txt` falls
  back to the built-in defaults or to the built-in skin's files. See §5 and §6 for what
  was observed when they are absent.

## 3. `numbers.bmp` / `nums_ex.bmp` substitution

- **Verified:** when a skin contains `nums_ex.bmp`, the main window's time display uses it
  **instead of** `numbers.bmp`. This was tested by adding a hand-made `nums_ex.bmp` with
  obviously wrong digits: the time display immediately showed those wrong digits.
- **Verified:** using `nums_ex.bmp` also **changes the digit layout** in the time display:
  the digit cells are not at the same x positions as with `numbers.bmp`, and the cell
  pitch changes. The exact positions and pitch of the `nums_ex` layout were not measured
  (see S01, open question 6).
- `numbers.bmp` itself: 11 cells of 9 x 13 px; cells 0–9 are the digits, cell 10 is a
  filler; the minus sign is a 5 x 1 region at (20, 6). Full table in S01 §2.3.
- **Not determined:** whether `nums_ex.bmp` is required to be a specific size, and what
  happens if it is present but the wrong size.

## 4. BMP encodings, palette and transparency

- **Verified:** every sheet in the base skin is **8-bit palettised** (256-colour or
  fewer).
- **Verified:** the base skin **mixes RLE8-compressed and uncompressed** sheets. Both
  kinds must be supported. RLE8 sheets in the base skin: `main`, `cbuttons`, `shufrep`,
  `posbar`, `balance`, `monoster`, `numbers`, `text`, `eqmain`, `gen`. Uncompressed
  sheets: `titlebar`, `volume`, `playpaus`, `pledit`, `eq_ex`, `genex`, `mb`, `video`.
- **Not determined:** whether 4-bit, 1-bit, 24-bit or 32-bit BMPs are accepted, and how
  32-bit alpha (if any) is treated. The base skin never exercises this.
- **Verified:** 2.95 copies the classic sheets **opaquely**; there is **no** per-sheet
  transparency colour key. Tested by replacing a region that is drawn on screen — the play
  button in `cbuttons.bmp` and a band of the title bar in `titlebar.bmp` — with the colour
  a key would use, and separately with palette index 0: in every case the replacement
  colour appears in the render unchanged. The pale cyan `(0, 198, 255)` that several base
  skin sheets use as filler is never drawn (the rendered main window contains no pixel of
  it), because those areas are not blitted at all. Full detail in S01 §6.
- Consequence for implementers: nothing has to be masked. Sprites carry their own
  background; the filler colour around them is in unblitted areas and is never read.
- **Not determined:** whether any third-party skin relies on a colour key, and what 2.95
  does with such a skin (no example found).

## 5. `pledit.txt`

- **Verified:** `pledit.txt` is an INI file with a single section, `[Text]`, and the keys
  `Normal`, `Current`, `NormalBG`, `SelectedBG` (colours written as `#RRGGBB`) and `Font`.
  The base skin's file is exactly:

  | Key | Base-skin value |
  |---|---|
  | `Normal` | `#00FF00` |
  | `Current` | `#FFFFFF` |
  | `NormalBG` | `#000000` |
  | `SelectedBG` | `#0000C6` |
  | `Font` | `Arial` |

- **Verified:** with `pledit.txt` absent the playlist still renders with these colours
  (green normal entries, white current entry, black background); this suggests that these
  values are also the built-in defaults for the base-skin look (inferred from one
  observation, not tested with other files present).
- **Verified:** the playlist list uses a **real font** (the `Font` key), not the
  `text.bmp` bitmap font. The rendered `Normal` colour is exactly the `#00FF00` from the
  file, with a darker shade of the same hue behind the glyphs.
- **Verified:** a selected row's background is drawn in the `SelectedBG` colour over the
  list area.
- **Not determined:** what happens with a malformed colour string, a missing key, or a
  `Font` name that is not installed.

## 6. `viscolor.txt`

- **Verified structure:** a flat list of **24** comma-separated `R,G,B` entries, one per
  line, with an optional trailing comment introduced by `//`. Slot numbers are implicit
  (the first line is slot 0).
- **Slot semantics (provisional; inferred from the rendered analyser and the ordering of
  the base skin's list, not from a controlled test):**

  | Slots | Meaning |
  |---|---|
  | 0 | analyser background (black in the base skin) |
  | 1 | analyser grid / dot colour (a grey in the base skin) |
  | 2–17 | the 16-colour spectrum gradient, slot 2 at the top of the bar and slot 17 at the bottom |
  | 18–22 | the five oscilloscope colours, slot 18 brightest, later slots progressively dimmer |
  | 23 | the analyser peak-dot colour |

- The base skin's list names its last oscilloscope slots the same way twice; the layout
  is treated here as five oscilloscope colours (18–22) with the duplicate name in the
  file ignored.
- **Not determined:** what happens with fewer than 24 entries (the base skin has exactly
  24), with values out of range, or with a missing file (a missing file was not tested in
  isolation).

## 7. `region.txt`

- **Verified syntax:** INI-style. Sections are `[Normal]`, `[WindowShade]` and
  `[Equalizer]`. Each section has `NumPoints` (a comma-separated list of the vertex count
  of each polygon) and `PointList` (one single comma-separated list of `x,y` pairs
  covering all polygons of that section, in order).
- **Verified semantics:** the listed polygons define the **visible** area of the window;
  everything outside them is transparent. The window rectangle itself is unchanged.
  Verified by writing an active `[Normal]` polygon covering only the title-bar band: the
  window still measured 275 x 116, but only the title-bar rows rendered and the rest of
  the client area was transparent.
- **Provisional (not confirmed by a controlled test):** coordinates are the outer pixel
  corners, listed clockwise, and the first vertex is **not** repeated at the end. Multiple
  polygons in a section are additive.
- The base skin's `region.txt` has every polygon commented out, so with the base skin the
  windows are plain rectangles and there is no masking.
- **Not determined:** whether `[Equalizer]` and `[WindowShade]` really apply to those
  windows (only `[Normal]` was exercised), and what happens with a malformed
  `NumPoints`/`PointList` (mismatched counts).

## 8. Cursors

- **Verified:** the base skin ships cursor files alongside the sheets. The set present is:
  `NORMAL`, `CLOSE`, `EQSLID`, `EQTITLE`, `POSBAR`, `PSIZE`, `PTBAR`, `PVSCROLL`,
  `TITLEBAR`, `VOLBAL`, `PNORMAL` (all with the `.cur` extension).
- **Verified:** they are ordinary Windows `.cur` files.
- **Not determined:** which window region each cursor belongs to, whether the program
  falls back to the built-in cursors when one is missing, and whether a cursor file of the
  wrong size is rejected.

## 9. Loading order and practical checklist

Derived from the above, the order a loader has to get right for the base skin to look
correct:

1. Find the skin (`.wsz` ZIP or directory), folding case on entry names.
2. Read 8-bit BMPs, both uncompressed and RLE8.
3. Blit each sheet opaquely; no colour key is applied (§4).
4. Prefer `nums_ex.bmp` over `numbers.bmp` for the time display if it exists, and switch
   the digit layout accordingly.
5. Read `pledit.txt` (colours and font), `viscolor.txt` (24 analyser colours) and
   `region.txt` (window masks) if present, otherwise use the built-in defaults.
6. For any sheet that is missing, substitute the built-in default skin's sheet for that
   element.

## 10. Open questions

1. The full missing-file fallback matrix (§2), including which sheets are fatal.
2. Whether any third-party skin relies on a transparency colour key (§4).
3. The `nums_ex.bmp` cell layout (§3).
4. Duplicate ZIP entries and ZIP subdirectories (§1).
5. `region.txt` handling of `[WindowShade]` and `[Equalizer]`, and malformed input (§7).
6. Cursor-to-region mapping and cursor fallback (§8).
7. Whether higher-bit-depth BMPs are accepted at all (§4).
