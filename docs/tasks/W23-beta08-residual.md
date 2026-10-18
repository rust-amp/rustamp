# W23 / BETA-08 — Residual Defect Resolution: Shuffle and Repeat Playback Behavior

Date: 2026-10-18.
Worktree: `C:\Users\micro\.paseo\worktrees\3ikyavhh\paseo-wave114-beta08`
Status: **RESOLVED (in owned crates `crates/rustamp-audio/**`); card documented**.
Specification: `specs/S15-shuffle-repeat.md` (S15).
Predecessor Card: `docs/tasks/W23-beta08-shuffle-repeat.md`.

---

## 1. Boundary, Ownership and Environment Constraints

- **Owned paths**: `crates/rustamp-audio/**`, `crates/rustamp-playlist/**`, and this card (`docs/tasks/W23-beta08-residual.md`).
- **Unowned / Forbidden crates**: `crates/rustamp-app`, `crates/rustamp-ui`, `crates/rustamp-core`, `crates/rustamp-output`, `crates/rustamp-ipc`, `crates/rustamp-mediahost`, `crates/rustamp-plughost`, `crates/rustamp-vis`. No files outside owned paths were modified.
- **Rules applied**:
  - Cleanroom boundary maintained: no external code, no disassembly, no Winamp binaries used.
  - No `cargo test` executed; no VM or VM broker invoked.
  - Permitted static verification gates: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo build --workspace`.

---

## 2. Confirmation and Mechanism of the Defect

### 2.1 Confirmation of Findings
The investigation confirmed the defect identified in card `W23-beta08-shuffle-repeat.md`:
While the historical readiness claim that shuffle/repeat was a universal placebo was refuted (both modes have actual implementations in the engine and decode loop since `ade10e64`), a critical residual defect was verified:

### 2.2 Mechanism Breakdown
1. **Decoder premature termination (`crates/rustamp-audio/src/decode.rs`)**:
   When the queue entries were fully read into the ring buffer and repeat was OFF, `Worker::run` encountered `next_target == None` and immediately executed `break;`. The worker thread flushed the converter, sent `DecodeOut::EndOfStream { timeline_end }`, and exited.
2. **Engine teardown latching (`crates/rustamp-audio/src/engine.rs`)**:
   `Engine::on_decode_out` received `EndOfStream` and latched `self.end_of_stream = Some(timeline_end)`. Upon worker exit, `DecodeOut::Stopped` caused `self.to_decode = None`, severing the control channel.
3. **Dead channel on late repeat toggle**:
   When the user or application toggled repeat ON (`Engine::set_repeat(true)`) while audio frames were still buffered in the ring:
   - `self.repeat` was set to `true` in `Engine`.
   - `Event::ModesChanged { repeat: true, .. }` was published to the UI bus, causing the UI repeat indicator light to switch ON.
   - `self.send(DecodeMsg::SetRepeat(true))` attempted to notify the decode worker, but because `self.to_decode` was `None`, the message was silently dropped.
   - `self.end_of_stream` remained `Some(timeline_end)`.
4. **Premature stop while repeat light shines**:
   In `Engine::poll()`, once the ring buffer drained (`written >= end && consumed >= written`), `finish_run()` was called unconditionally. Playback stopped dead, while the repeat indicator advertised that repeat mode was active.

---

## 3. Specification Scope: Settled vs. Open Questions (S15)

### 3.1 Settled Behaviors Implemented
- **Immediate repeat activation at queue end (S15 §2.1, §4.3)**:
  Toggling repeat ON before ring buffer drain revokes EOS and resumes seamless decoding from track 0 (sequential) or starts a new anti-repeat cycle (shuffle).
- **Anti-repeat across cycle boundaries (S15 §2.1)**:
  When a shuffle cycle loops under repeat, the first track of cycle $N+1$ is guaranteed not to repeat the last track of cycle $N$ (for queues with length > 1).
- **Shuffle mid-run replanning anchored to audible track (S15 §4.1)**:
  Enabling shuffle during active playback sets the currently audible track (track $K$, where $K = \text{audible\_index}()$) as index 0 of the new shuffle permutation, preserving user perceptual continuity regardless of decoder read-ahead.
- **Repeat disablement near cycle boundary (S15 §4.3)**:
  Disabling repeat while track $B$ (final track of Cycle 1) is still audible cuts off pre-buffered Cycle 2 audio at the cycle boundary, stopping playback cleanly when Cycle 1 ends.

### 3.2 Open Questions for Specification Owner (S15 §7)
The following behaviors are left unspecified in S15 and are documented here as questions for the owner rather than invented by implementation:
1. **Manual `Next` at queue end with Repeat OFF**:
   - *Question*: When playback reaches the end of the queue with Repeat OFF, does pressing manual `Next` wrap around to track 0 in a paused/stopped state, wrap and start playback, or act as a no-op?
2. **Elapsed-time threshold for `Prev`**:
   - *Question*: In Winamp 2.95, if track playback has elapsed past a threshold (typically 2–3 seconds), `Prev` jumps to the start (0:00) of the current track rather than stepping back to the previous track. Does S15 specify an exact threshold (e.g. 2000 ms), and how does it interact with the shuffle cycle history?
3. **Shuffle PRNG state across application sessions**:
   - *Question*: Should the shuffle permutation seed/history be persisted across application restarts, or reseeded with an ephemeral entropy source on each cold boot?

---

## 4. Implementation Details (Changes in Owned Paths)

### 4.1 `crates/rustamp-audio/src/decode.rs`
- **`DecodeOut::EndOfStreamRevoked`**: Added variant to notify `Engine` that an end-of-stream condition has been cancelled due to repeat activation or navigation.
- **`Worker::cycle_boundary`**: Added `Option<u64>` field to track timeline positions at cycle boundaries.
- **`Worker::wait_at_eos()`**: Implemented wait loop at end-of-stream:
  - Listens on `rx.recv()`.
  - On `DecodeMsg::SetRepeat(true)`: sets `self.repeat = true`, emits `DecodeOut::EndOfStreamRevoked`, computes next cycle target (track 0 for sequential, or anti-repeat cycle for shuffle), and resumes decoding.
  - On `DecodeMsg::SetShuffle`: updates active shuffle plan.
  - On `DecodeMsg::Stop` / `Quit` / channel disconnect: breaks and exits worker thread.
  - On `DecodeMsg::Next` / `Prev` / `SkipBy`: navigates within queue/cycles and revokes EOS.
- **Repeat cycle disablement**: In `check_control()`, when `DecodeMsg::SetRepeat(false)` is received and `consumed < cycle_boundary`, truncates decoding, emits `EndOfStream { timeline_end: boundary }`, and exits so Cycle 2 is never rendered.
- **Converter carry reset**: When transitioning from `wait_at_eos()`, clears converter carry (`carry = false`) to cleanly re-prepare the format converter for track 0 / cycle wrap.

### 4.2 `crates/rustamp-audio/src/engine.rs`
- **`audible_index(&self) -> Option<u32>`**: Implemented using `sync.consumed()` and `track_starts: Vec<(u32, u64)>` history. Queries which track is currently emitting from the audio hardware rather than which track the decoder is pre-buffering.
- **`set_shuffle()`**: Anchors mid-run shuffle replanning to `self.audible_index().or(self.current)` satisfying S15 §4.1.
- **`set_repeat(true)`**: Clears `self.end_of_stream = None;` immediately upon enabling repeat.
- **`on_decode_out()`**:
  - Handles `DecodeOut::EndOfStreamRevoked => { self.end_of_stream = None; }`.
  - Pushes `(index, timeline_start)` to `track_starts` on `DecodeOut::TrackStarted`.
- **`poll()`**: Updated EOS termination check to `written >= end && consumed >= end && self.transport == TransportState::Playing` to ensure accurate stopping when unplayed frames are truncated.
- **`join_decode()`**: Moved `self.to_decode = None;` before `handle.join()` ensuring the worker waiting at EOS unblocks immediately on channel disconnect.

---

## 5. Regression Test Suite (`crates/rustamp-audio/tests/shuffle_repeat.rs`)

Four deterministic integration tests were added, each targeting a specific failure condition:

1. **`regression_late_repeat_enablement_at_queue_end`**:
   - *Setup*: Queue with 2 short tracks (`SHORT_A`, `SHORT_B`), `repeat=false`, `shuffle=false`. Play track 0.
   - *Trigger*: Wait until both tracks are buffered (`wait_buffered()`, EOS received, buffer full). Call `set_repeat(true)` before the buffer drains.
   - *Failure condition without fix*: The worker thread had already exited (`break;`), `to_decode` was dropped, `set_repeat(true)` was ignored, and playback stopped after total frames drained.
   - *Verification*: Transport remains `Playing`, EOS is revoked, and track 0 loops and plays in Cycle 2.
2. **`regression_late_repeat_disablement_near_cycle_boundary`**:
   - *Setup*: `repeat=true`. Decoder prebuffers tracks A and B, plus begins Cycle 2 track A.
   - *Trigger*: Pump only track A frames so track B is audible. Call `set_repeat(false)`. Pump track B frames.
   - *Failure condition without fix*: Decoder continued decoding Cycle 2 and output device played past Cycle 1 end.
   - *Verification*: Transport transitions to `Stopped` and publishes `PlaylistEnded` at exactly the end of Cycle 1.
3. **`regression_shuffle_mid_run_replan_anchoring_to_audible_track`**:
   - *Setup*: 3-track queue, `shuffle=false`. Play track 0. Wait until queue is buffered (decoder has reached track 2).
   - *Trigger*: At 0 consumed frames (Track 0 audible), call `set_shuffle(true)`.
   - *Failure condition without fix*: Shuffle plan anchored to decoder position (Track 2) instead of audible track (Track 0).
   - *Verification*: `plan[0] == 0` (anchored to audible Track 0 per S15 §4.1).
4. **`regression_single_track_repeat_toggle_during_drain`**:
   - *Setup*: 1-track queue, `repeat=false`. Play track 0. Wait until track 0 is buffered into ring.
   - *Trigger*: Call `set_repeat(true)` while audio is in the ring. Pump through track 0 length.
   - *Failure condition without fix*: Single-track playback halted on EOS despite repeat being enabled.
   - *Verification*: Transport remains `Playing` and track 0 seamlessly loops.

---

## 6. External Changes Required (Unowned Crates / Application Handoff)

For Regression 5 (UI/Application button state vs. Engine state):
- **Location**: `crates/rustamp-app/src/app.rs` and `crates/rustamp-app/src/routing.rs`.
- **Description**:
  The application listens for `Event::ModesChanged { shuffle, repeat }` from the engine event bus. When repeat is toggled via UI button, the UI must avoid maintaining a separate optimistic latch if the engine rejects or modifies mode state.
- **Recommended change in `crates/rustamp-app`** (to be executed by application lane):
  Ensure that `App::handle_event` on `Event::ModesChanged` updates the UI light directly from the event payload without relying on asynchronous command confirmations.

---

## 7. Verification and Gating Results

The following permitted gates were run:

1. **`cargo fmt --all -- --check`**: Exit code 0.
2. **`cargo clippy --workspace --all-targets --all-features -- -D warnings`**: Exit code 0.
3. **`cargo build --workspace`**: Exit code 0.

### What Only the Orchestrator Virtual Machine Can Prove
While static analysis, clippy, and compilation confirm type safety, API conformance, and absence of compiler errors or warnings, the following runtime aspects can only be proven inside the orchestrator VM:
- Real audio hardware output latency and ring buffer underruns under system scheduling load.
- Exact audio sample alignment across WASAPI / DirectSound device buffers.
- End-to-end Winamp skin button click synchronization with the visual LED indicators under interactive X11 / Win32 message pumping.
