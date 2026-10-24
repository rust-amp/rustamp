# A8 — A stopped run leaves its audio in the ring

**Found** 2026-10-17, by the orchestrator while repairing the limiter test the VM
suite had caught. **Not yet fixed**; this card is the record and the brief.

## The defect

`Engine::stop()` pauses the device, joins the decode thread and resets the ring
**counters** (`sync.set_written(0)`, `sync.set_consumed(0)`) — but it does not
invalidate the ring's **content**. `Engine::start_run()` resets the same
counters and starts a new decode thread, and it never bumps the ring epoch
either, so the render callback has no reason to drop what is already in the
ring.

Only one thing bumps the epoch today: the decode thread's seek path
(`decode.rs`, `request_epoch` after a seek). A stop/start therefore reuses the
ring as-is.

**Consequence:** after Stop and Play, the first ~370 ms the device plays is the
**tail of the previous run** (up to one ring depth), not the start of the new
one. Audible as a fragment of the old track at the head of every restart. The
ring counters do not show it: `written` was reset to 0, so `buffered_frames()`
starts at 0 and the backlog is invisible.

**Evidence.** The limiter test
`limiter_limits_hot_signal_to_ceiling_and_disabling_returns_to_bit_exact` did
`stop()` → enable → `play_index(0)` on one harness and measured a peak of
**2.2** — the full unlimited signal — although every sample the limiter emits is
clamped to the ceiling (0.966). The recorded frames were the previous run's,
produced before the limiter was enabled. The test now uses a fresh harness per
phase and the card records the hole instead of hiding it.

## Why it was not fixed on the spot

The obvious fix — bump the epoch in `start_run` so the consumer drains the
stale content — is **not** enough on its own. The drain in the render callback
drops *everything* in the ring, including frames the new decode thread has
already written, because the decode thread only waits for a drain when it is
following a seek (`Worker::drain_epoch` is set by `apply_seek`). The seek path
solves exactly this with its three-step handshake; a start needs the same
ordering:

1. `start_run` bumps the epoch and hands that epoch to the decode thread.
2. The decode thread waits for the drain acknowledgement **before its first
   write** (its `write_all` already has an `await_drain` step for this).
3. Only then does it write the first frame of the new run, so no new frame can
   be dropped by the drain.

That is a contained change — a field on `Config`, the worker's initial
`drain_epoch`, and the epoch bump in `start_run` — but it touches the ring
handshake, so it wants its own lane and its own tests rather than a hurried
edit.

## What the fix must prove

- After `stop()` → `play_index(0)`, the recording contains **no** frame of the
  previous run (compare against the same recording from a fresh engine: they
  must be bit-identical).
- Nothing new is lost: the first frame the new run writes still reaches the
  device (the seek handshake's guarantee, restated for a start).
- The seek path is unchanged (its tests still pass).
- A paused-then-resumed run is unaffected.
- `buffered_frames()` cannot report 0 while the ring holds stale frames from a
  previous run.

## Related

- `docs/tasks/A4-ring-drain.md` — the consumer-side `consumed` counter, which
  made `buffered_frames()` honest about the *current* run.
- `docs/tasks/A3-engine-lifecycle-review.md` — the lifecycle invariant audit
  that did not cover stop-then-start ring content.
- `docs/tasks/A6-dsp-adversarial-review.md` — the review that broke the ring
  counter ordering on a seek.
