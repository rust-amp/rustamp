# Golden images

Pixel-exact reference images for headless-rendering tests. Helpers live in the
`rustamp-testkit` crate (add it as a `dev-dependency`).

## Skin namespaces

| Namespace | Meaning | Where goldens live | Committed? |
|---|---|---|---|
| `lookalike` | The classic look-alike skin (`BuiltinLookalikeSkin`) | `tests/golden/lookalike/<name>.png` | **Yes** |
| `rust-theme` | The built-in default Rust-themed skin (`BuiltinDefaultSkin`) | `tests/golden/rust-theme/<name>.png` | **Yes** |
| `classic-base` | The original base skin (separate download) | `$RUSTAMP_TESTDATA/golden/classic-base/<name>.png` | **Never** |

`RUSTAMP_TESTDATA` points at a checkout of the external `rustamp-testdata`
repository (reference captures from real Winamp 2.95, audio fixtures). When it
is unset, `classic-base` tests print `SKIP golden classic-base/<name>: ...` and
pass. Images from the original skin or captured from the real program must
never be committed to this repository; `tests/golden/classic-base/` should be
git-ignored as a safeguard.

## Writing a golden test

```rust
use rustamp_testkit::{assert_golden, Snapshot, SKIN_LOOKALIKE};

#[test]
fn golden_lookalike_main_window_stopped() {
    let window = build_main_window(/* ... */);       // implements Snapshot
    assert_golden("main_window_stopped", SKIN_LOOKALIKE, &window.render_image());
}
```

Conventions:

* Test fn names start with `golden_`, then the skin namespace with `-` as `_`
  (`golden_lookalike_*`, `golden_rust_theme_*`, `golden_classic_base_*`).
  `cargo xtask golden` filters on this prefix; `--skin lookalike` narrows it to
  `golden_lookalike`, `--skin rust-theme` narrows it to `golden_rust_theme`, and
  `--skin classic-base` narrows it to `golden_classic_base`.
* Case names are `snake_case`, e.g. `main_window_playing_clutterbar_o`. They
  may contain `/` for subfolders.
* Reach the state under test only through the crate's public API (for
  `rustamp-ui`: the state structs' public fields and the window models' public
  methods). A state that would need a private field is left out and reported,
  never forced by poking at internals.
* One case per state worth pinning, named for it: `main_window_playing` (a
  title, a time, a bitrate and a lit stereo label), `main_window_double_size`
  (the same default state at the settled 550 x 232), a non-flat
  `equalizer_window_*`, a `playlist_window_selection` (bottom page with a
  selected row), a `playlist_window_mid_page` (scrolled view with the handle
  between the settled points) and a `dialog_load_playlist` (the typed-path
  face). A renderer change then names the state it broke.
* PNG, 8-bit RGBA, comparison is exact (tolerance 0).

### Case coverage (`lookalike` and `rust-theme`)

Both `crates/rustamp-ui/tests/golden_lookalike.rs` and
`crates/rustamp-ui/tests/golden_rust_theme.rs` share identical cases from
`golden_cases/mod.rs` to allow direct visual comparison. All EQ names below
have the prefix `equalizer_window`:

| Suffix | Dimensions | Coverage |
|---|---|---|
| (none), `_raised_band_preamp` | 275 x 116 | Existing cases updated for the curve, preamp line and slider bodies |
| `_single_raised_band` | 275 x 116 | One raised band, otherwise flat |
| `_minimum`, `_maximum`, `_alternating` | 275 x 116 | Saved-value endpoints 0/63, steep segments and fill endpoints |
| `_preamp_minimum`, `_preamp_maximum` | 275 x 116 | Separate reference line at y35/y17 with flat bands |
| `_mid_travel`, `_mid_travel_dragged`, `_preamp_held` | 275 x 116 | Values 15/46, band drag and lone-down preamp thumb |
| `_on_held`, `_on_held_engaged`, `_auto_held`, `_auto_held_engaged` | 275 x 116 | S07 held cells, including the extra Auto column |
| `_unfocused` | 275 x 116 | S01 focus variant |
| `_double_size` | 550 x 232 | Exact 2x of the flat frame, also asserted in code |
| `_shade`, `_shade_unfocused` | 275 x 14 | Existing provisional EQ shade bands |
| `_shade_double_size` | 550 x 28 | Existing provisional shade-plus-scale composition |

These are regression baselines for **our** drawing, not reference captures.
S07 §7 leaves the curve and track recipe open: the renderer documents its
own polyline, value-to-row, colour rounding, body width and track extent
choices. Shade remains provisional under S05 §1.5. The pixel-contract case
also checks the flat ramp samples, full horizontal strip and sheet track
offsets; it reuses the existing flat PNG rather than adding a duplicate.

### Playlist and dialog cases (`lookalike` and `rust-theme`)

| Name | Dimensions | Coverage |
|---|---|---|
| `playlist_window` | 275 x 116 | Empty default playlist frame |
| `playlist_window_selection` | 275 x 116 | Bottom page (rows 57–60) with row 58 selected; handle at y = 60 |
| `playlist_window_mid_page` | 275 x 116 | Offset 28 of 60 (handle at the settled y = 40) with row 30 selected via `move_selection_down`; pins the own-choice linear handle interpolation and the S03 §2 provisional Down path |
| `dialog_load_playlist` | 300 x 100 | Load-playlist dialog with a typed path; the face the BETA-09 UI half types into |

## Workflow

```
cargo xtask golden                      # run all golden_* tests
cargo xtask golden --skin lookalike     # only lookalike namespace
cargo xtask golden --skin rust-theme    # only rust-theme namespace
cargo xtask golden --bless              # write/overwrite goldens (RUSTAMP_BLESS=1)
```

* Missing golden: the test fails with a hint to bless.
* Mismatch: the test writes `target/golden-out/<skin>/<name>.actual.png` and
  `<name>.diff.png` (red pixels over a dimmed copy of the expected image) and
  panics with the mismatch count, bounding box and max channel delta.
  `cargo xtask golden` lists those files afterwards.
* After an intended change, bless, review the PNG diff in your PR, and commit
  only repo-owned goldens (`lookalike` and `rust-theme`).

## Capturing references from a running program

`cargo xtask capture --list` and
`cargo xtask capture --title <substr> [--class <cls>] [--client-only] --out file.png`
(Windows only) produce reference PNGs of any window. Store results for the
original skin in `rustamp-testdata`, not here.
