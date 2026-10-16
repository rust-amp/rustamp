---
Spec ID: S-04
Title: Menu popups (main menu, Options, visualization, equalizer presets, playlist MISC — rendered rows, checks, arrows, hints, geometry)
Sources consulted (category only): dirty room: binary behaviour capture (sandbox run 20261016-011612); dirty room: offline capture reading (OCR + pixel arithmetic); dirty room: existing audited specs S03 (shortcuts), S06 (playlist), S02 (main-window behaviour); harness: tools/vm-capture README (method and limits)
Author agent: S04 spec editor (dirty room, lane s04x, 2026-10-16; no sandbox started by this session)
Auditor:
Audit date:
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561, extracted from winamp295.exe SHA-256 ef1423999cbefd4f2a3a98acee2b194a4260268cb4517abda9f12963e866b7e5)
Status: draft
---

# S04 — Menu popups

Scope: what the player's left-click-reachable menu popups *draw* — the main
menu, the Options popup, the visualization menu, the equalizer presets menu
and the playlist MISC popup: their rows in order, separators, check marks and
their cell, submenu arrows, drawn shortcut hints, popup sizes and classes,
and the row geometry the reading measured. Command ids are S03's and S10's
subject; what each item *does* when clicked is out of scope except where
stated; the operating-system popup's own behaviour (dismissal, keyboard) is
unmeasured.

Everything marked **settled** was captured in sandboxed run `20261016-011612`
and read offline by the reading session, with the capture and step named.
Everything marked **provisional** rests on a measurement whose label or meaning
is not established. Everything marked **open** was not measured. A value read
by eye or by OCR from a capture is marked **visual reading** and carries its
pixel evidence; a guessed label is never asserted in place of an unreadable one.

Provenance note: this spec exports the offline reading
(`audits/S04-menus-reading.md`) of run `20261016-011612` only. It does not
re-derive the resource-table trees of the earlier offline draft
(`drafts/S04-menus.md`): ids, nested submenus, context menus and every menu
that needs a right click stay with that draft and are not exported here.

## 0. How to read this spec

Evidence grades used:

| Grade | Meaning |
|---|---|
| **settled** | Direct measurement in a sandboxed 2.95 run, with the step and capture named |
| **provisional** | Measured, but its label or meaning is not established; the missing piece is named |
| **open** | No evidence of any kind; the capture that would settle it is named |
| **visual reading** | A string, row label or shape read from a capture by eye or by OCR, not produced by the comparator |
| **corroboration** | Two independent captures or passes agreeing, where neither alone would settle the point |

Run interpreted here:

| Run | Job | Result |
|---|---|---|
| `20261016-011612` | `tools/vm-capture/jobs/s04-menus.json`, 30 steps | Landmark gate passed (region 0,49,275,67, 0 differing pixels of 18425; twins byte-identical); status `steps-failed`, exit 5: the eight failures are classified — three twin `expect_change` identity failures (0 differing pixels, i.e. determinism proof, §1.5), five exit-3/exit-4 negatives (no `#32768` window, §1.6). Display facts stable start to end: dpi 144 (150%), screen 3050x1668, per-monitor-v2. Fresh launch with `winamp_args /NOREG`, nothing loaded. |

Pixel figures are offline readings (OCR plus pixel arithmetic), not harness
comparator verdicts: every popup step declares no landmark gate and verdict
`no-baseline`. Check-glyph facts rest on a gutter search over x=8–44 (glyphs
sit at x=16–29; a narrower window misses them): the checked shape is identical
everywhere (40 non-background px, black core `(26,26,26)` plus grey antialias
edge); an unchecked cell is plain background `(249,249,249)` with zero
non-background px in x=8–44 (Verified by the reading's gutter scan of each
capture below).

---

## 1. Menu inventory: which popup, what opens it, size and class

All popups are ordinary operating-system popup windows of class **`#32768`**
(Verified by `bootstrap.log` per-step target lines and the reading's header).

| # | Popup | Opened by (step) | Capture (step) | Client size (manifest row) | Evidence |
|---|---|---|---|---|---|
| 1 | Main menu | Posted left click at client (6,3) on the window-menu button (`s04_mm_open_posted`) | `003_mm_popup_posted_menu.png` (`s04_mm_popup_posted`) | **323 x 569** | **settled** |
| 2 | Options popup | Posted left click at client (14,25) on clutterbar letter 1 (O) (`s04_opt_open_posted`) | `009_opt_popup_posted_menu.png` (`s04_opt_popup_posted`) | **371 x 348** | **settled** |
| 3 | Options popup, toggled mode | Posted time-display click at (60,32) (`s04_timerem_posted`), then posted click at (14,25) (`s04_opt_after_toggle_open_posted`) | `030_opt_popup_after_toggle_menu.png` (`s04_opt_popup_after_toggle`) | **371 x 348** | **settled** |
| 4 | Visualization menu | Posted left click at client (14,56) on clutterbar letter 5 (V) (`s04_vis_open_posted`) | `012_vis_popup_posted_menu.png` (`s04_vis_popup_posted`) | **399 x 151** | **settled** |
| 5 | Equalizer presets menu | Posted left click at client (239,24) on the EQ PRESETS button (`s04_eqp_open_posted`); real-click path (`s04_eqp_open_real1`) | `015_eqp_popup_posted_menu.png` (`s04_eqp_popup_posted`); `018_eqp_popup_real_menu.png` (`s04_eqp_popup_real`) | **176 x 105** both | **settled** |
| 6 | Playlist MISC popup | Two real clicks at client (110,92) on the playlist MISC button (`s04_pl_misc_open_real1`, `s04_pl_misc_open_real2`) | `023_pl_misc_popup_real_menu.png` (`s04_pl_misc_popup_real`) | **474 x 81** | **settled** |

- The MISC popup exists **only** as a real-input capture: the posted path
  (`s04_pl_misc_popup_posted`, step 20) exited 3 (no `#32768` window), while
  the real-click path produced `023` (Verified by DONE.json step results).
- The main menu's real-click popup (`s04_mm_popup_real`, step 7) exited 3;
  the main menu is therefore evidenced only through the posted window-menu-button
  path in this material (Verified by DONE.json).
- The EQ/playlist title-bar probes (`s04_eqtitle_popup_posted`,
  `s04_pltitle_popup_posted`, steps 25/27) exited 3: no `#32768` popup follows
  a posted click at (6,3) on `Winamp EQ` / `Winamp PE` (Verified by DONE.json).
  This agrees with the S06 title-bar finding (S06 §1).
- The `s04_eqp_open_real2` real-click step exited 4 (pre-action size gate),
  yet `s04_eqp_popup_real` still produced `018`, identical in OCR to `015`
  (Verified by DONE.json and the reading §5).

### 1.5 Render determinism (twin agreement)

Each of the first three popups was captured twice without dismissing it; the
second capture is byte-identical to the first, and the twin step's
`expect_change` assertion failed with **0 differing pixels** — the failure
*is* the determinism proof:

| Popup | Captures (steps) | Result |
|---|---|---|
| Main menu | `003` (`s04_mm_popup_posted`) vs `004` (`s04_mm_popup_posted_twin`) | byte-identical OCR; 0 differing pixels (Verified by DONE.json failure line and the reading §1) |
| Options popup | `009` (`s04_opt_popup_posted`) vs `010` (`s04_opt_popup_posted_twin`) | identical (Verified by DONE.json and the reading §2) |
| Visualization menu | `012` (`s04_vis_popup_posted`) vs `013` (`s04_vis_popup_posted_twin`) | identical (Verified by DONE.json and the reading §4) |
| EQ presets, posted vs real | `015` (`s04_eqp_popup_posted`) vs `018` (`s04_eqp_popup_real`) | identical OCR; no rendering difference between posted and real-input paths (Verified by the reading §5) |

The playlist MISC popup has no twin: only the real-input capture `023`
exists (no posted twin was produced).

---

## 2. Main menu rows and separators

Rows in order, top to bottom (Verified by the reading §1: full-image OCR pass
plus twin agreement on `003_mm_popup_posted_menu.png`, step
`s04_mm_popup_posted`):

| # | Row (**visual reading**) | Separator after |
|---|---|---|
| 1 | `Nullsoft Winamp...` (title row, y≈11–28) | — |
| 2 | `Play ▶` (submenu) | — |
| 3 | `View file info` (right hint unreadable — see §3.4) | — |
| 4 | `Bookmarks ▶` (submenu) | separator at y=156 |
| 5 | `Main Window` ✓ | — |
| 6 | `Playlist Editor` ✓ | — |
| 7 | `Equalizer` ✓ | — |
| 8 | `Library` | — |
| 9 | `Minibrowser` | — |
| 10 | `Video` | separator at y=377 |
| 11 | `Options ▶` (submenu) | — |
| 12 | `Playback ▶` (submenu) | — |
| 13 | `Visualization ▶` (submenu) | — |
| 14 | `Skins ▶` (submenu) | separator at y=528 |
| 15 | `Exit` | — |

Separators are full-width rules of **303 non-background px** at y=40, 156,
377 and 528 (Verified by the reading's full-width non-background run scan of
`003`, step `s04_mm_popup_posted`). The y=40 rule sits between the title row
and `Play`.

Correspondence to S03 action names (S03 §1, §5): `Play` opens the play
submenu; `View file info` corresponds to the Alt+3 file-info action;
`Bookmarks` to the bookmark actions (Ctrl+Alt+I / Alt+I); `Main Window`,
`Playlist Editor`, `Equalizer`, `Minibrowser` (`Alt+T`, provisional in S03),
`Video` to the window toggles (Alt+W / Alt+E / Alt+G / Alt+T / Alt+V);
`Options`, `Playback`, `Visualization`, `Skins` to the submenu trees; `Exit`
has no S03 row. **Whether clicking any of these rows executes the
corresponding command was not measured** — no menu-item click step exists
(see §9).

---

## 3. Main menu checks, submenu arrows, Alt-hints, greyed state

### 3.1 Check marks and their cell

Three checked rows, all with the glyph in x=**16–29** (Verified by the
reading's gutter scan of `003`, step `s04_mm_popup_posted`; the reading
corrects the earlier too-narrow window — the cell is x=16–29, not narrower):

| Row | Check bbox (x=16–29) | Evidence |
|---|---|---|
| `Main Window` | y=**174–183** (bbox 16,174–29,183) | **settled** |
| `Playlist Editor` | y=**209–218** | **settled** |
| `Equalizer` | y=**244–253** | **settled** |

The gutter cells of `Play`, `View file info` and `Bookmarks` are verified
empty (zero non-background px in x=8–44) (Verified by the same scan).
What the checks *mean* (window open state) is not established by this
material; the run launched fresh with `/NOREG` and all three windows present.

### 3.2 Submenu arrows

Submenu arrows sit at x=**297–303**, about 12 px tall, on six rows (Verified
by the reading's ASCII pixel-dump scan of `003`, step `s04_mm_popup_posted`):

| Row | Arrow y |
|---|---|
| `Play` | 57–68 |
| `Bookmarks` | 127–138 |
| `Options` | 394–405 |
| `Playback` | 429–440 |
| `Visualization` | 464–475 |
| `Skins` | 499–510 |

No other main-menu row carries an arrow.

### 3.3 Alt-hints

The six window rows carry `Alt+*` hints occupying x≈239–293 (Verified by
full-image OCR with word boxes on `003`, step `s04_mm_popup_posted` —
**visual readings**):

| Row | Hint (**visual reading**) |
|---|---|
| `Main Window` | `Alt+W` |
| `Playlist Editor` | `Alt+E` |
| `Equalizer` | `Alt+G` |
| `Library` | `Alt+L` (engine rendered `Alt+l_`; lowercase-L rendering noted, not silently fixed) |
| `Minibrowser` | `Alt+T` |
| `Video` | `Alt+V` |

### 3.4 Unreadable hint (provisional — no label asserted)

A black text cluster sits at x=246–288, y=92–105 on the `View file info`
row (`A`-shaped first glyph x≈246–257, `l`, `t`, `+` crossbar at y=100
x≈269–278, narrow last glyph x≈281–288 — eye-consistent with `Alt+3` but the
last glyph is not positively identified). OCR returned **0 lines** on the full
pass, on a (245,90,296,107)@3× tight crop, and on a (200,90,323,107)@3× wide
crop (Verified by the reading §1, three OCR attempts on `003`, step
`s04_mm_popup_posted`). **Provisional**: geometry recorded, no label asserted;
S03 §1 binds Alt+3 to View file info, but that does not settle what this
capture draws. See open question 1.

### 3.5 Greyed rows: none

**Settled**: no main-menu row is greyed — every row has strong black text
cores (52–210 px/row) and zero mid-grey cores in x=45–235 (Verified by the
reading's grey test on `003`, step `s04_mm_popup_posted`). This is the
fresh-launch, nothing-loaded state only; the loaded-state comparison is open
(see §10).

---

## 4. Options popup before and after the time-display toggle

### 4.1 Rows before the toggle

Rows in order (Verified by the reading §2: full pass plus twin agreement on
`009_opt_popup_posted_menu.png`, step `s04_opt_popup_posted`):

| # | Row (**visual reading**) | Hint (**visual reading**) | Check | Separator after |
|---|---|---|---|---|
| 1 | `Preferences...` | `Ctrl + P` (full pass read `Ctrl *p`; (200,9,371,26)@3× crop reads `Ctrl + P`, confirmed) | unchecked | — |
| 2 | `Skins ▶` (arrow x=345–351, y=46–57) | — | — | separator at y=75 |
| 3 | `Time elapsed` | `(Ctrl+T toggles)` | ✓ bbox (**16,93–29,102**) | — |
| 4 | `Time remaining` | `(Ctrl+T toggles)` | unchecked (gutter verified empty) | separator at y=156 |
| 5 | `Always On Top` | `Ctrl+A` | unchecked | — |
| 6 | `Double Size` | `Ctrl+D` | unchecked | — |
| 7 | `EasyMove` | `Ctrl+E` | ✓ bbox (**16,244–29,253**) | separator at y=272 |
| 8 | `Repeat` | single glyph at x≈328–336, y=290–302, eye-read `R` — **provisional** (see §4.3) | unchecked | — |
| 9 | `Shuffle` | single glyph at x≈328–336, y=325–337, single-pass OCR `s` + eye shape agree — **provisional** (see §4.3) | unchecked | — |

No selection highlight is present in `009`. Greyed rows: none (black cores
52–216/row, zero mid-grey) (Verified by the reading's grey test on `009`).

Correspondence to S03: `Preferences...` (Ctrl+P), `Always On Top` (Ctrl+A),
`Double Size` (Ctrl+D), `EasyMove` (Ctrl+E), `Repeat` (R), `Shuffle` (S) all
have S03 §1 rows; `Time elapsed` / `Time remaining` name the Ctrl+T toggle
(S03 §1, command 40242; the text names the toggle, not the item's own id).
**Click-through is not measured** (see §9).

### 4.2 After the toggle: the check mark moves, nothing else changes

Between `009` (step `s04_opt_popup_posted`) and `030`
(`030_opt_popup_after_toggle_menu.png`, step `s04_opt_popup_after_toggle`,
opened after the posted time-display click `s04_timerem_posted` at (60,32)):
the pixel difference (12 640 px total per DONE.json; 12 640 px per the
reading) is confined to exactly two things (Verified by the reading §3):

1. the check cells at x=16–29 only — 40 px at y=90–110 (check **removed**
   from `Time elapsed`, cell now empty) plus 40 px at y=120–140 (check
   **added** on `Time remaining`, bbox **16,128–29,137**); the `EasyMove`
   check is unchanged (16,244–29,253 in both captures);
2. a hover-selection band x=5–365, y=278–312 filled `(240,240,240)` over the
   `Repeat` row, absent in `009` — a mouse-hover artifact, **not** a state
   change; the `Repeat` text is still black-core and checkless.

**Settled: the toggled row is `Time elapsed` → `Time remaining`; no other
row changed.** OCR text output is identical on `009` and `030` (check marks
are pixel-only, invisible to OCR). This corroborates S02 §4 (the
elapsed/remaining toggle) at the menu-rendering level.

### 4.3 Uncertain Repeat/Shuffle hints (provisional)

- `Repeat` hint `R`: eye-read only; (320,288,345,305)@3× OCR returned 0
  lines (full pass plus crop) — **provisional** (Verified by the reading §2
  on `009`, step `s04_opt_popup_posted`).
- `Shuffle` hint `S`: single-pass OCR `s` (WORD[329,325,8,13]) plus eye shape
  agree; (320,322,345,340)@3× crop OCR returned 0 lines — **provisional**,
  single-pass plus eye, unconfirmed by crop (Verified by the reading §2 on
  `009`). See open question 2.

---

## 5. Visualization menu

Rows in order (Verified by the reading §4: full pass plus twin agreement on
`012_vis_popup_posted_menu.png`, step `s04_vis_popup_posted`):

| # | Row (**visual reading**) | Hint (**visual reading**) | Separator after |
|---|---|---|---|
| 1 | `Visualization options...` | `Alt+K` is drawn per the earlier draft but **not** confirmed by this reading — see note | separator at y=40 |
| 2 | `Start/Stop plug-in` | `Ctrl+Shift+K` | — |
| 3 | `Configure plug-in...` | `Alt+K` | — |
| 4 | `Select plug-in...` | `Ctrl+K` | — |

Note on hints: the reading confirms the five row labels verbatim but does not
transcribe per-row hint boxes for this menu the way it does for the main and
Options popups; the hint strings above for rows 2–4 are carried from the
offline draft's resource-table rows (S03 §1 binds all three keys) and are
**provisional as rendered facts** until a hint-box transcription names them.
Checks: none (gutter x=8–44 empty except the y=40 rule). Submenu arrows:
none. Greyed rows: none.

Correspondence to S03 §1: `Visualization options` (Alt+O), `Start/Stop
plug-in` (Ctrl+Shift+K), `Configure plug-in` (Alt+K), `Select plug-in`
(Ctrl+K). Whether a plug-in list is appended below `Select plug-in...` when a
visualization plug-in is installed is **open** (the run had no vis plug-in
loaded). **Click-through is not measured** (see §9).

---

## 6. Equalizer presets menu

Rows (Verified by the reading §5: identical OCR on
`015_eqp_popup_posted_menu.png`, step `s04_eqp_popup_posted`, and
`018_eqp_popup_real_menu.png`, step `s04_eqp_popup_real`):

| # | Row (**visual reading**) |
|---|---|
| 1 | `Load` |
| 2 | `Save` |
| 3 | `Delete` |

No checks, no submenu arrows, no separators, none greyed (black cores
67–109/row). Posted vs real-input rendering: no difference. The contents of
the `Load` / `Save` / `Delete` submenus, their labels beyond these three
top-level entries, and the load/save path behaviour are **open** (opening a
submenu leaves two `#32768` windows and the harness refuses the pair; no
submenu step was planned).

---

## 7. Playlist MISC popup and its relationship to the playlist window

Rows (Verified by the reading §6: full pass plus two (0,y−2,474,y+15)@3× row
crops agreeing, on `023_pl_misc_popup_real_menu.png`, step
`s04_pl_misc_popup_real`):

| # | Row (**visual reading**) | Hint (**visual reading**) | Separator after |
|---|---|---|---|
| 1 | `Generate HTML playlist` (y≈11–28) | `(Ctrl+Alt+G)` | separator at y=40 (454 non-background px) |
| 2 | `Read extended info on selection` (y≈57–70) | `(Ctrl+Alt+E)` | — |

No checks, no submenu arrows, none greyed. This capture is real-input only;
no posted twin exists (§1).

Relationship to the playlist window (S06): the popup is opened by the MISC
bottom-bar button of `Winamp PE` (real clicks at (110,92); the posted probe at
the same point exited 3 — see §1). S06 §4 records the same 474 x 81 capture
with the same two rows and the same shortcut hints, read through a
single-engine OCR pass with an attached confidence caveat; this spec's
row-crop agreement corroborates that reading. The capture-plan expectation
of sort/randomise entries in the MISC popup is **refuted for this capture**:
`023` holds the two rows above and no sort/randomise text (Verified by the
reading §6 and S06 §4). Sort/randomise effects and per-item menu semantics
stay open (S06 §8). **Click-through is not measured** (see §9).

---

## 8. Row geometry and rendering

All sizes are manifest `client_size` readings at dpi 144 (150%); the popups
are system-drawn (`#32768`), not skin sheets, so metrics follow the system
menu font and DPI and are only comparable with captures at the same DPI
(Verified by DONE.json display facts, stable start to end).

| Popup | Client size | Evidence |
|---|---|---|
| Main menu | **323 x 569** | **settled** — manifest row for `003`/`004` (steps `s04_mm_popup_posted` / twin) |
| Options popup | **371 x 348** (both before `009`/`010` and after `030`) | **settled** — manifest rows for steps `s04_opt_popup_posted` / twin / `s04_opt_popup_after_toggle` |
| Visualization menu | **399 x 151** | **settled** — manifest rows for `012`/`013` |
| EQ presets menu | **176 x 105** (posted `015` and real `018` alike) | **settled** — manifest rows for steps `s04_eqp_popup_posted`, `s04_eqp_popup_real` |
| Playlist MISC popup | **474 x 81** (real-input `023` only) | **settled** — manifest row for step `s04_pl_misc_popup_real` |

Measured geometry (Verified by the reading's pixel scans on the captures and
steps named in §§2–4):

- Check-mark cell: x=**16–29** everywhere; glyph 40 non-background px, black
  core `(26,26,26)` plus grey antialias edge. The earlier too-narrow window
  is corrected: a narrower gutter search misses the glyph.
- Main-menu submenu arrows: x=**297–303**, ~12 px tall (§3.2).
- Options `Skins` arrow: x=**345–351**, y=46–57.
- Separators: full-width rules — main menu 303 non-background px at
  y=40/156/377/528; MISC 454 non-background px at y=40; visualization and
  Options rules at the y values of §§4–5.
- Alt-hint column: x≈239–293 on the six main-menu window rows (§3.3).
- Row-band method: row crops were taken as (0,y−2,width,y+15) at 3×
  nearest-neighbour per the project's OCR note; tight-crop second opinions
  via text-grab printed nothing and are not relied on.
- **Open**: item height, separator height, top/bottom padding, the drawn font
  family/size/weight, the non-client border (the harness captures the client
  rect only), and whether metrics scale with system DPI (needs the same menu
  at two DPIs).

---

## 9. What this material does not cover

- **Click-through behaviour.** No step clicks a menu item: a step's action
  and its capture target are the same window, and a click that executes an
  item dismisses the popup, so no such step was planned. What each item does
  is therefore **not covered** by this material; the S03 correspondences
  named in §§2–5 are action-name correspondences, not executed effects.
- **Operating-system popup behaviour.** Dismissal, keyboard navigation,
  hover-highlight semantics (beyond the §4.2 hover band, which is recorded as
  not-state), multi-popup tie-breaking and submenu opening are unmeasured.
- **Nested submenus.** `Play`, `Bookmarks`, `Options`, `Playback`,
  `Visualization`, `Skins` (main menu) and `Load` / `Save` / `Delete` (EQ
  presets) were never opened: opening one leaves two `#32768` windows and
  the harness refuses the pair.
- **Right-click menus.** The playlist context/entry menus, long-title menu,
  time-display menu, transport/toggle menus and EQ context menu need a right
  button the harness does not deliver; none is covered here.
- **Loaded-state and greyed-state comparison.** All captures are fresh-launch
  `/NOREG` with nothing loaded; which items grey with a file loaded, and the
  `(no bookmarks)`-style state-dependent rows, are not measured here.

---

## 10. Open questions (with the capture that would settle each)

1. **Main-menu `View file info` right hint.** Geometry x=246–288, y=92–105
   recorded; label unasserted after 3 OCR attempts returning 0 lines
   (**provisional**, §3.4). Settle with a higher-resolution capture of
   `003`-class popup (step `s04_mm_popup_posted` shape) plus a confirmed
   tight-crop OCR transcription at (245,90,296,107)@3× or better.
2. **Options `Repeat` / `Shuffle` hints.** `R` eye-only; `S` single-pass plus
   eye, both with 0-line crop OCR (**provisional**, §4.3). Settle with the
   same higher-resolution `009`-class capture plus confirmed crop
   transcriptions at (320,288,345,305)@3× and (320,322,345,340)@3×.
3. **Visualization-menu drawn hints.** Row labels settled; per-row hint-box
   transcription missing (**provisional**, §5). Settle with a hint-box OCR
   pass over a `012`-class capture.
4. **What the main-menu checks mean.** Three checks at x=16–29 settled as
   rendered facts; their binding to window-open state not established here.
   Settle with a toggle-then-recapture pair (e.g. close a window, recapture
   the `003`-class popup) mirroring the §4.2 check-move proof.
5. **Greyed state with media loaded.** Nothing greyed in the nothing-loaded
   state (§3.5); the loaded comparison was never captured. Settle with the
   planned loaded-state job (`s04-menus-loaded.json`: same five popups with a
   track loaded) against this run's captures.
6. **Nested submenu contents.** No submenu opened (two-`#32768` refusal).
   Settle with a same-class tie-break (foreground window) or a
   handle-pinned capture primitive, then one step per submenu.
7. **Click-through of any item.** No item click planned (dismissal shape).
   Settle with a two-step primitive (click in one step, capture the *parent*
   window in the next) plus before/after state read-back.
8. **Right-click menus.** No right-button gesture exists. Settle with a
   right-button variant of the posted and real-mouse gestures, then one
   open-plus-popup pair per menu.
9. **Metrics at a second DPI.** All sizes are 150% facts. Settle with the
   same job on a differently scaled host (the guest inherits the host
   display; not settable from the job).
10. **EQ preset submenu contents and load/save semantics.** Only the
    top-level `Load` / `Save` / `Delete` rows settled. Settle per question
    6, plus saved-state read-back (`preamp`, `eq_data`, `autoload_eq`,
    `use_eq`) before and after a load; S07 owns the semantics.

---

## 11. Run record — run 20261016-011612 (job s04-menus.json)

Guest: Windows Sandbox, dpi 144 (150%), screen 3050x1668, per-monitor-v2,
stable start to end. Harness vm-capture 1.2.0, install `C:\winamp`,
reference `C:\winamp295` (read-only mapping). **Landmark gate: PASSED** —
region 0,49,275,67, 0 differing pixels of 18425, size 275x116, twins
byte-identical, settle stable after 1.3 s. Status `steps-failed`, exit 5;
30 steps; every popup verdict `no-baseline` (new measurements).

| Step | Exit | Capture | Client size | Settles |
|---|---|---|---|---|
| `s04_mm_open_posted` (posted click (6,3) window-menu button) | 0 | `002_mm_open_posted_main.png` | 275x116 | the opening click delivery |
| `s04_mm_popup_posted` | 0 | `003_mm_popup_posted_menu.png` | 323x569 | main-menu rows, checks, arrows, hints (§§2–3) |
| `s04_mm_popup_posted_twin` | expect_change FAILED (0 px) | `004_…_twin_menu.png` | 323x569 | determinism (§1.5) |
| `s04_mm_open_real1` / `s04_mm_open_real2` | 0 / 0 | `005` / `006_mm_open_real*_main.png` | 275x116 | real-click path staging |
| `s04_mm_popup_real` | 3 | — (no capture) | — | negative: no real-click main popup in this material (§1) |
| `s04_opt_open_posted` (posted click (14,25) letter O) | 0 | `008_opt_open_posted_main.png` | 275x116 | the opening click delivery |
| `s04_opt_popup_posted` | 0 | `009_opt_popup_posted_menu.png` | 371x348 | Options rows and checks (§4.1) |
| `s04_opt_popup_posted_twin` | expect_change FAILED (0 px) | `010_…_twin_menu.png` | 371x348 | determinism (§1.5) |
| `s04_vis_open_posted` (posted click (14,56) letter V) | 0 | `011_vis_open_posted_main.png` | 275x116 | the opening click delivery |
| `s04_vis_popup_posted` | 0 | `012_vis_popup_posted_menu.png` | 399x151 | visualization rows (§5) |
| `s04_vis_popup_posted_twin` | expect_change FAILED (0 px) | `013_…_twin_menu.png` | 399x151 | determinism (§1.5) |
| `s04_eqp_open_posted` (posted click (239,24) PRESETS) | 0 | `014_eqp_open_posted_eq.png` | 275x116 | the opening click delivery |
| `s04_eqp_popup_posted` | 0 | `015_eqp_popup_posted_menu.png` | 176x105 | presets top level (§6) |
| `s04_eqp_open_real1` / `s04_eqp_open_real2` | 0 / 4 | `016_eqp_open_real1_eq.png` / — | 275x116 | real-click staging (exit-4 size gate on the second) |
| `s04_eqp_popup_real` | 0 | `018_eqp_popup_real_menu.png` | 176x105 | posted-vs-real equivalence (§6) |
| `s04_pl_misc_open_posted` (posted click (110,92) MISC) | 0 | `019_pl_misc_open_posted_pe.png` | 275x116 | the opening click delivery (popup itself absent) |
| `s04_pl_misc_popup_posted` | 3 | — (no capture) | — | negative: posted click opens no MISC popup (§1) |
| `s04_pl_misc_open_real1` / `s04_pl_misc_open_real2` | 0 / 0 | `021` / `022_pl_misc_open_real*_pe.png` | 275x116 | real-click staging |
| `s04_pl_misc_popup_real` | 0 | `023_pl_misc_popup_real_menu.png` | 474x81 | MISC rows (§7) |
| `s04_eqtitle_open_posted` / `s04_pltitle_open_posted` | 0 / 0 | `024_eqtitle_open_posted_eq.png` / `026_pltitle_open_posted_pe.png` | 275x116 | title-bar click delivery |
| `s04_eqtitle_popup_posted` / `s04_pltitle_popup_posted` | 3 / 3 | — | — | negatives: no EQ/PE title-bar menu (§1) |
| `s04_timerem_posted` (posted click (60,32) time display) | 0 | `028_timerem_posted_main.png` | 275x116 | the toggle delivery |
| `s04_opt_after_toggle_open_posted` | 0 | `029_opt_after_toggle_open_posted_main.png` | 275x116 | re-open delivery |
| `s04_opt_popup_after_toggle` | 0 (12640 px vs `009`) | `030_opt_popup_after_toggle_menu.png` | 371x348 | check-mark move + hover band (§4.2) |

No `S04-menus-audit.md` existed at spec time (checked); the loaded-state job
(`s04-menus-loaded.json`) has no run in this material.

---

## 12. Sources relied on

- `audits/S04-menus-reading.md` (the offline reading: row order, bboxes,
  gutter scans, OCR attempts — the sole rendered-fact source of §§2–8)
- `tools/vm-capture/_runs/20261016-011612/` (`captures/manifest.jsonl` for
  sizes and delivery paths; `DONE.json` for step verdicts and display facts;
  `bootstrap.log` for class `#32768` targets, points and the landmark gate)
- `drafts/S04-menus.md` (offline draft: resource-table trees, capture plan,
  harness limits — cited for scope boundaries only, not for rendered facts)
- `specs/S03-shortcuts.md` (action names the menu rows correspond to)
- `specs/S06-playlist.md` (§4: the MISC popup's playlist-window
  relationship and the refuted sort/randomise expectation)

*End of spec. Nothing executable, no source code, no pseudo-code was
included; every fact names its capture and step; provisional, visual-reading
and open values are marked as such, with the settling capture named.*
