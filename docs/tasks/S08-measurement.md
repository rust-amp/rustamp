# S08 measurement design — what the built-in visualiser draws (captures only)

**Status:** design only. Nothing here is measured; every "hypothesis" is a model to
falsify, not a fact about the reference. `specs/README.md` still lists S08 as
**not started**; `rustamp-vis` is a skeleton; the engine's vis tap publishes
waveform-only frames with the spectrum left at zero
(`crates/rustamp-audio/src/vistap.rs:13-16`, "S08 owns this").

**Owned path:** this file only (`docs/tasks/S08-measurement.md`). No crate, no
runner, no job JSON, no test was touched, nothing was run, nothing was committed.

**What this card designs:** the **capture lane** of S08 — the secondary instrument
named in [docs/tasks/S08-vis.md](S08-vis.md) ("Captures … for the falloff speeds,
the peak hold and the refresh rate — the parts that are *visual* and have no
plugin-facing number"). The **probe plugin** (the primary instrument) owns the
numeric facts; §4 lists which facts belong to it and why captures cannot reach
them. The two lanes must not duplicate work.

**Read as input:** AGENTS.md, CLEANROOM.md, S08-vis.md, HANDOFF.md,
M14-host-framework-review.md (the finding-table shape used below),
`crates/rustamp-audio/src/vistap.rs`, `docs/notes/vm-run-capture.ps1`,
`docs/notes/vm-broker/guest/run-capture.ps1`, `docs/notes/vm-capture-runbook.md`,
`docs/notes/VM-capture-host.md`, `docs/notes/ocr-for-captures.md`,
`specs/S01-window-geometry.md`, `specs/S02-main-window-behaviour.md`,
`specs/S03-shortcuts.md` (transport command ids), `specs/S04-menus.md` §5,
`specs/S09-plugin-abi.md` §3.8, `specs/S17-skin-loading-quirks.md` §6.

---

## 1. The questions

One row per question; each is answerable by **one capture job** (§2.5). The seven
rows the card asks for are Q1–Q7; Q8–Q11 are supporting rows that S08-vis.md's
scope ("the built-in window's rendering") forces anyway — Q8 is a precondition for
every analysis region, Q9 is the card's "oscilloscope styles", Q10 and Q11 are
one-capture states the render must reproduce.

Verdict vocabulary is §3. "H" = what the reference must be doing under the
hypothesis a later spec would state first.

| # | Question | H: what the reference must be doing | Alternative hypothesis and its visible signature | Decisive capture (one job) |
|---|---|---|---|---|
| Q1 | **Frequency range and resolution ("bin count")** | Bars map the spectrum logarithmically over roughly the audible band: equal-*ratio* tones light bars at equal log-spaced positions; tones outside the covered band light nothing | **Linear-in-Hz mapping:** equal-ratio tones advance by widening steps of bars — bar centroid vs log(f) is curved, vs f is straight. **Band-split:** every tone above some frequency lights only the top bar(s) | Steady frame per octave tone, 31.25 Hz … 16 kHz (§2.3), then the pass-2 edge probes (§2.6); a two-tone frame (200 Hz + 1.6 kHz) checks bar independence |
| Q2 | **Bar count and pitch** | The area decomposes into N discrete bars at a fixed pitch (bar width + gap); each bar is a contiguous column block | **Continuous raster:** no periodic gaps in the column profile. **Dot matrix:** periodicity in both axes (row *and* column profiles) | One full-scale square steady frame (all bars lit — the grid is maximally visible) plus one mid-tone frame; column profile |
| Q3 | **Peak hold and its decay** | A peak marker in a distinct colour (plausibly the slot-23 colour, S17 §6) holds at the highest reached row for a hold time, then falls at its own rate, independent of the bars | **No separate marker:** the top row's colour equals the bar colour and disappears with the bar. **Marker falls with the bars:** marker row and bar-top row decay identically | Stop-transition burst (§2.6): tone playing → posted stop → 120 frames at ~60 fps + 8 tail frames at 250 ms; marker row vs bar-top row per frame |
| Q4 | **Colour and height mapping** | Bar height is a monotone function of level, drawn as stacked rows coloured by the 16-colour ramp (S17 §6 slots 2–17) in a fixed orientation | **Colour by level, not by row:** the whole bar is one colour that changes with height. **Law ambiguity:** equal-dB steps move equal rows (dB law) vs equal-amplitude steps move equal rows (linear law) vs a power law — the ladder separates them | Steady frame per amplitude level, 1 kHz at 0/−3/−6/−12/−24 dBFS (§2.3); row profile per frame; rendered RGB matched against the base skin's `viscolor.txt` slots (free data) |
| Q5 | **Stereo split** | Both channels fold into the single row of bars (max or average), so a left-only and a right-only tone render the same | **Left channel only:** the right-only frame is silent. **Frame alternation:** a burst alternates between two patterns. **Two half-height displays:** upper and lower halves of the area respond to different channels | Steady frames: left-only, right-only, mono, antiphase (L = −R), two-frequency stereo (L 500 Hz, R 4 kHz); one short burst for the alternation case |
| Q6 | **Visible effect of refresh rate** | The vis region repaints on a fixed wall-clock cadence: between updates consecutive captures are identical, and a refresh setting (if one exists) changes the measured update period | **Unlocked cadence:** every capture differs (updates at or above the capture rate). **Audio-clocked cadence:** the inter-change gap correlates with playback position, not wall time | 240-frame burst at ~60 fps during a steady tone (~4 s), repeated per discovered refresh setting (§2.6 discovery step); the inter-change gap histogram is the metric |
| Q7 | **Bar falloff** | When the signal stops, bar tops fall by a fixed number of rows per update (linear in updates — the classic decay look) | **Exponential:** each update multiplies the height by a constant factor — straight on a log scale, not linear. **No falloff:** instant collapse. **Rate in ms, not updates:** the rate changes with the refresh setting while the per-update step stays fixed | The same stop-transition burst as Q3; bar-top row vs update index; repeat at two refresh settings if they exist |
| Q8 | **(supporting) Vis-area rectangle and static render** | A fixed rectangle in the main window; the play indicator at its left edge is the S01 anchor (9×9 at (26,28)); static states draw the background plus a dot/grid (slots 0/1) | **Fully blank static render** (no grid). **Grid only while playing.** The animated extent differs from the static extent | Stopped/no-track frame, playing-silence frame, and the union of changed regions over every other job's frames |
| Q9 | **(supporting) Oscilloscope styles and mode switching** | More than one scope style exists (S08-vis.md: "oscilloscope styles") and is reachable by a posted click inside the vis area; styles differ in stroke (connected line vs dots vs filled) | **One style only.** **Clicks do nothing** — then the control hunt moves to the preferences UI and the row stays open with that probe named | Posted left and right clicks at the measured area centre (§2.6); one frame after each; per discovered style, sine and full-scale-square frames |
| Q10 | **(supporting) Where the tap sits** | The analyser reads the signal **before** the volume control: bars identical at any volume | **Post-volume:** bars shrink proportionally with volume | One steady tone at two volume settings (set and read back per §2.6), one frame each |
| Q11 | **(supporting) Stopped and paused rendering** | Stopped blanks the area to the static render; paused freezes the last frame | **Stopped keeps the last bars.** **Paused keeps animating** (or blinks with the time display's measured 1.30 s half-period, S02 §4) | The stop-burst tail (Q3) and a 30-frame paused burst |

---

## 2. The instrument

### 2.1 Capture path (existing, unchanged)

Everything runs through the documented VM path — nothing new is built for this:

- Host runner `docs/notes/vm-run-capture.ps1`: restore baseline checkpoint
  `base-provisioned` → pin the virtual display (`Set1024`, self-verified
  1024×768) → stage harness + one job + reference copy + media (blind copies) →
  run **exactly one job** in the guest's interactive session
  (`docs/notes/vm-broker/guest/run-capture.ps1`, scheduled task in session 1) →
  `copyout` → SHA-256 re-verification → `run-provenance.json`
  (checkpoint name/creation, restore stamp, guest boot time, display facts,
  harness exit).
- Harness gates (unchanged): client rect exactly 275×116, capture 275×116, the
  landmark invariant region (0,49,275,67) at zero differing pixels plus twin
  SHA-256 identity. The landmark covers y 49–115 only — the vis area sits above
  it (y ≈ 28–43), so the gate does **not** freeze the region under test; the
  comparisons in §3 supply their own baseline.
- One job per run is enforced by the guest script; a question split across two
  jobs is two runs, not one.
- Burst capture at ~60 frames/s is a proven capability (S02 §3 measured the
  marquee step from consecutive captures at about 60 frames/s).
- Menu/label reading follows `docs/notes/ocr-for-captures.md` (`Windows.Media.Ocr`,
  3× NEAREST crop, `[uncertain: …]` markers). A guessed label is worse than an
  unreadable one.

### 2.2 Signals, and why these signals

| Signal | Makes visible | Why it discriminates |
|---|---|---|
| **Digital silence** (10 s WAV) | The static render: background, grid dots, nothing else | The only "zero" state; every lit-pixel definition and the falloff target hang off it; it is a byte-identity anchor |
| **Steady sine tones**, octave ladder (31.25 Hz … 16 kHz, −6 dBFS) | Exactly the bar(s) whose band contains f | Spectrally pure and amplitude-controlled: the tone→bar table *is* the frequency mapping; the octave spacing makes log vs linear separable by fit |
| **Amplitude ladder** (1 kHz at 0/−3/−6/−12/−24 dBFS) | Height law and colour law | Five known levels on one frequency: the height-vs-level curve and the per-row colour, free of frequency effects |
| **Full-scale square** (1 kHz, ±32767) | The complete bar grid at once; the colour ramp end-to-end; the scope's flat-top stroke; the saturation ceiling | Every band is excited simultaneously — bar edges are most visible; flat tops make the scope stroke's verticals obvious; clipping behaviour is pinned |
| **Stereo fixtures** (left-only, right-only, mono, antiphase, two-frequency) | The channel fold rule | L-only ≡ R-only rules out channel selection; antiphase separates max-folding (lit) from average-folding (≈ silence) from summing (clipped); two-frequency shows whether both channels' spectra appear |
| **Two-tone superposition** (200 Hz + 1.6 kHz) | Bar independence | Whether two bands light simultaneously and add, or one peak wins |
| **Message-driven transitions** (posted play/pause/stop mid-tone) | Attack, peak hold, falloff, pause/stop rendering | The transition time is controlled by the job, not by decode or file length; S03 §1 gives the audited ids: play 40045, pause 40046, stop 40047 |

Deliberately **not** used: noise and real music (uncontrolled bar patterns —
useless for a mapping table), frequency sweeps (harder to analyse than the ladder
for the same information), testmedia MP3 jingles (uncontrolled content; they stay
the precedent for *how* media is staged, not a signal).

### 2.3 Fixtures (exact)

All fixtures are **our own bytes**, generated by a small script of ours in the
executing lane's tree — no reference material is involved. Common form: 44 100 Hz,
16-bit signed little-endian, stereo, so the analyser's input rate is constant and
no resampler sits in the path. Full scale = 32767.

| File | Content |
|---|---|
| `s08-silence.wav` | 10 s of zeros |
| `s08-tone-<f>.wav` | sine at f ∈ {31.25, 62.5, 125, 250, 500, 1000, 2000, 4000, 8000, 16000} Hz, amplitude 16384 (−6 dBFS), both channels in phase, 10 s, trimmed to a whole number of cycles (no click at the loop point) |
| `s08-level-<db>.wav` | 1 kHz sine at 0 / −3 / −6 / −12 / −24 dBFS (amplitudes 32767, 23200, 16384, 8228, 2067), 10 s each |
| `s08-square-1k-full.wav` / `s08-square-1k-half.wav` | 1 kHz square at ±32767 / ±16384, whole cycles, 10 s |
| `s08-lonly-1k.wav`, `s08-ronly-1k.wav` | 1 kHz sine, amplitude 16384, in one channel, zeros in the other |
| `s08-mono-1k.wav` | 1 kHz sine, amplitude 16384, both channels |
| `s08-antiphase-1k.wav` | L = +16384, R = −16384 |
| `s08-stereo-2f.wav` | L = 500 Hz, R = 4000 Hz, each −6 dBFS |
| `s08-twotone.wav` | 200 Hz + 1600 Hz, each amplitude 11627 (−9 dBFS; peak sum 23254, no clipping) |
| `s08-tone-long.wav` | 60 s of 1 kHz at −6 dBFS (the transition host) |
| `s08.m3u` | the files above in a fixed order |

If the reference refuses `.wav`, the executing lane re-encodes the identical
generators to MP3 with the repo-available ffmpeg and records the substitution in
the run provenance (a codec then sits in the path — recorded as a held constant,
§2.4). The fixtures are staged into the guest exactly like the testmedia directory
(`D:\code\rustamp-testmedia` → `C:\rustamp\testmedia` precedent, blind copy).

### 2.4 Held constant across every job

- **Skin:** the original base skin `base-2.91` (the webamp MIT skin — free data,
  the S01 fixture). Colours are skin data; the measurement settles slot *usage*,
  not colours.
- **Display:** `Set1024` pin + guest pre-gate + harness gate (1024×768,
  `LogPixels` 96). All captures at 1×; double-size is a proven exact 2× doubling
  (S01), so 1× measurements transfer.
- **Provenance state:** checkpoint restore before every job; the run-provenance
  triple (checkpoint, restore stamp, boot time) present.
- **Audio state:** equalizer disengaged and flat (the EQ window stays closed; the
  ON state is visible in the render and is recorded); no DSP or vis plugin
  installed; every fixture 44.1 kHz; volume fixed except in the Q10 job.
- **Window state:** nothing moved, no third window, the main window focused;
  every capture asserts the landmark gate.
- **Input method:** posted window messages for transport and mode probing; real
  input only where a recipe says so (the S01 round-3 lesson: posted and real input
  differ — the second real click is the one that fires). The input method is
  recorded per step, per the provenance rule.

### 2.5 Jobs — one per question, frame budgets

| Job | Questions | Steps ≈ | Frames |
|---|---|---|---|
| `s08r1-static.json` | Q8, Q11 (stopped half) | ~30 | ~40 |
| `s08r1-map.json` | Q1, Q2 | ~90 | ~50 (ladder 10×3 + square + two-tone + pass-2 probes) |
| `s08r1-level.json` | Q4 | ~25 | ~17 |
| `s08r1-stereo.json` | Q5 | ~25 | ~17 + 30-frame burst |
| `s08r1-mode.json` | Q9 + Q6's discovery step | ~60 | ~25 + menu captures |
| `s08r1-time.json` | Q3, Q7, Q6, Q11 (paused half) | ~40 | ~120+8 (stop burst) + 30 (pause burst) + 240 per refresh setting |
| `s08r1-volume.json` | Q10 | ~15 | ~6 |

Run order and dependencies: `static` → `map` → (`level`, `stereo`) → `mode` →
`time` → `volume`. The `mode` and `time` jobs need the measured area rectangle
(Q8) and the discovered controls (Q9's clicks). Every discriminating job runs at
least **three times on three independent restore cycles** — that is what §3's
"settled" requires. The proven envelope is the S01 round-3/3b jobs (136 and 117
steps, 69 in the held-states run); keep every job at or under ~200 steps and split
a question into a second job rather than exceed it; the guest script's 30-minute
inner deadline is the hard wall — if a 240-frame burst threatens it, split the
burst across two jobs at a recorded frame boundary.

### 2.6 Step recipes (fixed rules, nothing invented at run time)

- **Static settle loop.** After any state change under a static condition: two
  frames 500 ms apart; proceed only when byte-identical; after five tries, capture
  anyway and mark the step `[uncertain: …]` (the OCR-workflow honesty rule).
- **Steady-state settle loop (playing).** Wait ≥ 2 s after the transition, then
  three frames 500 ms apart; proceed when the derived bar-top profile is equal in
  all three; otherwise `[uncertain]`.
- **Transition burst.** With the long tone playing and settled: post the stop
  command (40047, S03 §1) and capture 120 consecutive frames at the harness's
  fastest rate (~60 fps), then 8 more at 250 ms spacing (slow-tail check for a
  slow decay). Pause burst: post 40046 instead, 30 frames.
- **Mode/refresh discovery (before any mode job).** (a) The clutterbar-V
  visualization menu is already settled by S04 §5 as exactly four rows
  (`Visualization options...`, `Start/Stop plug-in`, `Configure plug-in...`,
  `Select plug-in...`) — no mode or refresh entry; do not expect one there.
  (b) Post a left click and then a right click at the centre of the measured vis
  area (Q8's rectangle), one whole-window frame after each; if a popup appears,
  capture it whole-window and OCR-read every row (popups extend past the client;
  the S04 menu captures are the precedent). (c) Any control found is recorded by
  the capture that exercised it; the mode/refresh value used by a later series is
  whatever that capture set, re-asserted by one frame before the series. (d) If
  no refresh control exists anywhere reached, Q6 still measures the default
  cadence, and "the visible effect of refresh rate" is recorded as an **open**
  row whose named probe is the preferences UI series — the absence of the control
  is itself a settled negative only for the surfaces actually measured.
- **Pass-2 edge probes (Q1).** After the ladder, compute each bar's centroid
  frequency from pass 1 and probe centroid ± ¼ of the bar's log-f width, plus
  octave extension at both ends: if the highest ladder tone lights the top bar,
  extend upward in octave steps until a tone lights nothing or Nyquist is reached;
  symmetric downward. The probe list is computed by this fixed rule from pass-1
  frames and written into the pass-2 job before it runs.
- **Volume setting (Q10).** Set the volume by the audited S02/S03 control path and
  read it back through the IPC volume query measured by the S10 probes (id and
  reply form per the S10 card — do not guess the id here). Two settings: the
  default and a low setting ≈ 25 %.

---

## 3. The comparison — from two frames to a verdict

### 3.1 Metrics

All comparisons are restricted to the **vis sub-rectangle R** measured by Q8
(until then: R = the bounding box of all changed pixels the map job produced,
frozen before any analysis). For frames A, B and region R:

- **Diff count** D = number of pixels in R whose RGB differs between A and B.
- **Bounding box** of the changed pixels.
- **Row profile** ρ_row[i] = lit-pixel count in row i; **column profile**
  ρ_col[j] = lit-pixel count in column j. "Lit" = differs from the silence
  baseline at the same position (the baseline carries slots 0/1; lit means the
  analyser drew there). This avoids every threshold debate.
- **Colour slots:** for each lit pixel, the nearest of the base skin's 24
  `viscolor.txt` RGB triples (free data, S17 §6), giving a slot histogram per row
  and per frame — the slot-per-row order is a measurement, not the provisional
  S17 reading.

House rules carried over from M14/HANDOFF: report **all four** numbers per
comparison; never report a whole-window total (the 1435-px session floor confined
to rows 4–11 makes whole-window counts noise); a bounding box plus its row
profile is the finding, the count is the receipt.

### 3.2 Verdict rules per question

| # | Settled when | Provisional when | Open when |
|---|---|---|---|
| Q1 | The tone→bar-centroid table is monotone, one of {log-spaced, linear} fits every tone within ±1 bar across 3 runs and the other fails by ≥ 2 bars on ≥ 3 tones | Monotone but both models tie within the frame noise | Ladder frames missing or a run refused |
| Q1 (resolution) | A **bracket**: lower bound = distinct bar responses observed across the ladder and pass 2; upper bound = bar count N (Q2). Two tones inside one bar ⇒ resolution ≤ that bar's width in log-f | Bracket from one run only | No pass-2 probes |
| Q2 | N and the pitch identical across 3 runs, and the column-profile minima between bars reach the background level (edges unambiguous) | N stable but edges ambiguous (1-px columns merging) | The profile has no periodic structure at all |
| Q3 | A marker row exists (a colour class above the bar top that outlives the bar), and hold time + fall rate reproduce across 3 runs | Marker exists but ties with "bar-top colour" | No burst captured |
| Q4 | The height-vs-level curve reproduces across 3 runs and exactly one of {linear, dB, power-k} fits all five points within ±1 row while the others fail by ≥ 2 rows on ≥ 2 points; every rendered row matches exactly one `viscolor.txt` slot, stably across frames | Two laws tie within noise | Ladder missing |
| Q5 | The fold rule is uniquely determined: e.g. L-only ≡ R-only ≡ mono excludes channel selection and half-split; antiphase lit excludes averaging; antiphase ≈ silence confirms averaging | Two fold rules fit the same frames | Fixtures missing |
| Q6 | An update period is measured (inter-change gap histogram with a dominant mode), reproduces across runs, and is wall-clock; per existing refresh setting the period differs measurably | Period measured once | No quantization visible at 60 fps — then the *settled negative* is "updates ≥ capture rate" and the visible-effect row stays open with the named faster-capture probe |
| Q7 | Bar-top-per-update is a constant decrement (linear) or a constant ratio (exponential) across 3 runs and the other model fails; with two refresh settings, whether the rate is per-update or per-ms is decided | Both models fit | No burst |
| Q8 | R and the static render reproduce **byte-identically** across 3 runs (static states meet the harness's own byte-identity standard) | — | Never captured |
| Q9 | ≥ 2 styles with per-style frames and the switching gesture named and reproducing | Styles exist, gesture not reproducible | Clicks do nothing on every measured surface and the preferences page was not checked (with the preferences series as the named probe) |
| Q10 | 0-px difference inside R between the two volume settings across 3 runs (pre-volume), or a proportional shrink with its measured ratio (post-volume) | Ambiguous | Missing frames |
| Q11 | Per state, the render rule is uniquely determined by the bursts | Two rules fit | Missing bursts |

### 3.3 Grades

The grades are the specs' own (S02 §0), applied like this:

- **settled** — direct measurement in the VM, the step and capture named, and it
  reproduced across ≥ 3 independent restore cycles. Static conditions must be
  byte-identical (the harness standard); animated conditions cannot be, so their
  settled bar is **metric-identical** — the derived numbers of §3.1 equal, with a
  declared 0-px tolerance on derived geometry (bar edges, marker rows).
- **provisional** — measured, but its meaning is not established or reproduction
  is pending; the missing piece is named in the row.
- **open** — no evidence of any kind; the capture that would settle it is named.
- **visual reading** — a string, row label or shape read by eye or OCR (the
  discovery step's menu rows); never the source of a pixel-derived number.
- **audited** — *not claimable by this lane.* That grade belongs to the
  2026-10-14 session's method; this design measures nothing itself, and every
  value that enters `specs/S08-vis.md` goes through the audit rule (an
  independent lane re-derives each load-bearing number from the run's own bytes).

---

## 4. What cannot be measured this way

| Fact | Why captures cannot settle it | Instrument that would |
|---|---|---|
| FFT size, window function, overlap | Pixels expose only the bar output; different windows can produce identical bars | The probe plugin (S08-vis.md staging step 1): log the PCM fed via `analyzer_feed_pcm`/`vis_feed_pcm` and the rows returned by `analyzer_feed_rows`/`vis_feed_frame`; re-derive the transform from known tones |
| Exact bin→bar weights and magnitude scaling | Captures give monotone bracketing only; the display quantizes | The probe: rows vs known tones at many frequencies and amplitudes |
| The 8-bit waveform scaling and sign convention (plugin-facing, `rustamp-core`'s open note) | Invisible in the built-in window | The probe: `vis_feed_frame` waveform rows vs the fed PCM |
| Timestamp semantics (audio clock vs wall clock, the frame-selection rule) | Pixels carry no timestamps | The probe's logged timestamps against the output time; `out_disk` for the audio side |
| Audio-to-pixel latency (which 576-sample window is on screen "now") | Needs simultaneous audio and pixel capture | A loopback-style rig (the A-suite VB-Cable pattern) or the probe's timestamps; out of scope here |
| Sub-pixel bar behaviour, values below one display row | The display quantizes; a 1-px ambiguity is a declared tolerance, not a fact | Only worth a probe if a defect shows |
| The exact refresh mechanism (timer vs paint loop) | Only the *visible cadence* is measurable; the mechanism is not | Not needed for the spec: S08 states observable cadence, not mechanism |
| The playlist's `WinampVis` child window (S01 §4.2, x 52–123 at y 92) | A separate surface; its render may share the analyser but is not settled by main-window captures | Its own capture series, same method, after an S06/S08 boundary decision |
| Behaviour while a vis plugin runs (S09 §5's built-in-off rule) | Needs a plugin loaded | The plugin-acceptance lane (P3) plus the probe |
| Skin-dependent colours | Skin data, freely readable from the MIT base skin | Not a measurement at all: the loader reads `viscolor.txt` |

---

## 5. Clean-room note

- Every proposed measurement observes the **running reference in the VM** — owner
  approval 2026-10-17 ("the S08 measurement runs may be executed", HANDOFF §1) —
  driven only by posted window messages and staged media. The reference is never
  read: no source, no SDK headers, no disassembly, and nothing from
  `D:\code\rustamp-research` crosses into `specs/`.
- The fixtures are **our own bytes** from our own generator; the base-2.91 skin is
  webamp's MIT skin (attribution already in `THIRD_PARTY.md`); the `viscolor.txt`
  values are skin data of that free skin.
- The probe plugin — the primary instrument for §4's rows — is our own code built
  on the S09 ABI facts; writing it reads nothing dirty.
- Only measured numbers with their run provenance cross back into
  `specs/S08-vis.md`; the audit rule of S08-vis.md staging step 3 applies
  unchanged. Hypotheses in §1 are ours, not the reference's, and never enter the
  spec unmeasured.
- This card is process and design: it contains no measured value and asserts
  nothing about the reference that a capture has not to establish first.

---

## 6. Boundary receipt

Only this card was written; no code was changed, nothing was run, nothing was
committed. `git status --short` inside this worktree (proof elicited after
writing):

```
(refilled below after the check)
```
