# A11: ring invalidation at the start of a run (A8), limiter on by default

**Date** 2026-10-17. **Lane** `rustamp-audio` + `rustamp-dsp/src/limiter.rs`.
This redoes the reverted A9 attempt (`d4893c8`, reverted by `f1b87b8`).
Nothing was run on the host. Only `cargo fmt --check`, `cargo clippy -D
warnings` and `cargo build` were used; the test suite runs in the VM.

## Why A9 failed in the VM

All three failures had one cause, and it was not the limiter. A9 bumped the
epoch in `start_run` and had the decode thread wait for the drain, which was
correct. But the drain happens inside a render callback that outputs
**silence**, and the device clock (`played_frames`) counts that callback. A9
never re-based the engine clock at the drain, so the new run's position ran
ahead of its audio by exactly the drain callback. In the tests that callback
is one `settle_drain` pump of 64 frames:

| Test (A9 line) | Observed | Explanation |
|---|---|---|
| `default_engine_limiter_is_enabled_and_limits_overs` (1449) | recorded `[1.058, 0.661, 0.264…]`, expected `[0.6625, 0.2655, 0.869…]` | recorded = `pattern(1, 0..)`, expected = `pattern(1, 64..)` (checked numerically). It is a 64-frame clock offset, not a one-sample shift. |
| `limiter_limits_hot_signal_…` (1503) | `p0_disabled = 64`, `p0 = 0` | the same 64 silent frames counted as position |
| `stopped_run_leaves_no_stale_audio_in_ring_on_restart` (1888) | `p_restarted != p_fresh` | the same |

The limiter's look-ahead never shifted the timeline. The decode thread
already trims the stage's start-up frames and flushes its tail
(`decode.rs`, `emit` / `flush_dsp`), and
`the_enabled_limiter_keeps_a_gapless_join_exact_and_undelayed` (the A6
finding 4 test) already showed this. The limiter only looked guilty because
the restart tests that failed toggled it.

## Design

It is the seek handshake (`ring.rs`), applied to the start of a run:

1. `Engine::start_run` resets `written`/`consumed` to 0 (the device is paused
   and the old decode thread joined). If the ring is non-empty (producer-side
   occupancy), or a seek epoch of the previous run was never drained, it bumps
   the epoch **after** the resets, so the drain aligns `consumed` with 0. It
   passes the epoch as `Config::start_epoch`.
2. The worker starts with `drain_epoch = start_epoch`, so its first
   `write_all` waits for the drain in the existing `await_drain`. Once the
   drain is done, `await_drain` sends `DecodeOut::StartDrained { epoch,
   timeline_pos }` and waits for `engine_ack`, as `apply_seek` does.
3. The engine handles `StartDrained` like `Seeked`: `clock_origin =
   played_frames()`, `timeline_origin = timeline_pos`, `ack_engine(epoch)`.
   Only then does the first frame of the new run enter the ring.

An initial seek (`start_at`) bumps a newer epoch before the first write.
Its drain covers the start drain, so `apply_seek` clears `report_start` and
its `Seeked` does the re-base. A run that starts with an empty ring (first
run, or after a natural end) skips all of this, so its behaviour is
unchanged. The render callback is not touched (CONTRIBUTING audio-thread
rule).

**Counters.** `buffered_frames()` now reports a previous run's audio:

- While stopped, it reports the ring's physical occupancy (read from the
  producer the engine holds).
- After a start, it reports the snapshot `stale_frames` until the drain is
  acknowledged. Nothing of the new run can be in the ring before that drain.

New accessors: `Engine::is_drain_pending()` (the newest epoch is not drained
yet) and `Engine::is_start_pending()` (the drain or the re-base after it is
outstanding).

**Rejected alternatives.**

- *Consumer-side skip by an absolute write mark, with no silent callback.*
  This does not help. The stale audio fills the ring, so the new run cannot
  pre-fill before a callback frees space, and the first callback still
  underruns.
- *Subtracting the drain callback's length from the clock.* This is
  deterministic, but it still counts the underrun that follows before the
  first write. The re-base with ack excludes both.

## Invariants, their tests, and what the tests cannot catch

- **No frame of the previous run is played after a restart.** Pinned by
  `stopped_run_leaves_no_stale_audio_in_ring_on_restart` (bit-identical to a
  fresh engine) and `a_restart_with_the_default_limiter_matches_a_fresh_run`
  (the same with the default chain).
- **The new run loses nothing and its position starts exactly at 0.** Pinned
  by the same two tests and by the two limiter tests (`p == 0`, recording ==
  `expected(…, 0, …)`).
- **Paused before the drain:** nothing is written before the drain, and the
  run starts clean after the resume. Pinned by
  `a_restart_paused_before_its_drain_starts_clean`.
- **An empty ring needs no drain.** Pinned by
  `a_restart_after_a_natural_end_needs_no_drain` and
  `second_run_resets_consumed_and_drains_correctly`.
- **`buffered_frames()` is never 0 while the ring holds stale audio.** Pinned
  by the stopped-run test (`stale > 0` after stop, `== stale` until the
  drain).
- **The seek path is unchanged.** The existing seek tests cover it.
- **What none of these tests can catch:**
  - Real-device timing. If the engine does not poll within
    `HANDSHAKE_TIMEOUT` (250 ms) after the drain, the decode thread writes
    anyway, and the late re-base then drops the frames already played from
    the position. This trade-off is shared with seeks.
  - Until the re-base arrives, the position briefly shows the drain
    callback's length (about one device period) and then snaps back to 0.
  - The "seek epoch left undrained by the previous run" branch in
    `start_run` has no test. In the harness, a seek issued while paused is
    not applied before the resume, so the state is hard to reach.
  - On a real backend, a callback still in flight when `pause()` returns
    could race the counter resets in `start_run`. This race existed before
    this change.

## Tests changed, and why

- **`engine_survives_a_second_run`:** adds `settle_drain()` before the pump
  (a no-op in round 0). Round 1 starts with round 0's audio in the ring.
  Without the settle, the 512 pumped frames are the drain's silence, and
  whether they count depends on whether the poll sees `StartDrained` before
  or after the pump. That is racy, and the honest value is 0. The assertion
  (`512`) is unchanged: after the start completes, the clock follows the
  device frame for frame.
- **`f2_invariants_hold_across_underruns_and_seeks`, step 5:**
  `buffered_frames() == 0` after `stop()` contradicts the A8 invariant
  whenever the decode thread had buffered anything. It only passed when the
  decode thread lost the race to `stop()`. It now asserts
  `ring_counters() == (0, 0)`, position 0, and a backlog bounded by the ring.
  The exact stale value is pinned by the stopped-run test.
- **Limiter on by default.** These were carried over from A9, unchanged.
  Tests that compare the raw pattern (which reaches 1.1, above the 0.966
  ceiling) disable the limiter to isolate their subject. The limiter tests
  were rewritten for the new default. `the_default_stage_is_enabled` replaces
  `the_default_stage_is_disabled` in `limiter.rs`.
- **`settle_drain` (harness):** it now waits for both the drain and the
  re-base. It pumps only while the start is pending, so every pumped frame
  falls before the re-base.
- **New tests:**
  - `a_restart_with_the_default_limiter_matches_a_fresh_run`
  - `a_restart_paused_before_its_drain_starts_clean`
  - `a_restart_after_a_natural_end_needs_no_drain`

## Open

- `crates/rustamp-dsp/src/lib.rs:9` still says the limiter is "disabled by
  default". That file is outside this lane's owned paths.
- The limiter's ceiling still holds only at its own output, before the
  resampler (A6 finding 3, unchanged).
