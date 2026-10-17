# S07r — Equalizer renderer catch-up

## Boundary and sources

Write only this lane's files in `wave52-eq-renderer`, and nothing under
`D:\code\Rustamp`. Owned paths: `crates/rustamp-ui/**`,
`tests/golden/lookalike/**`, `tests/golden/README.md`, and this card.
No access to the canonical checkout, research or reference-binary directories.
No dependencies, downloads, git writes, windows, audio or host test suite.
The explicitly authorized headless `cargo xtask golden` is the sole test-run
exception (it internally invokes Cargo's filtered golden test runner).

Allowed implementation sources: audited S01 §3/§3.1, S07 §§3/7/8,
S02/S17 conventions, S05 §1.5/§2.1, and the existing MIT-attributed webamp
sprite table. No Winamp source, SDK material, disassembly or dirty-room data
was consulted; no third-party implementation code is copied.

## Scope and evidence boundary

Before: background, focus title bar, graph background only, measured On/Auto
held variants, resting/dragged thumbs, normal Presets, provisional shade band,
and exact integer 2x scaling already existed. The missing curve and slider
body are added here; held/focus/scale paths are pinned, not claimed as new.
The first golden run also exposed a pre-existing EQ shade+2x size mismatch:
the renderer paints 550x28 but shared docking geometry returned 275x14.
`docking::window_size` now returns 550x28 for that combination, an explicit
own-choice consistency fix, not a newly measured S05 behaviour. A fifth unit
test covers both toggle orders and stack reflow; the golden helper checks
the reported size against actual pixels and exact scaling.
`set_equalizer_double_size` also refreshes stored width/height while shaded
(it previously skipped that update), keeping the playlist's dock edge aligned;
the unit test checks both orders and scaling back down.

S07 deliberately leaves graph mapping/interpolation, body width/tiling and
colour quantization open. The renderer must mark every chosen rule rather
than turn it into a measured classic fact. S05 §1.5 still does not settle EQ
shade drawing or its window-button positions; retain the existing fallback.

## Drawing added (own choices unless marked measured)

- Ten band points: graph-local x = nearest `band * 112 / 9`, including both
  endpoints. This is equal spacing, not measured frequency/log spacing.
- Band y: piecewise-linear saved-value anchors 0 -> 0, 31 -> 8, 63 -> 18,
  nearest-integer rounding. S01 measures the colour endpoints and flat rows
  7..9, **not** this complete y law. Values above 63 clamp to 63.
- A one-pixel, 8-connected integer Bresenham polyline, no antialiasing,
  smoothing, filter simulation, preamp addition or point markers. Sample
  the sheet ramp at each plotted y; clip segments to the graph rectangle.
- Preamp is a separate full-width horizontal strip, at graph-local
  `18 - round(saved * 18 / 63)` (flat -> 9). Its reversed endpoints match
  S07 §8.2's measured y35/y17. The strip identity, intermediate mapping,
  separation from band response and drawing it before the curve (curve wins
  intersections) are our choices. Enabled/Auto do not hide the configuration.
- Restore each 11-px track from sheet `(13,229,11,62)` at window `(thumb_x,38)`.
  This extends S01's empty `(13,229,11,11)` cell using the measured groove
  offset `sheet_y = window_y + 191`; only groove columns 14/15 are verified,
  **not** this full-width recipe. Bottom y100 exclusive is the measured last
  thumb top89 plus height11, not a claimed measured track cap.
- Fill a centred 7-px body (`thumb_x+2`) from the thumb's exclusive bottom to
  y100 exclusive, constant ramp colour `round(saved * 18 / 63)`, then blit
  the thumb. Fill direction/end colours are S01 facts; width, rounding,
  preamp sharing the ramp and live-drag updates are our choices. The final
  value63 has no exposed body below its thumb. No caps are invented.

This is a **provisional graphical profile**, not a measured classic curve or
the DSP's actual frequency response. No band/dB or shared-core contract changes.

## Cells used / retained

All following cells are `eqmain.bmp` unless stated otherwise:

| Cell | Source / evidence |
|---|---|
| Base `(0,0,275,116)`; focus pair `(0,134,275,14)` / `(0,149,275,14)` | S01 §3, measured; retained |
| Graph `(0,294,113,19)` at `(86,17)` | S01 §3/§3.1; retained |
| Ramp samples `(115,294+row)` | S01 §3.1 measured colour scale; webamp `EQ_GRAPH_LINE_COLORS`, line388. Sample colours, never blit the vertical strip |
| Reference `(0,314,113,1)` | webamp `EQ_PREAMP_LINE`, line391; S07 §7.3 explicitly leaves causal use unresolved |
| Track `(13,229,11,62)` | Within table `EQ_SLIDER_BACKGROUND` `(13,164,209,129)`, line373. S01 §3.1 gives empty cell and groove offset; full atlas single-blit is refuted |
| Thumbs `(0,164,11,11)` / `(0,176,11,11)` | S01 §3.1; table lines374–375; retained |
| On x10/69/128/187, y119, 25x12 | S07 §3.1 / S01 §3; all four states retained |
| Auto x36/95, y119, 32x12; held x153/212, 33x12 | S07 §3.1; measured held footprint starts at window x39, retained |
| Presets `(224,164,44,12)` | S07 §4.1; normal-only retained |
| `eq_ex.bmp` `(0,0,275,14)` / `(0,15,275,14)` | Table lines357–358, S01 provisional; retained |

The named table still has pre-S07 On width26 and Auto held x154/213 width32
(lines379–386). The existing renderer correctly uses explicit audited rects
instead. No skin files were edited. No required ramp/line/thumb cell is
missing. The shipped lookalike's track atlas is currently a uniform dark fill
(`generator/generate_skin.py`, lines884–885), not detailed groove/frame art;
improving that artwork belongs to the skin lane, not this change.

## Still provisional / measurement requests

1. S07 O5: reference captures at saved 0/15/31/46/63, separately for preamp
   and one band, with saved-state read-back, would settle body quantization,
   width/extents and whether the two ramps match. Repeat held/released to
   settle live recolouring. A coordinate-marked synthetic track atlas loaded
   by the reference would settle caps/tiling; do not restore the refuted atlas
   single-blit.
2. S07 O7: isolated bands, combined profiles and preamp-only sweeps with
   saved-state read-back plus graph pixel/ramp-row analysis would settle x/y
   mapping, sign, thickness, smoothing, preamp influence and disabled behaviour.
3. S07 O8: a synthetic skin with a nonuniform marked row314, actually loaded
   by the reference, would distinguish horizontal-strip blit from a colour line.
4. S07 §4.1: Presets held is measured at window level but its sheet identity
   is not settled. Normal remains drawn while held; no unsupported cell
   identity is invented. A marked normal/selected/held-cell skin would settle it.
5. S05 §1.5: EQ shade pixels and close/shade-button positions remain open;
   no mini sliders or held title buttons are invented. Measure the EQ's own
   shade toggle, client size and focused/unfocused frames. The existing
   275x14/550x28 composition is regression-pinned, not newly measured.
6. S07 §3.2/§12: retained On/use_eq association and Auto indicator-position
   disagreement remain spec caveats. A saved-state round-trip and fresh held
   captures in both engagement states would settle them.

## Golden inventory

18 new PNGs, all under `tests/golden/lookalike/`, prefix `equalizer_window`:

- 275x116: `_single_raised_band`, `_minimum`, `_maximum`, `_alternating`,
  `_preamp_minimum`, `_preamp_maximum`, `_mid_travel`, `_mid_travel_dragged`,
  `_preamp_held`, `_on_held`, `_on_held_engaged`, `_auto_held`,
  `_auto_held_engaged`, `_unfocused` (14 images).
- 550x232: `_double_size` (exact equality to 1x scaled2 asserted).
- 275x14: `_shade`, `_shade_unfocused` (provisional).
- 550x28: `_shade_double_size` (provisional, exact scaled2 equality asserted).

Existing `equalizer_window.png` and
`equalizer_window_raised_band_preamp.png` (both 275x116) intentionally change
for the added curve/reference/body. Main/playlist images must not change.
The golden pixel contract checks flat curve/ramp, all 113 reference source
pixels and all eleven tracks/bodies. Four additional unit tests use our own
coordinate-marked sheet and check declared row anchors/clamping, segment
endpoints/graph bounds, reference mapping, track/fill bounds and missing or
truncated sheets; compiled by clippy, not run on the host.

## Verification receipts

- Initial fmt/clippy/build: pass. Initial bless: 12/13 test functions pass;
  the new shade+2x size assertion caught the mismatch above before its PNG
  could be written. No test was weakened: geometry was corrected.
- Final `cargo fmt --all -- --check`: pass, exit0.
- Final `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  pass, exit0 (including all five new unit tests compiled, not executed).
- Final `cargo build --workspace`: pass, exit0.
- `cargo xtask golden --skin lookalike --bless`: final pass, 13/13 test
  functions; 25 PNGs total (20 EQ plus the unchanged 3 main/2 playlist).
- `cargo xtask golden --skin lookalike`: pass, 13/13 test functions, no
  mismatch artefacts. Full final logs: `target/eq-golden-bless.log` and
  `target/eq-golden-compare.log` (ignored build outputs, worktree-local).
- `RUSTAMP_TESTDATA` unset: external classic-base not run; no reference
  captures are claimed. Existing main/playlist PNGs remain byte-identical
  (no diff); only the two old EQ images change, plus the 18 new EQ PNGs.
- `git diff --check`: pass. Worktree status limited to the owned UI files,
  this card, the golden README and EQ PNGs; verify with final
  `git status --short` in the lane worktree.
- No standalone `cargo test`, no full host suite, no window, no audio, no
  commit/stage/git write commands, no dependencies/downloads. The explicitly
  allowed xtask golden wrapper internally invokes Cargo with a golden-only
  filter; all non-golden tests (including the five new unit tests) stayed
  filtered out. No forbidden directory was read or written.

## Handoff locations

- Before (`HEAD`): `eq_render.rs:171` drew only the graph background;
  `eq_render.rs:214` drew only thumbs. Held/focus/2x were already present.
- Now: `eq_render.rs:144` (body ramp law), `:150` (curve row law), `:166`
  (graph and reference), `:195` (integer segments), `:228` (track/body),
  `:351` (thumb composition), `:373` (four coordinate-sheet unit tests).
- Retained measured held cells: `eq_render.rs:120` / `:132`; focus/shade
  `:261` onward, whole-frame scaling `:70`.
- Size consistency fix: `docking.rs`, equalizer branch in `window_size`;
  provisional toggle-order test at the end of that file.
- Golden matrices and pixel contracts: `golden_lookalike.rs:167` onward.
