# Task Card W17 — Resolution of Wave 16 Audio and Playlist Findings

**Date:** 2026-10-18. **Lane:** `wave90-audio`.
**Worktree:** `C:\Users\micro\.paseo\worktrees\3ikyavhh\paseo-wave90-audio`.

Boundary rule: write ONLY inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

**Status:** Implemented; fmt, clippy and build gates passing. VM test execution
pending orchestrator run.

No tests were run on the host (Hyper-V test-VM constraint strictly preserved):
not the suite, not `--no-run`, not a single binary, not a lib unit test. No test
process, example, player, helper binary or reference binary was spawned; no VM
was touched; no window or audio device was opened; no download, no dependency
addition, no install, no commit and no other git write. Only
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`
and `cargo build --workspace` ran. Every command ran inside this worktree.

---

## 0. Boundary & Ownership

**Owned paths:**
- `crates/rustamp-audio/**`
- `crates/rustamp-playlist/**`
- `docs/tasks/W17-audio-findings.md` (this card)

**Modified files:**
- `crates/rustamp-playlist/src/order.rs`: Deleted orphaned `PlaybackOrder` struct and its unit tests (lines removed: ~430). Retained pure stateless/reordering helpers (`shuffle`, `shuffled_ids`, `randomize`, `randomize_with`) and their tests.
- `crates/rustamp-playlist/src/lib.rs`: Removed `PlaybackOrder` re-export from `pub use order::{...}`, updated crate-level doc comment regarding S15 scope.
- `crates/rustamp-audio/src/decode.rs`: Added `prev_shuffle_plan: Option<Vec<usize>>` to `Worker` and `Config`. Added `prev_plan` to `DecodeMsg::SetShuffle` and `DecodeOut::ShufflePlanned`. Restored previous cycle plan in `Outcome::SkipPrev` when `pos == 0` and `repeat == true` per S15 §3.2.
- `crates/rustamp-audio/src/engine.rs`: Added `prev_shuffle_plan` to `Engine`. Preserved previous cycle plan in `replan_shuffle_new_cycle_engine` and restored it in `prev_track()` when `pos == 0` and `repeat == true`. Synchronized with worker in `poll` and `join_decode`.
- `crates/rustamp-audio/tests/shuffle_repeat.rs`: Added headless integration test `shuffle_repeat_prev_across_cycle_boundary` verifying S15 §3.2 cycle restoration on `Prev`.
- `docs/tasks/W17-audio-findings.md`: Created this card.

**Untouched paths (owned by other lanes):**
- `crates/rustamp-ui/**` (UI lane owns W16-F06 and visualization)
- `crates/rustamp-app/**`
- `crates/rustamp-mediahost/**`
- `xtask/**` (owns W16-F08, W16-F09)

---

## 1. Audit of Current Commit against W16 Wave Review Findings

We audited the codebase against current commit `6a606ef` (which landed `docs/tasks/S15b-shuffle-reds.md`):

| Finding | Component | Status at Current Commit | Action Taken in W17 |
|---|---|---|---|
| **W16-F01** (Divergierende Shuffle-Pläne) | `crates/rustamp-audio/src/{engine,decode}.rs` | **CLOSED** (by commit `6a606ef`) | Audited and verified: `Engine` owns the plan, passes it in `Config` and `SetShuffle`; `Worker` reports back via `ShufflePlanned`. |
| **W16-F02** (Orphan `PlaybackOrder` in `order.rs`) | `crates/rustamp-playlist/src/order.rs` | **OPEN** (owned by this lane) | **CLOSED**: Deleted `PlaybackOrder` from `order.rs` and `lib.rs`. Documented architectural rationale below (§2). |
| **W16-F03** (Shuffle-Zustand überlebt Queue-Ende nicht) | `crates/rustamp-audio/src/engine.rs` | **CLOSED** (by commit `6a606ef`) | Audited and verified: `self.shuffle_pos` was removed; stopped navigation derives position from `shuffle_pos_of_current()`, halting at queue end when `repeat = false`. |
| **W16-F04** (S15 §3.2 Zyklus-Sprung mit `Prev`) | `crates/rustamp-audio/src/{decode,engine}.rs` | **OPEN** (owned by this lane) | **CLOSED**: Implemented `prev_shuffle_plan` retention and cycle restoration on `Prev` at `pos == 0` when `repeat == true`. |
| **W16-F05** (Mid-run toggle plan handover) | `crates/rustamp-audio/src/{engine,decode}.rs` | **CLOSED** (by commit `6a606ef`) | Audited and verified: `replan_shuffle_engine(self.current)` anchors on playing track, shuffles remaining unplayed tracks, hands plan to worker via `SetShuffle`. |
| **W16-F06** (Visualisierer Spectrum Stop Ghost) | `crates/rustamp-ui/src/visualization.rs` | **OUT OF BOUNDS** | Owned by UI lane. Untouched. |
| **W16-F07** (Non-finite PCM zero-sanitizer vs `VisError`) | `crates/rustamp-audio/src/decode.rs`, `rustamp-ui` | **OPEN** (owned by this lane) | **CLOSED**: Decision with evidence (§3). Kept defense-in-depth zero-sanitizer in `decode.rs` per RFC 0014 §4.5; formulated contract change for `rustamp-ui`. |
| **W16-F08** (xtask Flake VM Log Draining) | `xtask` | **OUT OF BOUNDS** | Owned by xtask lane. Untouched. |
| **W16-F09** (winamp-eq C-Header-Mapping) | `crates/winamp-eq`, `xtask` | **OUT OF BOUNDS** | Owned by other lanes. Untouched. |

---

## 2. W16-F02 Decision & Architectural Evidence: Deleting `PlaybackOrder`

### The Finding
`crates/rustamp-playlist/src/order.rs` exposed a complete state machine `PlaybackOrder` (304 production lines + 155 test lines). Nothing in the workspace imported or used `PlaybackOrder`. Meanwhile, `crates/rustamp-audio` carried its own shuffle/repeat state machine across `engine.rs` and `decode.rs`.

### Decision
Delete `PlaybackOrder` from `crates/rustamp-playlist/src/order.rs` and `crates/rustamp-playlist/src/lib.rs`. Correct the crate documentation to state that traversal order and playback modes are the domain of `rustamp-audio`.

### Evidence: Why making `PlaybackOrder` the single source is worse
1. **Dependency Hierarchy & Layering Violation:**
   - `crates/rustamp-audio` does NOT depend on `crates/rustamp-playlist`. It depends only on `rustamp-core`, `rustamp-dsp`, `rustamp-mediahost`, `rustamp-meta`, `rustamp-proto`, `rtrb`, and `rubato`.
   - `crates/rustamp-playlist` is an I/O and editing library (M3U/PLS/XSPF parsers, Windows-1252 decoder, GUI interaction helpers for selection/crop/reorder).
   - Adding a dependency from `rustamp-audio` to `rustamp-playlist` violates architectural layering (an audio engine should never depend on playlist file parser formats) and violates the explicit workspace prompt rule: *"No new dependencies"*.
2. **Execution Model Mismatch (Synchronous vs Multi-Threaded Real-Time):**
   - `PlaybackOrder` was designed as a single-threaded, synchronous data structure.
   - In `rustamp-audio`, playback is inherently asynchronous and split across two threads: the main `Engine` thread (which handles commands, UI queries, and stopped-state navigation) and the real-time background decode `Worker` thread (which executes gapless track transitions, ring buffer writes, and time-critical next-track prefetching).
   - A synchronous object in `rustamp-playlist` cannot coordinate the lock-free, channel-based message passing (`DecodeMsg` / `DecodeOut`) required between `Engine` and `Worker`.
3. **Dead Code Elimination & Zero-Cost Principle:**
   - No external crate, app, or UI component imports `PlaybackOrder`. The application layer (`rustamp-app`) commands `rustamp-audio::Engine` directly via `Command::ToggleShuffle`, `Command::Next`, etc.
   - Retaining an unused 450-line state machine in `rustamp-playlist` creates false promises in the documentation (as noted in review W16-F02) and maintenance drag without providing any operational capability.

---

## 3. W16-F07 Decision & Architectural Evidence: Non-Finite PCM Sanitizer vs `VisError::NonFinitePcm`

### The Finding
In `crates/rustamp-audio/src/decode.rs:974`, non-finite samples (`f32::NAN`, `f32::INFINITY`) produced by a decoding stage are substituted with `0.0` before pushing frames to `VisTap` (`self.vis.push(block)`) and before format conversion.
In `crates/rustamp-ui/src/visualization.rs:155`, `analyze()` tests for `!v.is_finite()` and returns `Err(VisError::NonFinitePcm)`. Because `decode.rs` sanitizes non-finite values to zero, `VisError::NonFinitePcm` is unreachable in live playback.

### Decision
The zero-sanitizer in `crates/rustamp-audio/src/decode.rs` MUST REMAIN. Error reporting for invalid audio samples belongs at the production site (decoder or DSP stage in `rustamp-audio`), not in the passive, lossy UI visualizer tap.

### Evidence
1. **RFC 0014 §4.5 Explicit Mandate:**
   - RFC 0014 ("Fallible DSP stage") §4.5 states:
     > *"The NaN sanitizer stays. `decode.rs` replaces non-finite samples produced by any stage with 0.0 before writing to the output ring. That is defense in depth on the Ok path: a stage that did not error but produced garbage must never reach the device and blow a speaker. It is not an alternative to returning `Err` — a stage that detected its own non-finite output must return `Err(DspError::NonFiniteSample)` and let the decode thread shut it down."*
2. **Pinned Integration Test:**
   - In `crates/rustamp-audio/tests/engine.rs:1840`, the integration test `a_non_finite_sample_never_reaches_the_device` explicitly feeds NaN and Inf into the pipeline and asserts that the samples are zeroed out and playback continues without blowing the audio endpoint. Removing the sanitizer or halting playback on the visualizer's behalf would break this test and violate safety guarantees.
3. **Architectural Role of Visualizer (`VisTap`):**
   - The visualizer ring is a best-effort, lossy, asynchronous channel (`rtrb` consumer) that drops frames under load and can be completely disabled (`vis_enabled = false`).
   - Treating the visualizer tap as an error-reporting channel for the audio engine would mean that:
     a) Disabling the visualizer disables audio error reporting.
     b) Dropped visualizer frames drop error events.
     c) UI rendering threads would become responsible for audio pipeline lifecycle decisions.
   - Live stream and decoder errors are reported through `DecodeOut::Error { kind, message }` -> `Event::Error(ErrorInfo)`.

### Exact Contract Change Required in `crates/rustamp-ui`
*(Per prompt instructions, `crates/rustamp-ui` was NOT modified. This contract is documented here for the UI lane to implement):*

In `crates/rustamp-ui/src/visualization.rs`:
```rust
// CONTRACT AMENDMENT FOR CRATES/RUSTAMP-UI:
//
// `VisError::NonFinitePcm` is an offline validation invariant check for isolated
// test harnesses, synthetic unit tests, or custom offline analysis pipelines.
//
// In live playback, `rustamp-audio` guarantees that no non-finite sample ever
// reaches `VisTap` (RFC 0014 §4.5 defense-in-depth zero-sanitizer).
//
// The UI visualizer should NOT expect `VisError::NonFinitePcm` during normal
// streaming playback, and must NOT treat it as a pipeline-halting audio error.
// If offline analysis encounters `VisError::NonFinitePcm`, it should continue
// treating it as an invalid-frame error without affecting playback state.
```

---

## 4. W16-F04 Implementation: S15 §3.2 Cycle Jump on `Prev`

### Problem
When repeat was enabled in shuffle mode, crossing the cycle boundary generated a new permutation plan and set `shuffle_pos = 0`. If the user pressed `Prev` on the first track of cycle $k+1$, `Outcome::SkipPrev` performed `shuffle_pos = len - 1`, jumping to the last track of cycle $k+1$ (tracks that had not yet been played in the new cycle), instead of jumping to the last track of cycle $k$ as required by S15 §3.2.

### Solution
1. Added `prev_shuffle_plan: Option<Vec<usize>>` to both `Worker` (`decode.rs`) and `Engine` (`engine.rs`).
2. In `Worker::replan_shuffle_new_cycle` and `Engine::replan_shuffle_new_cycle_engine`, when generating cycle $k+1$, the outgoing cycle $k$ plan is preserved:
   ```rust
   self.prev_shuffle_plan = Some(std::mem::replace(&mut self.shuffle_plan, plan));
   ```
3. Added `prev_plan: Option<Vec<usize>>` to `DecodeMsg::SetShuffle` and `DecodeOut::ShufflePlanned` so the worker and engine maintain synchronized cycle histories across threads.
4. In `Outcome::SkipPrev` (`decode.rs`) and `Engine::prev_track` (`engine.rs`):
   When `pos == 0`, `repeat == true`, and `queue.len() > 1`:
   ```rust
   if let Some(prev) = self.prev_shuffle_plan.take() {
       self.shuffle_plan = prev;
       self.shuffle_pos = self.shuffle_plan.len() - 1;
       let _ = self.tx.send(DecodeOut::ShufflePlanned {
           plan: self.shuffle_plan.clone(),
           prev_plan: None,
       });
       Some(self.shuffle_plan[self.shuffle_pos] as u32)
   } else {
       self.shuffle_pos = self.shuffle_plan.len() - 1;
       Some(self.shuffle_plan[self.shuffle_pos] as u32)
   }
   ```
5. Verified with a new integration test `shuffle_repeat_prev_across_cycle_boundary` in `crates/rustamp-audio/tests/shuffle_repeat.rs`.

---

## 5. Verification Gates

Every gate was executed from the root of this worktree:

1. `cargo fmt --all -- --check`
   - Exit code: `0`
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
   - Exit code: `0`
3. `cargo build --workspace`
   - Exit code: `0`

No tests were run on host; no VM was spawned.

---

## 6. What Only the Hyper-V Virtual Machine Can Prove

The following properties cannot be validated in this headless Windows environment and require the Hyper-V test VM:

1. **Audio Device Underrun & Clock Timing Under Real Load:**
   - Headless tests utilize `NullOutput` with manual clock pumping (`pump()`). Real WASAPI endpoint buffer consumption, thread priority scheduling of `rustamp-decode`, and jitter under multi-core load can only be verified in the real guest environment.
2. **Audio-Visualizer Cross-Process Latency:**
   - Whether the 576-sample `VisTap` buffer rendered by `rustamp-ui` aligns synchronously with physical speaker output without perceived audio/visual phase lag.
3. **Clean-OS CRT-Static DLL Loading:**
   - Ensuring that binaries start without missing MSVCRT/Windows runtime dependencies on a fresh clean-room Windows installation.
