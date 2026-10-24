# Task card J3 (wave 51) — More lookalike goldens

## Goal

Extend the committed golden-image suite for the look-alike skin so four more
states of the drawn chrome are pinned pixel by pixel, exactly
(tolerance 0), without touching any renderer.

*(It was the shipped default when this card was written; the built-in default
is now the Rust theme, task card J2. Corrected in the W11 docs sweep.)*

Lane: goldens. Owned paths: `crates/rustamp-ui/tests/**`,
`tests/golden/lookalike/**`, `tests/golden/README.md`, this card.

## Boundary and provenance

- Work happened only inside the wave-51 worktree; `D:\code\Rustamp`,
  `D:\code\rustamp-research` and `D:\code\rustamp-refbin` were not read or
  written (proved by `git status --short`).
- Every state is built from `rustamp-ui`'s public API. No renderer or other
  crate source was changed; no renderer looked wrong.
- No downloads, no new dependencies, no `cargo add`, no git write, no commit.
- Renderers run headlessly (`cargo xtask golden` renders in-process: no window,
  no audio, no clock).

## Cases added

All four are in `crates/rustamp-ui/tests/golden_lookalike.rs`, all in the
`lookalike` namespace, which is the only namespace that is committed.

| Case (golden file) | Size | State | What it pins |
|---|---|---|---|
| `main_window_playing` | 275 x 116 | `MainWindowState { transport: Playing, time_seconds: 222, title: "1. Rustamp - the look-alike default skin (3:42)", marquee_offset: 10, kbps: Some(192), khz: Some(44), channels: Stereo, position: 37, volume: 80, ..default }` | The marquee text row and its scroll offset (S02 §3), the four time digits with the blank leading cell (S01 §2.3, S02 §4), the `kbps`/`kHz` numeral fields (S02 §5), the lit stereo label (S02 §6), the playing indicator and the decoder-active indicator (S02 §1), and the moved position/volume thumbs (S01 §2.1) |
| `main_window_double_size` | 550 x 232 | `MainWindowState { double_size: true, ..default }` | The whole-frame 2x scale of S05 §2.1. The state is the same default as the existing `main_window` case, and the committed PNG is **exactly** the 2x of `main_window.png` (checked pixel by pixel: every 2x2 block equals its 1x pixel, 0 of 127600 pixels differ) |
| `equalizer_window_raised_band_and_preamp` | 275 x 116 | `EqualizerState { bands: [8, 31, 31, 45, 31, 31, 20, 31, 31, 55], preamp: 14, ..default }` | The eleven slider thumbs off the flat row (S01 §3.1 thumb-y mapping: 0 -> 38, 31 -> 63, 63 -> 89) and the graph background they sit over (S01 §3). Non-flat values are 0..=63 saved units with 31 flat (`saved_to_gain_db`: band 0 = +8.9 dB, band 3 = -5.25 dB) |
| `playlist_window_selection` | 275 x 116 | `PlaylistWindow` with 60 entries, `scroll_to(56)`, `apply_select(SelectOp::Set([EntryId(57)]))`; four labels for the visible rows | The selected row band (the `SelectedBG` fill of S01 §4 / S06 §2) on the second visible row, which also pins the entry-id-to-row mapping, and the scroll handle at its lowest settled point (S01 §4.2: y = 60 for a 60-entry list) |

The three pre-existing cases (`main_window`, `equalizer_window`,
`playlist_window`) are unchanged: re-blessing reproduced them byte for byte.

## State fields, and how they were found

* `crates/rustamp-ui/src/state.rs` — `MainWindowState`: `transport`,
  `time_seconds`/`time_visible`/`time_remaining`, `title`/`marquee_offset`,
  `kbps`/`khz`, `channels` (`MonoStereo`), `position`/`volume`/`balance`,
  `double_size`. All are public struct fields with a `Default` impl, so a case
  is a struct update expression.
* `crates/rustamp-ui/src/render.rs` — which field reaches which draw call:
  `draw_marquee` reads `title`/`marquee_offset`, `draw_time` reads
  `time_cells()`, `draw_text_field` reads `kbps`/`khz` (`None` draws nothing),
  `draw_indicators` reads `transport`, `decoder_active()` and `channels`, and
  `render_main_window` doubles the whole frame when `double_size` is set.
  `frame_size(&state)` (public) asserts the 550 x 232 of S05 §2.1 in the case.
* `crates/rustamp-ui/src/equalizer.rs` — `EqualizerState { bands, preamp, .. }`
  (0..=63, flat 31) and `EqualizerLayout::thumb_rect`, which is what
  `eq_render.rs` uses for the eleven thumbs.
* `crates/rustamp-ui/src/playlist_window.rs` — `render_playlist_window(skin,
  window, labels, focused)` reads the selection through
  `PlaylistWindow::selected_ids()` and maps `labels[i]` to
  `window.visible_ids()[i]`, so a selection needs the model, not a renderer
  flag. `PlaylistWindow::set_entries`, `scroll_to` and `apply_select`
  (`rustamp_core::command::SelectOp`) are the public API used.
* Specs used: S01 §1/§2.1/§2.3/§3.1/§4/§4.2, S02 §1/§3/§4/§5/§6, S05 §2.1,
  S06 §2/§3, S07 §7.3/§7.4.

## Not expressible / not pinned

* Nothing had to be left out: all four states are reachable through the public
  API.
* The equalizer **response curve** and the slider-body colouring are not drawn
  by `eq_render` at all (S07 §7.2/§7.3/§7.4 leave them open: "the graph curve
  and its mapping ... the per-value body colouring"), so
  `equalizer_window_raised_band_and_preamp` pins the graph *background* and the
  thumbs over it, not a curve. That is a renderer gap, reported here, not
  changed (the renderers are another lane's).
* The marquee's held-control hint text (S02 §2.1) is only the `title` field
  here, so no separate case: it would render the same pixels as any other title.

## Verification (all on the host, headless)

| Command | Result |
|---|---|
| `cargo xtask golden --skin lookalike --bless` | exit 0; wrote the four new PNGs, `BLESSED` each; no mismatch artefacts |
| `cargo xtask golden --skin lookalike` | exit 0; `test result: ok. 7 passed; 0 failed`; no mismatch artefacts |
| `cargo fmt --all -- --check` | clean (exit 0) |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean (`Finished` dev profile, no warnings) |
| `cargo build --workspace` | `Finished` dev profile (exit 0) |

No `cargo test` was run. No window was opened, no audio played and nothing was
committed (the commit is the lane owner's call).
