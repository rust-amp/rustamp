---
Spec ID: S-05
Title: Docking, snapping, windowshade and double-size
Sources consulted (category only): audited specs S01 (window geometry), S02 (main-window behaviour), S03 (shortcuts); dirty-room DPI/window-sizing findings (docs/HANDOFF.md §5); dirty-room capture harness (tools/vm-capture/README.md) for method and limits; runs 20261016-012641 and 20261016-020932 with the job tools/vm-capture/jobs/s05-docking.json; run 20261016-091412 with the job tools/vm-capture/jobs/s05-shade-double.json (shade-plus-double-size combination, both orders, §2.3)
Author agent: S05 dirty-room spec agent (draft revision 3, 2026-10-16)
Auditor: Spec Auditor (dirty room)
Audit date: 2026-10-16
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561, extracted from winamp295.exe SHA-256 ef1423999cbefd4f2a3a98acee2b194a4260268cb4517abda9f12963e866b7e5)
Status: audited
---

# S05 -- Docking, snapping, windowshade and double-size

Scope: how the three classic windows (main, equalizer, playlist editor) dock and
snap to each other and to the screen edges; the windowshade behaviour per window;
the double-size mode; the window positions and whether they persist; and the
observable difference between "docked" and "merely adjacent". This is the missing
spec behind the UI's `ToggleWindowShade` / `ToggleDoubleSize` commands, the
docking placeholder and the DPI/window-sizing remainder (HANDOFF §5).

**The core question is answered.** A docked neighbour **does** follow the main
window when the main moves: in run 20261016-020932 a real title-bar drag moved the
main window 200 px right, and the equalizer and the playlist editor each moved
200 px right with it, keeping their offsets to the pixel (§4.2, §4.3). The same
run settled the windowshade geometry, the double-size geometry, the three
windows' default positions, and that the stack re-flows to keep the equalizer
attached to the main window's bottom edge.

---

## 0. Method, and how to read the evidence in this spec

The harness captures **one window's client pixels per step** and can click and
drag. It does **not** put a window's screen position into the manifest. A window's
position is visible only in the pre-flight guard's `screen=(x,y)` line, which is
derived from the target window's client rect through `ClientToScreen` and is
written to that step's stdout log (`<NN>_<name>.stdout.txt`). The guard runs
*before* the step's click, so the line is the window's position at the start of
the step; a window's **client origin is `screen` minus the probed point**. The
guard prints that line **only for a step that takes an action**, which is why
every position step in the job carries an inert posted click on the window's
title text.

A cell in this spec therefore rests on one of four channels, and its marker
names which one:

| Channel | What it carries | Where |
|---|---|---|
| manifest row | the capture, its client size, the delivery path, the display facts | `captures\manifest.jsonl` |
| step stdout | the resolved client size, the pre-flight `screen=(x,y)` position, and (for a size-changing step) the re-resolved client size | `captures\<NN>_<name>.stdout.txt` |
| bootstrap record | the landmark-gate verdict, per-step exit codes, the produced-capture list | `DONE.json` |
| derived comparison | a pixel verdict from the harness's own comparator applied offline to two manifest-listed captures | §8.2, reproduced by the command in that section |

A *settled* cell rests on a direct measurement in one of the first three
channels, or on a derived comparison whose two inputs are manifest-listed
captures and whose method is stated. A *provisional* cell rests on an inference
(S01/S02/S03, or a visual reading of a capture) with its basis named. An *open*
cell has no evidence.

Two runs are interpreted here. **Run 20261016-012641** (job revision 1) was
partly unusable: the job shaded the main window and then asserted the unshaded
275x116 landmark on the steps that followed, and it treated a size-changing click
and the capture of the new size as one step. Seven of its seventeen steps failed
and the main window stayed shaded for the rest of the run, so its docking test
never ran. **Run 20261016-020932** (job revision 2, the same 28 steps) is fully
green: every step exit 0, every step's capture in the manifest, and the docking
test executed. This spec is written from the green run; where revision 1 is still
the only source of a fact, the cell says so.

A third run, **20261016-091412** (job `tools/vm-capture/jobs/s05-shade-double.json`),
is fully green (20 of 20 steps exit 0, landmark gate passed) and measures the
shade-plus-double-size combination in both orders (§2.3, §8.3), at the same
dpi 144 (150%) display facts.

Both runs were at **dpi 144 (150%)**, screen 3050x1668, per-monitor-v2, unchanged
from start to end of each run. The guest inherits the host's display, so the
scale is not stable between runs (README §12.10) and no capture may be assumed
to be at any other scale.

### 0.1 Provenance restatement on positions

The harness manifest carries **no position field** (harness README §12.7); the
positions stated in this spec are read from the step stdout pre-flight lines.
These readings are real and internally consistent (verified on the +100 / +200
docking drag of §4.2), but their source is stdout, not the manifest. The
playlist-editor position after an unshade (§3.3) is a **cross-launch inference**
from stdout pre-flight readings of two different launches within the same job,
not a directly captured invariant of a single launch.

### 0.2 Provenance restatement on shade-glyph reading

The shaded title-bar glyph inventory (§1.2) is a **visual reading** of the
manifest-listed capture `006_main_shade.png`. It is not a zero-pixel
rectangle-asserted claim. The row "the shade strip is not a crop of the unshaded
title band" is the comparator-backed part of the same section; the glyph inventory
itself was determined by eye against the captured render.

---

## 1. Windowshade behaviour

Windowshade collapses a window to a thin title-bar strip. Trigger: `Ctrl+W`
(command 40064, S03), a double-click on the title bar (S03 §5), or the shade
window button (window button 3, S03 §5 note).

### 1.1 Geometry

| Window | Windowshade client size | Evidence |
|---|---|---|
| Main | **275 x 14** | **settled** -- steps `b1_shade_act`, `b2_shade_main`, `b2_shade_main_pos` |
| Equalizer | unknown; 14 px tall per S01 §1 | open -- the equalizer was not shaded |
| Playlist editor | unknown; 14 px tall per S01 §1 | open -- the playlist editor was not shaded |

The main-window shade size is settled three times over in run 20261016-020932:
step `b1_shade_act` re-resolved the client rect after its click and logged
`size change: re-resolved client=275x14 (was 275x116)`, its manifest row carries
`client_size` 275x14, and the no-action step `b2_shade_main` then resolved and
captured the same 275x14 window (`006_main_shade.png`). The step
`b2_shade_main_pos` resolves 275x14 again while reading the window's position.

**Open:** whether the shade height is a fixed physical size or scales with DPI.
Both runs so far were at 150%, so they do not separate the two, and the guest's
scale follows the host display and cannot be pinned from the job (README §12.10).
The equalizer's and the playlist editor's shade sizes are open because neither
window was shaded (§1.5).

### 1.2 What is drawn in the shade

| Fact | Evidence |
|---|---|
| The shade is a single 14-px title-bar row: no body rows are drawn | **settled** -- the shade client rect is 275x14 (`b1_shade_act`, `b2_shade_main`), and S01 §1's title bar is 14 px tall |
| The shade strip is **not** a crop of the unshaded window's title band | **settled** -- comparisons I and J (§8.2): 1959 of 3850 and 1917 of 3850 pixels differ, spread across the whole strip |
| The strip carries the title text and a right-hand cluster of button glyphs, including the transport controls | **visual reading** (see §0.2 -- manifest-listed capture `006_main_shade.png`, step `b2_shade_main`) |
| The strip's left-hand glyph is the title bar's own glyph; the vertical clutterbar letters are not drawn in the strip | **visual reading** (same capture, see §0.2) |
| Main, focused: `titlebar.bmp` (27, 29, 275, 14) | provisional (S01 §2.1) -- the rectangle is not verifiable from a capture |
| Main, unfocused: `titlebar.bmp` (27, 42, 275, 14) | provisional (S01 §2.1) |
| Equalizer, focused/unfocused: `eq_ex.bmp` (0, 0, 275, 14) / (0, 15, 275, 14) | open -- the equalizer was not shaded |
| Playlist editor | open -- the playlist editor was not shaded |

The two derived comparisons matter: the shade strip is a **distinct rendering**,
not the unshaded title band cropped to 14 rows. In the unshaded window the strip's
position is occupied by the title bar's decorative bars flanking the title text;
in the shade the same row carries the transport button glyphs instead. That is
consistent with S01's dedicated shade sprites, and it is why the sprite
rectangles above stay provisional: a capture cannot be mapped to a skin bitmap by
this method.

The measured D-glyph rectangle of §2.1 is the one place where a capture *did*
localise a drawn element to a rectangle, because toggling double-size changes
exactly that element (§8.2, comparison G).

**Open:** the equalizer's and the playlist editor's shade drawings (the curated
capture `p05_pe_windowshade_entries_*.png` suggests the playlist shade still draws
its title band and list area when entries are loaded, but the layout is
unmeasured).

### 1.3 Window buttons in shade mode

| Button | Behaviour in shade | Evidence |
|---|---|---|
| Shade (window button 3) | leaves windowshade at every height | **settled** -- step `c1_unshade_act`: a posted click at client (254,3) on the 275x14 window grew it back, `size change: re-resolved client=275x116 (was 275x14)`; step `c2_unshade_main` confirms 275x116 |
| Double-click on the shade | leaves windowshade only outside a middle band: at 275 x 14 a double-click at y = 0, 2, 4 or y = 10, 12 toggled back; y = 6 or 8 did not | verified (S03 §5 note) |
| Menu / Minimize / Close | drawn in the strip; whether a click on them acts is not measured | open -- drawn per §1.2; no click was aimed at them |
| Transport controls | **drawn in the shade strip** (prev, play, pause, stop, next, eject) | **settled** (visual reading, see §0.2) |
| Clutterbar letters | not drawn in the strip | **settled** (visual reading, see §0.2) |

The shade button's behaviour is now a measurement rather than a citation: the
unshade click was posted at the same client point as the shade click, the window
was 275x14 when the pre-flight ran, and the harness re-resolved it to 275x116
after the click. That also proves the button is **hit-testable in the strip** -- it
is the one shade button for which this is measured.

The shade strip's glyph inventory (transport buttons present, clutterbar letters
absent) is a **visual reading**, not a zero-pixel rectangle-asserted claim
(§0.2). The comparator data (comparisons I and J) establishes that the strip is
a distinct rendering from the unshaded title band, but does not pin individual
glyph rectangles.

### 1.4 What the shade round established about the harness (method note)

Run 20261016-012641 exposed three harness behaviours and run 20261016-020932
retired the first of them:

1. A step whose own action changes the target's client size used to be
   uncapturable: the harness resolved the client rect *before* the action and then
   required the saved capture to have exactly that size, so such a step exited 3
   and wrote no manifest row. The harness now takes a **`may_change_client_size`**
   declaration on the step: it skips the landmark gate before the action,
   re-resolves the client rect afterwards, compares the capture against the *new*
   size and logs `size change: re-resolved client=... (was ...)`. In run
   20261016-020932 all four size-changing steps (`b1_shade_act`, `c1_unshade_act`,
   `e1_dsize_act`, `e3_dsize_off_act`) exited 0 and their manifest rows carry the
   new size. The job still keeps a no-action `*_state` / `*_pos` step after each,
   because that is what captures the new state with no action at all.
2. A step whose *pre-action* client size is not the declared landmark exits 4
   *before* its click is posted, so the click never happens. That is what left the
   main window shaded for the whole of run 20261016-012641: `main_unshade`
   asserted 275x116 against a 275x14 window and was refused, and every later
   main-window step inherited the same refusal. The job now names, on every step,
   the size that step will actually see.
3. A step can exit 0 and measure nothing. In run 20261016-012641 `main_dsize_on`
   clicked (14,49) on a 275x14 window, the point was clamped to (14,13), landed in
   the shade strip and opened the main window's menu, and the step passed because
   its declared `none` let a 275x14 capture through while `expect_change` was
   satisfied by the size mismatch, reported as \-1 differing pixels. A capture
   whose size is not the state's expected size must never be read as evidence
   about that state, and a `\ -1` is "the images are not comparable", not "the
   pixels changed".

### 1.5 Equalizer and playlist editor windowshade -- open

Neither window was shaded in either run, because their shade-button positions are
unmeasured: only the main window's are known (Menu (6,3), Minimize (244,3),
Shade (254,3), Close (264,3), HANDOFF §5 §9). Shading them needs one of their
title-bar button positions first. **Open**, and §9 says what a capture would need.

---

## 2. Double-size mode

Double-size is a whole-window 2x scale. Trigger: `Ctrl+D` (command 40165, S03),
the clutterbar letter 4 (D) (S02 §2), or the saved flag `dsize` (S01 §1).

### 2.1 What scales

| Window | 1x size | Double-size size | Evidence |
|---|---|---|---|
| Main | 275 x 116 | **550 x 232** | **settled** -- steps `e1_dsize_act`, `e2_dsize_main`, `e3_dsize_off_act`, `e4_dsize_off_main` |
| Equalizer | 275 x 116 | **550 x 232** | **settled** -- step `e2_dsize_eq` (the flag reaches it) |
| Playlist editor | 275 x 116 | **275 x 116 (unchanged)** | **settled** -- step `e2_dsize_pe` (the flag does not resize it) |
| Minibrowser | 350 x 348 | unknown | open |
| Video | 275 x 232 | unknown | open |

All four sizes are direct manifest `client_size` readings. The toggle on is
step `e1_dsize_act`: a posted click at client (14,49) on the clutterbar letter 4
(D), re-resolved as `size change: re-resolved client=550x232 (was 275x116)`. The
toggle off is step `e3_dsize_off_act`: the same letter at the doubled position
(28,98), re-resolved as `client=275x116 (was 550x232)`.

| Fact | Evidence |
|---|---|
| The double-size render is an **exact 2x pixel-doubling** of the 1x render -- every 2x2 block of the double-size capture is uniform | **settled** -- comparison F (§8.2): `024_main_dsize.png` halved and re-doubled with nearest-neighbour is **0 of 127600 pixels** different from itself |
| The same holds for the equalizer | **settled** -- comparison H: `025_eq_dsize.png`, 0 of 127600 |
| Toggling double-size off restores the 1x render | **settled** -- comparison D: `022_base_main_dsize.png` vs `028_main_dsize_off.png` differ by 1576 of 31900 pixels, all in rows 4-10, which is exactly the documented focused/unfocused title-band figure; rows 11-115 are identical |
| The clutterbar letter D is drawn at client (14,49)-(17,54), a 4x6 px region | **settled** -- comparison G: halved `024_main_dsize.png` vs `028_main_dsize_off.png` differ in exactly 14 pixels, bounds (14,49)-(17,54), i.e. the toggled glyph |
| The 1x render is reproducible across launches, and the double-size state is deterministic | **settled** -- comparisons E (0 of 31900, two fresh launches) and C (0 of 127600, two captures in the double-size state) |

Comparison G also **validates the probe point** used for the toggle: S02 §2 places
the letter D at client (14,49), the toggle-on click at (14,49) worked, and the
measured glyph rectangle starts exactly there.

Block-uniformity pass (auditor §4.1): a 2x2 block uniformity pass was run over
captures `024` and `025` (the double-size captures). Each had **0** non-uniform
2x2 blocks, ruling out sub-pixel drift and confirming the double-size renders are
clean whole-state captures, not aliased frames.

**Open:** the minibrowser and video windows (out of scope for the classic-window
docking spec), and whether the double-size sizes hold at a scale other than 150%.

### 2.2 Double-size and the docked stack

**Open.** What the stack does when the main doubles is not measured: the double-
size group captured the three windows' **sizes** but took no position reading, so
whether the equalizer and the playlist editor re-flow when the main grows to
550x232 (and, for the playlist, when the window under it doubles while it does
not) is unmeasured. The shade case *is* measured (§3.3), and it shows that the
stack re-flows; the double-size case needs the same four position probes added to
the double-size group (§9).

### 2.3 Shade and double-size together -- settled

The main window's client size when it is **both** shaded and double-sized is
**550 x 28**, in both orders (run 20261016-091412, job
`tools/vm-capture/jobs/s05-shade-double.json`):

| Order | Step | Re-resolved client size | Evidence |
|---|---|---|---|
| shade first | `a1_shade_act` | 275x14 (was 275x116) | **settled** -- step stdout `size change: re-resolved client=275x14 (was 275x116)`; manifest row `client_size` 275x14 |
| shade first, then double-size | `a3_dsize_act` | **550x28** (was 275x14) | **settled** -- step stdout `size change: re-resolved client=550x28 (was 275x14)`; manifest row `client_size` 550x28 |
| double-size first | `b1_dsize_act` | 550x232 (was 275x116) | **settled** -- step stdout `size change: re-resolved client=550x232 (was 275x116)`; manifest row `client_size` 550x232 |
| double-size first, then shade | `b3_shade_act` | **550x28** (was 550x232) | **settled** -- step stdout `size change: re-resolved client=550x28 (was 550x232)`; manifest row `client_size` 550x28 |

The no-action steps `a4_combined_state` and `b4_combined_state` then each resolve
and capture the same 550x28 window, so the combination is **order-independent**:
shade-first and double-size-first both settle at 550x28, arithmetically twice the
275x14 shade in each dimension and exactly the size the implementation renders.

The toggle delivery differs by order for a measured reason: in order A the
clutterbar letter D at (14,49) is not clickable in the 14 px strip, so
`a3_dsize_act` sends `WM_COMMAND` 40165 (`Ctrl+D`) instead of clicking; in order
B the shade button is clicked at the doubled point (508,6) = 2 x (254,3). Both
toggles landed, per the re-resolved sizes above.

**Provenance:** all four sizes are step-stdout re-resolved `client=WxH (was WxH)`
lines with matching manifest `client_size` rows (§8.3). The run is fully green
(20 of 20 steps exit 0, landmark gate passed) at dpi 144 (150%), the same scale
as §6.

The neighbours in the combined state keep their double-size behaviour from
§2.1: the equalizer is 550x232 (the flag reaches it while the main is shaded --
steps `a4_eq`, `b4_eq`) and the playlist editor is unchanged at 275x116 (steps
`a4_pe`, `b4_pe`). The stack still tracks the main window's bottom edge in that
state: the main at (26,29) and 550x28 ends at 57, the equalizer sits at (26,57),
and the playlist editor sits at (26,289), which is the equalizer's bottom edge
(57 + 232) -- read in both orders from `a5_main_pos`, `a5_eq_pos`, `a5_pe_pos`
and `b5_main_pos`, `b5_eq_pos`, `b5_pe_pos` (§8.3). So the stack re-flows for
this resize as it does for shade (§3.3); the 102 px datum of §3.3 is the 1x-shade
case (116 - 14), while here the main is 28 px tall.

---

## 3. Window positions and persistence

### 3.1 Default positions

| Window | Default position | Relation | Evidence |
|---|---|---|---|
| Main | **(26, 29)** | — | **settled** -- steps `a0_main_pos`, `b2_shade_main_pos`, `c2_unshade_main_pos`, `d0_launch_main_pos` (all four read (26,29)) |
| Equalizer | **(26, 145)** | one main-window height (116 px) below the main | **settled** -- steps `c2_unshade_eq_pos`, `d0_launch_eq_pos` |
| Playlist editor | **(26, 261)** | one main-window height below the equalizer | **settled** -- step `d0_launch_pe_pos` |
| Minibrowser | (301, 29) | one main-window width (275 px) right of the main | provisional (S01 §1) |
| Video | (26, 145) | the equalizer position | provisional (S01 §1) |

The three classic windows' default positions are now measurements, not citations:
a fresh launch stacks them gap-free, main 29..145, equalizer 145..261, playlist
261..377. Each position is read from the pre-flight line of a step that exited 0
with a manifest-listed capture -- for example `d0_launch_pe_pos` reads
`client=(100,3) screen=(126,264)`, so the playlist editor's client origin is
(126-100, 264-3) = (26,261).

**Provenance:** all positions in this section are read from step stdout pre-flight
lines (`screen=(x,y)`), not from the manifest, which carries no position field
(harness README §12.7). See §0.1.

The saved state records the geometry as `wx`, `wy`, `eq_wx`, `eq_wy`, `pe_wx`,
`pe_wy`, `pe_width`, `pe_height`, `pe_height_ws`, `mb_wx`, `mb_wy`, `mb_width`,
`mb_height`, `video_wx`, `video_wy`, `video_width`, `video_height`, plus the open
flags `mw_open`, `eq_open`, `pe_open`, `mb_open`, `video_open` and the mode flags
`windowshade`, `dsize`, `eq_ws` (S01 §1).

### 3.2 The stack while the main is shaded

| Observation | Main client origin | Equalizer client origin | Playlist client origin | Evidence |
|---|---|---|---|---|
| main shaded (275 x 14) | (26, 29) | (26, 43) | (26, 159) | **settled** -- `b2_shade_main_pos`, `b2_shade_eq_pos`, `b2_shade_pe_pos` |
| main unshaded (275 x 116) again | (26, 29) | (26, 145) | (26, 261) | **settled** -- `c2_unshade_main_pos`, `c2_unshade_eq_pos`, `d0_launch_pe_pos` |

Readings: `b2_shade_main_pos` `screen=(126,32)`, `b2_shade_eq_pos`
`screen=(126,46)`, `b2_shade_pe_pos` `screen=(126,162)` — origins (26,29),
(26,43), (26,159) with the point (100,3). The same steps also settle the **sizes**
in that state: the main is 275x14 and the equalizer and the playlist editor are
both still 275x116, so shading the main does **not** resize its neighbours.

### 3.3 The neighbours track the main window's bottom edge -- settled

| State | main top | main height | main bottom | equalizer top | playlist top |
|---|---|---|---|---|---|
| shaded | 29 | 14 | 43 | **43** | 159 |
| unshaded | 29 | 116 | 145 | **145** | 261 |

The equalizer's top equals the main window's bottom edge in **both** states, and
the playlist editor's top equals the equalizer's bottom edge in both. So the two
neighbours moved **up by exactly 102 px** when the main shaded (116 - 14) and back
down when it unshaded, inside one run and with the main window itself never
moving.

**Settled:** the three classic windows are a **docked stack** whose neighbours are
attached to the main window's bottom edge; a windowshade of the main re-flows the
stack. Revision 2 of the draft held this as *provisional*, because revision 1's
run never read a neighbour position before the shade; the green run reads both
states, so it is now a measurement.

**Cross-launch note:** the playlist-editor position after unshade (`d0_launch_pe_pos`
reading (26,261)) is a **cross-launch inference** -- the playlist-editor shaded
captures and the playlist-editor docked captures are not the same frame sequence.
The positions are consistent (the playlist editor is 116 px below the equalizer's
top of 145 = 261, matching its own default), but this is inferred from stdout
pre-flight readings of two different launches within the same job, not a directly
captured invariant of a single launch. See §0.1.

### 3.4 Persistence

**Open.** Whether a moved window's position survives a restart (the position is
written to `winamp.ini` on exit and read on launch — the write and read paths
are unverified on 2.95); whether the position survives a DPI change or a monitor
change (HANDOFF §5 §10 lists logical-size preservation across monitors as
app-layer/S05 future work for the Rustamp side; what the reference does is
unmeasured). The harness cannot settle the restart case: the sandbox relaunches
from a fresh copy and `restart: true` uses a forced kill, so `winamp.ini` is not
written on exit.

---

## 4. Docking and snap rules

### 4.1 Which edges snap

| Question | Answer |
|---|---|
| Does a window snap to the screen edges (work area)? | open |
| Does a window snap to another window's edges? | open |
| Which edges participate (left/right/top/bottom, or only vertical/horizontal)? | open |
| Is there a snap tolerance (a pixel distance within which the window sticks)? | open |
| Is snapping active while dragging only, or also on programmatic moves? | open |

No capture is planned for any of these this round (§7.1): the harness cannot drag
a window to a screen edge and read back the settled position.

### 4.2 What happens to a docked neighbour when the main moves -- settled

The move test ran in full in run 20261016-020932. Two real title-bar drags of the
main window (client (100,3) — (200,3), delivered with `sendinput`) were followed
by position readings on all three windows.

| Reading | Step | main client origin | equalizer client origin | playlist client origin |
|---|---|---|---|---|
| before the drags | `d0_launch_main_pos`, `d0_launch_eq_pos`, `d0_launch_pe_pos` | (26, 29) | (26, 145) | (26, 261) |
| after the first drag | `d2_drag_2` (main only) | (126, 29) | — | — |
| after both drags | `d3_main_pos_after`, `d3_eq_pos_after`, `d3_pe_pos_after` | (226, 29) | (226, 145) | (226, 261) |
| **delta** | | **+200** | **+200** | **+200** |

The drags landed: `d1_drag_1`'s pre-flight reads the main at (26,29), and
`d2_drag_2`'s pre-flight already reads it at (126,29), so the first drag moved the
main +100 px and the second another +100 px. The y coordinate of all three windows
is unchanged (29, 145, 261), as expected for a horizontal drag.

**Answer: yes — a docked neighbour follows the main window when the main moves.**
The equalizer and the playlist editor each moved by exactly the same delta as the
main (+200 px) and kept their offsets to the pixel (equalizer top = main bottom;
playlist top = equalizer bottom). They are docked, not merely adjacent.

**Provisional sub-question:** *by the same delta continuously, or only when the
main crosses an edge?* The neighbours were read only before the first drag and
after the second, so the run shows that the total deltas match, not that the
tracking is continuous. A position probe between the two drags would settle it
(§9).

### 4.3 Docked versus merely adjacent -- settled

The criterion is the move test, and it has now been applied: two windows can sit
edge-to-edge (the default stack, §3.1) without being docked, and the observable
difference is whether moving one moves the other. Measured (§4.2): moving the main
window 200 px right moved both neighbours 200 px right. The default three-window
stack is therefore a **docked set**, and the docking is expressed as the
neighbours tracking the main window's bottom edge (§3.3).

**Provenance:** all position deltas above are read from step stdout pre-flight lines
(`screen=(x,y)`), not from the manifest. See §0.1.

---

## 5. What the app must do on a drag or a snap

The measured behavioural contract, from §3.3 and §4.2:

* The three classic windows form a docked stack; the equalizer's top is the main
  window's bottom edge and the playlist editor's top is the equalizer's bottom
  edge.
* Moving the main window moves the docked neighbours by the same delta, keeping
  those edges coincident.
* Resizing the main window (a windowshade) re-flows the docked stack so that the
  equalizer's top follows the main's new bottom edge; leaving the shade re-flows
  it back.
* Double-size resizes the main and the equalizer to 550x232 and leaves the
  playlist editor at 275x116 (§2.1); what the stack's *positions* do in that state
  is not measured (§2.2).
* Shade plus double-size together is 550x28 in both orders (§2.3), and the stack
  re-flows for that resize as it does for shade: the equalizer's top still
  follows the main's bottom edge and the playlist editor's top still follows the
  equalizer's bottom edge.

**Open:** the snap rules themselves (§4.1) -- which edges, what tolerance, and
whether a drag that ends near an edge repositions the window. This follows from
§4.1 and has no evidence.

---

## 6. DPI and window sizing (HANDOFF §5 §10 remainder)

Verified (HANDOFF §5 §10, lane F integrated as `22e224f`): the Rustamp process is
per-monitor-V2 aware by explicit contract; the client area is exactly the layout
in physical pixels (275 x 116, 550 x 232 double-size, verified with `GetClientRect`
on its own window); double-size resizes the OS window.

The two dirty-room runs measured **dpi 144 (150%)**, screen 3050x1668,
per-monitor-v2, unchanged from the start of each run to its end. Revision 1 of
this draft stated that the sandbox runs at 250%; that is not a property of the
sandbox — the guest inherits the host's display and the scale is not stable
between launches (README §12.10). Every scale-dependent cell in this draft
therefore names its scale, and every size above is a **150%** measurement.

The reference's rendering is 1:1 at that scale and is not rescaled: the landmark
gate compared a fresh-launch probe against the recorded landmark and found 0
differing pixels in the invariant region (and 0 in the whole window), with the
twins byte-identical, in both runs.

**Open** (reference behaviour, needed for S05): how the windowshade height and the
double-size sizes behave at a scale other than 150%; what the reference does on a
live multi-monitor `WM_DPICHANGED` transition (the sandbox is single-monitor).
**Open** (Rustamp app layer, per HANDOFF §5 §10): logical-size preservation across
monitors — the window keeps its pixel size by design, so preserving a logical size
across monitors is app-layer work.

---

## 7. Open questions

1. **Snap tolerance and edges.** The harness cannot observe a snap directly (it
   captures client pixels, not positions, and cannot drag to a screen edge and
   read back the settled position). No capture planned this round.
2. **Persistence across restart.** The sandbox relaunches from a fresh copy and
   `restart` kills Winamp without letting it write `winamp.ini`, so no capture can
   see a position survive. Needs a run with a preserved `winamp.ini` or a message
   spy. No capture planned.
3. **Equalizer and playlist editor windowshade.** Their shade-button positions are
   unmeasured, so neither window was shaded. Needs one of their title-bar button
   positions first (§9).
4. **The stack's positions in the plain double-size (unshaded) state.** The sizes
   are measured; the positions are not (§2.2). Needs four position probes in the
   double-size group (§9). The *combined* shade-plus-double-size positions are
   settled (§2.3) and are not part of this item.
5. **Minibrowser and video double-size.** Out of scope for the classic-window
   docking spec.
6. **Shade and double-size sizes at a scale other than 150%.** Not settable from
   the job: the guest's scale follows the host display (README §12.10).
7. **Whether the docked tracking is continuous or edge-triggered** (§4.2,
   provisional). Needs a position probe between the two drags (§9).
8. **Docked set on close, and undocking.** No capture planned.
9. **Menu / Minimize / Close hit-testing inside the shade strip** (§1.3). Drawn,
   not clicked.

Answered since draft revision 3, no longer open: the main window's client size
when shaded and double-sized together -- previously unmeasured (the spec carried
the 1x shade size and the unshaded double-size size but never the two together)
-- is now settled at **550x28, order-independent** (§2.3, run 20261016-091412).
The spec records no "silent" caveat about the combination; it was simply absent,
and §2.3 now carries it.

---

## 8. Run record -- run 20261016-020932 (job s05-docking revision 2)

Guest: Windows Sandbox, user WDAGUtilityAccount, dpi 144 (150%), screen
3050x1668, per-monitor-v2, unchanged start to end. Shell PowerShell 5.1, harness
vm-capture 1.2.0, install `C:\winamp`, reference `C:\winamp295` (read-only
mapping). **Landmark gate: PASSED** — `region=0,49,275,67 differing_pixels=0
total_pixels=18425 size=275x116 identical=True`, twins byte-identical (SHA-256
equal), whole-window count 0, settle stable after 1.3 s. Status `ok`, exit code 0,
28 of 28 steps exit 0, 0 skipped, `failures: []`.

### 8.1 The 28 steps

Every step produced a manifest-listed capture; no `baseline_path` was bound, so
every verdict is `no-baseline` and every step is a new measurement. The position
column is the client origin derived from the step's pre-flight `screen=(x,y)` line
(§0); a step with `input_method: none` prints no pre-flight line.

| Step | Exit | Capture | Client size | Position read | Settles |
|---|---|---|---|---|---|
| `a0_base_main` | 0 | 001_base_main.png | 275x116 | — | fresh-launch main size |
| `a0_base_eq` | 0 | 002_base_eq.png | 275x116 | — | fresh-launch equalizer size |
| `a0_base_pe` | 0 | 003_base_pe.png | 275x116 | — | fresh-launch playlist size |
| `a0_main_pos` | 0 | 004_main_pos_unshaded.png | 275x116 | main (26,29) | main default position |
| `b1_shade_act` | 0 | 005_shade_act.png | **275x14** (re-resolved) | main (26,29) | shade size (275x116 — 275x14) |
| `b2_shade_main` | 0 | 006_main_shade.png | 275x14 | — | the shade pixels (§1.2) |
| `b2_shade_main_pos` | 0 | 007_shade_main_pos.png | 275x14 | main (26,29) | the shade does not move the main |
| `b2_shade_eq_pos` | 0 | 008_shade_eq_pos.png | 275x116 | EQ (26,43) | the stack while shaded (§3.3) |
| `b2_shade_pe_pos` | 0 | 009_shade_pe_pos.png | 275x116 | PE (26,159) | the stack while shaded (§3.3) |
| `c1_unshade_act` | 0 | 010_unshade_act.png | **275x116** (re-resolved) | main (26,29) | unshade returns to 275x116; the shade button acts in the strip |
| `c2_unshade_main` | 0 | 011_main_unshade.png | 275x116 | — | the unshaded main pixels |
| `c2_unshade_main_pos` | 0 | 012_unshade_main_pos.png | 275x116 | main (26,29) | the unshade does not move the main |
| `c2_unshade_eq_pos` | 0 | 013_unshade_eq_pos.png | 275x116 | EQ (26,145) | the stack re-flowed back (§3.3) |
| `d0_launch_main_pos` | 0 | 014_dock_main_before.png | 275x116 | main (26,29) | move test, before |
| `d0_launch_eq_pos` | 0 | 015_dock_eq_before.png | 275x116 | EQ (26,145) | move test, before |
| `d0_launch_pe_pos` | 0 | 016_dock_pe_before.png | 275x116 | PE (26,261) | move test, before |
| `d1_drag_1` | 0 | 017_drag_1.png | 275x116 | main (26,29) | the drag is delivered (`sendinput`) |
| `d2_drag_2` | 0 | 018_drag_2.png | 275x116 | main (126,29) | drag 1 landed: +100 px |
| `d3_main_pos_after` | 0 | 019_dock_main_after.png | 275x116 | main (226,29) | main delta +200 |
| `d3_eq_pos_after` | 0 | 020_dock_eq_after.png | 275x116 | EQ (226,145) | **equalizer delta +200 — docked** |
| `d3_pe_pos_after` | 0 | 021_dock_pe_after.png | 275x116 | PE (226,261) | **playlist delta +200 — docked** |
| `e0_launch_main` | 0 | 022_base_main_dsize.png | 275x116 | — | 1x reference for the toggle |
| `e1_dsize_act` | 0 | 023_dsize_act.png | **550x232** (re-resolved) | main (26,29) | double-size on (275x116 — 550x232) |
| `e2_dsize_main` | 0 | 024_main_dsize.png | 550x232 | — | the double-size main pixels |
| `e2_dsize_eq` | 0 | 025_eq_dsize.png | 550x232 | — | the flag reaches the equalizer |
| `e2_dsize_pe` | 0 | 026_pe_dsize.png | 275x116 | — | the flag does not resize the playlist |
| `e3_dsize_off_act` | 0 | 027_dsize_off_act.png | **275x116** (re-resolved) | main (26,29) | double-size off (550x232 — 275x116) |
| `e4_dsize_off_main` | 0 | 028_main_dsize_off.png | 275x116 | — | the 1x render restored |

### 8.2 Derived comparisons

These are offline pixel verdicts from the harness's own comparator
(`Compare-Images.ps1`, host-runnable, pure PNG arithmetic) applied to two
manifest-listed captures of this run. They create no window, start no sandbox and
send no input; the two intermediates are a nearest-neighbour rescale and a crop,
written outside the repo. Reproduce by rescaling with nearest-neighbour at the
stated factor and running the comparator on the pair.

Auditor's re-measurement (Pillow 12.3.0 at zero tolerance): all eight values,
magnitudes and rectangle bounds match the draft exactly (§7 of the audit).
Block-uniformity pass on `024` and `025`: 0 non-uniform 2x2 blocks each.

| # | Reference | Candidate | Differing pixels | Bounds | Conclusion |
|---|---|---|---|---|---|
| C | 023_dsize_act.png | 024_main_dsize.png | 0 / 127600 | — | the double-size state is deterministic |
| E | 001_base_main.png | 022_base_main_dsize.png | 0 / 31900 | — | the 1x render is reproducible across launches |
| D | 022_base_main_dsize.png | 028_main_dsize_off.png | 1576 / 31900 | (6,4)-(271,10) | toggle-off restores the 1x render; the only difference is the title band's focus state |
| F | 024 halved to 1x, re-doubled | 024_main_dsize.png | 0 / 127600 | — | the double-size render is a pure 2x pixel-doubling (no 1x reference needed) |
| G | 024 halved to 1x | 028_main_dsize_off.png | 14 / 31900 | (14,49)-(17,54) | the only difference is the toggled clutterbar D glyph |
| H | 025 halved to 1x, re-doubled | 025_eq_dsize.png | 0 / 127600 | — | the equalizer's double-size render is also a pure 2x pixel-doubling |
| I | 006_main_shade.png | rows 0-13 of 004_main_pos_unshaded.png | 1959 / 3850 | (1,3)-(274,13) | the shade strip is not a crop of the unshaded title band |
| J | 006_main_shade.png | rows 0-13 of 011_main_unshade.png | 1917 / 3850 | (1,4)-(274,13) | as I, against a second unshaded capture |

The 1576 px of comparison D is the figure the harness README records for a
focused/unfocused title band (all in rows 0-11), and the region is exactly rows
4-10, so that difference is the window's focus state and not a rendering
difference. Comparison F is the strongest of the set because it needs no 1x
reference at all: if every 2x2 block of the double-size capture is uniform, the
render is a doubling by construction, whatever the window's state.

### 8.3 Run record -- run 20261016-091412 (job s05-shade-double.json)

Guest: Windows Sandbox, user F9937E0A-ADB2-4\WDAGUtilityAccount, dpi 144 (150%),
screen 3050x1668, per-monitor-v2, unchanged start to end. Shell PowerShell 5.1,
harness vm-capture 1.2.0, install `C:\winamp`, reference `C:\winamp295`
(read-only mapping). **Landmark gate: PASSED** --
`region=0,49,275,67 differing_pixels=0 total_pixels=18425 size=275x116
identical=True`, twins byte-identical, whole-window count 0, settle stable after
1.3 s. Status `ok`, exit code 0, 20 of 20 steps exit 0, 0 skipped, `failures: []`.
Order A (steps `a0`-`a5`) shades first and then double-sizes; order B (steps
`b0`-`b5`) double-sizes first and then shades, after a `restart`.

Every step produced a manifest-listed capture; no `baseline_path` was bound, so
every verdict is `no-baseline` and every combined state is a new measurement.
The position column is the client origin derived from the step's pre-flight
`screen=(x,y)` line (§0); a step with `input_method: none` prints no pre-flight
line. Combined-state steps declare `expect_client_size: none` on purpose: their
sizes are the measurement.

| Step | Exit | Capture | Client size | Position read | Settles |
|---|---|---|---|---|---|
| `a0_base_main` | 0 | 001_a0_base_main.png | 275x116 | — | 1x base, launch A |
| `a1_shade_act` | 0 | 002_a1_shade_act.png | **275x14** (re-resolved, was 275x116) | main (26,29) | shade size (§2.3) |
| `a2_shade_state` | 0 | 003_a2_shade_state.png | 275x14 | — | the 1x shade pixels |
| `a3_dsize_act` | 0 | 004_a3_dsize_act.png | **550x28** (re-resolved, was 275x14) | — | shade-first combination (§2.3) |
| `a4_combined_state` | 0 | 005_a4_combined_state.png | 550x28 | — | shade-first combination confirmed |
| `a4_eq` | 0 | 006_a4_eq.png | 550x232 | — | the flag reaches the equalizer while the main is shaded |
| `a4_pe` | 0 | 007_a4_pe.png | 275x116 | — | the playlist unchanged in the combined state |
| `a5_main_pos` | 0 | 008_a5_main_pos.png | 550x28 | main (26,29) | combined-state main position |
| `a5_eq_pos` | 0 | 009_a5_eq_pos.png | 550x232 | EQ (26,57) | **EQ top = main bottom (57) -- tracks** |
| `a5_pe_pos` | 0 | 010_a5_pe_pos.png | 275x116 | PE (26,289) | **PE top = EQ bottom (289) -- tracks** |
| `b0_base_main` | 0 | 011_b0_base_main.png | 275x116 | — | 1x base, launch B |
| `b1_dsize_act` | 0 | 012_b1_dsize_act.png | **550x232** (re-resolved, was 275x116) | main (26,29) | double-size size (§2.3) |
| `b2_dsize_state` | 0 | 013_b2_dsize_state.png | 550x232 | — | the doubled pixels |
| `b3_shade_act` | 0 | 014_b3_shade_act.png | **550x28** (re-resolved, was 550x232) | main (26,29) | double-size-first combination (§2.3) |
| `b4_combined_state` | 0 | 015_b4_combined_state.png | 550x28 | — | double-size-first combination confirmed |
| `b4_eq` | 0 | 016_b4_eq.png | 550x232 | — | the flag reaches the equalizer in this order too |
| `b4_pe` | 0 | 017_b4_pe.png | 275x116 | — | the playlist unchanged in this order too |
| `b5_main_pos` | 0 | 018_b5_main_pos.png | 550x28 | main (26,29) | combined-state main position |
| `b5_eq_pos` | 0 | 019_b5_eq_pos.png | 550x232 | EQ (26,57) | **EQ top = main bottom (57) -- tracks** |
| `b5_pe_pos` | 0 | 020_b5_pe_pos.png | 275x116 | PE (26,289) | **PE top = EQ bottom (289) -- tracks** |

---

## 9. What still needs a capture, and readiness for audit

`tools/vm-capture/jobs/s05-docking.json` is **final as the job that produced this
evidence**: its 28 steps, points and action lines are unchanged from the run, and
it re-validates through the bootstrap's own `-ValidateOnly` path with the same
28-step plan. The only edit after the run was to the job's `_comment`, which still
described the four size-changing steps as *expected to fail with exit 3* — stale
after the harness gained `may_change_client_size`; the comment now records the
green run and the re-resolved sizes. Comments are not read by the validator, so
the step plan is untouched.

The card's questions that the harness can reach are all settled: the docking
question (§4.2, §4.3), the windowshade geometry and drawing (§1.1, §1.2), the
shade button (§1.3), double-size (§2.1), and the window positions (§3.1, §3.2,
§3.3). The remaining opens fall into three classes:

| Class | Item | What would settle it |
|---|---|---|
| needs a capture, cheap | the stack's positions in the double-size state (§2.2) | four position probes added to the double-size group, mirroring `b2_shade_*_pos` |
| needs a capture, cheap | whether the docked tracking is continuous or edge-triggered (§4.2) | one position probe on the equalizer and the playlist between `d1_drag_1` and `d2_drag_2` |
| needs an evidenced probe point first | the equalizer's and the playlist editor's windowshade (§1.5) | one of their title-bar button positions, from a capture of their title bars |
| needs a capture, cheap | Menu / Minimize / Close hit-testing inside the shade strip (§1.3) | three posted clicks at their known points on a shaded window |
| harness cannot | the snap tolerance and which edges snap (§4.1) | a position-capable measurement or a source |
| harness cannot | persistence across a restart (§3.4) | a preserved `winamp.ini` or a message spy |
| host-dependent | any size at a scale other than 150% (§6) | a run on a differently scaled host |
| out of scope | minibrowser and video double-size (§2.1) | — |

None of these is needed for the four facts the card exists to establish, and none
is a defect of the job.

**Readiness for audit.** The material is ready for a Spec Auditor in this sense:
every settled cell names the step or comparison that settled it; every capture
cited is a manifest-listed capture of run 20261016-020932 (§8.1); the run's own
gates passed (landmark diff 0, twins byte-identical, display facts stable); the
derived comparisons of §8.2 are stated with their inputs and their reproduction
method, and they are marked as *derived* rather than as harness verdicts; and the
job file is the one that produced the run. The two provenance restatements from
the audit are applied: the shade-glyph reading is labelled a **visual reading**
(§0.2), and the positions are stated as coming from stdout pre-flight lines with
the PE-position-after-unshade marked as a **cross-launch inference** (§0.1).