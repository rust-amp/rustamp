---
Spec ID: S-07
Title: Equalizer interior, value laws, presets, band steps and EQ/DSP chain order
Sources consulted (category only): dirty room: binary behaviour capture (sandbox runs below); dirty room: shipped skin assets (test input); dirty room: existing audited specs S01 (rounds 2, 3, 3b, held-states run), S02 and S03; dirty room: audit note held-states-interpretation.md; docs/HANDOFF.md §§5 and 12 (12.12, 12.13, 12.15, 12.16); docs/rfc/0005-app-commands.md §16 (O10, O11); free: webamp (cross-check only)
Author agent: dirty-room S-agent (draft) with spec-editor pass over the held-states run
Auditor: Spec Auditor (dirty room)
Audit date: 2026-10-16
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561, extracted from winamp295.exe SHA-256 ef1423999cbefd4f2a3a98acee2b194a4260268cb4517abda9f12963e866b7e5)
Status: audited
---

# S07 — Equalizer interior, value laws, presets, band steps and chain order

Scope: the equalizer content that S01 deliberately left open after round 2
(the slider-track decomposition and body colouring, the dragged and held
sprites, the two graph line strips, the held On/Auto rects and the Presets
rect), the **volume and balance laws**, the EQ band and preamp steps, and the
**order of EQ and DSP in the signal chain**. Window geometry, exterior sprites
and the EQ panel's own geometry live in S01; main-window transport and
behaviour (including the `VOLUME:` / `BALANCE:` marquee hints) live in S02;
the shortcut tables that drive the EQ live in S03; menu trees belong to S04.

This spec states every load-bearing fact with its step or capture. A
provisional or inferred value is marked exactly so, with the capture that
would settle it. A value read from a hint string rendered by the skin (the
`VOLUME:` and `BALANCE:` marquee text) is read by eye and is labelled as such;
its agreement with the pixel-counted thumb positions is the corroboration.

---

## 0. How to read this spec

Two incompatible measurement regimes appear below and must not be confused:

- **Pixel comparisons** (the harness comparator, zero tolerance). The harness
  captures a window, compares it rectangle-by-rectangle against a baseline and
  reports a count of differing pixels plus the bounding box. These are
  objective and are quoted as `N / total`, `(x,y)-(x,y)`.
- **Hint-string readings** (the only on-screen numbers a slider exposes). The
  harness cannot read text, so every `VOLUMEn%` and `BALANCE: …` value below
  is a string read by eye from the rendered marquee strip and then
  transcribed. They are flagged *visual reading*. The integer laws were then
  derived from the independently pixel-counted thumb positions, which agree with
  the readings; a transcription error could still exist, which is why the thumb
  positions are the load-bearing corroboration and the readings are treated as
  corroboration only.

Evidence grades used:

| Grade | Meaning |
|---|---|
| **settled** | Direct measurement in a sandboxed 2.95 run, or an offline comparison whose two inputs are manifest-listed captures, with the step, comparison and method named |
| **provisional** | Reported or inferred, with its basis named; not confirmed by this spec's runs |
| **open** | No evidence of any kind; the settling capture is named |
| **visual reading** | A string or shape read from a capture by eye, not produced by the comparator |
| **corroboration** | Two independent measurements agreeing, where neither alone would settle the point |

**Session-noise calibration (correction carried).** In the held-states run a
fresh equalizer launch differs from a recorded baseline by **1435 px confined to
rows 4–11 (the title band) and by exactly 0 px in rows 30–115**. Any
whole-window count in the range ~1400–2000 px whose bounding box is the title
band is therefore session noise, not a sprite: read the bounding box and its
row profile, never the total. This invalidates the whole-window readings 300,
401 and 534 by themselves (§2).

**Held controls cannot be captured by a real-mouse drag in this harness.**
`Invoke-RealMouseAction` always releases the button in a `finally` before
`Invoke-Capture.ps1` runs, so any hold-time effect is gone before the frame. The
held-states run instead posted a **lone `WM_LBUTTONDOWN` (`0x201`)** through the
`send-message` primitive, with no `WM_LBUTTONUP`, leaving the control held
while the capture ran. Every held measurement below rests on that primitive.

---

## 1. Scope and carry-over

What this spec does not re-derive:

- S01 §3 and §3.1: the EQ panel's exterior geometry, the title-bar sprite pair,
  the band labels, the slider columns (`x = 22` preamp, `x = 79 + 18k` bands,
  18 px pitch), the groove, the thumb-y mapping (`y = 38` at saved value 0,
  `y = 63` at 31, `y = 89` at 63, ≈ 0.81 px per saved unit), the graph
  background and graph area, the band-value colouring endpoints (row 0 = value
  0, row 18 = value 63) and the saved-state keys `preamp` / `eq_data` /
  `autoload_eq` / `use_eq`.
- S02 §2.1: the marquee hint strings `VOLUME: <n>%` and
  `BALANCE: <CENTER or side>`; these are the only on-screen statements of a
  slider value and therefore the evidence the volume and balance laws rest on.
- S03 §3: the EQ shortcut rows (digit keys raise a band, QWERTYUIOP lower it,
  grave/Tab raise/lower the preamp, `N` toggles EQ enabled, `A` toggles
  autoloading, `S` opens the presets menu, `Ctrl+S` loads a preset).

---

## 2. Corrections carried from the held-states run

Run `20261016-050228` (`held-states.json`, 69/69 steps, `status=ok`,
`exit_code=0`, landmark gate `0,49,275,67` with 0 differing pixels of 18425;
client size 275×116 throughout; all `send-message` steps took the posted
branch). The corrections below supersede the working values that earlier runs
and the earlier S07 draft carried.

1. **Auto held rect corrected, On confirmed.** On held: **(14, 18, 25, 12)**,
   300/300 px changed, measured in both the engaged and the not-engaged held
   state and matching both recorded round-2 held captures at 0 px inside the
   footprint. Auto held: **(39, 18, 33, 12)**, 395/396 px changed, in both
   engagement states. The earlier working value **(40, 18, 32, 12)** is one
   column short on the left: 12 of the 395 px fall at `x = 39`, in the gap
   between the On button (which ends at `x = 38`) and the Auto body (which
   starts at `x = 40`). The rendering is identical to the recorded captures at
   0 px, so this is a width error in the footprint, not a different sprite.
   Engagement contributes only **6 px**: On (20, 24)–(22, 25), Auto
   (45, 24)–(47, 25). (Steps `on_down_*` / `on_held_*`, 00–09, and
   `auto_down_*` / `auto_held_*`, 10–19.)

2. **The "534 px" figure was never a held sprite.** The real held Presets
   sprite is **528 px** (a 44×12 footprint at window (217, 18)): a lone down
   changes all 528 px of that footprint and the same 528 px change back on
   release, and the held run reproduces the recorded `s01r2_eq_presets_pressed_eq.png`
   at 0 px inside the footprint. The 534 is **528 + the 6 px Auto cell**, where
   the run's own post-release frame disagreed with the recorded baseline. The
   decisive check is that the earlier run's capture differs from **its own**
   resting equalizer by **0 px inside (217, 18, 44, 12)**. The same
   decomposition explains the other two whole-window claims: 300 (On sprite, no
   cell term) and 401 (395 + 6). All three S07 "held" figures were
   post-release differences measured after the button was already up, not
   held-sprite facts. (Steps `presets_down` / `presets_held` / `presets_up`,
   64–66, versus the recorded baseline and versus the run's own flat frame.)

3. **Volume law.** Volume is **linear in pixels at 5 core units (0–255) per
   client pixel**: `v = 5 × (thumb_left − 107)`, clamped to 0…255; display
   percentage is `floor(v × 100 / 255)`; the value is **committed on the
   button down**, and a down inside the thumb starts a drag without jumping.
   The 0 % end is measured (thumb_left 107), the 100 % end is **predicted at
   thumb_left 158 (click x = 165) and was not measured** (§5).

4. **Balance law.** Track 38 px, thumb 14 px, travel 24 px, centre at
   thumb_left 189, extremes 177 and 201. Centre is written **`CENTER`**,
   off-centre as **`100% LEFT`** (a percentage plus the side). The right
   half, every intermediate value and the sign convention for right are **not
   measured** (§6).

5. **Preamp drag.** The dragged-versus-resting delta is **73 px** and
   reproduces the round-2 real-drag measurement exactly (pixel-identical in
   rows 30–115, i.e. the sliders, thumb, curve and body are unchanged). The
   pointer tracks the thumb **1:1 with a constant 5 px offset**
   (thumb_left = pointer_y − 5); the earlier **"target y minus 6"** is off by one
   against this single direction and single point, so it is recorded as weak.
   The drag commits the **preamp minimum**, not the maximum: the thumb moves
   toward the track's top (`y = 38`, saved value 0) and the graph carries the
   113-px full-width strip-colour run at row 35, which is what the recorded
   `s01r2_eq_min_eq.png` (preamp = 0) carries at row 35. Consequently
   README §12.16 has the min/max rows **swapped** ("preamp to max → row 35",
   "preamp to min → row 17"): the captures say min → 35, max → 17. This is a
   **documentation defect in the README**, not a statement about the player, and
   must not be read as a Winamp behaviour correction. (Steps `preamp_down` /
   `preamp_move` / `preamp_held` / `preamp_up`, 21–25, versus `s01r2d_realinput_eq_ondrag_mid_eq.png`
   and `_ondrag_after_eq.png`.)

6. **Session floor.** 1435 px, rows 4–11 only, and rows 30–115 at exactly 0 in
   every held-versus-baseline comparison. Whole-window totals cannot be used as
   sprite measures (item above). (Method note as stated in §0.)

---

## 3. Equalizer button states and held rects

### 3.1 Sheet sprites (from S01, not re-derived)

| Element | Sprite (sheet rect) | Window position |
|---|---|---|
| On, normal | `eqmain.bmp` (10, 119, 25, 12) | (14, 18) |
| On, engaged | `eqmain.bmp` (69, 119, 25, 12) | (14, 18) |
| On, held (not engaged) | `eqmain.bmp` (128, 119, 25, 12) | (14, 18) |
| On, held and engaged | `eqmain.bmp` (187, 119, 25, 12) | (14, 18) |
| Auto, normal | `eqmain.bmp` (36, 119, 32, 12) | (40, 18) |
| Auto, engaged | `eqmain.bmp` (95, 119, 32, 12) | (40, 18) |
| Auto, held (not engaged) | `eqmain.bmp` (153, 119, 33, 12) | (39, 18) |
| Auto, held and engaged | `eqmain.bmp` (212, 119, 33, 12) | (39, 18) |

### 3.2 Held rects as measured under a lone button-down (settled)

Captured with a lone `WM_LBUTTONDOWN` (0x201), no `WM_LBUTTONUP`, while the
capture ran; every step below is 0/69 failures in run `20261016-050228`.

| Button | Held rect (window) | Changed pixels | Engagement state |
|---|---|---|---|
| On | (14, 18, 25, 12) | 300 / 300 | same rect, held engaged **and** held not-engaged |
| Auto | (39, 18, 33, 12) | 395 / 396 | same rect, held engaged **and** held not-engaged |

Engagement difference at window level: **6 px and nothing else**.

| Button | Engagement cell (window) | Size |
|---|---|---|
| On | (20, 24)–(22, 25) | 3 × 2 |
| Auto | (45, 24)–(47, 25) | 3 × 2 |

Against the recorded round-2 held captures: On matches at 0 px inside the
footprint in both engagement states; Auto matches at 0 px inside the footprint
in both states (and also at 0 px inside the earlier working rect (40, 18,
32, 12), which is the cause of the earlier value: the render did not move).

**Provenance caveats, stated bluntly:**

- Two sheet cells that differ in only 6 px render identically in any window
  capture, so whether (128, 119, 25, 12)/(187, 119, 25, 12) and
  (153, 119, 33, 12)/(212, 119, 33, 12) are two sprites or one sprite with a
  6-px overlay cannot be decided from captures. Only the window-level 6 px is
  settled. (Open item O1 in §12.)
- The recorded baselines `s01r2c_eq_autoheld_on_eq.png` and
  `s01r2_eq_presets_pressed_eq.png` place the Auto indicator at
  **(44, 22)–(46, 23)**, while this run and `s01r2c_eq_onheld_on_eq.png` place
  it at **(45, 24)–(47, 25)** — a 1 px / 2 px disagreement *inside* the
  recorded set (that is the 6-px `y22 = 3, y23 = 3` term in the 1441-px
  whole-window verdicts). **Visual reading.**
- `s01r2b_msgON_eq.png` was captured with the preamp **not** at flat (its
  thumb sits near `y = 44`); the 430-px residual in that comparison is
  preamp/band state, not the On button. The On rect itself is 0 px there.
  (Provenance, not behaviour.)

---

## 4. Presets button and its popup menu

### 4.1 The held sprite: real, measurable, and no longer confused with 300 / 401 / 534

| Measurement | Value | Evidence |
|---|---|---|
| Window footprint that changes on a lone down | (217, 18, 44, 12) | `presets_down` (64) vs `presets_state` (63): 528 / 528 px, bounds (217, 18)–(260, 29) |
| Change back on release | same 528 px | `presets_up` (66) vs `presets_held` (65): 528 px |
| Reproduction of the recorded round-2 baseline | 0 px inside the footprint | `presets_held` (65) vs `s01r2_eq_presets_pressed_eq.png`: 0 px in (217, 18, 44, 12) |
| Against its **own** resting equalizer (earlier run) | 0 px inside the footprint | the earlier run's capture vs its own flat frame, (217, 18, 44, 12) |

Sprite rows (from S01): normal **(224, 164, 44, 12)** at (217, 18);
engaged **(224, 176, 44, 12)** at (217, 18). Whether a distinct *sheet*
rect corresponds to the held sprite (it occupies the same window position as
the normal button) is not separable from these captures; do not invent one.

Whole-window decompositions (for provenance only — session noise excluded):

| Claim | Decomposition |
|---|---|
| 300 (On held) | On sprite, absent from the post-release frame; no cell term |
| 401 (Auto held) | 395 (Auto at the corrected 33 px) + 6 (On indicator cell at the round-2 position) |
| 534 (Presets held) | 528 (real held sprite) + 6 (Auto indicator cell) |

All three were measured after the button was already up (the release-in-`finally`
behaviour of the real-mouse primitive), hence differences of *state*, not of
held sprites.

### 4.2 The popup menu

Pressing the Presets button (measured 2026-10-16, run
`20261016-024437`, steps `eq_preset_dismiss` → `eq_preset_open` →
`eq_preset_popup`) opens a popup menu of window class **`#32768`** in the same
process:

| Property | Value | Evidence |
|---|---|---|
| Window class | `#32768` | pre-flight `hitClass=#32768` at (239, 24) |
| Client size | 176 × 105 (window 182 × 111) | `eq_preset_popup`, client-size read |
| Top-level entries, top to bottom | **Load**, **Save**, **Delete** | visual reading of the 176 × 105 capture |
| Each entry | carries a submenu arrow | visual reading of the same capture |

The contents of the submenus, their labels beyond the three top-level entries,
the command ids and the loading/saving path behaviour (whether loading a preset
touches `autoload_eq`, the preset file on disk, or the AUTO toggle) belong to
S04 / remain open. (Open item O6.)

---

## 5. Volume law (S02/S01 open question closed)

Held-state measurements, run `20261016-050228`, steps `vol_down_*` /
`vol_held_*` / `vol_up_*` (clicks 108, 122, 135, 148, 161). The displayed
figures below are **visual readings** of the marquee hint; the integer law is
derived from the pixel-counted thumb positions and the two agree.

| Click x (client) | Visual reading `VOLUME:` | Thumb top-left (measured, released frame) |
|---|---:|---:|
| 108 | 0 % | (inside the thumb; see click rule) |
| 122 | 15 % | 115 |
| 135 | 41 % | 128 |
| 148 | 66 % | 141 |
| 161 | 92 % | 154 |

**Law (settled, subject to the visual-reading caveat of §0).** Thumb is 14 px
and is drawn centred on the click, clamped to the track:

- `thumb_left = click_x − 7`, clamped to the track;
- **`v = 5 × (thumb_left − 107)`**, engine domain 0..=255: **exactly 5 core
  units per client pixel**, linear, clamped at both ends;
- the three consecutive 13-px steps (115, 128, 141, 154) each change v by 65,
  i.e. **5.000 units / px**;
- display percentage = **`floor(v × 100 / 255)`** (all four non-degenerate
  readings match to the digit);
- **0 % at `thumb_left = 107`** (any click with click_x ≤ 114 clamps there).

**Commit rule (settled).** The value is committed on the **button down**; the
released frames already carry the moved thumb, and their difference from the
resting state is the full track difference (386 / 452 / 452 / 452 px at
(107, 58)–(170, 68)). The smaller down-vs-released difference
(~183–191 px) is only the held-state chrome clearing as the hint disappears.

**Click inside the thumb does not jump (settled).** Click 108 produced 0 px
released-vs-resting: the down landed inside the thumb (client x 107–120), so it
started a drag instead of jumping. The same rule (§6) holds for the balance
slider.

**Endpoints.** The left endpoint (0 %) is **measured**. The right endpoint
(100 %) is **predicted at `thumb_left = 158`, i.e. click x = 165**, and was
**not measured**: the rightmost measurement (click 161) reads 92 %, so the
earlier draft's assumption that x = 161 is the maximum is **wrong by 8 %**
and its step labels `volume_hold_064/128/192/255` are wrong by construction.
The true values at x = 122 / 135 / 148 / 161 are
**40 / 105 / 170 / 235 = 15 / 41 / 66 / 92 %**. Clicks at 165 and 168 (to
confirm 100 % and the right clamp) are still open (§12, open item O3).

**Geometry note.** The track rectangle recorded in S01
((107, 57), 68 × 13) stays **provisional**; the held run only gives the
moved thumb's band, (107, 58)–(170, 68) (14 × 11). The sheet-frame
mapping (28 frames of 15 rows plus the 13-row thumb strip) is not resolved
here and stays an S01 open question.

---

## 6. Balance law (S02/S01 open question closed — partially)

Held-state measurements, run `20261016-050228`, clicks 178, 184, 190, 196,
202. Visual readings of the marquee hint, with the thumb positions pixel-counted
independently.

| Click x (client) | Visual reading `BALANCE:` | Released vs resting |
|---|---|---|
| 178 | `100% LEFT` | 346 px, (177, 58)–(214, 68) |
| 184 | `100% LEFT` | same 346 px |
| 190 | `CENTER` | 0 px |
| 196 | `CENTER` | 0 px |
| 202 | `CENTER` | 0 px |

**Geometry (settled for width and left edge, not for the exact vertical band).**

- Track **38 px (x 177 … 214)**, thumb **14 px**, travel **24 px**, centre at
  `thumb_left = 189`, extremes **177** and **201**.
- S01's "38 × 13 at (177, 57)" is confirmed in width and left edge; the
  measured changed band is **y 58 … 68 (11 rows)**, not 13 rows at y = 57.
- The left clamp is proven: click 178 → `thumb_left` 171 → clamped to 177.

**Wording (settled).** At centre the hint is the word **`CENTER`** (three
independent points). Off-centre the wording is **`100% LEFT`** — a percentage
plus the word `LEFT`. By symmetry the right side is expected to read
`100% RIGHT`, but **that is not measured**.

**Commits (settled).** The left / 25-left released frames differ from the
resting state by the same 346 px, so the thumb stays at the left extreme after
the release; the value is taken on the down, as with volume (§5).

**Right half: unmeasured.** Clicks 190, 196 and 202 all landed **inside** the
default thumb (189 … 202) and produced **0 px** — they started a drag and never
jumped. Consequently:

- there is **no reading for any right-hand value**, no intermediate percentage
  and no sign convention for right;
- only **two** distinct balance states were measured (left extreme and centre),
  not five (`bal_left` and `bal_25l` are the same measurement);
- the engine-side balance domain (signed, `±100 %` versus `±127`) is invisible
  to the hint and stays open. (Open item O4: clicks ≥ 208 plus 201 / 204 /
  208 for the integer law and sign convention.)

---

## 7. Interior that stays open (carried from the draft)

The following material is restated from the S07 draft without new measurements.
Nothing in it has changed in status.

### 7.1 Slider track decomposition

| Item | Prior evidence | Status |
|---|---|---|
| Track column on screen | preamp x = 22, bands x = 79 + 18k | verified (S01 r2) |
| Groove source column | column 14, identical to 15, sheet y = window y + 191 | verified (S01 r2) |
| Resting thumb | (0, 164, 11, 11) | verified (S01 r2) |
| Dragged thumb | (0, 176, 11, 11) | verified (S01 r2) |
| Thumb y, value 0 / 31 / 63 | 38 / 63 / 89 | verified (S01 r2) |
| Track body extents and tiling above/below the thumb | 63 px body suggested from 52 px travel + 11 px thumb; the 13×129 source rectangle already fails as a single blit | **open — to be measured** (see §12, O5) |
| Top / bottom 1-px caps | suspected, not named as a sprite | **open** |
| Whether body fills tile or are recomputed per value | not determined | **open** |
| Whether anything is drawn **above** the thumb | S01 reports the fill runs thumb-to-bottom only | **open — to be measured** |

### 7.2 Per-band body colouring as a function of the saved value

S01 r2 established the red (top of travel) through orange and yellow to green
(bottom) scale, with endpoints at row 0 (value 0) and row 18 (value 63). The
exact function, and whether preamp and bands share one ramp, remains open.

| Saved value (0..63) | Thumb y | Body fill run | Status |
|---:|---:|---|---|
| 0 | 38 | thumb to track bottom, row 0 colour | open (endpoint) |
| 15 | ≈ 49.7 | to be measured | open |
| 31 | 63 | to be measured | open |
| 46 | ≈ 76.3 | to be measured | open |
| 63 | 89 | to be measured (row 18 colour) | open (endpoint) |

Additional items open (carried verbatim from the draft):

- whether band-value and preamp-value scales are the **same ramp** (the preamp
  behaves as a band in the captures, but this is not proven);
- whether the colour is recomputed **per frame during a drag** or only on
  release;
- whether the flat-state curve colour uses the same ramp rows as the body
  (S01 r2 reports the flat curve at rows 7–9, minimum at row 0, maximum at
  row 18);
- the quantisation from a 0..63 integer to one of the 19 ramp rows (nearest,
  floor, or band-specific table).

### 7.3 Graph strips

| Strip | Status |
|---|---|
| Response background, `eqmain.bmp` (0, 294, 113, 19) at window (86, 17) | verified with explained residual (flat curve + horizontal reference line), S01 r2 |
| Colour scale, `eqmain.bmp` (115, 294, 1, 19), 19 rows / 12 colours | verified as colour scale only, never blitted; S01 r2 |
| Flat curve rows | 7–9; minimum-value curve row 0; maximum-value curve row 18; S01 r2 |
| Non-flat curve row assignment, smoothing / interpolation between the ten band frequencies, sign (negative above or below centre), and whether the preamp is applied to the curve at all | **open** (O7) |
| Horizontal strip, `eqmain.bmp` (0, 314, 113, 1) | **closed as not decidable by capture with this harness**: a 1-px blit and a line in that colour are pixel-identical by construction. Decidable only by a synthetic skin whose `(0, 314, 113, 1)` row is marked, loaded by the reference; a skin and a loader primitive are both missing. (O8.) |

### 7.4 Graphical response behaviour still open

- Exact mapping from each of the 0..63 saved band values (and from the preamp)
  to curve y, individually and combined.
- Smoothing / interpolation between the ten band frequencies.
- Whether negative values flip the curve above centre.
- Whether the preamp is applied to the curve or only to the signal.

---

## 8. Dragged thumb and preamp drag

### 8.1 Dragged-thumb offset

S01 r2 recorded **target y minus 6**. The held-states run gives a second point
and disagrees by one:

| Measurement | Pointer y → thumb top-left y | Offset |
|---|---|---|
| S01 r2 real drag (single direction) | — | "target y minus 6" (recorded) |
| Held run, button down then move | 68 → 40, pointer Δ −23, thumb Δ −23 | **5 px** (`top_left = pointer_y − 5`) |

The held-states reading is **one direction at one point** and the earlier one is
the only point in the opposite convention; treat both as weak until a second
drag direction (downward) is captured. (Open item O9: a downward preamp drag.)

### 8.2 Preamp drag committed the minimum (settled where noted)

Observed held-states run, steps 21–25 (`preamp_down` / `preamp_move` /
`preamp_held` / `preamp_up`), versus `s01r2d_realinput_eq_ondrag_mid_eq.png`
and `_ondrag_after_eq.png`:

| Fact | Measurement |
|---|---|
| Dragged-vs-resting delta | **73 px**, (22, 63)–(32, 73); identical on the lone down (no actual move required) |
| Reputation of the round-2 real drag | rows 30–115 pixel-identical to the real-input captures; the whole 1441-px verdict decomposes into the title band (1435, rows 4–17) plus the 6-px Auto cell (rows 22–23) |
| Tracking | pointer y 68 → 45 (Δ −23), thumb top-left 63 → 40 (Δ −23) → **1:1 in y, constant 5 px offset** |
| Commit | mid-drag and released row profiles are identical (thumb top-left y = 40 in both); the value is taken from the thumb, not from the pointer |
| Committed value | **preamp minimum**: thumb moved toward y = 38 (saved value 0); the graph carries a **113-px full-width run of the strip colour (186, 203, 221) at row 35**, which is exactly the row the recorded `s01r2_eq_min_eq.png` (preamp = 0) carries it on |
| Maximum reference | the recorded `s01r2_eq_max_eq.png` (preamp = 63) carries the same strip at **row 17** |

**README §12.16 is wrong about the rows.** It states "preamp to max → row 35" and
"preamp to min → row 17"; the captures say the opposite (min → 35, max → 17).
This is a documentation defect in that README table, not a Winamp behaviour
claim, and it must not be cited as a 2.95 fact. Do not implement from the
swapped rows.

Cross-check: the released graph differs from the flat graph by 117 px and from
the minimum by 218 px — a flat curve shifted down by the preamp cut, with all
bands still at 31. (Steps 21–25 vs the two round-2 real-drag captures.)

---

## 9. EQ band step and pan step (not measurable by capture here)

S03 §3 gives the key bindings; the step *size* is not yet measured and no
capture step is planned for it in this spec revision.

| Item | Status |
|---|---|
| Band step, commands 40267 … 40276 (raise one band) / 40279 … 40288 (lower one band) | **open**; candidate one saved unit on the 0..63 scale (also one pixel ≈ 0.81 units is equally plausible) |
| Preamp step, commands 40277 (raise) / 40278 (lower) | **open**, including direction (S03 already noted unresolved; S07 has not resolved it) |
| Pan step, commands 40313 (left) / 40314 (right) | **open** |
| Pan command delivery to the slider | the keyboard binding is provisional in S03 |

Measuring this needs real or posted keyboard input (`WM_KEYDOWN` on the EQ
window) plus a read-back of the saved EQ value (`preamp`, `eq_data`) before and
after each press, and a reserved section in a capturing manifest; image
captures alone cannot read saved values. (Open item O10; this is an S03/RFC
0005 follow-up, not an S07 surface.)

---

## 10. EQ versus DSP order in the chain

This is **not an imaging question.** No shipped DSP plugin is present in the
stock 2.95, and neither captures nor posted mouse messages expose a per-plugin
signal, so an order cannot be observed from window pixels. The S09/S10
runtime-check list names it as an open runtime check.

**Conceptual contract fixed by this spec (implementation still provisional).**
Until a measured answer exists one and only one of three positions applies, and
the answer must be stated rather than left implicit:

- EQ runs on the **decoded PCM stream before the final output scaling** that
  implements the volume and balance laws (§5, §6);
- any DSP that runs must see **one** of: the pre-EQ stream, the post-EQ stream,
  or the post-volume stream;
- the selected position is named in that crate's contract (RFC 0005 O11 area);
  it is not silently "all of them".

The engine keeps its provisional law for both gain and ordering until the
measured determination (audio-level or spectrum comparison through `out_disk`,
or a plugin's reported signal). (Open item O11.)

---

## 11. Visual-reading provenance (single source of truth)

Numbers that were **not** produced by the comparator and therefore carry the
visual-reading caveat:

| Value | How it was obtained |
|---|---|
| 0 / 15 / 41 / 66 / 92 % (volume) | strings read by eye from the `VOLUME: …` marquee strip at clicks 108 / 122 / 135 / 148 / 161 (table §5) |
| `100% LEFT`, `CENTER` (balance) | strings read by eye from the `BALANCE: …` marquee strip at clicks 178 / 184 / 190 / 196 / 202 (table §6) |
| Auto indicator position (44, 22)–(46, 23) in the recorded set | eye reading of the recorded `s01r2` captures, flagged as a disagreement |
| Preamp colour (186, 203, 221) at row 35 (min) | colour of a full-width run read by eye, cross-checked against the recorded min capture and against the strip-colour identity |
| Presets popup entries Load / Save / Delete and their submenu arrows | visual reading of the 176 × 105 capture (§4.2) |
| Auto-track titles on the PRESETS entries | visual reading, not a comparator figure |

Every one of these is **corroborated** by an independent pixel measurement:
the integer volume law reproduces all four readings to the digit; the 346-px
left-extreme released frame and the 0-px centre frames agree with the `100% LEFT`
/ `CENTER` boundary; the min-graph 113-px run matches the preamp-0 capture;
the 528-px held sprite change is pixel-counted. A remaining transcription
error on a single digit cannot be excluded by the harness, and is named as
such.

---

## 12. Open questions (with the capture that would settle each)

1. **Held-sprite sheet identities** (engaged vs not-engaged overlay, 6 px).
   Decide whether the 6-px cell is a second 3×2 sprite or an overlay on one
   25 / 33-px button sprite. Window capture cannot separate the two. Settle by
   extracting the sheets (`eqmain.bmp` (128, 119, 25, 12) vs (187, 119, 25, 12)
   and (153, 119, 33, 12) vs (212, 119, 33, 12)) and comparing the 6 px
   directly. (O1, not planned here.)
2. **Auto indicator position disagreement.** `s01r2` recorded set places it at
   (44, 22)–(46, 23); the held run places it at (45, 24)–(47, 25). Re-capture
   with a preserved `winamp.ini` controlling `autoload_eq` in one run, reading
   the solved-state bytes rather than matching sheets. (Provenance open.)
3. **Volume right end.** Clicks at 165 and 168 to confirm 100 % and the right
   clamp (and consequently clicks 165, 168 for the 100 % prediction stated in
   §5). Lane: a further `send-message` lone-down job group, one step per click
   plus a released-capture control.
4. **Balance right half, all intermediate values, sign convention.** Clicks ≥ 208
   for `100% RIGHT`, plus 201 / 204 / 208 for the integer step. Same kind of
   job group as §12.3. (O4.)
5. **Body-colour function** and whether preamp and bands share one ramp. Settle
   by capturing the 11 recorded equality pairs (preamp values 0, 15, 31, 46, 63
   on a single band, and band values 0, 15, 31, 46, 63 in isolation) and
   extracting the sheet row. A synthetic skin marking the ramp colours would make
   the function readable; neither exists. (O5.)
6. **Whole track single-blit and tile recipe.** Needs a synthetic-marked skin;
   no such skin is in the reference copy. (O5.)
7. **Graph response mapping, smoothing, sign, preamp-to-curve.** Not decidable
   from captures alone unless the skin colours are read directly; needs
   saved-state read-back before and after. (O7.)
8. **Horizontal strip** `eqmain.bmp` (0, 314, 113, 1). Closed as not decidable
   by capture; needs the synthetic-skin experiment of O8.
9. **Dragged-thumb 5-vs-6 offset.** A downward preamp drag (and, for
   cross-check, upward and downward band drags) to settle the direction
   dependence. (O9.)
10. **Band step size, preamp direction, pan step.** Keyboard-driven run with
    saved-value read-back. (O10, S03.)
11. **EQ/DSP order.** Audio-level or spectrum comparison via `out_disk`, or a
    plugin's reported signal; not reachable with the image-and-mouse harness. (O11.)
12. **Preset load semantics.** Whether loading a preset changes `preamp`,
    `eq_data`, `autoload_eq` and `use_eq`, and whether the window close saves
    the current values as a preset, and the contents of the Load / Save / Delete
    submenus. S04 owns the menu tree.
13. **Engagement setter.** `autoload_eq` toggles the Auto cell at (44,22)–(46,23)
    exactly (measured). The corresponding On cell (18–20, 22–23) is **associated
    with** `use_eq` in the same saved-state file but was not independently
    re-verified (S01 round 3 open item 7.22). Open until a dedicated save/restore
    round-trip is captured. (§1.)

---

## 13. Run record (evidence this spec draws on)

### 13.1 Runs

| Run | Job | Status | Used for |
|---|---|---|---|
| `20261016-050228` | `held-states.json` | 69/69, `status=ok`, landmark gate passed, 0 px differing (18425), dpi 144, 3050×1668, per-monitor-v2 | Held rects (§3.2), Auto correction, Presets 528 / 534 (§4.1), volume and balance laws (§5, §6), preamp drag (§8.2), session floor (§0, §2.6) |
| `20261016-024437` | `s07-eq-dsp.json` (rev 2) | 18/18, gate passed | Presets popup menu (§4.2); volume/balance drags did not expose hints (clarifies why the 300 / 401 / 534 figures were post-release) |
| `20261016-014557` | `s07-eq-dsp.json` | 15 captures, `eq_preset_click` exit 4, popup skipped | Earlier failed preset path (provenance, §4.2) |
| S01 round 2 / round 3 / round 3b (2026-10-15, 2026-10-16) | `s01r2-open.json`, `s01r2-right-edges.json` | 136/136 and 117/117, gates passed | Everything carried in §1 and §7 without re-derivation |

Both held-rect and preset-sprite results reproduce the round-2 baselines at
0 px inside their footprints (§3.2, §4.1). The held-states run used a lone
`WM_LBUTTONDOWN` (0x201) posted with no `WM_LBUTTONUP`; all 37 `send-message`
steps took the posted branch (reply empty, `input_api=send-message`).

### 13.2 Calibration against whole-window noise

1435 px, rows 4–11 only, rows 30–115 at exactly 0, in every
held-versus-baseline comparison of run `20261016-050228`. Base-line conclusion:
whole-window counts in the ~1400–2000 px range with a title-band bounding box
are **session noise**, not sprites; compare released vs resting on the bounding
box, never on the total. (Correction carried in §2.6.)

### 13.3 Decisive held (button-down) steps, run `20261016-050228`

| Step | Comparison | n | Bounds |
|---|---|---|---|
| `on_down_engaged` / `on_held_engaged` | 02 / 03 vs the run's own resting On frame | 300 / 300 | (14, 18)–(38, 29) |
| `on_down_notengaged` / `on_held_notengaged` | 08 / 09 | 300 / 300 | (14, 18)–(38, 29) |
| `auto_down_engaged` / `auto_held_engaged` | 12 / 13 | 395 / 396 | (39, 18)–(71, 29) |
| `auto_down_notengaged` / `auto_held_notengaged` | 18 / 19 | 395 / 396 | (39, 18)–(71, 29) |
| `presets_down` / `presets_held` / `presets_up` | 64 / 65 / 66 | 528 / 528 | (217, 18)–(260, 29) |
| `preamp_down` / `preamp_move` / `preamp_held` / `preamp_up` | 22 / 23 / 24 / 25 | 73 and 510 | (22, 63)–(32, 73) and (22, 26)–(198, 98) |

Engagement difference: `eq_on_held_engaged` vs `eq_on_held_notengaged` → 6 px,
(20, 24)–(22, 25); the same for Auto at (45, 24)–(47, 25).

### 13.4 Post-release decomp (earlier run, 20261016-024437)

| Capture | vs resting (earlier run) | Decomposition |
|---|---|---|
| `on_held_off` (`17_on_held_off`) | 300, bounds (14, 18)–(38, 29) | On sprite, no cell term |
| `auto_held_off` (`18_auto_held_off`) | 401, bounds (18, 18)–(71, 29) | 395 + 6 |
| `eq_preset_pressed` (`13_eq_preset_pressed`) | 534, bounds (44, 18)–(260, 29) | 528 + 6 |

Against each of their **own** resting equalizers the earlier run differs by 0 px
inside the button footprint, and by 6 px for `auto_held_off` (the Auto cell
disagreement noted in §3.2).

---

## 14. Sources relied on

- `D:\code\rustamp-research\drafts\S07-eq-dsp.md` (the S-agent draft, spec round,
  including §12 source list)
- `D:\code\rustamp-research\audits\held-states-interpretation.md` (held-states
  run, steps 00–66, 69/69)
- `tools/vm-capture/_runs/20261016-050228` (held states), `…/20261016-024437`
  (18/18), `…/20261016-014557`
- `D:\code\Rustamp\docs\tasks\S07-eq-dsp.md` (task card S07)
- `D:\code\Rustamp\specs\S01-window-geometry.md` (§3, §3.1, §7 round 2 / round 3
  / round 3b / held-states items), `…\S02-main-window-behaviour.md` (§2.1, §3,
  §7, §8), `…\S03-shortcuts.md` (§3, §5, §6)
- `D:\code\Rustamp\docs\HANDOFF.md` (§5 value ranges, §6 item 12, §12 items
  12.12, 12.13, 12.15, 12.16) and `…\docs\rfc\0005-app-commands.md` (§16,
  O10, O11)
- `rustamp-audio/src/gain.rs` (provisional volume / balance laws, cited, not
  specified here)

Contract changes in `rustamp-core` or implementations arising from this spec
are out of scope and require an RFC in `docs/rfc/` before merge.

---

*End of spec. Nothing executable, no source code, no pseudo-code was included;
every fact names its step or capture; provisional, visual-reading and inferred
values are marked as such, with the settling capture named.*
