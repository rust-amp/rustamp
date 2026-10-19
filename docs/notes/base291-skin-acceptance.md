# base-2.91 skin acceptance (wave140)

## Input and invocation

External, MIT-licensed webamp artwork; no base-skin pixels are bundled in this
change. Acceptance input: `D:\code\rustamp-testskins\base-2.91.wsz`, 102133 bytes,
SHA-256 `cb2e5361a5208878855ae2c6c4a842df6cb71590b2c4405fe8776258c82e59bb`.

`crates/rustamp-ui/tests/base291_acceptance.rs` loads the real ZIP each time.
`RUSTAMP_BASE291_SKIN` overrides the file; otherwise `RUSTAMP_TESTSKINS` supplies
its parent (also supported by the existing VM runner), then the above local
path is tried. Missing input fails explicitly, never silently skips acceptance.
Arial and Courier New must be installed for the font-family control test.

```powershell
cargo test -p rustamp-ui --test base291_acceptance
```

All ordinary cases are headless: image buffers, font files, ZIP bytes and pure
state transitions only. No presenter, native window, DC, audio device or child
process is started. The ignored capture comparator is **not run on the host**.

## Feature matrix

Every test below has the `base291_` prefix.

| Skin feature | Test suffix | Result / repair |
|---|---|---|
| All 18 native sheets, 8-bit RGB/RLE8, original bytes, opaque RGBA | `archive_native_sheets_metadata_and_tables` | Asserted; RLE8 opacity fix inherited from merged wave139, not duplicated |
| All named sprites on present sheets, numeric cells, EQ graph strips, playlist menu sheets, generic sprites | `archive_native_sheets_metadata_and_tables` | Resolved and in bounds; corrected On widths and held Auto rectangles |
| `nums_ex` absence, independent per-file fallback and real `numbers` selection | `archive_native_sheets_metadata_and_tables` | `time_digit_sheet()` selects Numbers; sheets are not aliased; optional EX minus-cell layout is not claimed for an absent sheet |
| Added real EX sheet, with or without real Numbers, no cross-sheet substitution | `shipped_extended_digits_are_selected_without_sheet_aliases` | Asserted with a controlled EX addition to the real archive; `time_digit_sheet()` drives the rendered EX cells |
| Pledit colors/font, all 24 vis colors, commented-out regions, 11 original cursor resources | `archive_native_sheets_metadata_and_tables` | Asserted as loaded data; applying cursors/OS regions is not a headless-render claim |
| Main normal/shade, active/inactive, exact integer 2x, every transport/window button | `main_focus_shade_scale_and_pressed_buttons` | Asserted; shade now overlays held window controls |
| Playing/paused/stopped indicators, all digits/minus, marquee offset, kbps/kHz, clutter O/A/I/D/V, engaged toggles, all slider endpoints | `main_readouts_marquee_clutter_toggles_and_sliders` | Asserted; transport cell now wins its overlap with the decoder strip |
| Main held toggles and volume/balance thumbs, real behaviour-machine down/up path | `held_toggles_and_slider_sprites_follow_real_mouse_state` | Repaired previously ignored depressed skin sprites |
| Slider frame strips, blank digits/readouts, no cyan atlas-filler leak, spectrum/scope/off colors | `slider_frames_blank_readouts_and_visualization_palette` | Asserted; balance samples its declared x9 strip instead of x0 |
| EQ normal/shade, focus, exact integer 2x, On/Auto engagement and holds, Presets hold, all 11 thumb columns and extremes, graph ramp | `equalizer_focus_shade_scale_buttons_thumbs_and_curve` | Asserted; Presets now uses its held source row176 |
| Playlist focus, title tiles, five buttons, scrollbar top/middle/bottom, selected color | `playlist_focus_buttons_scroll_and_skin_colours` | Asserted; repaired partial-tile phase per S01 §4.1 |
| Playlist shade and held close/collapse/expand sprites; playlist unaffected by 2x | `playlist_shade_and_held_title_controls_use_pledit` | Added renderer states; resting title controls no longer use held cells |
| Actual installed `Font=` family, current/normal and selected/background colors, independence from text.bmp | `playlist_font_is_the_requested_system_family_not_text_bmp` | Replaced bitmap stand-in with headless system-font rasterization |
| Missing title sheet only, documented default fallback, unaffected sheets retain original bytes, real transport pixels survive | `missing_sheet_is_local_documented_fallback` | Asserted without extracting or modifying the external archive |
| Actual presented client pixels vs approved reference captures | `vm_presented_pixels_match_reference_captures` | Ignored, VM-only; **not executed, no reference-equivalence result claimed** |

These are pixel-origin and behaviour assertions, not proof that every complete
frame matches Winamp. Existing own-art lookalike/rust-theme snapshots were
refreshed for the intentional balance/indicator/title/font changes; external
base-skin reference snapshots were neither generated nor blessed.

## Remaining boundaries

* `rustamp-app/src/playlist_window.rs` still explicitly treats the playlist Shade
  click as a no-op. The renderer now supports it through
  `PlaylistWindow::windowshade`, but wiring that app action and held title state
  to OS sizing/input is outside this skin/UI lane. Do not report the app's
  actual playlist Shade click as working solely because the headless case passes.
* EQ shade composition is still provisional, as identified in S01/S05. Its
  title-band source and focus/scale are tested; shade-mode volume/balance
  controls and native cursor/region application are not newly implemented here.
* The EQ curve/track interpolation and visualization algorithms retain their
  documented own-choice status. Their **skin pixels/colors** are tested, not
  their equivalence to an unmeasured reference algorithm.
* Font family and colors now work. 13px raster scale/baseline are own choices;
  glyph coverage is quantized to a 1-bit mask before blitting. Every row pixel
  is exactly its skin ink or background, never an antialiased mixture.
  ab_glyph is not GDI and exact classic hinting is a separate VM comparison.
  If no installed font can be loaded, the existing bitmap mask remains the
  documented last-resort fallback. The real-font test would detect that fallback.
* There is no new claim about optional `nums_ex` geometry: base-2.91 has no EX
  sheet, and S01/S17 explicitly leave its exact layout open.

The ignored comparator requires the staged guest tree
`C:\rustamp\code\Rustamp`, `RUSTAMP_BASE291_VM_CAPTURES`, a
`session-proof.json`, and paired `<case>.presented.png` and
`<case>.reference.png` files for the eleven case names in the test. The files
must be real interactive-session client captures with a common skin, state,
size and DPI. The comparator neither captures a window nor fabricates images;
the capture runner must produce those inputs and provenance first.

## Dependency / provenance review

No Winamp source, SDK headers, disassembly or research tree was read. Inputs
are S01/S02/S17 and the explicitly allowed MIT webamp skin. No third-party
source code was copied.

`rustamp-ui` directly adds `fontdb 0.24.0` (MIT, `fs` only, default features off)
and `ab_glyph 0.2.32` (Apache-2.0, already locked transitively). New locked
transitives are `slotmap 1.1.1` (Zlib) and `tinyvec 1.13.3`
(Zlib OR Apache-2.0 OR MIT). License evidence is each cached package's
`Cargo.toml`; all four are permissive and GPL-3.0 compatible, without upstream
copyleft that would limit our plugin exception. No fonts are redistributed.
These dependencies enter the GPL UI closure, not a protected permissive root.
`THIRD_PARTY.md` records purposes and the complete updated lock inventory.

The license audit passed for all five protected roots, both Windows targets,
all features (306 external versions match notices/lock/metadata). No commit was
created. The owner-requested stash / fast-forward main / stash-pop synchronized
the worktree to `8b7f169`. The sole conflict was bmp.rs: the upstream decoder and
tests were retained unchanged, removing the duplicate local fix. The UI uses
`Skin::time_digit_sheet()` instead of inferring digit choice from substitutions.

## Sync verification receipts (before the monochrome follow-up)

* `cargo fmt --all -- --check`: passed.
* `cargo clippy -p rustamp-skin -p rustamp-ui --all-targets --all-features -- -D warnings`: passed.
* `cargo test -p rustamp-skin -p rustamp-ui --all-features`: 13 test binaries,
  393 passed, 0 failed, 2 ignored (one existing OS-dialog test and the new VM
  capture comparator); both crates' doc-test runs passed.
* New archive acceptance: 11 passed, 1 VM-only ignored, no missing-fixture skips.
* `python .github/scripts/license-audit.py`: passed.

The commands ran in this worktree, headlessly. No VM or real-window test was
executed, and these receipts do not claim a workspace-wide or GUI test run.

## Monochrome follow-up: app regression fixed

The original system-font rasterizer converted fractional glyph coverage into
fractional alpha. `Image::blit_alpha` correctly blended those values, but that
violated the playlist's strict two-color row invariant and broke four app tests.
The main renderer (`render.rs`) was not the cause.

System-font coverage now produces only opaque ink (coverage >= 0.5) or no
pixel. The bitmap-font fallback also quantizes source alpha to a 1-bit mask.
The UI row test and the real base-2.91 acceptance test now enforce exact
`pixel == ink || pixel == bg`, with no blending allowance. A new rasterizer
test checks exact ink/transparent pixels for Arial, Courier New, an unknown
family's fallback, clipping and empty labels. The app tests were **not edited**.
The four affected own-art playlist text goldens were regenerated for monochrome
text; external reference captures remain untouched.

Latest verification (headless, this worktree):

* `cargo fmt --all -- --check`: passed.
* `cargo clippy -p rustamp-ui -p rustamp-app --all-targets --all-features -- -D warnings`: passed.
* `cargo test -p rustamp-app -p rustamp-ui --all-features`: 24 test binaries,
  **600 passed, 0 failed, 12 ignored**; both doc-test runs passed.
  * `rustamp-app`: 287 passed, 0 failed, 10 ignored.
  * `rustamp-ui`: 313 passed, 0 failed, 2 ignored.
* The app's full playlist unit-test group: 29 passed, 0 failed, 1 VM-only
  ignored, including all four reported regressions unchanged.
* `git diff --exit-code -- crates/rustamp-app`: clean, proving no app-test edits.
