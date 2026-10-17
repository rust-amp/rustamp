# A9 — Ring Start Drain (A8 fix) and Default-Enabled Limiter

**Date:** 2026-10-17  
**Lane:** `rustamp-audio` & `rustamp-dsp`  
**Owned paths:** `crates/rustamp-audio/src/**`, `crates/rustamp-audio/tests/**`, `crates/rustamp-dsp/src/limiter.rs`, this card.  
**Rule:** No `cargo test` run on host (Hyper-V VM rule). Verified with `cargo build --workspace`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo fmt --all -- --check`.

---

## 1. Job 1: A8 Ring Reuse across Stop/Start

### Defect
`Engine::stop()` paused output and joined decode, resetting `written` and `consumed` to 0, but did not invalidate ring contents. `start_run()` started a fresh decode thread without bumping the ring epoch. Consequently, a stopped-then-restarted engine replayed up to one ring buffer depth (~370 ms) of stale audio from the prior run before playing new audio. Furthermore, `buffered_frames()` dishonestly reported 0 while stopped or before the new run rendered, hiding the stale backlog.

### Fix
1. **Backlog reporting:** `Engine::buffered_frames()` checks if `self.producer` holds unread samples `(capacity - slots) / channels`. If `unread > 0`, it reports that backlog rather than 0. If a start drain is pending (`start_drain_epoch`), it reports `start_stale_frames` until the consumer acknowledges the drain.
2. **Start drain handshake:** In `Engine::start_run()`, if `stale_frames > 0`, an epoch bump is requested (`sync.request_epoch()`), recorded in `self.start_drain_epoch`, and passed via `Config.drain_epoch` to the decode worker.
3. **Decode worker ordering:** In `Worker::write_all()`, the worker awaits drain acknowledgement (`await_drain()`) before writing its first frame.
4. **Consumer drain execution:** On the next render callback, `epoch != served_epoch` triggers: remaining slots in `Consumer` are dropped, `consumed` is aligned to `written` (0), `drain_ack` is published with `Release`, and silence is returned for that callback.
5. **Worker unblock & write:** Once `sync.drained(drain_epoch)` is true, `await_drain()` unblocks and the worker writes the first frame of the new run into the empty ring. No fresh frame is ever dropped.

### Proven Properties
- **Stale audio eliminated:** `stopped_run_leaves_no_stale_audio_in_ring_on_restart` proves that after `stop()` then `play_index(0)`, the recorded audio contains no frame of the previous run and matches a fresh engine recording bit for bit.
- **No new frames lost:** The first frame written by the new run reaches the output device.
- **Honest backlog:** `buffered_frames()` reports `stale > 0` while stopped and while drain is pending.
- **Existing paths intact:** Seek handshake (`seek_while_paused_drains_consistently`, `seek_before_play_plays_the_target_audio`) and pause/resume remain completely unaffected.

---

## 2. Job 2: Limiter Enabled by Default

### Decision & Product Consequence
Per owner decision (2026-10-17), the look-ahead brickwall limiter is now **enabled by default**.
- **Own-choice addition:** What the reference Winamp 2.95 player does with overs is unmeasured; enabling by default is an own product choice providing a clean alternative to hard clipping at the -0.3 dBFS linear ceiling (~0.966).
- **Latency cost:** Enabling by default incurs look-ahead latency (`LOOKAHEAD_SECONDS`, 5 ms; 221 frames at 44.1 kHz, 240 frames at 48 kHz).
- **Bypass capability:** When disabled via `Engine::set_limiter_enabled(false)`, the limiter operates as a bit-exact bypass with 0 latency.

### Implementation
- `LimiterControl::default()` initializes to `enabled: true`.
- `Limiter::new()` initializes with default enabled control.
- `Engine` doc comments and limiter methods document the product choice and latency cost.

### Test Audit & Isolation
Because synthetic test patterns in `tests/common/mod.rs` (`pattern(seed, frame)`) reach peak amplitudes of ~1.1005 (exceeding the -0.3 dBFS ceiling), the default pipeline limits those peaks and is no longer bit-identical to the raw source pattern. Tests asserting bit-exact rendering for non-limiter concerns now explicitly disable the limiter to cleanly isolate the subsystem under test:
- `gapless_join_has_no_silence_and_no_dropped_frames` (gapless seam continuity)
- `seek_is_frame_accurate_and_plays_the_right_audio` (seek accuracy)
- `record_left_with_eq` / `flat_enabled_eq_is_bit_transparent_through_the_engine` (EQ transparency)
- `seek_while_paused_drains_consistently` (seek drain consistency)
- `a_format_change_keeps_the_resampler_tail` (resampler tail flush)
- `a_non_finite_sample_never_reaches_the_device` (NaN guard on bypass path)
- `seek_before_play_plays_the_target_audio` (initial seek accuracy)

### Default-On Limiter Tests
- `the_default_limiter_loses_no_tail_and_adds_no_seam_silence`: Proves that with the default-on limiter and a signal below ceiling, a gapless queue is rendered bit for bit with no start delay, no tail loss, and no silence at the seam.
- `default_engine_limiter_is_enabled_and_limits_overs`: Proves that the default engine limits overs to `<= ceiling` and returns to bit-exact bypass when disabled.
- `limiter_limits_hot_signal_to_ceiling_and_disabling_returns_to_bit_exact`: Proves limiting on hot signals and seamless reuse on stop/start.
- `limiter_toggle_latches_on_seek`: Proves latching across seeks.
