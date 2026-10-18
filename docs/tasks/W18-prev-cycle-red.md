# W18 — Fix red test `shuffle_repeat_prev_across_cycle_boundary`

**Date** 2026-10-18. **Lane** `paseo-wave91-prev`.
**Worktree** `C:\Users\micro\.paseo\worktrees\3ikyavhh\paseo-wave91-prev`.

Boundary rule: write ONLY inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

**Status:** implemented; fmt, clippy and build gates passing. VM test execution
pending orchestrator run.

No tests were run on the host (Hyper-V test-VM constraint strictly preserved):
not the suite, not `--no-run`, not a single binary, not a lib unit test. No test
process, example, player, helper binary or reference binary was spawned; no VM
was touched; no window or audio device was opened; no download, no dependency
addition, no install, no commit and no other git write. Only
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`
and `cargo build --workspace` ran.

The red, as named by the suite run:
`shuffle_repeat` integration test: `shuffle_repeat_prev_across_cycle_boundary`,
panicking at `crates/rustamp-audio/tests/shuffle_repeat.rs:370`:
`assertion left == right failed, left: 2, right: 0`.

---

## 0. Boundary & Ownership

Owned paths: `crates/rustamp-audio/**`, `crates/rustamp-playlist/**`, and this card.
Touched:
- `crates/rustamp-audio/tests/shuffle_repeat.rs`
- `docs/tasks/W18-prev-cycle-red.md` (this card)

Not touched:
- `crates/rustamp-app/**`
- `crates/rustamp-ui/**`
- `crates/rustamp-mediahost/**`
- `xtask/**`
- All other crates in the workspace.

Read only (not modified): `AGENTS.md`, `specs/S15-shuffle-repeat.md`,
`docs/tasks/W17-audio-findings.md`, `docs/tasks/S15b-shuffle-reds.md`,
`crates/rustamp-audio/src/engine.rs`, `crates/rustamp-audio/src/decode.rs`,
`crates/rustamp-audio/tests/common/mod.rs`.

---

## 1. Root Cause Analysis

### 1.1 The Failure
Commit `702719c` implemented review finding W16-F04 (S15 §3.2 cycle jump on `Prev` with `Repeat ON`)
and added the integration test `shuffle_repeat_prev_across_cycle_boundary`.
When executed in the quiet host VM test environment, the test failed at line 370:
```rust
let plan1 = h.engine.shuffle_plan().to_vec();
assert_eq!(plan1.len(), 3);
assert_eq!(plan1[0], 0); // assertion left == right failed, left: 2, right: 0
```

### 1.2 Verdict: The Expectation (Test) Was Wrong
The engine and decode worker implementation of review finding W16-F04 is completely in accordance
with S15 §3.2 and the architectural division of plan ownership from S15b.
The test asserted an incorrect premise (`assert_eq!(plan1[0], 0)`).

### 1.3 Detailed Mechanism
1. In `Harness::new(&[SHORT_A, SHORT_B, SHORT_C])`, a 3-track queue is configured with tracks `[0, 1, 2]`.
2. Calling `h.engine.set_shuffle(true)` invokes `replan_shuffle_engine(None)` because playback has not started (`self.current.is_none()`).
3. Per S15 §1.1, the engine generates a pseudo-random permutation of `{0, 1, ..., N - 1}` without replacement using the PRNG. With 3 items and default seed `0x9E37_79B9_7F4A_7C15`, the resulting permutation `self.shuffle_plan` is `[2, 0, 1]`. The first track of cycle 1 is track 2 (`plan1[0] == 2`).
4. The test author intended to start playback at the beginning of cycle 1, but mistakenly wrote:
   ```rust
   // Start playback on first track of cycle 1
   h.engine.play_index(0).unwrap();
   assert!(h.wait_track(0));

   let plan1 = h.engine.shuffle_plan().to_vec();
   assert_eq!(plan1.len(), 3);
   assert_eq!(plan1[0], 0);
   ```
   The author confused queue index 0 with cycle position 0 ($k = 0$).
5. `Engine::play_index(0)` does not re-plan the shuffle permutation (as established in S15b, `ensure_shuffle_plan()` keeps the existing plan since it is valid and contains track 0). Thus, track 0 is played at position 1 in `[2, 0, 1]`.
6. At line 370, `assert_eq!(plan1[0], 0)` compared `plan1[0]` (which is 2) with 0 and panicked.

---

## 2. Changes Made

In `crates/rustamp-audio/tests/shuffle_repeat.rs`:
1. Updated `shuffle_repeat_prev_across_cycle_boundary` to start playback at the first track of cycle 1 using `h.engine.play().unwrap()` (as specified in S15 §1.1 and demonstrated in `manual_advance_shuffle_history`), reading `plan1[0] as u32` rather than assuming track 0.
2. Removed the erroneous assertion `assert_eq!(plan1[0], 0)`.
3. In the cycle 2 -> cycle 1 wrap check with `prev_track()`, strengthened the condition in `wait_pumping` to wait for both `e.current_index() == Some(plan1[2] as u32)` and `e.shuffle_plan() == &plan1[..]`, guaranteeing that the state transition across the decode worker/engine thread boundary has fully propagated before asserting final plan equivalence.

---

## 3. Verification

The following gates were run on the workspace:
- `cargo fmt --all -- --check`: Exit code 0
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: Exit code 0
- `cargo build --workspace`: Exit code 0
