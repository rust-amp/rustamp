# S08 production visualisation feed (2026-10-18)

**Status:** app wiring and six headless test cases implemented. Runtime tests and
live-player acceptance are reserved for the orchestrator's Hyper-V VM. No S08
reference identity, measured cadence or audible-time alignment is claimed.

## Scope and boundary

- Worktree: `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave88-vis-wiring`.
- Initial `git log --oneline -1`: `6ec4cb8 Give a DSP stage a way to fail loudly,
  and delete the workaround that existed without one`; initial status empty.
- Own only `crates/rustamp-app/**` and this card. Write nothing under
  `D:\code\Rustamp`, `D:\code\rustamp-research` or `D:\code\rustamp-refbin`.
- No UI/audio/other-crate change, dependency, download, contract/RFC change,
  git write, VM operation, window or audio-device run.

## Insertion and cost

`crates/rustamp-app/src/app.rs`, `App::pump`: immediately after `Engine::poll`
and the complete collected event batch, immediately before the final
`MainWindow::refresh`. The new private `pump_visualization(now_ms)` calls
`Engine::take_vis_frame()` once and forwards its newest actual snapshot to
`MainWindow::push_vis_frame` with `Engine::source_spec().sample_rate` (or 0 if
unavailable). Analysis errors go through the existing bounded `note` route,
printed by the presenter; they do not fail playback or invent replacement PCM.

The existing chain then supplies analysis -> envelopes ->
`MainWindowState::visualization` -> `render_main_window` -> palette pixels.
`window.rs::Windowed::about_to_wait` already pumps, requests redraw and requests
a 16 ms wait; `Windowed::paint` already presents `App::render`. No new timer,
presenter hook or renderer patch is necessary. The 16 ms request is not a measured
60 Hz cadence guarantee; input/other events can also cause pumps.

Incremental cost per pump:

- One inherited non-blocking drain, O(R) whole-frame moves for R queued frames;
  only the newest is considered. Default tap capacity is 8, but a concurrently
  refilling producer means that is not a strict per-call drain-work bound.
- Zero or one UI-side analysis: 2 channels x 288 bins x 576 samples = **331,776**
  DFT sample/bin iterations, plus Hann preparation, scope sampling and O(19)
  envelope updates. No DFT for missing, discarded, paused or stopped input.
- Successful feed uses fixed-size storage, no new heap allocation/lock/thread;
  an error formats a diagnostic string. This is not a no-allocation claim for
  the whole app pump or the existing image renderer. Actual elapsed cost and
  usability of the direct DFT on the UI thread require VM measurement.

## Missing frames, rates and correspondence limits

No frame means no call to `push_vis_frame`; normal refresh still runs. The last
actual target stays fresh through 100 ms of active UI time; at >100 ms the scope
disappears and existing bars/peaks decay. Missing is never digital silence: no
zero waveform, placeholder bars, timestamp-derived samples, interpolation or
sample-rate conversion is supplied by the app.

The tap precedes device conversion. A 32 kHz source on a 44.1 kHz device is
analysed at **32,000**, never 44,100 or a rounded readout. A missing rate is 0,
which the analyser refuses. The UI also refuses non-finite waveform data. A
source-rate change on accepted input resets the existing UI envelopes. Track
and stream-info events additionally invalidate history immediately, including
when no new frame arrives. Every stream-info event is treated conservatively
as a boundary, even a readout-only update (an own choice).

`invalidate_visualization` clears history while keeping the selected mode and
marks the next drained batch for discard. Seek commands (UI and external),
track events and stream-info events use it. A transition-batch snapshot is
discarded even if it happens to be new/valid; subsequent snapshots resume the
display without filling the gap. Multiple boundaries coalesce into one flag
and one drain in the following pump.

**This is a conservative discard, not a generation handshake.** The frozen
`VisFrame` has no source-rate tag, track/seek generation or valid-count field;
decode reports and frames use separate channels, and a producer can race a
drain/poll. A later untagged snapshot may still cross a boundary. The app cannot
prove a frame/rate pairing or post-seek correspondence. It neither infers a
seek acknowledgement from position nor interprets timestamps as a wall clock.
The tap can lead audible output; exact audio-to-pixel selection/latency requires
an engine/protocol follow-up and the measurement design's probe/loopback rig.

## Transitions (Rustamp own choices; Q11 remains open)

| Transition | Display / feed |
|---|---|
| Initial playback start | Static grid until an actual accepted snapshot. Track/stream events reset history and sacrifice their batch; later frames animate. No test samples are injected in production. |
| Stop, fade-stop, natural completion or error-driven stop | Stopped transport discards offered frames, immediately removes scope/fresh target/timestamp and lets the existing spectrum tail fall. At maximum height, bars reach zero within 667 ms and peaks within 1,300 ms of active envelope time (300 ms hold + 16 rows / 16 rows/s). Continued refresh therefore cannot retain stale bars indefinitely. Stop from pause unfreezes decay too. |
| Pause / resume | Pause freezes scope, bars, peaks and freshness; incoming snapshots are drained and discarded, not saved for resume. Resume accepts later actual snapshots; no pause-duration catch-up. |
| Absolute/relative seek, including external commands | Clear history immediately, preserve mode, discard the following pump's batch. Subsequent actual snapshots can draw; their post-seek generation is not provable with the current contract. A refused seek still causes this conservative visual reset. |
| Track change / restart | Track event resets history and discards the boundary batch. No previous peaks are carried into an accepted silent new track. A quick restart can show the prior bounded stop tail until the new track event; then it resets. |
| Source/stream change | Reset immediately on stream-info event; no frame leaves the static state, not fabricated PCM. Later snapshots use the decoder's latest source rate; the untagged correspondence limitation above still applies. |

## Headless receipts and VM acceptance

Six new cases in `crates/rustamp-app/src/visualization_tests.rs`, type-checked with
all targets/features but **not executed by this lane**:

1. Production pump -> actual test decoder -> tap -> analysis -> state -> changed
   palette pixels, with source 32 kHz, device 44.1 kHz, unavailable stream readout
   and volume 0. No direct UI-frame injection in this end-to-end case.
2. Pause freezes the complete state, discards input; resume accepts a changed
   source timestamp.
3. Stop from playing and paused removes the scope (exact static-region pixels),
   then bars/peaks reach zero and the spectrum region equals its static render.
4. External absolute and UI-origin relative seek reset history, consume the
   boundary flag and resume actual input. Explicitly not a post-seek-tag proof.
5. Actual track/source-rate change from tone/32 kHz to silence/48 kHz leaves no
   inherited bars or peaks and displays the actual zero scope.
6. Isolated deterministic-envelope case, clearly test-fixture-only: no tap input
   preserves the original timestamp; exact 100/101 ms freshness, partition
   identity at 425 ms (9 bar rows / 14 peak rows), zero tail at 1,300 ms.

Existing UI numeric/envelope and exact-pixel golden cases remain unchanged.
No golden blessing or tolerance change is needed: this patch changes the
production feed, not the established render law or baseline images.

**VM-only pending:** execute these six cases and the unchanged workspace suite;
exercise a live window with real playback, start/stop/pause/seek/track changes,
scope modes and both shipped skins; measure pump/paint cost, visible cadence,
event starvation and responsiveness; verify audio vs pixel alignment with a
simultaneous rig. Headless null-output tests cannot prove any of those live
properties. All S08 Q1-Q11 reference questions stay **open**; captures/probe and
independent audit are still required by `S08-measurement.md`.

## Verification and provenance

`cargo fmt --all -- --check`: pass. The first invocation reported only local
formatting differences; they were fixed without formatting another crate.
`cargo clippy --workspace --all-targets --all-features -- -D warnings`: pass
(39.09 s, offline; test targets type-checked, not executed).
`cargo build --workspace`: pass (46.41 s, offline).
`git diff --check`: pass.

Source receipts: `crates/rustamp-app/src/app.rs` 3,855 lines (+49/-3),
`crates/rustamp-app/src/visualization_tests.rs` 294 new lines; this card is the
only other source change. Final card line count and `git status --short` are
supplied in the metadata report. All Cargo products use this worktree's `target`
and `CARGO_NET_OFFLINE=true`; no download was attempted.

No direct `cargo test`, golden run, VM test, window, audio, commit or other git
write is performed by this lane.
No check, lint, gate, ignore flag, golden baseline or tolerance was weakened.

Inputs: repository code and the two S08 cards, plus the binding clean-room,
architecture and contribution instructions. No reference source/SDK/disassembly,
reference binary or research tree was read. No third-party code copied.
