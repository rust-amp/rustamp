# W23 \u2014 Regression Test Resolution: Late Repeat Disablement Near Cycle Boundary

Date: 2026-10-18.
Worktree: `C:\Users\micro\.paseo\worktrees\3ikyavhh\paseo-wave117-repeat`
Status: **RESOLVED (test harness synchronization defect corrected; product code verified against S15)**.
Specification: `specs/S15-shuffle-repeat.md` (S15 \u00a72, \u00a74.3).
Predecessor Cards: `docs/tasks/W23-beta08-residual.md`, `docs/tasks/W23-beta08-shuffle-repeat.md`.

---

## 1. Boundary, Ownership and Environment Constraints

- **Owned paths**: `crates/rustamp-audio/**` and this task card (`docs/tasks/W23-repeat-cycle-boundary.md`).
- **Unowned / Forbidden crates**: `crates/rustamp-app`, `crates/rustamp-core`, `crates/rustamp-output`, `crates/rustamp-ui`, `crates/rustamp-ipc`, `crates/rustamp-mediahost`, `crates/rustamp-plughost`, `crates/rustamp-playlist`, `crates/rustamp-vis`. None of these crates were touched.
- **Rules applied**:
  - Cleanroom boundary maintained: no external source code, no SDK headers, no disassembly, and no research directories consulted.
  - No `cargo test` executed; no VM or VM broker invoked.
  - Permitted static verification gates executed:
    - `cargo fmt --all -- --check`: Exit Code 0
    - `cargo clippy --workspace --all-targets --all-features -- -D warnings`: Exit Code 0
    - `cargo build --workspace`: Exit Code 0

---

## 2. Evidence and Defect Analysis

### 2.1 Guest VM Evidence
The test `regression_late_repeat_disablement_near_cycle_boundary` failed at `crates/rustamp-audio/tests/shuffle_repeat.rs:463` with:
```text
assertion failed: `(left == right)`
  left: `Some(0)`,
 right: `Some(1)`
```
Line 463 asserts that the engine audible index has advanced to Track B (index 1), but the engine returned Track A (index 0).

### 2.2 Specification Requirements (S15 \u00a74.3)
S15 \u00a74.3 ("Toggling Repeat ON / OFF mid-run") specifies:
> "Takes effect immediately on the queue boundary check. If Repeat is toggled OFF while playing the final track of a queue or shuffle cycle, playback will terminate upon track end instead of looping."

### 2.3 Product Code Verification
Investigation of the audio engine and decode loop proved that the product implementation in `crates/rustamp-audio/` is correct and adheres strictly to S15 \u00a74.3:
1. **Advancement under repeat (`crates/rustamp-audio/src/decode.rs`)**:
   In `Worker::run`, upon track completion (`Outcome::Ended`), `next = index + 1`. For a 2-track queue (`SHORT_A`, `SHORT_B`):
   - At Track 0 completion, `next = 1 < queue.len()`, advancing cleanly to Track 1.
   - At Track 1 completion (Cycle 1 end), `next = 2 == queue.len()`. Because `self.repeat` is true, the worker records `self.cycle_boundary = Some(self.timeline)` (2000 frames) and sets `next_target = Some(0)`, beginning Cycle 2 Track 0.
2. **Cycle boundary truncation on repeat toggle (`crates/rustamp-audio/src/decode.rs`)**:
   In `Worker::check_control()`, when `DecodeMsg::SetRepeat(false)` is received while `self.cycle_boundary` is set:
   If `consumed < cycle_boundary`, the worker drops Cycle 2 prebuffering, flushes the converter, emits `DecodeOut::EndOfStream { timeline_end: boundary }`, and halts further decoding.
3. **Audible index tracking (`crates/rustamp-audio/src/engine.rs`)**:
   `Engine::audible_index(&self)` evaluates `pos = self.sync.consumed()` against `self.track_starts: Vec<(u32, u64)>` in reverse order. For `timeline_start = 1000` (Track 1) and `consumed = 1000`, `1000 >= 1000` evaluates to `Some(1)`.
4. **Transport termination (`crates/rustamp-audio/src/engine.rs`)**:
   When `consumed` reaches `timeline_end` (2000), `Engine::poll()` invokes `finish_run()`, transitioning transport to `TransportState::Stopped` and publishing `Event::PlaylistEnded`.

### 2.4 The Test Defect
The fault was entirely in the test harness synchronization within `regression_late_repeat_disablement_near_cycle_boundary`:
- **Documented Precondition**:
  The task card `W23-beta08-residual.md` \u00a75.2 and the test's own comment state:
  `// Under repeat, decoder decodes track A, track B, and starts prebuffering Cycle 2 track A.`
  `// Pump only track A so track B is currently playing/audible.`
- **Flawed Execution**:
  The test only executed `assert!(h.wait_track(0));`.
  `wait_track(0)` only waits for `current_index() == Some(0) && duration_frames().is_some()`. This condition is satisfied inside `Worker::prepare_track(0)` *before* a single audio frame has been decoded or written to the ring buffer.
  Immediately following `wait_track(0)`, the test executed `h.clock.pump(len_a)`.
  Because the background decode worker had not yet decoded Track A, `NullClock::pump` encountered an underrun in the ring buffer (`available == 0`). `render` filled scratch with silence, and `sync.consumed()` remained 0 (or far below 1000 frames).
  Furthermore, the worker had not yet reached Track 1, so Track 1 had not even been registered in `self.track_starts`.
  When `h.engine.poll()` and `assert_eq!(h.engine.audible_index(), Some(1))` executed, `pos < 1000` yielded `Some(0)` instead of `Some(1)`.

---

## 3. Changes Applied

In `crates/rustamp-audio/tests/shuffle_repeat.rs`:
Added condition-based polling synchronization to establish the exact setup condition required by the test:
```rust
    let len_a = track_frames(SHORT_A);
    let len_b = track_frames(SHORT_B);
    assert!(h.wait_idle(|e| e.buffered_frames() > len_a + len_b, WAIT));
```
- Uses the harness's standard `wait_idle` helper (no sleep ordering mechanism).
- Waiting for `buffered_frames() > len_a + len_b` guarantees that:
  1. Track A (1000 frames) and Track B (1000 frames) are fully decoded into the ring buffer.
  2. The cycle boundary check occurred and `cycle_boundary = Some(2000)` was registered.
  3. Cycle 2 Track A began prebuffering (written frames > 2000).
  4. Both `TrackStarted(0, 0)` and `TrackStarted(1, 1000)` were processed by `Engine::poll()`.
- Pumping `len_a` (1000 frames) now reliably consumes exactly 1000 audio frames without underrun, placing `consumed = 1000` and rendering Track B audible (`audible_index() == Some(1)`).

---

## 4. Assertion Integrity

The assertion `assert_eq!(h.engine.audible_index(), Some(1));` was **not weakened**:
- It was neither deleted nor modified.
- It continues to assert exact equality with `Some(1)`.
- All subsequent assertions verifying `!engine.repeat()`, transition to `TransportState::Stopped`, and publication of `Event::PlaylistEnded` upon consuming `len_b` frames remain strictly intact.

---

## 5. Verification Gates

All workspace gates passed with Exit Code 0:
- `cargo fmt --all -- --check`: Exit 0
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: Exit 0
- `cargo build --workspace`: Exit 0
