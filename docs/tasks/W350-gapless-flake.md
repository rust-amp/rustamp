# W350 - `resident_to_inprocess_source_change_is_gapless` flake

## What failed
`crates/rustamp-audio/tests/resident.rs`, `resident_to_inprocess_source_change_is_gapless`,
line 612 `assert!(h.wait_track(0))`, after the full 30 s `WAIT`. Reproduced under
CPU load (16 busy loops, 16 logical cores): 2 failures in 40 runs, both at the same
line; 0 failures in isolation.

## Cause (test harness, not the engine)
`Engine::current_index()` is set when the decode thread reports `TrackStarted`, not
when the device plays the track. The test does not pump the clock before the first
assertion, so the decode thread writes track 0 (3000 frames) and moves on to track 1.
If the test thread is delayed (load), its first `poll()` already sees index 1, and
`wait_track(0)` waits 30 s for a transient state that never returns. The engine
behaves as designed (`current` follows the decoder; the seam and timeline are
unaffected); the test asserted an intermediate state.

## Fix
The test no longer waits for `current_index() == 0`. It waits for the monotone
condition (`wait_buffered`: end of stream known, both tracks written) and asserts
from the event stream that `TrackChanged` fired for index 0 and then 1. All data
assertions (`left[2999]`, `left[3000]`, no silence, seam exactness) are unchanged.
The other multi-track test (`die7_4410` + `track2`) never lets the second track start
before the first dies, so it does not share the pattern.

## Evidence
100 consecutive runs of the 9 tests of `resident.rs` under the same 16-core load:
100 passed, 0 failed.
