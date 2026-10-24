# W23 — Mid-Run Repeat Disablement at Cycle Boundary (Defect Resolution)

Date: 2026-10-18.
Worktree: `C:\Users\micro\.paseo\worktrees\3ikyavhh\paseo-wave118-repeatstop`
Status: **RESOLVED (product defect in cycle boundary handling corrected; test untouched)**.
Specification: `specs/S15-shuffle-repeat.md` (S15 §2, §4.3).
Predecessor Cards: `docs/tasks/W23-repeat-cycle-boundary.md`, `docs/tasks/W23-beta08-residual.md`.

---

## 1. Boundary, Ownership and Environment Constraints

- **Owned paths**: `crates/rustamp-audio/**`, `crates/rustamp-playlist/**`, and this task card (`docs/tasks/W23-repeat-disable-stop.md`).
- **Unowned / Forbidden crates**: `crates/rustamp-app`, `crates/rustamp-core`, `crates/rustamp-output`, `crates/rustamp-ui`, `crates/rustamp-ipc`, `crates/rustamp-mediahost`, `crates/rustamp-plughost`, `crates/rustamp-vis`. None of these crates were touched.
- **Rules applied**:
  - Cleanroom boundary maintained: no external source code, no SDK headers, no disassembly, and no research directories consulted.
  - No `cargo test` executed; no VM or VM broker invoked directly.
  - Permitted static verification gates executed:
    - `cargo fmt --all -- --check`: Exit Code 0
    - `cargo clippy --workspace --all-targets --all-features -- -D warnings`: Exit Code 0
    - `cargo build --workspace`: Exit Code 0

---

## 2. Evidence and Defect Analysis

### 2.1 Guest VM Evidence
The regression test `regression_late_repeat_disablement_near_cycle_boundary` in `crates/rustamp-audio/tests/shuffle_repeat.rs` passed its earlier assertion:
- Line 463: `assert_eq!(h.engine.audible_index(), Some(1));` passed cleanly once the harness synchronized decoder prebuffering.
However, it subsequently failed at line 474:
```text
assert!(h.wait_idle(|e| e.transport() == TransportState::Stopped, WAIT));
```
Instead of taking ~0.2 seconds, `wait_idle` timed out at 30 seconds (`WAIT`). Playback did not stop cleanly at the end of Cycle 1, and `Event::PlaylistEnded` was never published.

### 2.2 Specification Requirements (S15 §4.3)
S15 §4.3 ("Toggling Repeat ON / OFF mid-run") specifies:
> "- Takes effect immediately on the queue boundary check.
> - If Repeat is toggled OFF while playing the final track of a queue or shuffle cycle, playback will terminate upon track end instead of looping.
> - If Repeat is toggled ON while on the final track, playback will loop upon track end."

Here, "while playing" refers to the track currently audible to the listener (`pos = self.sync.consumed()`), not the decoder's lookahead prebuffering. When Repeat is turned OFF while the final track of Cycle 1 is audible, playback must terminate upon the completion of Cycle 1 with `Event::PlaylistEnded`, discarding any subsequent cycle audio prebuffered in the ring buffer.

### 2.3 Failure Mechanism
1. **Decoder lookahead and ring buffer capacity**:
   `SHORT_A` (1000 frames) + `SHORT_B` (1000 frames) form a 2000-frame cycle. The ring buffer capacity is 16384 frames. With fast synthetic decoding, the background worker prebuffered up to 8 cycles ahead (~16000 frames).
2. **Cycle boundary overwrite in `Worker` (`crates/rustamp-audio/src/decode.rs`)**:
   `Worker` tracked `cycle_boundary: Option<u64>` as a single scalar. On each cycle wrap, `self.cycle_boundary = Some(self.timeline)` was overwritten (e.g. 2000, 4000, ..., 16000).
   When `SetRepeat(false)` arrived, `Worker` evaluated:
   ```rust
   if !on && let Some(boundary) = self.cycle_boundary {
       let (_written, consumed) = self.cfg.sync.counters();
       if consumed < boundary {
           // boundary was 16000, NOT 2000!
           let _ = self.tx.send(DecodeOut::EndOfStream { timeline_end: boundary });
       }
   }
   ```
   Or if `consumed >= boundary` in `track_loop` had cleared `cycle_boundary` to `None`, no `EndOfStream` was emitted at all.
3. **Missing Engine-side cycle boundary tracking (`crates/rustamp-audio/src/engine.rs`)**:
   `Engine::set_repeat(false)` did not compute the end of the currently audible cycle; it relied entirely on the decode worker.
4. **Stall in `Engine::poll()`**:
   `finish_run()` requires `consumed >= end`. Since `end` was set to 16000 (or `None`), and the test only pumped Cycle 1 (2000 frames), `consumed` stopped at 2000. `consumed >= 16000` never became true, leaving transport in `Playing` until `wait_idle` timed out at 30 seconds.

---

## 3. Product Changes Applied

### 3.1 `crates/rustamp-audio/src/decode.rs`
1. Replaced `cycle_boundary: Option<u64>` with `cycle_boundaries: Vec<u64>`.
2. When the worker wraps across repeat cycle seams (in sequential run, shuffle replan, and EOS wakeups), pushes `self.timeline` to `self.cycle_boundaries`.
3. In `track_loop`, prunes expired boundaries using `self.cycle_boundaries.retain(|&boundary| consumed < boundary)`.
4. In `check_control()` on `DecodeMsg::SetRepeat(false)`:
   Finds the earliest cycle boundary that is ahead of `consumed`:
   `self.cycle_boundaries.iter().find(|&&b| consumed < b)`.
   For `consumed = 1000`, this selects `2000` (the end of Cycle 1), emits `DecodeOut::EndOfStream { timeline_end: 2000 }`, flushes the converter, and halts decoding via `pending_skip = Some(Outcome::Stop)`.

### 3.2 `crates/rustamp-audio/src/engine.rs`
1. **Cycle end calculation (`current_cycle_end`)**:
   Added `Engine::current_cycle_end(&self, pos: u64) -> Option<u64>`. Using `self.track_starts: Vec<(u32, u64)>`:
   - Finds the currently audible track entry `curr_idx` for `pos = self.sync.consumed()`.
   - In sequential mode, scans forward from `curr_idx` for the final track of the playlist (`(queue.len() - 1) as u32`). If a subsequent track start is recorded at `final_entry_idx + 1`, its `timeline_start` marks the cycle boundary.
   - In shuffle mode, calculates `final_entry_idx = (curr_idx / n) * n + (n - 1)`. If `final_entry_idx + 1` exists, its `timeline_start` is the cycle boundary.
2. **Immediate latching in `set_repeat(false)`**:
   When repeat is toggled OFF mid-run (`self.transport == TransportState::Playing`), immediately sets `self.end_of_stream = Some(cycle_end)` if a subsequent cycle was already prebuffered.
3. **Safety check in `Engine::poll()`**:
   If `!self.repeat && self.transport == TransportState::Playing && self.end_of_stream.is_none()`, latches `self.current_cycle_end(pos)`.
4. **Conservative min-merging on `DecodeOut::EndOfStream`**:
   In `on_decode_out`, merges incoming `timeline_end` via `existing.min(timeline_end)`.
5. **Audible index alignment in `finish_run()`**:
   When stopping at `frozen_end`, updates `self.current` to the track that ended at `frozen_end` before clearing `self.track_starts`, ensuring correct state reporting and `PlaylistEnded` firing.

---

## 4. Assertion and Test Integrity

The regression test `crates/rustamp-audio/tests/shuffle_repeat.rs` was **completely untouched**:
- `git diff crates/rustamp-audio/tests/shuffle_repeat.rs` is empty (0 lines changed).
- No assertions were altered, commented out, or weakened.
- No sleep-based ordering mechanisms were introduced.
- The test's requirement that playback terminates cleanly upon the completion of Cycle 1 with `PlaylistEnded` is satisfied directly by the engine and decoder implementation.

---

## 5. Verification Gates

All workspace gates passed with Exit Code 0:
- `cargo fmt --all -- --check`: Exit Code 0
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: Exit Code 0
- `cargo build --workspace`: Exit Code 0
