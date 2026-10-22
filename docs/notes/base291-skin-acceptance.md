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

## Presented capture runner (wave336)

The presented side of the `base291` capture set — the same eleven states
rendered by our player — is produced by a new VM runner that mirrors the
reference path rather than inventing a second architecture.

### Host runner: `vm-run-presented.ps1`

`docs/notes/vm-run-presented.ps1` is the host-side entry point. It:

1. Restores the `base-provisioned` checkpoint (or a named child checkpoint).
2. Pins the virtual display to 1024×768 via the broker's `set-vdd-mode.ps1`
   (same as the reference runner).
3. Stages the harness, the job file, **our built tree** (taken from the
   `vm-run-tests.ps1` staging at `D:\code\rustamp-vm\staging\code\Rustamp`) and
   the reference skin `base-2.91.wsz`.
4. Copies the staged tree into the guest at `C:\rustamp\capture`.
5. Runs one job through the broker's `guest` op with the allow-listed
   `run-presented.ps1`.
6. Copies the results back with `copyout`, flattens the one-level nesting
   that the broker introduces, and verifies every file against the guest's
   `hashes.json`.
7. Emits `run-provenance.json` with the same fields as the reference runner:
   checkpoint name + creation time, restore timestamp, guest boot time, DPI
   and screen readings from both the size step and the capture script, harness
   exit code, and file count.

Invocation (one case):

```powershell
powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-presented.ps1 -Job base291-presented-main-active -DisplayPin Set1024
```

The built tree is **not rebuilt here**; `vm-run-tests.ps1` already stages a
complete, receipt-verified tree at `D:\code\rustamp-vm\staging\code\Rustamp`.
This runner reuses that staging. If the receipt is missing or the workspace
inputs have changed, `vm-run-tests.ps1 -SkipBuild` will refuse and the
orchestrator must run a fresh `vm-run-tests.ps1` first.

### Guest script: `vm-broker/guest/run-presented.ps1`

`docs/notes/vm-broker/guest/run-presented.ps1` runs inside the guest (session 0
via PowerShell Direct) and:

1. Proves the baseline (records guest boot time, user, OS).
2. Re-checks the display gate (`LogPixels = 96`, virtual screen = 1024×768)
   right before launching our player — drift between the size step and the
   player cannot slip through.
3. Registers a **one-shot scheduled task** as the console user with an
   `Interactive` logon (same pattern as `run-capture.ps1` and
   `run-gui-session.ps1`). The task payload:
   - Launches `rustamp.exe` with `--no-audio --skin <staged base-2.91.wsz>
     --volume 0 /NOREG /CONFIG=<fresh ini inside the run dir>`.
   - Waits for the main window (`Class 'Winamp v1.x'`, process `rustamp`).
   - Opens the equalizer and the playlist with `WM_COMMAND` 40036 and 40040,
     because the recorded 1:1 landmark of the reference has both windows open
     and the comparison must start from the same state.
   - Runs each job step through the harness `Invoke-Capture.ps1` with the
     exact same fields the reference jobs use:
     `-ProcessName rustamp`, the step's `-Class`, `-InputMethod`, `-Down/-Up`,
     `-Msg/-WParam/-LParam`, `-Out`, `-ExpectClientSize` (same schema).
   - Captures stdout/stderr of every harness call.
   - Writes `inner-done.json` with per-step results.
4. Session 0 polls for `inner-done.json` (≤ 30 min), verifies the task ran in
   the **interactive session** (not session 0), and refuses loudly otherwise.
5. Writes `guest-transcript.txt` (human log + all harness stdout/stderr),
   `hashes.json` (SHA-256 of every produced file), and `DONE.json` (per-step
   results + summary).
6. Emits **exactly one object** on stdout (the summary), like
   `run-capture.ps1` does.

### Job files: `vm-broker/jobs/base291-presented-<case>.json`

Eleven jobs, one per case, with **exactly the same steps and the same client
sizes** as the reference jobs. Capture filenames are `<case>.presented.png`
instead of `<case>.reference.png`. The eleven cases:

| Case | Window | Normal size | Shade size | Double size |
|------|--------|-------------|------------|-------------|
| `main-active` | Winamp v1.x | 275×116 | 275×14 | 550×232 |
| `main-inactive` | Winamp v1.x | 275×116 | — | — |
| `main-shade` | Winamp v1.x | — | 275×14 (gate) | — |
| `main-double` | Winamp v1.x | — | — | 550×232 (gate) |
| `eq-active` | Winamp EQ | 275×116 | — | — |
| `eq-inactive` | Winamp EQ | 275×116 | — | — |
| `eq-shade` | Winamp EQ | — | **not established** (`none`) | — |
| `eq-double` | Winamp EQ | — | — | **not established** (`none`) |
| `playlist-active` | Winamp PE | 275×116 | — | — |
| `playlist-inactive` | Winamp PE | 275×116 | — | — |
| `playlist-shade` | Winamp PE | — | **not established** (`none`) | — |

The EQ and playlist shade/double sizes are **not established**; those jobs
use `expect_client_size: "none"` so the harness measures and records them
without gating.

### Provenance

Every presented capture carries the same provenance as the reference side:

- **Checkpoint**: name and creation time (from `broker -Op list` before restore).
- **Restore timestamp**: host clock when the broker reported the restore done.
- **Guest boot time**: `Win32_OperatingSystem.LastBootUpTime` queried inside the
  guest by the capture script (a restore-then-boot must show a boot time *after*
  the restore timestamp).
- **DPI/screen**: `LogPixels` and virtual screen bounds read twice — once by the
  size step (`set-vdd-mode.ps1`) and once by the capture script's pre-gate —
  with per-monitor bounds. Captures from runs with different display facts are
  never compared.
- **Input method**: recorded per step (`real-mouse`, `send-message`, `none`).
- **Harness exit**: the exit code of the harness (0 = all steps ok).

The presented runner **cannot claim** that the pixels match the reference —
that is the comparator's job (`base291_vm_presented_pixels_match_reference_captures`,
currently ignored). It only claims: "these are the pixels our player produced
when driven through the same scripted steps, from the same checkpoint, at the
same DPI, with the same skin, starting from the same window state (EQ+PL
open)." The provenance travels with the files so the comparison can be audited
independently.

## Measured in the 2026-10-20 VM campaign (S05 open points)

The eleven-state capture campaign measured two window sizes that S05 §1.1 lists
as **unknown**, in the same guest, DPI and skin as every other fact here
(`base-2.91.wsz`, 1024x768, 96 dpi, per-monitor-v2, input method `send-message`):

| Window | Windowshade client size | Double-size client size | Evidence |
|---|---|---|---|
| Equalizer | **275 x 14** | **550 x 232** | `base291-eq-shade` / `base291-eq-double`, the action step's own manifest row and the following capture |
| Playlist editor | see the campaign log | not a command state | `base291-playlist-shade` |

The main window's 275x14 and 550x232 were already settled (S05 §1.1) and were
reproduced by `base291-main-shade` / `base291-main-double`, which is a useful
determinism check on the whole pipeline.

**These are measurement results, not spec text.** S05 is audited; the lines
above belong in a S05 revision that an auditor gates, with the manifest rows as
the evidence. Until that happens the campaign output is the reference, not the
spec.

How the sizes were reached, so the claim can be re-run: the equalizer is open at
launch (the recorded 1:1 landmark pins that start state), then `WM_COMMAND`
40064 (windowshade) or 40165 (double size) is **posted to the equalizer's own
window** — no real click, no menu, no keyboard. Each job captures the state
before and after, which is what proves the action changed something.

## First measurement, 2026-10-21 (superseded by the section below)

The comparator compares the **client area** of the reference (`Winamp 2.95`, skin
`base-2.91.wsz`, 1024x768, 96 dpi) with ours, same state, same skin, same size.

| Case | differing pixels | share |
|---|---|---|
| `main-active` | 3 675 / 31 900 | 11.5 % |
| `main-inactive` | 2 099 / 31 900 | 6.6 % |
| `main-shade` | 1 246 / 3 850 | 32.4 % |
| `main-double` | 14 700 / 127 600 | 11.5 % |
| `eq-active` | 7 408 / 31 900 | 23.2 % |
| `eq-inactive` | 7 408 / 31 900 | 23.2 % |
| `playlist-active` | 1 510 / 31 900 | 4.7 % |
| `playlist-inactive` | 1 510 / 31 900 | 4.7 % |
| `eq-shade`, `eq-double`, `playlist-shade` | **state not reached** | - |

**Over the eight comparable states: 39 553 of 322 850 pixels differ (12.25 %).**

### Three states the player does not reach at all (measured defects)

The reference and our player disagree on the *size*, so these three are not pixel
differences but missing behaviour:

| Case | Reference | Ours | Command |
|---|---|---|---|
| equalizer, windowshade | 275x14 | 275x116 | `WM_COMMAND` 40064 to the equalizer |
| equalizer, double size | 550x232 | 275x116 | `WM_COMMAND` 40165 to the equalizer |
| playlist, windowshade | 275x14 | 275x116 | `WM_COMMAND` 40064 to the playlist |

The **main** window answers both commands correctly (275x14, 550x232), so routing
and resizing exist; the equalizer and the playlist never apply the command to
themselves. `wave339` fixes it; these three rows are re-measured afterwards.

### How to read the numbers

A differing pixel is a pixel whose colour is not the reference's. The shares say
**nothing** about why; the bounding boxes of each comparison do (`bounds=` in the
comparator's stdout): the title band, the marquee, the analyser and the playlist
rows differ in different cases. Turning a share into a work item means reading
the diff image, not the percentage.

## Measured parity after the fixes, 2026-10-21 (all eleven states)

Supersedes the first table above. Between the two measurements the player gained
the ABI window classes (`Winamp EQ`, `Winamp PE`), sibling windows that no longer
take the focus, and a Win32 seam that delivers `WM_COMMAND` 40064/40165 to the
equalizer and the playlist - so the three states that were previously
*unreachable* now have their sizes, and the focus states are real states.

| Case | differing pixels | share | before |
|---|---|---|---|
| `main-shade` | 190 / 3 850 | **4.9 %** | 32.4 % |
| `playlist-active` | 1 510 / 31 900 | **4.7 %** | 4.7 % |
| `playlist-inactive` | 1 510 / 31 900 | **4.7 %** | 4.7 % |
| `main-active` | 2 099 / 31 900 | **6.6 %** | 11.5 % |
| `main-inactive` | 2 099 / 31 900 | **6.6 %** | 6.6 % |
| `main-double` | 8 396 / 127 600 | **6.6 %** | 11.5 % |
| `playlist-shade` | 507 / 3 850 | **13.2 %** | unreachable |
| `eq-active` | 7 408 / 31 900 | **23.2 %** | 23.2 % |
| `eq-inactive` | 7 408 / 31 900 | **23.2 %** | 23.2 % |
| `eq-double` | 29 632 / 127 600 | **23.2 %** | unreachable |
| `eq-shade` | 1 072 / 3 850 | **27.8 %** | unreachable |
| **all eleven** | **61 831 / 458 150** | **13.5 %** | - |

**The focus rendering is right.** Our `main-active` and `main-inactive` captures
differ by 1 576 pixels in the title band `(6,4)-(271,10)`; the reference's two
captures differ by **the same 1 576** in the same band. Same for the equalizer
(1 435) and the playlist (1 388). The focused and unfocused title bands are
therefore pixel-identical to the reference's; none of the remaining differences
is a focus problem.

**Where the remaining differences live** (from each comparison's bounding box):

- `main-*`: the whole client area, `(16,27)-(263,105)` - the readout, the
  marquee text and the analyser. `main-shade` is down to its 38x5 analyser
  rectangle `(188,5)-(225,9)`, the one difference S08 has to decide (the
  reference leaves its own skin graphic there).
- `eq-*`: the interior, `(22,18)-(253,100)` - the slider frames, the graph ramp
  and the value strip. This is the largest single gap in the set.
- `playlist-*`: the rows and the title cells, `(6,4)-(271,93)`; the shade strip
  is `(6,1)-(271,12)`.

**Double size is global, windowshade is not.** Measured with two jobs that send
the command to the equalizer and capture the **main** window: 40165 took the
main window from 275x116 to 550x232 (`base291-global-double`), 40064 left it at
275x116 (`base291-global-shade`). One `dsize` flag for the player (S01 section
1), windowshade per window. The player's command handler was corrected to match,
and `window_commands.rs` pins it.

## The equalizer interior, re-measured in the VM after `wave342` (2026-10-21)

The headless claim was 0 differing pixels; the real OS window confirms it to
within a state difference:

| Case | before | after | where the remainder is |
|---|---|---|---|
| `eq-active` | 7 408 (23.2 %) | **6** | `(44,22)-(46,23)` - the auto indicator |
| `eq-inactive` | 7 408 (23.2 %) | **6** | same rectangle |
| `eq-double` | 29 632 (23.2 %) | **24** | the same 6 px at 2x |

Six pixels in a 3x2 rectangle are the **auto flag**, not a drawing error: the
indicator is lit on one side and dark on the other. That is a state question
(does our player start with auto enabled?), not a renderer question, and it is
the last thing the equalizer still owes.

`eq-shade` stays at 1 072 / 3 850 (27.8 %) for a reason outside the interior:
the reference capture shows the **unfocused** shade band (`eq_ex.bmp` row 15)
while ours shows the focused one, and Winamp additionally draws an "EQ" label in
the band with a system font that the skin does not carry. Both are their own
cases - focus state, and system-font text - not slider drawing.

## Open, and named so it is not lost: the shade bands' focus state

`eq-shade` (1 072 / 3 850) and `playlist-shade` (507 / 3 850) both compare a
**focused** strip of ours against an **unfocused** strip of the reference. The
skin carries both variants (`eq_ex.bmp` row 0 vs row 15), and the focus
rendering itself is proven exact by the active/inactive pairs (their deltas match
the reference's to the pixel). So this is a **state** difference in the capture
job, not a drawing defect:

- the reference job toggles the shade with a posted command and the window is
  not focused afterwards,
- our presented job opens the equalizer with a posted command, and the window is
  focused when it is captured.

Two things to settle before either number can be read as a drawing difference:
1. does our player take the focus when a window command arrives (a posted
   `WM_COMMAND` should not change the focus at all)? If it does, that is a real
   defect in the seam, not in the renderer;
2. the capture job should put both sides in the same focus state - add a variant
   that clicks the main title bar before capturing the shaded strip, and keep the
   existing one, so the two states are both recorded instead of compared against
   each other.

`eq-shade` also carries the "EQ" label Winamp draws into the band with a system
font the skin does not ship - its own case (system-font text), not a strip
drawing question.

## Measured 2026-10-22 (after the resize-storm fix `4478a0e`)

The six states the last three UI lanes changed were re-captured in the guest
(`campaign6`, all runs exit 0, every file verified byte-identical) and compared
against the reference captures pixel-exactly with
`tools/vm-capture/Compare-Images.ps1`.

| state | differing pixels | what is left |
|---|---|---|
| `main-active` / `-inactive` | 132 | 120 are "RUSTAMP" where the reference draws "WINAMP 2.95" (our identity, not a defect); 12 are the idle decoder cell |
| `main-shade` | 190 | the shade strip's remaining glyph cells |
| `main-double` | 528 | the 2x text run of the same title row |
| `eq-active` / `-inactive` | 6 | the auto indicator's state |
| `eq-double` | 24 | the same indicator at 2x |
| `eq-shade` | 1 072 | focused strip vs the reference's unfocused one, plus the system-font "EQ" label |
| `playlist-active` / `-inactive` | 97 | GDI text the skin does not ship |
| `playlist-shade` | 86 | the same text on the band |
| **total** | **2 370 / 458 150 = 0.52 %** | |

Before this measurement the set stood at 17 419 / 458 150 (3.8 %). The two
largest single findings on the way are recorded above: the reference draws no
position bar and no analyser while nothing is loaded, and our playlist rendered
its focus state from the main window's flag.

The three states that still dominate are all **not rendering defects** but open
questions: `eq-shade` needs both sides captured in the same focus state,
`main-double` shares the identity text, and the playlist rows are text the skin
does not carry.


### What the remaining `main-shade` pixels are (diff image, 2026-10-22)

`diff-main-shade.png` (from `Compare-Images.ps1 -DiffOut`) shows the difference is
one contiguous run: after the play button the reference draws the shade strip's
transport glyphs (pause, stop, next, eject), and our strip draws a row of dark,
near-empty cells instead. Nothing else in the 275 x 14 strip differs - logo,
"WINAMP"/"RUSTAMP" text, the EQ and PL toggle icons, the green indicator, the
volume bar and the window buttons all match.

So `main-shade` is a **missing sprite row in the shade strip**, not a state or
focus question, and it is the last rendering gap in the main window. `eq-shade`
(1 072) is still unresolved: it needs a capture pair in the same focus state
before the number means anything.

