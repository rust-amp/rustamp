# Task Card S15 — Shuffle & Repeat Implementation (BETA-08 Resolution)

**Goal:** Implement the shuffle and repeat playback specification (S15) across the playlist data model (`rustamp-playlist`) and the audio playback engine (`rustamp-audio`), closing BETA-08 from the second readiness review (`docs/tasks/BETA-readiness-2.md`).

**Lane:** Spec + Implementation.
**Owned Paths:**
- `specs/S15-shuffle-repeat.md`
- `crates/rustamp-playlist/**`
- `crates/rustamp-audio/**`
- `docs/tasks/S15-shuffle-repeat-impl.md`

**Allowed Sources:**
- Clean-room specification: `specs/S15-shuffle-repeat.md`
- Related specs: `specs/S06-playlist.md`, `specs/S02-main-window-behaviour.md`
- Architecture and contracts: `docs/rfc/0005-app-commands.md`, `crates/rustamp-proto`

**Forbidden Sources:**
- Winamp source code, SDK headers, disassembly, reverse engineering binaries
- Directories: `D:\code\rustamp-research`, `D:\code\rustamp-refbin`

---

## 1. Scope & Architecture

### A. Specification (`specs/S15-shuffle-repeat.md`)
Authored clean-room spec establishing:
1. Four distinct playback modes: Sequential No-Repeat, Sequential Repeat, Shuffle No-Repeat, Shuffle Repeat.
2. Cycle semantics: Shuffled playback visits every entry exactly once per cycle (Fisher-Yates permutation).
3. Anti-immediate-repeat cycle boundary guarantee: For queue length $N > 1$, the first entry of cycle $k+1$ is guaranteed not to equal the last entry of cycle $k$.
4. Deterministic manual navigation (`Next`, `Prev`, `SkipBy`) respecting permutation history.
5. Mid-run toggle behaviour preserving current track and planning remaining unplayed entries.
6. Playlist window presentation invariant: Display order in GUI remains unaltered; current playing indicator moves along the active traversal order.

### B. Playlist Model (`crates/rustamp-playlist/src/order.rs`)
- Introduced `PlaybackOrder` managing traversal order, cycle counting, history stack for backward navigation, and mid-run mode switching.
- Exported via `crates/rustamp-playlist/src/lib.rs`.

### C. Audio Engine & Decode Worker (`crates/rustamp-audio`)
- `decode.rs`:
  - Added `DecodeMsg::SetShuffle(bool)` and `DecodeMsg::SetRepeat(bool)`.
  - Added `shuffle`, `repeat`, `shuffle_seed` to `Config`.
  - Implemented clean-room xorshift64 PRNG in `Worker` for permutation planning.
  - Added cycle boundary anti-repeat swap logic in `replan_shuffle_new_cycle`.
  - Implemented mid-run re-planning in `replan_shuffle_mid_run`.
  - Integrated traversal decisions in `Worker::run` for `Outcome::Ended`, `Outcome::SkipNext`, `Outcome::SkipPrev`, and `Outcome::SkipBy`.
- `engine.rs`:
  - Added `shuffle`, `repeat`, `shuffle_seed`, `shuffle_plan`, `shuffle_pos` fields to `Engine`.
  - Added accessors: `shuffle()`, `repeat()`, `set_shuffle()`, `toggle_shuffle()`, `set_repeat()`, `toggle_repeat()`, `set_shuffle_seed()`, `shuffle_plan()`.
  - Wired `Command::ToggleShuffle`, `Command::ToggleRepeat`, `Command::SetShuffle`, `Command::SetRepeat` in `apply()`.
  - Updated stopped manual navigation (`next_track()`, `prev_track()`, `skip_by()`) to obey active shuffle/repeat modes.
  - Updated `finish_run()` to respect repeat mode when audio clock reaches end of stream.

---

## 2. Test Coverage (Headless, Fail-able)

### A. Model Unit Tests (`crates/rustamp-playlist/src/order.rs`)
- `test_sequential_no_repeat`: Validates strict monotonic index advance, stops at end.
- `test_sequential_repeat`: Validates seamless wrap from $N-1$ to 0.
- `test_shuffle_no_repeat_exhaustive_permutation`: Proves all items visited exactly once without repetition, stops at cycle end.
- `test_shuffle_repeat_anti_immediate_repeat`: Tests 100 consecutive cycle boundaries with randomized seeds, asserting no adjacent track repetition across cycles.
- `test_manual_prev_history_in_shuffle`: Verifies `prev` steps backwards along the exact permutation history.
- `test_mid_run_shuffle_toggle`: Verifies toggling shuffle mid-track generates a permutation over remaining unplayed entries.
- `test_mid_run_repeat_toggle`: Verifies toggling repeat dynamically converts queue-end stop into seamless cycle wrap.
- `test_single_entry_queue`: Validates single-item edge cases across all 4 modes.
- `test_empty_queue`: Asserts safe empty queue operations without panic.

### B. Engine Integration Tests (`crates/rustamp-audio/tests/shuffle_repeat.rs`)
- `mode_sequential_no_repeat_queue_end`: Verifies engine plays through queue and emits `Event::PlaylistEnded` and `TransportState::Stopped`.
- `mode_sequential_repeat_loops_indefinitely`: Verifies engine wraps around and continues playback without emitting `PlaylistEnded`.
- `mode_shuffle_no_repeat_queue_end`: Verifies engine visits full permutation and halts on cycle finish.
- `mode_shuffle_repeat_anti_immediate_repeat`: Verifies continuous cycle renewal without queue termination.
- `manual_advance_sequential_modes`: Verifies `next_track` and `prev_track` advance and wrap according to repeat state.
- `manual_advance_shuffle_history`: Verifies manual `next_track` and `prev_track` follow the shuffle plan and permutation history.
- `toggle_shuffle_and_repeat_mid_run`: Verifies live mode toggling during active decoding, emitting `Event::ModesChanged` and selecting next track from unplayed set.
- `single_entry_queue_all_modes`: Verifies single-track playback across all four modes.

---

## 3. What Only the Reference Run or VM Could Add

1. **Winamp 2.95 PRNG Algorithm Matching:**
   Whether Winamp 2.95 uses standard C runtime `rand()`, a Linear Congruential Generator (LCG), or a custom PRNG table. Clean room uses an independent, high-entropy xorshift64 PRNG. A black-box measurement in the reference sandbox could observe sequence patterns if exact parity is desired.
2. **First Track on Mid-Run Shuffle Toggle:**
   Whether Winamp immediately replaces the remaining playlist or re-indexes based on the current track's original position in the playlist window.
3. **Double Click Selection Interaction:**
   When a user double-clicks an item in the playlist while shuffle is active, whether that track becomes index 0 of a fresh permutation or replaces the current position in the existing permutation.
4. **GUI Rendering of Shuffle/Repeat Buttons:**
   Verification in Hyper-V VM of visual skin button depression and lighting states under diverse skin bitmap configurations.
