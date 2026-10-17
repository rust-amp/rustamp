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
   - Implements 15 case runner functions covering all UI windows, controls, and states.
   - Provides a declarative macro `register_golden_tests!` that expands into the 15 `#[test]` functions with the required naming conventions (`golden_lookalike_*` and `golden_rust_theme_*`).
2. Updated `crates/rustamp-ui/tests/golden_lookalike.rs`:
   - Uses `lookalike_skin` and namespace `SKIN_LOOKALIKE` ("lookalike").
   - Registers all 15 tests with the `golden_lookalike_*` prefix.
3. Created `crates/rustamp-ui/tests/golden_rust_theme.rs`:
   - Uses `rust_theme_skin` and namespace `SKIN_RUST_THEME` ("rust-theme").
   - Registers all 15 tests with the `golden_rust_theme_*` prefix.

## Cases and coverage (identical across both skins)

All 15 test cases are fully expressible for both skins; no case was omitted or modified between skins.
The 15 tests generate and check 27 golden images per skin:

| Category | Golden image name | Dimensions | Coverage |
|---|---|---|---|
| Main window | `main_window.png` | 275 × 116 | Default main chassis, stopped readout, resting sliders |
| Main window | `main_window_playing.png` | 275 × 116 | Playing indicators, scrolled marquee title, time, bitrate, lit stereo |
| Main window | `main_window_double_size.png` | 550 × 232 | Exact whole-frame 2x integer scale of default main window |
| Playlist window | `playlist_window.png` | 275 × 116 | Default playlist chassis with track rows |
| Playlist window | `playlist_window_selection.png` | 275 × 116 | Scrolled playlist at bottom page, selected row band, scroll handle at lowest point |
| Playlist window | `playlist_window_mid_page.png` | 275 × 116 | Offset 28 of 60 (settled handle y = 40), row selected via `move_selection_down` |
| Dialog | `dialog_load_playlist.png` | 300 × 100 | `LoadPlaylist` dialog with a typed path |
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

## Merge onto main (2026-10-17, subagent)

Main (commit `35ceff1`, playlist S06 work) added two cases to
`crates/rustamp-ui/tests/golden_lookalike.rs` after this lane's base
(`3ecc79b`): `golden_lookalike_playlist_window_mid_page` (S01 §4.2 handle at
y = 40, `move_selection_down` Down path) and
`golden_lookalike_dialog_load_playlist` (typed-path `LoadPlaylist` dialog,
300 x 100). The lane's restructure (shared `golden_cases/mod.rs` + per-skin
`register_golden_tests!` lists) conflicted with that addition in the same
file; resolved by keeping the shared-module structure on both hunks and
porting main's two cases into it as `case_playlist_window_mid_page` and
`case_dialog_load_playlist`, registered for both skins. Main's inline bodies
(using `builtin_skin()` = `BuiltinDefaultSkin`, the bug this lane fixes) were
dropped in favour of the per-skin loaders; no case was dropped. The
auto-merged `tests/golden/README.md` keeps both sides (namespace table and
`--skin rust-theme` docs from this lane; the "one case per state" bullet and
the playlist/dialog table from main, whose section header was widened to both
namespaces). Main's two new `lookalike` PNGs merged in untouched.

## Inexpressible cases

None. All 15 test cases are fully expressible through the public API for both
skins; 13 of them have committed PNGs in both namespaces.

## Still needs the bless host (VM)

Two `rust-theme` images do not exist yet and were deliberately NOT blessed on
this host:

- `tests/golden/rust-theme/playlist_window_mid_page.png`
- `tests/golden/rust-theme/dialog_load_playlist.png`

Run on the VM: `cargo xtask golden --skin rust-theme --bless`, review the two
PNGs, commit. Until then `cargo xtask golden` fails those two cases with the
missing-golden hint (expected). `lookalike` needs no bless: main's two PNGs
are already committed.

## Verification

1. `cargo xtask golden --skin lookalike --bless`: Passed (13/13 passed).
2. `cargo xtask golden --skin rust-theme --bless`: Passed (13/13 passed).
3. `cargo xtask golden`: Passed (26/26 golden tests passed, 0 failures, 0 mismatch artefacts).
4. `cargo fmt --all -- --check`: Clean (0 differences).
5. `cargo clippy --workspace --all-targets --all-features -- -D warnings`: Clean (0 warnings).
6. `cargo build --workspace`: Clean.
7. Verification of constraints: No `cargo test` run directly on host; no windows opened; no audio played; no commits made; write boundary strictly respected within worktree.

## Merge verification (2026-10-17, subagent; `git merge main` + resolution)

1. `cargo fmt --all -- --check`: clean (exit 0).
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`: clean (exit 0, no warnings; compiles both golden test binaries incl. the two new shared cases).
3. `cargo build --workspace`: clean (exit 0, dev profile).
4. Not run: `cargo test` / `cargo xtask golden` (per lane rules these run in the Hyper-V VM; only the orchestrator runs them). VM must still: run the full golden suite, bless the two missing `rust-theme` PNGs listed above, review them, commit.
5. Write boundary: all writes inside this worktree; `git status --short` shows only the merged main tree plus the four lane files touched by the resolution.
