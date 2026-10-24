# W19 — Visualiser contract amendment and seam audit (2026-10-18)

**Status:** Implemented in `crates/rustamp-ui` and `crates/rustamp-app`. Static
gates pass (formatting, clippy, build). Runtime/golden execution, the remaining
reference questions and every live-desktop property stay with the orchestrator's
Hyper-V VM — no test process was started here.

**Worktree:** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave92-ui`.
Write only inside this worktree. Nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

**Owned paths:** `crates/rustamp-ui/**`, `crates/rustamp-app/**`, this card.
`crates/rustamp-audio`, `crates/rustamp-mediahost`, `crates/rustamp-dechost`,
`crates/rustamp-dsphost` and `xtask` were **not** touched.

## 0. Closing the review's last two items

The adversarial review (`docs/tasks/W16-wave-review.md`) left two visualiser
items open: **W16-F07** (non-finite PCM sanitised before the tap, so
`VisError::NonFinitePcm` cannot fire live) and **W16-F06** (stop did not clear
the screen). W16-F06 was closed by `docs/tasks/W17-vis-clear.md`. This card
closes W16-F07 as its owning lane (W17-audio-findings §3) specified — by making
the UI crate **say** what the error means and then verifying the code already
**behaves** that way — and then audits the visualiser against its own cards.

---

## 1. The contract amendment

### 1.1 What the amendment says

From `docs/tasks/W17-audio-findings.md:109-126` (the written-out amendment),
`docs/tasks/W17-audio-findings.md:93` and `docs/notes/lane-jobs.md:83`:

- `VisError::NonFinitePcm` is an **offline validation invariant** for isolated
  harnesses, synthetic unit tests and custom offline analysis pipelines.
- It is **not** a live-pipeline error: the decode path guarantees that no
  non-finite sample reaches `VisTap`, because `crates/rustamp-audio/src/decode.rs`
  replaces every non-finite sample a stage produced with `0.0` (RFC 0014 §4.5,
  defence in depth), and a stage that detects its own non-finite output returns
  `Err` on the decode path instead.
- It **must not halt playback**. An offline analysis that meets a non-finite
  frame reports the frame as invalid and continues; playback state is untouched.

The sanitizer is **not** removed and no test is weakened. The check in
`analyze` stays exactly as it was.

### 1.2 Where it now lives

| Location | Change |
|---|---|
| `crates/rustamp-ui/src/visualization.rs:19-22` | Module doc: the arm is an offline invariant the live path cannot reach and must not halt playback. |
| `crates/rustamp-ui/src/visualization.rs:135-141` | `VisError` enum doc: these are analysis errors, never playback errors; the live caller only notes them. |
| `crates/rustamp-ui/src/visualization.rs:146-162` | `VisError::NonFinitePcm` variant doc: the full statement — offline validation invariant, RFC 0014 §4.5 keeps it off the live path, never a pipeline-halting audio error, playback state not touched. This is the placement the amendment asked for ("make the documentation of the error variant state it"). |
| `crates/rustamp-ui/src/visualization.rs:173-181` | `analyze` doc (new at 175-177): it is the offline entry point as well as the live one; a bad frame is reported without touching playback or display state. |
| `crates/rustamp-ui/src/visualization.rs:321-327` | `push` doc (new at 323-325): a rejected frame is reported only and never halts playback or clears the last accepted display state. |
| `crates/rustamp-ui/src/behaviour.rs:181-185` | `MainWindow::push_vis_frame` doc: a returned `VisError` is an analysis report; a caller notes it and keeps playing, it must not stop/pause/reset the transport. |
| `crates/rustamp-app/src/app.rs:768-771` | `App::pump_visualization` doc: an analysis error is only noted, never fatal. |

### 1.3 Did the code already behave that way?

**Yes — verified by reading, and now pinned by two new tests.** `analyze`
(`visualization.rs:182-223`) is a pure function over the frame and the rate: it
returns `Err(VisError::NonFinitePcm)` and mutates nothing. `Visualizer::push`
propagates that error before it writes `last_input`, `state.input`,
`state.source_timestamp_us`, `state.scope`, `targets`, `bars` or `peaks`, so a
rejected frame cannot become an accepted one and cannot clear the display. The
only state the rejected call touches is the display clock (`advance`, which ages
display envelopes — not playback state). On the live path the app only calls
`self.note(error.to_string())` (`app.rs:782`), and `note`
(`app.rs:1336-1340`) appends to a bounded diagnostic vector: the engine,
transport and queue are never consulted. `VisError` is used nowhere else in the
workspace (checked with `git grep`). So the amendment needed documentation, not
a behaviour change; the code was left semantically untouched.

### 1.4 New pins (nothing removed or weakened)

- `crates/rustamp-ui/src/visualization.rs:566-591` —
  `non_finite_pcm_is_an_offline_invariant_and_never_halts_playback`: `analyze`
  rejects NaN/±Inf, a rejected `push` leaves the last accepted `VisState` byte
  for byte, and the next finite frame is still accepted with the mode intact.
- `crates/rustamp-app/src/visualization_tests.rs:299-322` —
  `a_non_finite_frame_is_only_reported_and_playback_keeps_running`: an injected
  non-finite frame returns `Err(VisError::NonFinitePcm)` at the UI boundary while
  `MainWindowState::transport` stays `Playing` and no frame becomes fresh input.
  (Injection is necessary because the live decoder sanitizes first — that is the
  point of the amendment.)

---

## 2. Seams found: cards vs `crates/rustamp-ui/src/visualization.rs`

Every S08-vis-impl §3 row (area, transform, normalisation, stereo, bands, height
law, bars/pitch, palette, bar envelope, peaks, envelope precision, freshness,
scope sampling, strokes, modes, cadence) was checked against the code and
**matches**; the same holds for S08-vis-wiring's insertion/cost, missing-frame
and per-transition rows, with the single exception of the stop/decay wording
(exception 1 below) and the app-seam wording (exception 2). The conflicts below
are the exceptions. Nothing here was invented beyond the cards.

| # | Claim (file:line) | Code | Verdict / action |
|---|---|---|---|
| 1 | `docs/tasks/S08-vis-impl.md:83` — "Stop clears targets/scope immediately **but lets spectrum tail fall**". Same model in `docs/tasks/S08-vis-wiring.md:84` ("lets the existing spectrum tail fall … bars reach zero within 667 ms and peaks within 1,300 ms") and `:87` ("can show the prior bounded stop tail"). | `crates/rustamp-ui/src/visualization.rs:282-289`: `advance` on `Stopped` resets the whole visualiser to `Self::default()` (mode preserved) at once — no tail exists in any mode. | **Card wrong, code correct** (this is exactly what W17-vis-clear changed, `f7b09ea`). The three card lines are stale and were **not** edited: S08-vis-impl / S08-vis-wiring are not owned paths. Correction recorded here. |
| 2 | `docs/tasks/S08-vis-impl.md:101-104` — "The production app has no frame-forwarding call today"; `:110-116` — the required insertion reads the rate from `self.engine.state().stream.map(\|s\| s.sample_rate_hz)`. | `crates/rustamp-app/src/app.rs:772-785` forwards the newest tapped frame with `self.engine.source_spec().sample_rate`. The readout rate is deliberately unavailable in the fixtures (`crates/rustamp-app/src/visualization_tests.rs:56-64` returns `sample_rate_hz: 0`) and the end-to-end test asserts source 32 kHz vs device 44.1 kHz vs readout 0 (`:167-169`), so the card's snippet would refuse every frame. S08-vis-wiring:22-24 documents the correct source. | **Card stale, code correct.** The §4 snippet and its "no call today" premise are superseded by `feea30a` and S08-vis-wiring. Not edited (not owned); recorded here. |
| 3 | `docs/tasks/S08-vis-impl.md:86-90` — the golden snapshots "expose separated bands, decay/hold, pause, stroke differences and full height"; `:144` lists `vis_stopped_tail`. | `crates/rustamp-ui/tests/golden_cases/mod.rs:409-413` builds the 1425 ms snapshot after `Event::StateChanged(Stopped)`, so it is now the **cleared** frame (bars/peaks zero, mode `Spectrum`). The images were re-blessed in `9467e47`. | **No code defect.** The fixture name still says "tail" while the state is the cleared screen; only the name is historical. Renaming a golden changes filenames, which is churn rather than a defect fix, so it was left and reported. |
| 4 | `docs/tasks/W17-vis-clear.md:59` — re-blessing `vis_stopped_tail.png` (both skins) "must be executed inside the VM" as pending work. | Commit `9467e47` re-blessed `tests/golden/lookalike/vis_stopped_tail.png` (3034→2956 B) and `tests/golden/rust-theme/vis_stopped_tail.png` (2965→2890 B) in the guest. | **Card bullet historical, not a defect.** Recorded so it is not re-opened. |
| 5 | `docs/tasks/W17-vis-clear.md:20` — clearing applies uniformly in **all** modes (`Spectrum`, `Scope`, `ScopeDots`); `:30-31` describes the unit test as iterating over **both** `Spectrum` and `Scope`. | The reset at `visualization.rs:282-289` is mode-agnostic (mode is the only surviving field), so the behaviour does hold in all four `VisMode`s; the tests at `visualization.rs:648` and `crates/rustamp-app/src/visualization_tests.rs:202` loop over two modes. | **No code defect**: the two-mode loop pins the mode-independent mechanism. Reported as under-coverage, and the two card lines are internally slightly inconsistent about "all" vs "both". Not changed — widening the loop would make `W17-vis-clear.md:30-31` wrong, and a card is not a defect. |

**Nothing in the visualiser contradicts a card by doing too little.** No card
claim had to be fixed in code.

---

## 3. What the cards do not claim and the code does anyway (reported, unchanged)

None of these is a defect; they are recorded so the next reader is not surprised.

- `App::render` (`crates/rustamp-app/src/app.rs:691-692`) calls `ui.refresh(now)`
  as well as `App::pump`, so the visualiser advances on render too, while
  S08-vis-wiring:27-32 presents the pump as the feed point. Benign: the envelope
  is a function of cumulative active time and is refresh-partition independent
  (`visualization.rs:623-645`), so the split does not change the result.
- `Visualizer::push` (`visualization.rs:328-335`) runs `advance` **before**
  validating, so a rejected frame still ages the *display* envelopes. That is
  display state, not playback state; §1.3 relies on the same boundary.
- `draw_visualization` (`visualization.rs:389`, `:438-448`) always fills a 76×16
  scratch and clips the blit to `area.width.min(VIS_WIDTH)` /
  `area.height.min(VIS_HEIGHT)`, so a smaller or larger layout area cannot spill
  or panic. Not claimed by any card; defensive.
- `analyze` (`visualization.rs:182-186`) validates the sample rate before
  finiteness, so a frame that is both rate-less and non-finite reports
  `MissingSampleRate`, not `NonFinitePcm`. The precedence is not stated anywhere;
  harmless, and the existing test pins the rate-first order.
- The right-click path never reaches the visualiser: only
  `MouseButton::Left` is routed to `mouse_down`/`mouse_up`
  (`crates/rustamp-app/src/window.rs:1593-1620`); right clicks take the
  context-menu path (`:1633-1638`). This matches S08-vis-impl:82's "left
  press+release".

---

## 4. Verification gates

Run from the root of this worktree with `CARGO_NET_OFFLINE=true`; no download,
no dependency, no test process, no VM, no window, no audio.

| Gate | Exit |
|---|---|
| `cargo fmt --all -- --check` | `0` |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `0` (44.39 s; all test targets type-checked, none executed) |
| `cargo build --workspace` | `0` (52.74 s) |

No `cargo test` was run (Hyper-V VM only, orchestrator's call). No check, lint,
ignore flag, golden baseline or tolerance was weakened, and no test was removed.

---

## 5. What only the VM or a real desktop can decide

- That the six app wiring cases plus the new pins execute green (this lane
  type-checked them; it did not run them).
- That the analyser's direct 576-point DFT is cheap enough on the UI thread at
  the real 16 ms cadence, and that a note from a rejected frame never disturbs
  the audible stream.
- That stop/start/pause/seek/mode clicks look right on a real device in both
  shipped skins, without flicker or tearing, and that audio-to-pixel alignment
  is acceptable.
- Every S08 Q1–Q11 reference question remains **open**; the measurement design's
  probe/rig and the independent audit are still required.

## 6. File and boundary receipt

`git diff --stat` (4 files, +96/−4):

```
 crates/rustamp-app/src/app.rs                 |  4 ++
 crates/rustamp-app/src/visualization_tests.rs | 27 +++++++++++-
 crates/rustamp-ui/src/behaviour.rs            |  6 +++
 crates/rustamp-ui/src/visualization.rs        | 63 +++++++++++++++++++++++++--
```

Line counts after the change: `crates/rustamp-ui/src/visualization.rs` 748,
`crates/rustamp-ui/src/behaviour.rs` 1579,
`crates/rustamp-app/src/app.rs` 4105,
`crates/rustamp-app/src/visualization_tests.rs` 322, this card 188.
`git status --short` contains only the four owned crate files above and this new
card. The canonical tree was never opened for writing and never used as a command
working directory; no commit or other git write was performed.
