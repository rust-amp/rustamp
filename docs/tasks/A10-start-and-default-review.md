# A10 — Adversarial review of A9 (start-of-run drain, limiter on by default)

**Date:** 2026-10-17
**Reviews:** commit `d4893c8` ("Invalidate the ring at the start of a run and
enable the limiter by default"), card [A9](A9-ring-start-and-limiter-default.md).
**Mode:** findings only. No code was changed. `cargo test` was **not** run (VM
rule); every verdict below comes from reading the code and tracing it by hand.
`cargo fmt --all -- --check` and
`cargo clippy --workspace --all-targets --all-features -- -D warnings` pass on
this tree.

Note: card A9 §header says it was checked only with build/clippy/fmt, so the
tests it added or changed have **never run**. Finding 2 shows that three of
them fail every time.

---

## 1. Claim table

| # | Claim | Verdict | Evidence | Smallest reproduction |
|---|---|---|---|---|
| C1 | After stop → play, the ring does not replay the previous run (the ring held stale frames) | **Holds** | `engine.rs:1008-1023` bumps the epoch when the parked producer has unread samples; `decode.rs:321` hands it to the worker; `decode.rs:677` awaits it before the first write; `ring.rs:287-308` drops the ring on the next callback. The worker cannot write before the ack (`decode.rs:828-843` has no timeout and ends only on drain, stop or quit) | — |
| C2 | No frame of the previous run reaches the speaker | **Fails (suspected, needs hardware)** | Finding 3: the engine never calls `OutputDevice::flush`; WASAPI keeps up to 200 ms of the old run in its endpoint buffer | §3 F3 |
| C3 | The start drain drops no frame of the new run | **Fails (proven)** | Finding 1: the start epoch depends on `stale_frames > 0` only (`engine.rs:1014`). A seek epoch from the previous run that was never acked survives `stop()`, and if the ring is empty, the first callback of the new run drains the new run's audio | §3 F1 |
| C4 | A start that races a render callback cannot lose the first frame or keep a stale one | **Holds** for ring content | A callback still serving the old epoch reads only stale frames, because the worker is parked in `await_drain`. The next callback sees the new epoch and drains. A residual counter race exists only when no epoch is bumped (F8, suspected, low) | — |
| C5 | Start while paused (`play_index` from Paused) | **Holds** | `play_index` → `stop()` (`engine.rs:682-684`) leaves the paused ring full → stale > 0 → epoch | — |
| C6 | Start after a seek (seek before play, ring holds stale frames) | **Holds** | The worker's start epoch E1 is overwritten by the seek epoch E2 (`decode.rs:791-792`). Any ack ≥ E2 also satisfies `drained(E1)` (`ring.rs:239-241`), so the engine bookkeeping (`engine.rs:849-854`) clears too. The orphan variant (empty ring) is C3 | — |
| C7 | Second run, and a run after the previous run ended naturally (audio content) | **Holds** | After a natural end, `finish_run` needs `consumed >= written` (`engine.rs:845`). The ring is empty, no epoch is pending, and the new run writes straight away | — |
| C8 | The position stays frame-accurate across a restart | **Fails (proven)** | Finding 2: the silent drain callback advances `played_frames` after `clock_origin` was set (`engine.rs:993`), and nothing re-bases the clock (contrast: the seek path re-bases at `engine.rs:899-901`) | §3 F2 |
| C9 | `buffered_frames()` never goes negative or underflows across a start | **Holds** | `saturating_sub` (`engine.rs:421`); `capacity - slots` cannot underflow | — |
| C10 | `buffered_frames()` and the counters stay consistent, with no stale backlog across a start | **Fails by design** | `engine.rs:407-419` reports the stale ring while stopped and while the start drain is pending, and `ring_counters()` reads `(0, 0)`. A8 asked for exactly this, but it contradicts "no stale backlog" and the `written - consumed` contract in the method's own docs (`engine.rs:394-401`). It makes an older test race (F5) | §3 F5 |
| C11 | The default-on limiter costs no tail and puts no silence into a same-format gapless seam | **Holds** (by construction; tested only in the linear regime) | `prepare_track` sets `dsp_trim = latency` (`decode.rs:490`); at track end `flush_dsp` pushes `latency` zeros through the chain (`decode.rs:642-654`); the stage is reset at the seam (`limiter.rs:371-383`). `the_default_limiter_loses_no_tail_and_adds_no_seam_silence` (`tests/engine.rs:1914-1949`) pins this only for a signal below the ceiling. A hot signal at the seam is not pinned | — |
| C12 | The tests that now disable the limiter hide no real regression | **Fails (proven)** | Finding 4: on the default path, one NaN or Inf now ducks both channels for about 10 ms and the audio takes about 0.23 s to recover. The A6 finding 8 test now disables the limiter (`tests/engine.rs:1776`), so the default path is no longer covered | §3 F4 |
| C13 | The tests changed for the default flip still pin what they claim | **Fails (proven)** | F2: three tests fail deterministically. F5: one older test now races. F7: several pins got weaker | §3 F2, F5, F7 |

**Counts:** 13 claims. 7 hold, 4 fail (proven), 1 fails by design (a
contract conflict), 1 fails (suspected, needs hardware).

---

## 2. Findings, in severity order

| Sev | ID | Status | Summary |
|---|---|---|---|
| High | F1 | proven | An orphaned seek epoch plus an empty ring: the new run's first ring-full (up to about 370 ms) is dropped, and the position is permanently offset |
| High | F2 | proven | Restart position is ahead of the audio by the drain callback; three A9 tests fail every run |
| Medium | F3 | suspected (hardware) | The device buffer is never flushed on stop; WASAPI replays up to 200 ms of the previous run |
| Medium-low | F4 | proven | The default limiter turns a single NaN/Inf into a dropout plus a slow recovery; the A6 finding 8 test now hides it |
| Low-medium | F5 | proven reachable, flaky | `buffered_frames()` ≠ `written - consumed` while stopped; `f2_invariants…` step 5 now races |
| Low | F6 | proven | Docs and comments made stale or broken by the diff |
| Low | F7 | proven | Weakened or redundant pins (default-path seek, restart and format-change alignment are unpinned) |
| Low | F8 | suspected | With no start epoch, an in-flight WASAPI callback can leave `consumed > written` for the new run |

---

## 3. Details and reproductions

### F1 — The start drain keys on ring content, not on a pending drain (High, proven)

`start_run` bumps an epoch only if `stale_frames > 0` (`engine.rs:1008-1023`).
`stop()` does not touch the epoch (`engine.rs:652-675`). A previous run's
**seek** epoch that was bumped (`decode.rs:791`) but never drained, because the
worker was stopped inside `await_drain` (`decode.rs:837-839`), therefore stays
pending (`drain_ack < epoch`). If the ring is empty at that moment, the next
run gets `drain_epoch = None`, the worker writes immediately, and the first
render callback sees `epoch != served_epoch` (`ring.rs:286-308`). That
callback drops everything the new run has written, aligns `consumed` to it and
returns silence. Up to one ring (16384 frames, about 370 ms) of the new run's
start is lost. `clock_origin` was set before the drain, so for the rest of the
run the position lags the audio by the dropped amount.

This breaks the "no new frame dropped" direction of the handshake, which is
the exact guarantee A8 asked for.

Smallest reproduction (deterministic on the null device; no callback runs
until the test pumps):

```rust
let mut h = Harness::new(&[LONG_A]);
h.engine.set_limiter_enabled(false);
h.engine.seek_frames(1500).unwrap();   // start_at
h.engine.play().unwrap();               // worker: TrackStarted, begin_seek -> epoch 1, await_drain
assert!(h.wait_track(0));               // no pump: epoch 1 is never acked
h.engine.stop().unwrap();               // worker leaves await_drain; ring is empty
assert!(h.engine.is_drain_pending());   // orphaned epoch 1
assert_eq!(h.engine.buffered_frames(), 0); // empty ring -> start_run will not bump
h.engine.play_index(0).unwrap();        // drain_epoch = None, worker writes at once
let p0 = h.wait_refilled(4096).unwrap();   // 0
let mut rec = Vec::new();
h.clock.pump_record(1024, &mut rec);
let left: Vec<f32> = rec.iter().step_by(2).copied().collect();
assert_eq!(left, expected(1, p0, 1024)); // FAILS: all zeros (drain callback),
                                         // the next audio starts at frame >= 4096
```

Real-world trigger: Stop or Pause within one device period (about 10 ms)
after a Play that carries a seek-before-play. A scripted IPC sequence can do
that. So can a seek during an underrun (empty ring) followed by Pause and Stop
before the next callback. This is rare by hand but reachable by legal calls.

Direction of a fix (not applied): bump the start epoch when
`stale_frames > 0 || sync.is_drain_pending()`, or always. Either way, F2 has
to be fixed first, because every bump currently costs a position offset.

### F2 — The silent drain callback is counted in the position; three A9 tests fail (High, proven)

`start_run` sets `clock_origin = played_frames()` (`engine.rs:993`) **before**
the drain. The drain callback plays silence (`ring.rs:307`), and the device
clock counts it (`null.rs:303-308`; WASAPI `wasapi_out.rs:727`). Unlike the
seek path, which re-bases at `Seeked` (`engine.rs:899-901`) after the drain,
the start path never re-bases. After every restart with a stale ring, the
position runs ahead of the audio by the length of the drain callback. That is
the whole free device buffer on WASAPI, which is sized up to 200 ms
(`wasapi_out.rs:59`, `:693-711`).

`Harness::settle_drain` (`tests/common/mod.rs:448-459`) pumps 64 frames, and
that callback *is* the drain. So `wait_refilled` returns position 64 while
the audio starts at frame 0:

| Test | Line | Asserted | Actual (by trace) |
|---|---|---|---|
| `stopped_run_leaves_no_stale_audio_in_ring_on_restart` | `tests/engine.rs:1888` | `p_restarted == p_fresh` (0) | 64 → **fails** |
| `default_engine_limiter_is_enabled_and_limits_overs` | `tests/engine.rs:1449` | left == `expected(1, 64, 1024)` | audio is `expected(1, 0, 1024)` → **fails** |
| `limiter_limits_hot_signal_to_ceiling_and_disabling_returns_to_bit_exact` | `tests/engine.rs:1503` | `p0_disabled == p0` (0) | 64 → **fails** |

Smallest reproduction:

```rust
let mut h = Harness::new(&[LONG_A]);
h.engine.play_index(0).unwrap();
assert!(h.wait_track(0));
h.wait_refilled(4096).unwrap();
h.engine.stop().unwrap();               // ring holds stale frames
h.engine.play_index(0).unwrap();        // start epoch bumped
assert!(h.settle_drain());              // one 64-frame callback = the drain (silent)
assert_eq!(h.engine.position_frames(), 0); // FAILS: 64; no audio frame has played yet
```

`engine_survives_a_second_run` (`tests/engine.rs:372-383`) still passes. Its
round 2 asserts position 512 after a pump whose single 512-frame callback was
the silent drain, so it now pins a position for audio that was never played.

### F3 — The device buffer survives stop; WASAPI replays the old run (Medium, suspected — needs hardware)

The trait says `pause` keeps the device buffer and that `flush` is "used on
seek / stop" (`rustamp-core/src/audio.rs:330-337`). The engine never calls
`flush`. No `.flush()` call on an output exists anywhere in `crates/`, and
`stop()` only calls `output.pause()` (`engine.rs:655`). WASAPI pause →
`stop_stream` (`wasapi_out.rs:612-616`) keeps the endpoint buffer, and
`reset_stream` runs only on the flush flag (`wasapi_out.rs:659-662`). The
render thread keeps that buffer topped up (`write_available` fills all free
space, up to the 200 ms buffer, `wasapi_out.rs:693-711`). So on Stop → Play,
up to about 200 ms of the **previous** run plays first. That is the A8
symptom at about half its size, below the ring. The ring fix cannot see it.

Reproduction (hardware only, not run): WASAPI default device with loopback
capture. Play a 1 kHz tone track for at least 1 s, `stop()`, then
`play_index` a silent track. The first ≤ 200 ms of the capture should contain
the tone. The same root cause makes a seek play up to one device buffer of
pre-seek audio while the position already shows the target (pre-existing,
same class).

### F4 — With the limiter on by default, one NaN costs a dropout; the A6 finding 8 test hides it (Medium-low, proven)

The non-finite guard runs **after** the DSP chain (`decode.rs:616-632`). The
limiter is the last stage in that chain, so it sees the NaN or Inf first. A
non-finite frame asks for gain 0 (`limiter.rs:328-340`). The hold keeps 0 for
`L + 1` frames, the attack ramps down over the preceding `L` frames, and the
release climbs back with a 50 ms one-pole (`limiter.rs:344-349`). The gain is
linked, so both channels duck.

At 44.1 kHz (`L = 221`), one bad sample therefore causes:
- about 5 ms of fade-out and about 5 ms at zero;
- about 0.23 s until the gain is back within 1 % (2205 · ln 100 frames);
- about 1 s until the snap restores bit-exact output (2205 · ln 1e9 frames).

With the limiter off (the old default), the same input zeroed one sample.
The limiter docs describe this as intended. It is still a change in the
**default** behaviour, and the only test for it,
`a_non_finite_sample_never_reaches_the_device`, now disables the limiter
(`tests/engine.rs:1776`). The default path has no NaN coverage at all.

Reproduction: delete line `tests/engine.rs:1776`. The per-frame assertion at
`:1789` (`*s == 0.0 || *s == want`) then fails at the first frame inside the
ramp. The poison repeats every 150 frames, so the release never recovers and
the whole track comes out attenuated.

### F5 — `buffered_frames()` no longer equals `written - consumed` while stopped; an older test races (Low-medium)

`engine.rs:407-413` returns the parked producer's unread count whenever it is
non-zero, while `ring_counters()` reads `(0, 0)` after `stop()`
(`engine.rs:664-665`). `f2_invariants_hold_across_underruns_and_seeks` ends
with `stop(); assert_eq!(buffered_frames(), 0)` (`tests/engine.rs:1383-1384`).

Between the seek settling and `stop()`, the worker is released by the engine
ack (`decode.rs:801-803`) and can commit its first post-seek block within
microseconds. If it does, the ring is non-empty at `stop()` and the assertion
fails. If it does not, the assertion passes. So the test now **races**. It was
deterministic before A9, which leaves the backlog at 0 after stop.

The deterministic form of the inconsistency: play `LONG_A`,
`wait_refilled(4096)`, `stop()`. Then `buffered_frames() > 0` while
`ring_counters() == (0, 0)` and the position is 0.

There is a related hazard. During a pending start drain, `wait_refilled`
(`tests/common/mod.rs:464-470`) is satisfied at once by the phantom
`start_stale_frames` (`engine.rs:414-419`). Any test that calls it after a
restart without `settle_drain` gets a position from before the audio exists.

### F6 — Docs broken or made stale by the diff (Low, proven)

- `engine.rs:64-67`: the sentence "…before the format converter and" is cut
  off. "the ring buffer." was lost when the next paragraph was inserted.
- `decode.rs:26-28`: still says "with the default chain (equalizer, limiter
  disabled) the latency is 0 and nothing is trimmed or pushed". The default
  latency is now `L`.
- `ring.rs:124-125` and `ring.rs:15-17`: say only the decode thread bumps the
  epoch, on a seek. `start_run` bumps it now too.
- `engine.rs:1089-1090` (`join_decode`): says "a thread waiting for a ring
  drain must not block the join". `await_drain` ignores `is_active`
  (`decode.rs:828-843`), so a worker parked in the new start drain blocks a
  join that is not preceded by `Stop`. Today only `start_run`'s own
  `join_decode` (`engine.rs:990`) can hit this, after an earlier `start_run`
  failed past `spawn` (`engine.rs:1068`/`1071` `?`). That gap existed before
  A9, but A9 adds a new place where the worker parks.
- Card A9 line 26 ("seek … completely unaffected") and line 56 ("seamless
  reuse on stop/start") state results of tests that never ran and, per F2,
  fail.

### F7 — Weakened or redundant pins (Low, proven)

- Default-path alignment is no longer pinned anywhere. Every bit-exact seek,
  seek-before-play, seek-while-paused, format-change and restart test now runs
  with the limiter off: `tests/engine.rs:260, 950, 1721, 1801, 1844/1855`.
  The limiter's `dsp_trim` after reset or seek (`decode.rs:782`) is correct by
  construction, but a regression that shifts default-path audio by `L` frames
  would pass the whole suite. `limiter_toggle_latches_on_seek` checks only the
  peak and the position (`:1554-1558`). Most of these could use the 0.5× stage
  instead of disabling.
- `the_default_limiter_loses_no_tail_and_adds_no_seam_silence`
  (`:1914-1949`) is `the_enabled_limiter_keeps_a_gapless_join_exact_and_undelayed`
  (`:1686-1711`) minus one line. It adds the default assertion, but it tests a
  linear (sub-ceiling) signal only. Card A9's "the default-on path is checked
  for a lost tail" holds only in that regime.
- `default_engine_limiter_is_enabled_and_limits_overs` phase 1
  (`:1432-1436`) and `limiter.rs:462-471` (`the_default_stage_is_enabled`)
  assert only `peak <= ceiling`, which a muted output also passes. The old
  test asserted bit-exactness. The hot-signal test keeps a lower bound
  (`:1488-1491`); these two do not.
- `limiter_limits_hot_signal…` lost its "fresh harness, enable before the run"
  pin (A8 evidence) and its old `p0_enabled == p0` alignment check. Phase 3
  (`:1520-1523`) checks only a flag and no audio.
- `gapless_join_has_no_silence_and_no_dropped_frames`, the EQ helper and the
  format-change test disable the limiter for a good reason (the pattern peaks
  at 1.1). They no longer cover the shipped configuration.

### F8 — Counter race with an asynchronous pause when no epoch is bumped (Low, suspected)

WASAPI `pause()` only sets a flag (`wasapi_out.rs:396-399`), so a callback can
still be running while `stop()` and `start_run` run. When `stale_frames == 0`,
no epoch realigns `consumed`. A callback that loaded `consumed` before
`start_run`'s `set_consumed(0)` (`engine.rs:1007`) and stores it after that
point leaves `consumed > written` for the new run. A short track could then
satisfy `consumed >= written` early (`engine.rs:845`). The window is
microseconds wide inside a stop that joins a thread, so this is very unlikely.
Bumping the epoch on every start (see F1) would close it.

---

## 4. Not verifiable without hardware (or without running tests)

- F3, the audible replay of the WASAPI endpoint buffer after Stop → Play,
  and its exact length (it depends on how full the buffer is at pause).
- The size of F2's position offset on a real device (one `write_available`
  of free space after resume).
- F8, whether a WASAPI callback can actually straddle `stop()` and
  `start_run()`.
- Whether the VM suite really fails the three F2 tests and races F5. That
  conclusion is a hand trace of `NullClock::drive` (`null.rs:281-314`, chunk
  1024, `pump(64)` = one callback) and has not been executed.
- Audibility of F4's dropout on real material. The NaN sources in practice
  (decoders, the plug-in bridge) were not surveyed.
- Product note, not a defect: no crate outside `rustamp-audio` calls
  `set_limiter_enabled`, so the shipped app has no way back to bit-transparent
  playback.
