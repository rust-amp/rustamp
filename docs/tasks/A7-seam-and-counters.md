# A7 — Resampled seams, ring counters and DSP latency (fixes for A6)

Lane: `rustamp-audio`. Owned paths: `crates/rustamp-audio/src/**`,
`crates/rustamp-audio/tests/**`, this card. `rustamp-dsp` was not edited (the
limiter already reports its latency through `DspStage::latency_frames`).
Base: `0213fad`. Sources: the A6 review, the A4/A5 cards, the code in this
crate, and the rubato 5.0.1 source (MIT OR Apache-2.0) from the local cargo
registry, read to check the FFT engine's block sizes and delay.

**Nothing was executed.** Under the VM rule no `cargo test` ran on this host;
`cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features
-D warnings` and `cargo build --workspace` pass. Every test below needs its
first run in the Hyper-V VM. Expected values in the tests are derived, not
measured.

## Findings, in severity order

### A6-1 (HIGH) — the resampled gapless seam. Confirmed in code; magnitude disputed.

Confirmed: `prepare_track` called `conv.prepare` (reset) for every track, and
the end of every track called `flush` (silence padding, then reset), so each
track was resampled on its own and the two halves were cut at the seam.

Disputed: the review estimated a dip to half the level. At the sample instants
the error is smaller. The band-limited step response of a truncated signal is
near full level at the last input sample (for an ideal kernel it is exactly
full level there). For 44.1 → 48 kHz the last output frame of the first track
lands 0.08 input frames after its last sample, about 6 % low. The first frame
of the second track is about 2.5 % low. The rest is the Gibbs ringing each half
owes the other, which is cut off. This is still a real, avoidable error at
every seam, but it is about -25 to -30 dB, not -6 dB. These figures are
analytic estimates.

Fix (`decode.rs` run loop, `convert.rs` `continues`/`stream_frames`): when the
previous track ran to its end and the next track has the same format, the
converter is neither flushed nor reset. The tracks are resampled as one stream,
so the seam is exactly the resampled joined signal. The converter is flushed at
the end of the run, at a seam between two formats, before a mid-stream format
change, and after an error ends the run. Because the converter still holds the
first track's tail when the second one opens, `TrackStarted::timeline_start` is
now the **predicted** seam, `stream_base + ceil(in_frames * up / down)`, not the
frames written so far. For the equal-rate path this is the same number as
before. The per-track `ceil` no longer adds a frame per seam.

Trade-offs:
- A seam between two different formats is still flushed and restarted. Two
  rates cannot share a resampler. Making that seam continuous would need a
  crossfade or priming with the previous tail, and that is not done.
- The DSP chain is still prepared at every track. The `DspStage` contract
  resets it at every track change, so the equalizer's filter history
  transient at a seam (A6-4, last paragraph) stays. It predates A6.
- The equal-rate path is untouched: `convert` adds one counter increment and
  allocates nothing. The engine's bit-transparency tests still apply.

Tests:
- `convert::tests::a_continued_stream_has_no_seam`: the continued stream is the
  joined signal bit for bit. A control asserts that the old flush-and-restart
  seam deviates by more than 5e-3, so the measurement can tell the two apart.
- `tests/engine.rs::a_resampled_gapless_seam_is_continuous`: two 44.1 kHz
  tracks on a 48 kHz `NullOutput` that together form one 1 kHz tone, with the
  seam on a crest. Asserts: exact length (48 000 frames), the seam at exactly
  24 000, both track lengths, every frame within 5e-3 of the ideal tone (edges
  of 10 ms skipped), the sample step within 2 % of the tone's own largest
  step, and the RMS of every period across the seam within 1 %.
  **What it proves:** same-format resampled seams carry no truncation error
  above 5e-3 (about -40 dB of the tone), and the timeline is exact.
  **What it cannot prove:** inaudibility on real material, behaviour at
  seams between formats (not smooth, see above), or real devices. The 5e-3
  tolerance is set by the converter's documented passband bound
  (±0.05 dB ≈ 3e-3 at this level), not by a measurement. The margin over the
  old seam (about 0.03) is an estimate.

### A6-2 (MEDIUM) — seek counter ordering. Confirmed, including the suspected second variant.

Confirmed: `apply_seek` called `request_epoch` before `set_written`. The second
variant also holds: `await_drain` returned at once while the output was
inactive, which is the case for a run's initial seek (`start_at`), since the
engine starts the output only after it spawns the thread. Post-seek frames were
then committed before `set_written` and dropped by the later drain.

Fix:
- `RingSync::begin_seek(base)` stores `written = base` and then bumps the
  epoch. `apply_seek` uses it. A callback that sees the new epoch also sees the
  new `written`.
- `await_drain` no longer gives up while the output is inactive. It waits,
  servicing control messages, and a Stop or Quit still ends it. A skip that
  arrives during the wait is acted on after the drain (`track_loop` now checks
  `pending_skip`). Effect: no post-seek frame is ever written before the drain,
  so the "first block dropped after a seek while paused" price is gone.
  Cost: a seek requested while the decode thread is idle-paused is reported to
  the engine only on resume. Before, that was already the case whenever the
  thread was waiting for ring space, which is the usual state.
- The decode thread publishes `written` per chunk **before** `commit_all`.
  `consumed` is stored with `Release` and loaded with `Acquire`, and
  `RingSync::counters()` loads `consumed` before `written`. Outside a seek a
  reader can then never observe `consumed > written`. Inside a seek, a
  callback of the old epoch can still briefly count stale frames after a
  backward seek. The drain closes that window, and the finish condition cannot
  fire inside it. This is documented in `ring.rs`.

Tests: `ring::tests::a_callback_between_the_seek_stores_cannot_misalign_consumed`
(the review's interleaving, single-threaded, backward and forward),
`consumed_resets_to_written_after_epoch_drain` (rewritten, see below),
`begin_seek_publishes_written_before_the_epoch`, and the engine tests
`seek_before_play_plays_the_target_audio` (audio and counters after an initial
seek) and the tightened `seek_while_paused_drains_consistently`.

### A6-4 (MEDIUM) — the limiter's latency. Confirmed.

Confirmed: nothing used `latency_frames()`. Every track lost its last `L`
frames and gained `L` zeros at its start, and the position was off by `L`.

Fix (`decode.rs` `emit`/`flush_dsp`): handled the same way as the converter's
latency. After every `prepare`/`reset` the first `sum(latency_frames())` frames
the chain emits are dropped. At the end of a track, before a format change and
after a decoder error, that many frames of silence are pushed through the
chain. It is generic, so any stage that reports latency is covered. With the
default chain (latency 0) nothing changes.

What remains: the limiter is still reset at every track (contract), so its
look-ahead never sees across a seam. The first track's tail is limited against
silence, not against the next track's start. A6-3 (the resampler rebuilding
inter-sample peaks above the ceiling) is **not** fixed. It is now documented in
the engine module docs. It needs either a true-peak limiter or a limiter after
the converter, which is an owner/DSP-lane decision.

Test: `the_enabled_limiter_keeps_a_gapless_join_exact_and_undelayed`. With the
limiter on and a signal below the ceiling, two gapless tracks come out bit for
bit with no delay, at exact length and position.

### Test audit (A6 claim 7)

- `ring::consumed_resets_to_written_after_epoch_drain`: rewritten. The seek
  base (3 and 5000) now differs from "read + dropped" (10), so an
  implementation that adds the dropped frames fails. Post-seek counting and the
  finish point are also checked.
- `engine::f2_invariants_hold_across_underruns_and_seeks` and
  `seek_while_paused_drains_consistently`: they now read the raw counters
  through the new `Engine::ring_counters()` instead of the saturating
  `buffered_frames()`. They assert `consumed <= written`, alignment to the seek
  base (`consumed >= base`), and, in the paused test, the exact audio and
  counter values after the seek. The dead `written` variable is gone.
- The limiter ceiling tests in `rustamp-dsp`
  (`a_hot_signal_is_held_at_the_ceiling`,
  `look_ahead_shapes_the_transient_instead_of_clipping_it`) were **not
  edited**, because this lane may not touch that crate. They are superseded by
  `tests/limiter_gain.rs::the_ceiling_comes_from_the_linked_gain_not_from_the_clamp`.
  That test drives the public `Limiter` with a hot left and a quiet right
  channel and requires `out/in` to be equal on both sides within 1e-6
  relative. A clamp that did more than absorb rounding would cut only the hot
  side and fail it. Recommendation for the DSP lane: relabel or delete the two
  clamp-level tests.
- `convert::output_is_time_aligned_with_the_exact_length` now covers 32 ↔ 44.1
  kHz at a fifth of a frame (it rejects the old 0.5 and 0.36 frame errors).
  The new `fft_blocks_are_even_on_both_sides` pins the cause.

### Smaller findings

| A6 # | Finding | Status |
|---|---|---|
| 5 | Tail lost on a format change | **Fixed.** `on_format_changed` flushes the DSP chain and the converter before it re-prepares. Test: `a_format_change_keeps_the_resampler_tail` (44.1 kHz part at full resampled length, 48 kHz part bit-exact after it). |
| 6 | Approximate delay trim | **Fixed for the FFT engine.** The block count is rounded up to an even number (`fft_chunk_hint`), so both FFT sizes are even and the trimmed `fft_size_out/2` is the filter's exact delay. For pairs that were already even (44.1 ↔ 48 kHz, 22.05 → 48 kHz and so on) the arguments to rubato are unchanged, so their output is unchanged. Pairs with one block (32 ↔ 44.1, 8 → 11.025 kHz) get a block twice as long, which doubles their look-ahead and gives a longer filter. **Not fixed for the sinc fallback**: rubato truncates its delay to whole frames (less than one frame, inaudible). This is documented in the `convert.rs` module docs. |
| 7 | Tail lost on a decoder error | **Fixed.** The DSP tail is pushed out and the converter is flushed by the run loop. Test: `a_decoder_error_keeps_the_audio_decoded_before_it`. |
| 8a | A trailing partial frame passes the limiter unlimited | **Not reachable, not fixed.** `decode.rs` only passes whole frames (`frames * channels`). The fix belongs in `limiter.rs` (DSP lane). |
| 8b | An unprepared limiter bypasses while enabled | **Not reachable, not fixed.** `decode.rs` prepares every stage before the first `process`. DSP lane. |
| 8c | NaN reaches the device | **Fixed.** Non-finite samples leaving the DSP chain become `0.0` before the vis tap, the converter and the ring. This is on the decode thread, not the render thread, and finite samples are untouched, so the bit-exact path is unchanged. Test: `a_non_finite_sample_never_reaches_the_device`. |
| 3 | Resampler rebuilds peaks above the limiter ceiling | **Not fixed** (design decision, see A6-4 above). Documented. |

## New observation (not in A6, not fixed)

`Engine::stop()` resets `written`/`consumed` but does not drain the ring. After
Stop in the middle of a track, up to a ring of the old run's audio stays in the
ring. The next run's first callbacks play it and count it as consumed. That
would play stale audio and could end a short next run early. This was found by
reading only. Fixing it means changing the start-of-run handshake (for example
an epoch drain before the first write). That was out of scope here and needs a
VM test first.

## Files

- `crates/rustamp-audio/src/convert.rs`: even FFT blocks, `continues`,
  `stream_frames`, docs, three tests.
- `crates/rustamp-audio/src/decode.rs`: carry across seams, DSP trim/flush,
  format-change and error flushes, NaN guard, `begin_seek`, drain waiting,
  publish-before-commit.
- `crates/rustamp-audio/src/ring.rs`: `begin_seek`, `counters`,
  Release/Acquire `consumed`, docs, three tests.
- `crates/rustamp-audio/src/engine.rs`: `ring_counters()`, consistent counter
  reads, limiter docs.
- `crates/rustamp-audio/tests/common/mod.rs`: scripted decoders (`tone`,
  `fmt`, `err`).
- `crates/rustamp-audio/tests/engine.rs`: two tightened tests, six new ones.
- `crates/rustamp-audio/tests/limiter_gain.rs`: new.
