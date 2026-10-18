# S15b — Fix the three shuffle/repeat reds named by the suite run

**Date** 2026-10-18. **Lane** `wave89-shuffle-reds`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave89-shuffle-reds`.

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
and `cargo build --workspace --offline` ran (receipts in §6). Every command ran
inside this worktree.

The three reds, as the guest named them:

1. `shuffle_repeat` binary: `manual_advance_shuffle_history`, panicking at
   `crates/rustamp-audio/tests/shuffle_repeat.rs:214` (`assert!(h.wait_track(t1))`).
2. `shuffle_repeat` binary: `toggle_shuffle_and_repeat_mid_run`, panicking at
   `crates/rustamp-audio/tests/shuffle_repeat.rs:256`.
3. `rustamp_audio` lib binary: `engine::tests::stop_does_not_advance_to_prefetched_next_track_during_gapless`,
   panicking at `crates/rustamp-audio/src/engine.rs:1908`.

---

## 0. Boundary & ownership

Owned paths: `crates/rustamp-audio/**`, `crates/rustamp-playlist/**`, and this
card. Touched (all inside `crates/rustamp-audio`):

- `crates/rustamp-audio/src/engine.rs` (production + the `mod tests` unit test)
- `crates/rustamp-audio/src/decode.rs` (production)
- `crates/rustamp-audio/tests/shuffle_repeat.rs` (one wait fixed)
- `docs/tasks/S15b-shuffle-reds.md` (this card)

Read only (not modified): `AGENTS.md`, `CLEANROOM.md`, `CONTRIBUTING.md`,
`docs/HANDOFF.md`, `specs/S15-shuffle-repeat.md`,
`docs/tasks/S15-shuffle-repeat-impl.md`, `docs/tasks/M29-liveness-rest.md`,
`docs/tasks/M30-erasure-twins.md`, `docs/tasks/A25-erasure-path.md`,
`crates/rustamp-audio/tests/common/mod.rs`, `crates/rustamp-audio/src/ring.rs`,
`crates/rustamp-output/src/null.rs`, `crates/rustamp-playlist/**` (read in full;
no change needed — see §3.4), and the other crates' shuffle call sites
(`rustamp-app`, `rustamp-ui`: the app owns shuffle/repeat and never enables the
engine's, so the engine changes cannot reach their tests).

Not touched: `crates/rustamp-mediahost`, `crates/rustamp-ui`,
`crates/rustamp-app`, and every other crate — another lane owns them right now.

Sources: the task prompt, the cards and specs named above, and the existing
clean repository code only. No Winamp source, SDK material, disassembly,
research tree or reference binary was read; no third-party code was copied; no
dependency was added.

---

## 1. Verdicts and mechanisms

### 1.1 Red 1 — `manual_advance_shuffle_history` (tests/shuffle_repeat.rs:214)

**Verdict: the CODE is wrong; the expectation is right.** S15 §1.1/§3.1/§3.2
define one permutation `P` — the one the engine publishes through
`Engine::shuffle_plan()` — and manual `Next`/`Prev` traverse it.

**Mechanism (two divergent plans + a wrong start track):**

1. `Engine::set_shuffle(true)` generates the engine's plan
   (`replan_shuffle_engine`, engine.rs:623) from `self.shuffle_seed` and
   publishes it via `shuffle_plan()`.
2. But the decode worker built its **own private permutation** in
   `Worker::run` (`init_shuffle_plan(start_index)`, decode.rs:510) from a
   *different* PRNG state: `Config.shuffle_seed` carried the engine's seed
   **after** the engine's replan had advanced it, plus the `+1` increment in
   `start_run` (engine.rs:1526), and the worker drew with `rng_below`
   (rejection sampling) where the engine drew with plain modulo.
3. Live manual navigation is decided by the **worker** along *its* plan
   (`DecodeMsg::Next`/`Prev` → decode.rs:560-610), and the engine's `current`
   only follows the worker's `TrackStarted`. So after `next_track()` the
   engine lands on `worker_plan[1]`, which is generally not the published
   `plan[1]` = `t1`, and `wait_track(t1)` — which correctly waits for
   `current_index() == Some(t1)` — times out at line 214.
4. Additionally `Engine::play()` from stopped started at
   `current.unwrap_or(0)` — queue index 0 — instead of the permutation's
   first track, so even the first track matched the published plan only when
   `plan[0] == 0` (which happens to hold for seed 999; that is why the red is
   at line 214 and not at line 210).

**Was the wait waiting for the wrong thing?** No. `wait_track(t1)` polls the
right condition (`current_index() == Some(t1) && duration_frames().is_some()`);
the engine simply never reaches `t1` because the worker traverses a different
permutation. The wait is a correct condition wait; the code under it is not.

**Fix (engine owns the plan; the worker follows it):**

- `Config` gains `shuffle_plan: Vec<usize>`; `Engine::start_run` hands the
  engine's plan to the worker (engine.rs:1606) after `ensure_shuffle_plan()`
  makes it coherent with the queue and the current track.
- `DecodeMsg::SetShuffle` now carries the plan (`{ on, plan }`); the worker
  adopts it on a mid-run toggle (decode.rs:1094), anchored at the track that is
  playing, instead of re-planning its own.
- The worker traverses the engine's plan and derives its position from its
  current track (decode.rs:530-547); `init_shuffle_plan` survives only as a
  fallback for when the engine handed no usable plan.
- `Engine::play()` from stopped with shuffle on and nothing selected starts at
  `shuffle_plan[0]` (engine.rs:1540) — S15 §1.1's "the permutation is
  generated upon starting playback with Shuffle ON".
- The engine's stopped navigation (`next_track`/`prev_track`/`skip_by`)
  derives its position from the current track (`shuffle_pos_of_current`)
  instead of a stored `shuffle_pos` field, which went stale while a run
  advanced the plan; the field is removed.
- The engine's stopped cycle-boundary replan (`replan_shuffle_new_cycle_engine`)
  now generates a full permutation whose first entry is not the track the
  exhausted cycle ended on — the old code re-anchored the plan at the last
  track, i.e. it repeated it, violating S15 §2.1.
- The worker reports plans it generates itself (cycle boundaries, fallback
  inits) as `DecodeOut::ShufflePlanned`; the engine adopts them in `poll` and
  in `join_decode`'s drain, so `shuffle_plan()` stays the plan being traversed
  across live cycle crossings and stop/finish teardowns.

The engine's PRNG sequence is preserved exactly (the `rng_step` refactor
produces the same raw values as the old inline loop for every non-zero seed), so
`mode_shuffle_no_repeat_queue_end` and `mode_shuffle_repeat_anti_immediate_repeat`
keep their plans; those tests never asserted which permutation plays, only
counts and states, and they stay green by construction (the worker now visits
every entry of the engine's permutation exactly once per cycle).

### 1.2 Red 2 — `toggle_shuffle_and_repeat_mid_run` (tests/shuffle_repeat.rs:256)

**Verdict: the expectation is right; the test's wait is wrong — it is not a
wait at all.** S15 §4.1: after toggling shuffle ON mid-run, the remaining
unplayed tracks are permuted and `Next` advances to the first of them, i.e.
one of `[1, 2, 3]` here.

**Mechanism (a poll-once-and-read instead of a condition wait):** the test
sent `next_track()` (a channel message, engine.rs:983 → `DecodeMsg::Next`),
then called `engine.poll()` **exactly once** (line 254) and read
`current_index()` (line 255). A manual skip while playing is asynchronous: the
worker notices the message in `check_control` (decode.rs:1028, serviced every
block, or every 2 ms while it is blocked on a full ring), abandons the current
track, opens the next one and only then queues its `TrackStarted`. That takes at
least a scheduling round-trip; the test reads the index microseconds after the
send, so `current_index()` is still the stale `Some(0)` and
`[1, 2, 3].contains(&next_idx)` fails. This is the project's recurring
"poll once and read a state the decode thread has not produced yet" pattern.

**Was the wait waiting for the wrong thing?** Yes — symmetrically to red 3: the
test asserted a state (`current_index()` in `[1,2,3]`) without waiting for the
condition that produces it. (The engine-side plan handoff from §1.1 also makes
the mid-run toggle coherent: the worker now traverses the engine's anchored
plan, so the landed track is the engine's published `plan[1]`.)

**Fix (tests/shuffle_repeat.rs:252-266):** wait for the condition itself —
`h.wait_pumping(|e| matches!(e.current_index(), Some(i) if (1..=3).contains(&i)), WAIT)`
— and then assert the same membership. The claim is unchanged (still "next
lands on one of the remaining unplayed tracks"); the wait now actually waits
for it, with the harness's 30 s stall escape (`common::WAIT`, the M10/M24
rule). No sleep was introduced as an ordering mechanism; the 1 ms cadence is
the harness's poll interval.

### 1.3 Red 3 — `engine::tests::stop_does_not_advance_to_prefetched_next_track_during_gapless` (src/engine.rs:1908)

**Verdict: the expectation is right; the test's first wait is wrong — it
asserts one state while polling another.** M30 §1.2 proved why `stop()` must
discard the prefetched `TrackStarted`: delivering it would make a user who
stops on track 0 see the player jump to track 1. A25/M30's `join_decode` holds
exactly that start, and `stop()` discards it — the code is correct.

**Mechanism (the wait's polling destroys the state it asserts):** the first
wait was `h.wait_pumping(|e| e.duration_frames() == Some(2048), wait)` —
"track 0 must start". `wait_pumping` **polls** the engine, and `Engine::poll`
drains the whole `from_decode` queue in one call (engine.rs:1159-1173). Track 0
was only 2048 frames, so the worker decoded it in two blocks and gaplessly
prefetched track 1 within microseconds — long before the test's first poll —
queueing `TrackStarted{track 1}` (decode.rs:541, 781). The first poll therefore
processed `TrackStarted(0)` **and** `TrackStarted(1)` in the same drain:
`duration_frames()` jumped from `Some(2048)` straight to `Some(44100)` and
`current` to `Some(1)`. The asserted state (`duration_frames() == Some(2048)`)
is transient and unreachable; the wait can only succeed if the first poll
lands in the microsecond window between the two sends. That is a deterministic
red at engine.rs:1908, not a flake. The recent engine work is what makes the
queued start observable at stop time (A25/M30's held start), but it is the
wait's polling that consumes it before the stop. The sibling test
`engine_publishes_the_start_when_output_fails_right_after_it` passes because its
first wait uses the monotone `duration_frames().is_some()` and its first run
arms stop-after-current, so track 1's start is never queued.

A second, latent defect: the test's local `wait` was 5 s — the flake class M24
proved in this guest. The M29 liveness sweep converted the shared harness
(`tests/common/mod.rs`) to 30 s but never reached this private test module in
`src/engine.rs`.

**Fix (engine.rs:1991-2058, rewritten test):** make the "track 0 started,
track 1's start queued but unprocessed" state deterministic instead of racy:

- Track 0 is now 44100 frames — longer than the ring (16384 frames) — so the
  decode thread blocks mid-track-0 (the null device consumes nothing unless
  pumped, null.rs:312-318) and **cannot** have opened track 1 yet.
- The test waits (without polling) until `sync.counters().0 > 0`: the worker's
  first write comes after `prepare_track` queued `TrackStarted(0)`, and the
  ring still holds the rest of track 0 — the same non-polling pattern the
  sibling test's `start_queued` wait already used.
- One `poll()` inside that guaranteed window publishes track 0's start
  (asserted: `duration_frames() == Some(44100)`, `current_index() == Some(0)`);
  it cannot consume track 1's start because that message does not exist yet.
- Then the test pumps until `written > 44100` (track 0 finished, track 1
  opened, its start queued — still no polling, so the start stays queued) and
  asserts the device has only partially played track 0
  (`clock.frames() < 44100` — now a real partial playback, ~28k of 44100
  frames, where the old test never pumped at all and the assertion was
  trivially `0 < 2048`).
- `stop()` and every original assertion are kept verbatim: `Stopped`,
  `current_index() == Some(0)`, no `Event::TrackChanged` for track 1.
- The local stall escape is raised to 30 s with a comment citing the M10/M24
  rule and the M29 sweep's blind spot.

No claim was weakened; the construction is strengthened (published start,
real partial playback, deterministic window).

---

## 2. Assertions kept (none weakened)

- Red 1: the test is unchanged; the engine now satisfies it. The spec claims
  (permutation traversal, history-based `Prev`, `play()` starting the cycle)
  are met by construction, independent of the seed.
- Red 2: the membership claim `[1, 2, 3]` and the `ModesChanged` claims are
  byte-identical; only the missing wait was added.
- Red 3: `Stopped`, `current_index() == Some(0)`, and "no `TrackChanged` for
  track 1" are unchanged; the "track 0 must start" claim is kept as an
  explicit deterministic assert (`duration_frames() == Some(44100)`).
- No sleep was introduced as an ordering mechanism anywhere; the only sleeps
  are the harnesses' 1 ms poll cadences. No pump-count proxy was introduced:
  every wait measures its own condition (engine state or `sync.counters()`).
- The M30 sibling test `engine_publishes_the_start_when_output_fails_right_after_it`
  is untouched and keeps its 5 s local bounds (it is deterministic and was not
  red; flagged here as the same M24 flake class for a follow-up lane).

## 3. Residuals (deliberately not fixed here)

1. **Engine plan vs. worker queue snapshot on `set_queue` mid-run:** the worker
   snapshots `cfg.queue` at run start; replacing the queue while a run is live
   replans engine-side only. Pre-existing, out of scope for these reds.
2. **`shuffle_plan()` between a live cycle crossing and the next poll** reflects
   the plan handed at run start; the worker's `ShufflePlanned` report closes it
   on the next poll (and at teardown via the join drain). Inherent to the
   poll-driven design.
3. **`rustamp-playlist`'s `PlaybackOrder` is a second, standalone S15 model**
   (order.rs) with its own plan/pos fields and its own unit tests (green). The
   engine does not use it; nothing in the three reds involves it, so it was
   read and left alone. A future lane should decide which model the app's
   playlist window consults.
4. The engine's stopped shuffle navigation now derives its position from the
   current track; a plan that is stale in *content* (same length, different
   queue) is still trusted — `set_queue` replans, so this needs a queue change
   without a replan, which no path produces.

## 4. What only the VM can prove

The suite executes inside the Hyper-V test VM; this lane ran no test. The
orchestrator should run, in the guest:

```
cargo test -p rustamp-audio --test shuffle_repeat
cargo test -p rustamp-audio --lib
cargo test --workspace --all-features
```

Expected outcome:

- `shuffle_repeat` binary: `manual_advance_shuffle_history` and
  `toggle_shuffle_and_repeat_mid_run` green, and the five other tests in the
  binary (`mode_sequential_no_repeat_queue_end`, `mode_sequential_repeat_loops_indefinitely`,
  `mode_shuffle_no_repeat_queue_end`, `mode_shuffle_repeat_anti_immediate_repeat`,
  `manual_advance_sequential_modes`, `single_entry_queue_all_modes`) still green.
- `rustamp_audio` lib: `engine::tests::stop_does_not_advance_to_prefetched_next_track_during_gapless`
  and its sibling `engine::tests::engine_publishes_the_start_when_output_fails_right_after_it`
  green; no other lib test regressed.
- Everything else in the workspace unchanged (the engine's public surface is
  unchanged; `rustamp-app`/`rustamp-ui` never enable the engine's shuffle).

Per M26 §5, one green run is not evidence for a race: at least a few consecutive
green runs of the previously-red binaries are the bar. If a red appears, the
failing line decides:

- Red at `shuffle_repeat.rs:210` (`wait_track(t0)`): `play()` did not start at
  the plan's first track — dump `shuffle_plan()` and `current_index()` there.
- Red at `shuffle_repeat.rs:214` (`wait_track(t1)`): the worker did not follow
  the handed plan — dump `shuffle_plan()` and the drained `TrackChanged`
  indices.
- Red at `shuffle_repeat.rs:256` region: the wait timed out — the worker never
  landed on an unplayed track; dump `current_index()` and `shuffle_plan()`.
- Red at `engine.rs` in the rewritten test's `track0_started`/`prefetched`
  waits: a real stall (the 30 s escape names the step); red at the single-poll
  asserts: the window was not deterministic on that guest.

## 5. Files and line counts

All inside the owned paths. `git diff --stat`:

```
 crates/rustamp-audio/src/decode.rs           |  80 +++++++--
 crates/rustamp-audio/src/engine.rs           | 249 ++++++++++++++++++++-------
 crates/rustamp-audio/tests/shuffle_repeat.rs |  15 +-
 3 files changed, 268 insertions(+), 76 deletions(-)
```

File line totals (after the change):

- `crates/rustamp-audio/src/engine.rs`: 2073 lines (was 1948)
- `crates/rustamp-audio/src/decode.rs`: 1278 lines (was 1220)
- `crates/rustamp-audio/tests/shuffle_repeat.rs`: 356 lines (was 347)
- `docs/tasks/S15b-shuffle-reds.md`: this card (new)

No `Cargo.toml`/`Cargo.lock` change, no new dependency, no `THIRD_PARTY.md`
change. No file in any other crate was touched.

## 6. Gate receipts

Executed strictly within this worktree
(`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave89-shuffle-reds`):

- `cargo fmt --all -- --check`: **PASS** (exit 0, no diffs; one rustfmt
  reformatting of a match-guard arm applied with `cargo fmt --all` first).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **PASS** (exit 0, no warnings; compiles every edited test target — nothing
  was executed). Two findings fixed on the way: a `usize`/`u32` mismatch in the
  new start-index selection, and a `collapsible_match` on the join-drain arm.
- `cargo build --workspace --offline`: **PASS** (exit 0).
- `cargo test`: **NOT RUN** — not the suite, not `--no-run`, not a single
  binary, not a lib unit test (Hyper-V orchestrator rule respected).
- No GUI window opened, no audio device accessed, no external dependency
  added, no git commit created, no process spawned (beyond the toolchain's own
  compile steps), no download, no VM touched.

## 7. Boundary receipt

`git status --short` inside this worktree (read-only) lists exactly the three
modified files and this card:

```
 M crates/rustamp-audio/src/decode.rs
 M crates/rustamp-audio/src/engine.rs
 M crates/rustamp-audio/tests/shuffle_repeat.rs
?? docs/tasks/S15b-shuffle-reds.md
```

Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or
`D:\code\rustamp-refbin` was read or written, and no file outside the owned
paths was modified.

## 8. Provenance

Implemented only from the task prompt, the cards and specs named in §0
(S15, S15-impl, M29, M30, A25), and the clean repository code (read-only outside
the owned paths). No Winamp source, SDK headers, disassembly or forbidden-tree
contents were read. No third-party code copied. License impact: none (no
dependency change; `rustamp-audio` stays GPL-3.0-or-later).
