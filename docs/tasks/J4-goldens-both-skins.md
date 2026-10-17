# Task card J4 (wave 54) — Golden-image suite for both built-in skins

## Goal

Provide a dedicated golden-image namespace for each of the two shipped built-in skins:
- `lookalike`: classic look-alike skin (`BuiltinLookalikeSkin`), rendered into `tests/golden/lookalike/`
- `rust-theme`: built-in default Rust theme (`BuiltinDefaultSkin`), rendered into `tests/golden/rust-theme/`

Both skins are renderable through the public API and have their own committed golden image set covering identical cases for direct visual comparability.

Lane: goldens. Owned paths: `crates/rustamp-ui/tests/**`, `tests/golden/**`, `tests/golden/README.md`, this card.

## Inherited state and diagnosis

Prior to this task, the single committed golden namespace was `lookalike`. However, `crates/rustamp-ui/tests/golden_lookalike.rs` was loading `BuiltinDefaultSkin`, which had become the Rust theme upon completion of task J2 (`crates/rustamp-skin/src/skin.rs`). Consequently:
- `golden_lookalike.rs` was rendering Rust-theme sprites while asserting against `tests/golden/lookalike/*.png` (which contained lookalike pixels), causing tests to fail on mismatch.
- The Rust theme lacked its own committed golden suite in `tests/golden/rust-theme/`.
- The look-alike skin (`BuiltinLookalikeSkin`) was not being exercised by any dedicated golden test suite.

## How the cases were shared

Rather than duplicating the 13 test function bodies across two separate files:
1. Created `crates/rustamp-ui/tests/golden_cases/mod.rs`:
   - Defines snapshot helpers (`Shot`) and skin loaders (`lookalike_skin()`, `rust_theme_skin()`).
   - Implements 13 case runner functions covering all UI windows, controls, and states.
   - Provides a declarative macro `register_golden_tests!` that expands into the 13 `#[test]` functions with the required naming conventions (`golden_lookalike_*` and `golden_rust_theme_*`).
2. Updated `crates/rustamp-ui/tests/golden_lookalike.rs`:
   - Uses `lookalike_skin` and namespace `SKIN_LOOKALIKE` ("lookalike").
   - Registers all 13 tests with the `golden_lookalike_*` prefix.
3. Created `crates/rustamp-ui/tests/golden_rust_theme.rs`:
   - Uses `rust_theme_skin` and namespace `SKIN_RUST_THEME` ("rust-theme").
   - Registers all 13 tests with the `golden_rust_theme_*` prefix.

## Cases and coverage (identical across both skins)

All 13 test cases are fully expressible for both skins; no case was omitted or modified between skins.
The 13 tests generate and check 25 golden images per skin:

| Category | Golden image name | Dimensions | Coverage |
|---|---|---|---|
| Main window | `main_window.png` | 275 × 116 | Default main chassis, stopped readout, resting sliders |
| Main window | `main_window_playing.png` | 275 × 116 | Playing indicators, scrolled marquee title, time, bitrate, lit stereo |
| Main window | `main_window_double_size.png` | 550 × 232 | Exact whole-frame 2x integer scale of default main window |
| Playlist window | `playlist_window.png` | 275 × 116 | Default playlist chassis with track rows |
| Playlist window | `playlist_window_selection.png` | 275 × 116 | Scrolled playlist at bottom page, selected row band, scroll handle at lowest point |
| Equalizer | `equalizer_window.png` | 275 × 116 | Default flat equalizer chassis and sliders |
| Equalizer | `equalizer_window_raised_band_preamp.png` | 275 × 116 | Preamp above center and profiled bands (thumbs off flat row) |
| Equalizer | (in-memory assertions) | N/A | `curve_and_track_pixels`: curve sampling and slider track offsets |
| Equalizer | `equalizer_window_single_raised_band.png` | 275 × 116 | Band 4 raised to maximum (0), others flat |
| Equalizer | `equalizer_window_minimum.png` | 275 × 116 | All bands 0, preamp 0 |
| Equalizer | `equalizer_window_maximum.png` | 275 × 116 | All bands 63, preamp 63 |
| Equalizer | `equalizer_window_alternating.png` | 275 × 116 | Alternating bands 0 / 63, preamp 31 |
| Equalizer | `equalizer_window_preamp_minimum.png` | 275 × 116 | Flat bands, preamp at minimum (0) |
| Equalizer | `equalizer_window_preamp_maximum.png` | 275 × 116 | Flat bands, preamp at maximum (63) |
| Equalizer | `equalizer_window_mid_travel.png` | 275 × 116 | Preamp 15, band 4 at 46 |
| Equalizer | `equalizer_window_mid_travel_dragged.png` | 275 × 116 | Same mid-travel with band 4 slider dragged |
| Equalizer | `equalizer_window_preamp_held.png` | 275 × 116 | Preamp held/dragged thumb sprite with flat bands |
| Equalizer | `equalizer_window_on_held.png` | 275 × 116 | ON button held down, unengaged |
| Equalizer | `equalizer_window_on_held_engaged.png` | 275 × 116 | ON button held down, engaged |
| Equalizer | `equalizer_window_auto_held.png` | 275 × 116 | AUTO button held down, unengaged |
| Equalizer | `equalizer_window_auto_held_engaged.png` | 275 × 116 | AUTO button held down, engaged |
| Equalizer | `equalizer_window_unfocused.png` | 275 × 116 | Unfocused window titlebar variant |
| Equalizer | `equalizer_window_double_size.png` | 550 × 232 | Exact 2x scale of flat equalizer frame |
| Equalizer | `equalizer_window_shade.png` | 275 × 14 | Windowshade focused mode |
| Equalizer | `equalizer_window_shade_unfocused.png` | 275 × 14 | Windowshade unfocused mode |
| Equalizer | `equalizer_window_shade_double_size.png` | 550 × 28 | Windowshade 2x scaled mode |

## Inexpressible cases

None. All 13 test cases and all 25 golden images are fully expressible, functional, and pinned for both skins.

## Verification

1. `cargo xtask golden --skin lookalike --bless`: Passed (13/13 passed).
2. `cargo xtask golden --skin rust-theme --bless`: Passed (13/13 passed).
3. `cargo xtask golden`: Passed (26/26 golden tests passed, 0 failures, 0 mismatch artefacts).
4. `cargo fmt --all -- --check`: Clean (0 differences).
5. `cargo clippy --workspace --all-targets --all-features -- -D warnings`: Clean (0 warnings).
6. `cargo build --workspace`: Clean.
7. Verification of constraints: No `cargo test` run directly on host; no windows opened; no audio played; no commits made; write boundary strictly respected within worktree.
