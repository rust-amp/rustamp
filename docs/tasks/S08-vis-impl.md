# S08 built-in visualisation implementation (2026-10-18)

**Status:** own-choice analyser, scope, display envelopes, UI input/render path
and golden baselines implemented. **Production engine-to-UI feed still needs the
orchestrator's app change below.** This lane does not claim a running-player
acceptance, a passed numeric suite, an S08 measurement, or reference identity.

**Worktree / boundary:**
`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave87-vis`.
Initial `git status --short` was empty. Write only the owned UI files, the tap's
documentation, this card and the blessed images here; nothing under
`D:\code\Rustamp`, `D:\code\rustamp-research` or `D:\code\rustamp-refbin`.
Cargo products/logs remain under this worktree's `target/`.

## 1. What existed

| Evidence | Existing implementation / limitation |
|---|---|
| `crates/rustamp-vis/src/lib.rs:4-8` | Skeleton only; no analyser, scope, or conversion. Left untouched: not owned by this lane. |
| `crates/rustamp-audio/src/vistap.rs:8-16,106-161` | Actual latest 576 waveform samples/channel, oldest first; mono duplicated; spectrum zero; decode-end timestamp; one publication per sufficiently large decode block, not a fixed-hop analysis stream. Only explanatory documentation added. |
| `crates/rustamp-proto/src/vis.rs:12-28` | Frozen 2 x 576 frame, no source-rate/seek-generation/valid-sample-count metadata. No contract change. |
| `crates/rustamp-audio/src/engine.rs:678-687` | `take_vis_frame()` already drains to the newest frame. |
| `crates/rustamp-audio/src/decode.rs:670-698` | Tap after ReplayGain/DSP/startup trimming, before device conversion. `crates/rustamp-audio/src/ring.rs:357-358` applies volume/balance later in the output callback. These are Rustamp code facts, not reference findings. |
| `crates/rustamp-app/src/app.rs:647-657,685-716` | App already refreshes/renders `MainWindow`, but never calls `take_vis_frame()` or supplies PCM to it. No existing call site anywhere in that crate. App remains untouched. |
| `crates/rustamp-ui/src/render.rs:149-199` (pre-change) | Main renderer drew other controls only, with no visualisation. Layout and render state likewise had no analyser field. |
| `docs/HANDOFF.md:162,170`; `docs/tasks/S08-vis.md:3-33` | Raw tap / skeleton and the missing derivation/cadence were explicitly known. |

## 2. What the measurement design does and does not settle

`docs/tasks/S08-measurement.md:3-17` is **design only**: no reference value is
measured. Its hypotheses are not an implementation specification. No
`specs/S08-vis.md` was created or implied here.

All Q1-Q11 reference rows remain **open**, in the S02 §0 / design §3.3 sense:
there is no evidence to grade as settled, provisional, visual reading or audited.
In particular, "provisional" means *measured but not understood*, not *our guess*.
The module header, constant/API comments and layout field say **own choice**
and **reference grade: open** rather than relabelling guesses as measurements.

Capture jobs may settle **observable** mapping/resolution brackets, count/pitch,
marker hold/fall, height/colour law, stereo folding, visible cadence, bar fall,
area/static pixels, mode gestures/styles, volume sensitivity and stop/pause
rules (Q1-Q11, design lines 41-53). The prescribed receipts are region-only diff
count, bbox, row/column profiles and colour-slot histograms (lines 213-231),
across at least **three independent restores**, with static byte identity or
metric-identical animated geometry at 0-pixel tolerance (lines 233-267).

Captures **cannot** establish the exact transform/window/overlap, exact bin
weights/scaling, plugin 8-bit scaling/sign, timestamp semantics or audio-to-pixel
selection/latency (lines 271-284). Those need the primary probe or simultaneous
audio/pixel rig. Exact timer implementation is not a required observable spec.
Playlist `WinampVis` and built-in behaviour while a plugin runs need their own
series/acceptance. Skin RGB values are data, not measured behaviour.

Replacement probes remain exactly those named by the design: `static`, `map`
(including pass-2 edges), `level`, `stereo`, `mode`, `time`, `volume`, then the
primary plugin probe and audio/pixel rig for non-visible facts. Nothing here
advances their grade or claims they ran.

## 3. Implementation and every own-choice number

The implementation lives in `crates/rustamp-ui/src/visualization.rs`, deliberately
separate from the neutral/plugin-facing `VisFrame::spectrum`. No new dependency,
copied algorithm, allocation on the audio path, core RFC or wire change.

| Choice | Value / rule | Why; replacement evidence |
|---|---|---|
| Main-window area | `(24,43,76,16)` at 1x; half-open rectangle | Fits below the measured time display and left of volume/stream fields without overlap. **Not measured by S01.** Even the design's approximate y region is a hypothesis. Q8 must replace it if different. Shade draws no analyser; existing exact integer 2x doubles everything. |
| Transform | Direct real DFT of **576 actual samples**, f64 accumulation/rotation recurrence; periodic Hann `0.5 - 0.5*cos(2*pi*n/576)`; subtract channel mean first | Uses every available sample without inventing a larger FFT history or zero padding. A dependency was forbidden for this lane; bounded tiny direct transform runs on UI only. No overlap/hop assumption; each offered snapshot analysed independently. Exact transform is primary-probe work. |
| Magnitude normalisation | `2/sum(window)` for ordinary positive bins, `1/sum(window)` at Nyquist; clamp display amplitude at 1 | Integer-bin full-scale sine is 1; real Nyquist coefficient is not doubled. PCM is never altered. Plugin scaling is left untouched; primary probe decides any future compatibility derivation. |
| Stereo | Max of per-channel bin magnitudes, then max bin per band | Both channels contribute without antiphase cancellation. Multi-channel tap already exposes only its first two rows (mono duplicates). Q5 / primary probe can replace this. |
| Bands | **19** half-open bands with edges `[1,2,3,4,5,7,9,12,16,21,28,37,49,65,86,114,151,200,264,289]` | Every available positive bin 1..288 used once, DC excluded; widening high bands fit a small display. Not a claim of reference logarithmic mapping. Bin k is `k*source_rate/576` Hz: bin 1 at 44.1 kHz is **76.5625 Hz**, Nyquist at 48 kHz is **24 kHz**. Cannot pretend to resolve the design's 31.25 Hz ladder with this short tap. Q1 gives display brackets, exact weights need probe. |
| Height law | **-60..0 dBFS**, `20*log10(amplitude)`, nearest integer **0..16** rows, silence/below-floor 0 | Useful quiet-to-loud dynamic range at only 16 rows; clips display above full scale. Q4 / primary probe. |
| Bars / pitch | **3 px** wide + **1 px** gap, **4 px** pitch, 19 pitches = 76 columns; no vertical row gaps | Legible separated bands in the chosen area; last column remains a gap. Q2. |
| Background / palette | Opaque alpha **255**; slot **0** background, **1** grid dots on even x/y (**2 px** pitch), **2..17 bottom-to-top** gradient, **23** marker, **18** scope | Uses the loaded 24-slot palette, no RGB invented by the renderer. The gradient orientation deliberately remains an own choice, not a promotion of S17 §6's *provisional* top-to-bottom wording. Q4/Q8 determine real slot usage/orientation. |
| Bar envelope | Immediate attack, linear **24 rows/s** fall | Responsive rise, visible decay. No frame-rate dependence; no reference rate claimed. Q7. |
| Peaks | Equal/higher supplied height renews **300 ms** hold; then **16 rows/s** linear fall; never below bars; **1 px** high marker, same 3-px width | Separate readable slower marker. Hold/fall rates are not measured. Q3. |
| Envelope precision | **1000 units/row**, injected **ms** clock; ceiling when rasterising falling heights | Integer arithmetic, exact time partitioning and reproducible pixels without float drift. These units encode the chosen rates, not reference sub-pixel facts. |
| Frame freshness | **100 ms** of active UI time, including the boundary; later stale | Avoids retaining a disconnected source forever. Split integration at the expiry boundary so one large step equals many small steps. Missing/stale is explicitly not silence: scope is absent, last bars/peaks fall. Q6/Q11 or a better engine presentation feed could replace it. |
| Scope sampling | **76** evenly distributed nearest actual indices spanning **0..575**; arithmetic mean of the two channels; positive up; clamp display to [-1,1]; nearest rows **0..15**, real zero at row **8** | No interpolated/fabricated PCM; endpoint and sign are numeric tests. Antiphase scope cancels by explicit choice, unlike spectrum. Q5/Q9 / primary probe. |
| Scope strokes | **1 px** dots, or dots joined by one-column vertical bridges | Two recognisably different deterministic styles without anti-aliasing. Q9. |
| Modes / gesture | Spectrum (default) -> connected scope -> dots -> off -> spectrum; left press+release inside area | UI-local own choice, no new core command, plugin mode, right-click claim or persistence. Off draws only background; never changes engine/plugin data. Q9. |
| Pause / stop / discontinuity | Pause freezes envelopes, scope and freshness; supplied paused/stopped frames discarded. Stop clears targets/scope immediately but lets spectrum tail fall. Seek/track change resets; rate change clears envelopes. | Defined usable behaviour where Q11 is unmeasured. Invalid/non-finite PCM or rate 0 returns an error, not substituted audio. Missing input never becomes a synthetic zero waveform. |
| Cadence / timestamps | No new timer. Existing app presenter requests **16 ms** frames (`crates/rustamp-app/src/window.rs:77`); analysis follows accepted snapshots, envelopes follow supplied time; source timestamp retained unchanged | Actual visible refresh and audio-clock alignment are unmeasured. Tap publishes on decode blocks, may drop when full and may lead audible output. No sample-accurate lip-sync claim. Q6 / primary probe / rig. |

Golden-only fixture numbers are likewise ours: 44.1 kHz input, bins 8/128 at
amplitudes 0.5/0.25 (separate channels), timestamp 13061 us; scope square ±0.8
with a 144-sample period / 72-sample half; snapshots at 0, 425, 500, 1000 and
1425 ms. They expose separated bands, decay/hold, pause, stroke differences and
full height without depending on host clock/audio. They are not reference facts.

## 4. Wiring and the non-owned app seam (required before calling this complete)

Within owned paths the chain is complete:

`MainWindow::push_vis_frame(frame, source_rate_hz, now_ms)` -> pure analysis ->
`Visualizer` envelopes -> `MainWindowState::visualization` ->
`render_main_window` -> palette pixels. `MainWindow::refresh` advances it;
track/seek paths reset it; the existing pointer route reaches the new widget.

**The production app has no frame-forwarding call today.** A UI crate cannot
borrow the app's private `Engine`, and changing the tap into a global singleton
or encoding data in unrelated events would be a dishonest architectural bypass.
This lane obeyed the express prohibition on modifying `crates/rustamp-app`.

The app owner must add, in `App::pump` **after engine polling and event draining**
and before its final `self.ui.refresh`:

```rust
if let Some(frame) = self.engine.take_vis_frame() {
    let source_rate_hz = self.engine.state().stream.map(|s| s.sample_rate_hz).unwrap_or(0);
    let now_ms = self.now_ms();
    if let Err(error) = self.ui.push_vis_frame(&frame, source_rate_hz, now_ms) {
        self.note(error.to_string()); // Use the app's normal diagnostic route.
    }
}
```

Use the actual source rate, never the output-device rate or displayed/rounded
`khz`. Rate **0** explicitly refuses analysis. The frame itself cannot prove
format/generation correspondence across seeks/track switches; external seeks
not passing through the UI should call `reset_visualization`. The app owner
must test transport/event ordering and frame arrival. No synthetic timer-based
bars or placeholder samples stand in for this missing call.

The analyser is frequency-normalised (no printed Hz labels); a later correct
presentation feed can improve sample/rate/generation pairing and audible-time
selection. A latest decode snapshot is not proof of what is audible now.

## 5. Tests, goldens, verification and VM boundary

- **11 new unit tests** in `crates/rustamp-ui/src/visualization.rs`: silence/DC,
  Hann coherent gain and concrete dB heights, exact band coverage and Hz units,
  left/right/antiphase and two tones, Nyquist scaling, actual scope endpoints,
  missing-rate/non-finite refusal and ignoring plugin spectrum, exact hold/fall/
  stale boundaries, refresh partition/pause/backwards clock, stop/reset/rate
  change, palette/pitch/clipping, distinct scope strokes and no fake quiet line.
- **3 new headless integration tests** in
  `crates/rustamp-ui/tests/visualization.rs`: PCM -> render state, pause/seek/
  track lifecycle, mode hit/press/release without emitting an audio command.
- **10 new images per skin**, shared public-API case generator in
  `crates/rustamp-ui/tests/golden_cases/mod.rs:347`: `vis_spectrum`,
  `vis_spectrum_decay`, `vis_spectrum_paused`, `vis_scope`, `vis_scope_dots`,
  `vis_scope_silence`, `vis_scope_missing`, `vis_off`, `vis_stopped_tail`,
  `vis_spectrum_double_size`. Both lookalike and rust-theme, exact tolerance 0;
  paused identity and whole-frame exact 2x asserted. Also refreshed the three
  existing `main_window*` images per skin for the new static palette/grid area.
- Inspected the generated Rust-theme spectrum and scope PNG bytes through the
  file reader; did not open a browser, native image viewer or player window.
- `cargo fmt --all -- --check`: **pass**.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **pass** after fixing local import / initialiser diagnostics. No lint weakened.
- `cargo build --workspace`: **pass**, offline.
- `cargo xtask golden --skin lookalike --bless` and
  `cargo xtask golden --skin rust-theme --bless`: **pass**, 16 selected golden
  tests per namespace, zero failures. The first lookalike invocation hit the
  shell's 120-second compile/startup timeout; rerun with 300 seconds completed.
- Matching `--check` runs: **pass**, 16 selected tests each, no mismatch
  artefacts. `RUSTAMP_TESTDATA` unset; no classic-base comparison claimed.
  Logs: `target/golden-lookalike-check.log`,
  `target/golden-rust-theme-check.log`, `target/golden-rust-theme-bless.log`.

**No direct `cargo test`, non-golden numeric/integration test, VM, audio or GUI
run.** The expressly authorised `cargo xtask golden` internally compiles and
invokes libtest with a namespace filter; only the headless golden cases execute.
The numeric/integration tests compiled under clippy/golden but their runtime
results still belong to the orchestrator's VM suite. No commit or other git
write; no download/install/new dependency. All clean-room inputs were repository
code, S01/S02/S17 and the two S08 cards; no dirty source, SDK, disassembly,
reference binaries or research tree was read. No third-party code copied.

**Only the VM can prove:** the app seam actually consumes tap frames during real
playback, live mode clicks, actual pause/seek/track transitions, output-device
behaviour, usable cadence/performance and timing vs audible audio. Reference
identity additionally requires the design's captures/probe/audit; passing our
own golden images is not that evidence.

## 6. File and boundary receipt

Final line counts and `git status --short` are supplied in the lane's final
metadata report. Status must contain only this card, the owned UI/tap paths and
the 26 intended golden image changes (20 new + 6 refreshed). The canonical tree
was neither opened for writing nor used as a command working directory; the
orchestrator independently checks its status during integration.
