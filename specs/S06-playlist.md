---
Spec ID: S-06
Title: Playlist-window behaviour (title bar, bottom-bar buttons, list selection, scrolling, removal, dialogs, message exposure)
Sources consulted (category only): dirty room: binary behaviour capture (sandbox runs 20261016-080820, 20261016-081527, 20261016-085037); dirty room: offline capture reading (OCR + pixel arithmetic); dirty room: existing audited specs S01 (rounds 2, 3, 3b) and S04; harness: tools/vm-capture README (method and limits)
Author agent: S06 dirty-room spec agent (draft revisions, 2026-10-16)
Auditor: Spec Auditor (dirty room)
Audit date: 2026-10-16
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561, extracted from winamp295.exe SHA-256 ef1423999cbefd4f2a3a98acee2b194a4260268cb4517abda9f12963e866b7e5)
Status: audited
---

# S06 — Playlist-window behaviour

Scope: what the playlist editor window (`Winamp PE`, 275 x 116) *does* — its
title bar, its five bottom-bar buttons (ADD, REM, SEL, MISC, LIST OPTS), list
selection, scrolling, entry removal, the dialogs and popup it opens, and what
the window class exposes to messages. Geometry and sprite rectangles are in
S01 (§4, rounds 2/3/3b); menu *contents* overlap S04; docking is S05.

Everything marked **settled** was measured by driving the running 2.95 in the
Windows Sandbox and comparing manifest-listed captures pixel by pixel (zero
tolerance), or read from window metadata (class, title, client size) in the
run logs. Everything marked **provisional** rests on a measurement whose
meaning is not established. Everything marked **open** was not measured and no
negative result is implied. A value read by eye or by OCR from a capture is
marked **visual reading** and is never load-bearing on its own.

---

## 0. How to read this spec

Evidence grades used:

| Grade | Meaning |
|---|---|
| **settled** | Direct measurement in a sandboxed 2.95 run, with the step or capture named |
| **provisional** | Measured, but its meaning is not established; the missing piece is named |
| **open** | No evidence of any kind; the capture that would settle it is named |
| **visual reading** | A string, row label or shape read from a capture by eye or by OCR, not produced by the comparator |
| **corroboration** | Two independent measurements agreeing, where neither alone would settle the point |

Runs interpreted here (all at dpi 144 / 150%, per-monitor-v2; landmark gate
region 0,49,275,67 passed in each):

| Run | Job | Result |
|---|---|---|
| 20261016-080820 (R1) | 29 targeted playlist steps | landmark gate passed; 21 of 29 steps ok; the ten failures are classified measurement negatives or job-design artefacts (§0.1) |
| 20261016-081527 (R2) | `s06-playlist.json`, 32 steps | status `steps-failed`, exit 5: all 32 steps exit 0 but the two dialog-twin `expect_same` assertions failed (25 px each, §7). 30 of 32 assertions clean |
| 20261016-085037 (R3) | `s06-rem.json`, 15 steps | landmark gate passed; 13 of 15 steps ok (`rem_dropdown` exit 3, `rem_item_click` skipped by `depends_on`); settle 14.5 s |

Pixel figures are whole-window differing-pixel counts from the harness
comparator unless a bounding box says otherwise. Window titles, classes and
client sizes come from `bootstrap.log` and the manifest, not from the client
pixels. Dialog title-bar strings come from window metadata (the client-area
capture carries no caption text).

### 0.1 Method note: job-design artefacts retired, not exported as facts

The following negatives are explained by the harness and the window's
activation behaviour, and carry no behavioural claim:

- A lone first posted click on a non-foreground playlist window is often
  swallowed by activation (S01 round 3). This explains the R1 ADD, LIST OPTS,
  REM and SEL first-click negatives (exit 3 / 0 px), not a missing capability
  (Verified by R2, where two pinned `mouse_event` real clicks — activation
  then action — open or fire every button).
- An R1 MISC probe at (112,92) reported `hitIsTarget=False`: the point sits on
  the `WinampVis` child window, which covers client x 52–123 at y = 92
  (S01 §4.2). R2 moved the real MISC point to x = 110 and fired.
- R2's 0-px REM second click is explained: R2 staged no selection before its
  REM clicks (fresh launch, nothing to remove), while R3 staged one and S01's
  positives all ran against selected rows (§5.3).

---

## 1. Title bar: no window-menu button (Q1)

**Settled, with qualification** (Verified by R1 steps 4–6 in run
20261016-080820):

- A posted left click at (6,3) on `Winamp PE` was delivered (step 4
  pre-flight `hit=(6,3) hitIsTarget=True`) and no `#32768` popup existed
  afterwards (`pe_tb_mm_popup` / `_twin` exit 3). The playlist title bar
  therefore carries no window-menu button, matching the S04 finding on the
  neighbouring windows.
- Qualification: this rests on one timing in R1 (R2's `pe_tb_mm_click` exits 0
  with no popup assertion and contributes nothing). It is a single-timing
  negative, cited to the R1 steps above.

---

## 2. List selection: single click and SEL (Q4, Q14)

**Settled** — the single click is the strongest cell in this spec:

| Behaviour | Evidence |
|---|---|
| Single posted click at (60,28) selects the first visible row | **2918 px**, bbox (12,23)-(254,93) (Verified by R2 step 26 vs step 25; reproduced three times: R2 step 26 and R3 `rem_select_control` / `rem_posted_select`, each 2918 px vs fresh) |
| SEL (second real click at (74,92)) selects all visible rows | click2-vs-click1 **12203 px**, bbox (12,23)-(254,93); after-vs-before **13591 px**; step 16 byte-identical to step 15 (Verified by R2 steps 14–16). "All" is a visual/blue-band inference consistent with the counts |

- Selection highlight colour (0,0,198), one row band at x ≈ 12–254,
  y ≈ 23–34: **visual reading** (offline pixel scan of
  `026_pe_list_single_click.png` vs `025_pe_list_single_before.png`), stated as
  corroboration only.
- Row order is unchanged by the single click: **visual reading** (same four
  row labels before and after, OCR best-effort on the small skin font —
  `57 longtltle?`, `tl`, `tone`, `ref` — uncertain strings marked `?`).

---

## 3. Scrolling: one arrow step is one page (Q8)

**Settled as a pixel symmetry; the row count is a visual reading** (Verified
by R2 steps 26–29):

- Up-arrow click at (263,81): **1501 px** vs the bottom-page baseline.
  Down-arrow click at (263,87): **1501 px** back, returning to the bottom
  page. Bounding box (12,25)-(267,77) in both directions.
- "One page of four rows": **visual reading** — OCR reads the top visible row
  moving 57 → 53 → back to 57 ("approximately 4 rows") on the uncertain skin
  font. The 1501/1501 px symmetry is solid; the four-row count is an OCR
  reading of the rows, not a comparator verdict, and is labelled as such.
- Handle drag from (263,20) to (263,60) moves the view toward the top of the
  list (first visible entries read `44 ref`, `45 tl`, `47` — **visual
  reading**); it moves the view, not a selection. No separate dragged-handle
  sprite is established here (S01 §4.1: the handle is the same sprite moved).

---

## 4. The dialogs and the MISC popup (Q2, Q15, Q16)

**Settled for existence, size and reproducibility; contents are visual
readings** (Verified by R2 steps 7–8, 19–20, 23–24 in run 20261016-081527):

| Button | Opens | Class / client size |
|---|---|---|
| ADD (second real click at (23,92)) | `Add file(s) to playlist` | `#32770`, 1503 x 778 (manifest + auditor PNG size) |
| LIST OPTS (second real click at (242,92)) | `Load Playlist` | `#32770`, 1503 x 778 |
| MISC (second real click at (110,92)) | `#32768` popup, empty window title | 474 x 81, captured twice with **0 differing pixels** (steps 19–20); auditor SHAs equal (`9fec8448…` both) |

- The "titles reversed vs button captions" note (ADD opens an *Add* dialog,
  LIST OPTS opens *Load Playlist*) is recorded as observed from the window
  metadata.
- MISC popup contents, **visual reading** (single OCR-engine pass over
  `019_pe_misc_popup.png`, no `?` marks in the source read — attach an OCR
  confidence caveat; menu-font strings from one engine only): two rows —
  `Generate HTML playlist` with hint `(Ctrl+Alt+G)`, and `Read extended info
  on selection` with hint `(Ctrl+Alt+E)` — separated by a horizontal band at
  approximately y = 40 (**visual reading** of the pixel scan). No submenu
  arrows were legible; only the shortcut hints appear on the right.
- Expectation gap, recorded honestly: the capture plan expected
  sort/randomise entries in the MISC popup. The 474 x 81 capture holds the two
  rows above and no sort/randomise text. That expectation is **refuted for
  this capture**; the sort/randomise effects (Q3) and the per-item menu
  semantics (Q15) stay open (§8).

---

## 5. REM removes the selected entry on the action click; both paths agree (Q13)

**Settled**, with the restatement in §5.2 (Verified by R3 steps 003/004 and
010/011/012 in run 20261016-085037, corroborated by byte identity with the
frozen S01 states):

### 5.1 The removal

- Real path: `004_rem_open_click2` differs from `003_rem_open_click1` by
  **3475 px**, bounds (12,23)-(254,93) — the list band only. The post-click
  state (`007`) is byte-identical to it (SHA `0a4355ddaa862f65…` =
  recorded round-3b real-click removal `081_o5real_rem_click2_pe.png`).
- Posted path: `011_rem_posted_click2` differs from `010_rem_posted_click1`
  by **3475 px**, same bounds (12,23)-(254,93). `012` is byte-identical to
  `011` (SHA `6ac754b2c11acb06…` = frozen S01 grid removal
  `008_o5b_grid_x046_pe.png` from run 20261015-163106). Same bytes, same
  state: the posted pair removes standalone, outside the cumulative grid
  context.
- The two SHAs differ by the title-band activation component only; the
  list-band change is the same 3475 px (`007` vs select = 4863 = 1388 + 3475).
- The action click is the **second** click of the pair: the first real click
  only activates (1388 px, title band (6,4)-(271,19) — Verified by R2
  `012_pe_rem_after` vs `009_pe_rem_before`), and the first posted click is a
  no-op (0 px — Verified by R3 `010` vs select).

### 5.2 Restatement (binding): what "no menu" means here

REM removes the selected entry with **no `#32768` window present at capture
time and no extra block in the click-1 frame; no menu choice is required.**
`rem_dropdown` exit 3 (`no visible top-level window with class '#32768'`)
plus `rem_item_click` skipped on `depends_on` proves no menu item was ever
clicked, yet both removals fired — so removal-through-dropdown-item in its
pre-declared shape is refuted, and "no menu choice required" is proved.

What is *not* claimed: the exit-3 probe proves no top-level `#32768` window
existed **at capture time**, and the click-1 pixel math (003 vs fresh = 4306
= 1388 + 2918 exactly, bbox (6,4)-(271,93)) proves the click-1 **frame**
contains no extra block. Neither proves a transient never rendered
(open-then-close before capture is unexcluded). Whether the S01 round-3
rendered block (1428 px at x 40–64, rows 32–99, class never established —
S01 §7.21) is a transient or state-dependent stays open, explicitly off the
removal path.

### 5.3 Context dependence

Without a staged selection the button is a no-op: R2's REM second click
changed 0 px (nothing staged), R3's changed 3475 px (one row staged). R2 vs
R3 therefore support, rather than contradict, each other.

### 5.4 The 2162 px after-state

**Settled as arithmetic; the row-level renumber is cited, not re-measured**
(Verified by R3 `007_rem_state_after` vs `001_rem_state_before` = 2162 px,
bounds (6,4)-(271,70)):

- 2162 = 1388 (title-band activation, §5.1) + 774 (residue in rows 23–70).
- The residue is the renumbering after the removal (rows 57–60 → 57–59, no
  selection): cited from the S01 round-3b read of the same byte state, not
  re-measured in R3. The bounds cut-off at row 70 is pixel coincidence (rows
  below happen to match fresh), not a semantic boundary.

---

## 6. `Winamp PE` message exposure: `WM_USER` is non-discriminating (Q9)

**Settled as a consistency observation, not as a proved refusal** (Verified
by R1 IPC probe steps in run 20261016-080820):

- `Winamp PE` returns **0** to the `WM_USER` probe, consistent with the EQ
  window's non-forwarding behaviour.
- Correction carried: the control probe to the main window also returns 0
  (the loaded list is dead, so the player sits in the stopped state), and the
  probe asserted a reply of 0 — so a refusal that answers 0 matches
  **silently**, not loudly. Refusal vs a genuine 0 reply cannot be separated
  in this list state. The draft's "asserted 0 makes refusal a loud mismatch"
  is backwards and is not exported.
- Export wording: *PE returns 0, consistent with EQ non-forwarding; refusal
  vs genuine-0 not discriminated in this list state; one IPC id only.*
  Everything else about `Winamp PE` message exposure (`WM_COMMAND` ids,
  `WM_NOTIFY`, drag-and-drop) is open (§8, Q10).

---

## 7. Determinism caveat: the 25 px caret blink

**Settled as a caveat, not a defect** (Verified by the R2 dialog twins,
auditor re-measured):

- `pe_add_dialog_twin` and `pe_listopts_dialog_twin` each differ from their
  first frame by exactly **25 px**: a single column at x = 286, y 676–700,
  black (0,0,0) ↔ white (255,255,255). That is the blinking text caret in the
  dialog's edit field at a different blink phase, not a layout difference;
  everything else of each 1503 x 778 dialog is byte-identical, at identical
  position and cause in both dialogs.
- Consequence pinned: the twin remains a useful identity assertion **only
  with a caret exclusion** (a `landmark_region` excluding the edit strip, or
  a stated 25-px tolerance at that strip). As a bare `expect_same` it is a
  phase-dependent flaky gate. The steady-state pin (rename the field, or wait
  for a steady capture) is open (§8).

---

## 8. Open questions

1. **Q3 — Sort/randomise effects.** The MISC capture holds two rows and no
   sort/randomise text (§4). The effect of each sort/randomise entry on a
   loaded multi-entry list (order, stability, selection after sort,
   duplicates/dead handling) was never driven. Settle with a posted click on
   a menu item inside a captured popup plus before/after list captures.
2. **Q5 — Double-click semantics.** **Provisional, not settled.** R1 double
   (step 24 vs 23: 4465 px, bbox (6,4)-(271,93), title band included) differs
   unexplained from R2 double (step 27 vs 26: 3077 px, bbox (12,23)-(254,93),
   list band only); the auditor cross-check (`027` vs `025` = 364 px, bounds
   (12,25)-(68,70)) is consistent with the double clearing the single-click
   highlight back near baseline, but whether a double click plays, edits or
   opens file info is not established from counts alone. The reading's
   row-level claims are OCR-uncertain.
3. **Q6 — Drag semantics.** **Provisional, not settled.** Both drags (R1 step
   25, R2 step 28: 4932 px, bbox (6,4)-(271,93)) include the title band, so
   the capture mixes activation/focus with the drag effect — an
   activation confound. "Reorder vs selection block" cannot be stated from
   counts alone, and the reading's row-order change is OCR-uncertain.
4. **Per-item REM submenu effects** (`REM MISC` / `REM ALL` / `CROP` /
   `REM SEL` individually, as rendered in the S01 click-1 block). The item
   click was correctly skipped by design (no `#32768` existed); the opener of
   a `#32768` REM menu, if one exists, is still unknown.
5. **SEL submenu items** (`INV SEL` / `SEL ZERO` / `SEL ALL` / …). Never
   driven; beyond the all-select of §2 nothing about SEL visuals or
   multi-selection is settled (Q11, Q14 remainder).
6. **Q7 — Keyboard.** Cannot be measured with the current primitives (no
   keyboard input primitive): up/down, page up/down, enter, delete, ctrl-A
   and all other key behaviour stay open.
7. **Q10 remainder — other message exposure.** Only one `WM_USER` IPC id was
   probed (§6). `WM_COMMAND` ids handled, `WM_NOTIFY`, file drag-and-drop and
   right-click context menus (no right-button gesture in the harness) stay
   open.
8. **Natural end of file.** The reference playlist is dead entries, so no
   track ever plays to a natural end; playback-to-end behaviour is not
   exercised anywhere in this spec.
9. **Dialog-twin steady-state pin.** The 25-px caret exclusion of §7 is
   pinned as a requirement; the steady capture itself (renamed field or
   waited phase) was not taken.
10. **Click-1 transient nature.** Whether the S01 rendered block is a
    transient menu or state-dependent (S01 §7.21 counterpart) stays open,
    explicitly off the removal path (§5.2).
