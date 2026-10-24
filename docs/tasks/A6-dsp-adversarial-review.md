# A6 — Adversarial review of the resampler, limiter and ring-drain change

Scope: `6157c3d` (band-limited resampler, limiter) and `fb7a68d` (tail flush in the decode
worker), read against the ring-drain invariants (F2, `A4-ring-drain.md`). Base of this review:
`a31448f`. Findings only. No code was changed and nothing was executed. Under the VM rule,
no `cargo test` ran on this host. Every "proven" below comes from reading the code, a
counting argument or an analytic signal argument. None of them comes from a test run. Each
one gives the smallest reproduction that would confirm it in the VM.

rubato 5.0.1 source (MIT OR Apache-2.0, freely licensed) was read from the local cargo
registry to check the claims about allocation, delay and padding. Paths are cited as
`rubato/src/...`.

## Claim table

| # | Claim | Verdict | Evidence | Smallest reproduction of the failure |
|---|-------|---------|----------|--------------------------------------|
| 1 | The equal-rate path is bit-exact | **Holds** | `convert.rs:365-370` (copy via `remix`, `in_ch == out_ch` → `copy_from_slice`, `convert.rs:489-491`); `flush` returns 0 for equal rates (`convert.rs:401-404`); `same_rate` is decided after the `max(1)` clamp (`convert.rs:255-259`); a format change re-runs `prepare` (`decode.rs:710`), and with equal rates that again selects the copy path. Pinned by `equal_rate_path_is_bit_exact_and_holds_nothing_back` (bit compare, blocks 1/7/1024/4096, including flush) and the engine bit-transparency tests (`engine.rs:170`, `:250`, `:677`). | No failure found. Mono→stereo duplicates exactly and dropping channels is exact. Stereo→mono averages, so it is not bit-exact, but the docs never claim it is. Gap: no test changes the format mid-stream at equal rates. |
| 2 | `convert` allocates nothing and takes no lock | **Holds** (with caveats) | The FIFO and the block are allocated in `prepare` (`convert.rs:287-288`); `InterleavedSlice::new/new_mut` only wraps a slice (`audioadapter-buffers/src/direct.rs:1097,1113`); `Indexing::new()` holds `None` for the mask (no `Vec`); `Fft::process_into_buffer` (`rubato/src/synchro.rs:497-658`) and `Async::process_into_buffer` (`rubato/src/asynchro.rs:640-732`) only copy into pre-sized scratch and call `process_with_scratch`; `trace!` and `debug!` compile to nothing because rubato's `log` feature is off (`rubato/src/lib.rs:14-24`; `cargo tree -e features -i rubato` shows only `default`/`fft_resampler`). | No allocation or lock is reachable. Caveats: (a) panic paths `.unwrap()` at `rubato/src/synchro.rs:158,185` if an FFT length invariant ever breaks; (b) if any crate in the graph ever enables `rubato/log` (feature unification), `trace!("Start processing, {:?}", self)` formats the whole resampler on every call (`synchro.rs:523`, `asynchro.rs:658`); (c) all of this runs on the decode thread, **not** the render thread, so the claim matters less than it sounds. |
| 3 | Fixed chunking makes the output independent of how the input is split | **Holds** within one stream | The resampler is called only when the FIFO holds exactly `chunk` frames (`convert.rs:385-388`); `chunk = input_frames_next()` stays constant for `FixedSync::Input`/`FixedAsync::Input` (`rubato/src/synchro.rs:414-418`, `670-672`); flush calls with `partial_len(fifo_frames)`, which is `in_total mod chunk` and so independent of the split, and rubato zero-fills the rest (`synchro.rs:542-549`). Pinned by `phase_carries_across_blocks` (bit compare, blocks 1/7/255/256/257/1000, three pairs). | Within one stream: no failure. A seek with a partly filled FIFO, a flush with a partly filled FIFO and a chunk boundary exactly at the end all depend only on `in_total mod chunk`. **Across a reset** (seek, track boundary, format change) the output is deliberately not continuous; that is what makes finding 1 possible. |
| 4 | The group delay is trimmed exactly, `latency_frames()` is the only latency, and the decode worker flushes into a buffer that is never too short | **Partly fails** | Trim: `convert.rs:343,467-469`. FFT delay `fft_size_out/2` (`rubato/src/synchro.rs:692-694`), filter centred at `floor(fft_size_in/2)` input frames (`rubato/src/sinc.rs:33`, `synchro.rs:108`). Sinc delay is truncated with `as usize` (`rubato/src/asynchro.rs:748`). Flush is called at `decode.rs:481-490` → `decode.rs:523-535`. Buffer sizing is at `decode.rs:438-442` and `:717-721`. | (a) Trimming is **not exact** when `fft_size_out` is odd. 32 kHz → 44.1 kHz gives `fft_in=320, fft_out=441`: the true delay is 160·441/320 = 220.5 output frames but 220 is trimmed, so the output lags by half a frame. 44.1 → 32 kHz gives `fft_in=441`: the true delay is 220·320/441 ≈ 159.64 against 160 trimmed, so the output leads by 0.36 frame. Sinc pairs truncate by less than 1 frame. Reproduction: `output_is_time_aligned_with_the_exact_length` with `(32_000, 44_100)` fails its quarter-frame phase bound. (b) The tail is **lost** on a mid-stream format change (`decode.rs:708-711` calls `prepare` without `flush`) and on a decoder error (`decode.rs:497-500`). (c) Buffer sizing **cannot be short**, see the note under the table. |
| 5 | The limiter cannot overshoot by construction; it is bit-exact when disabled, and bit-exact but delayed when enabled on a quiet signal | **Partly fails** | Proof sketch in `limiter.rs:44-47` checked: the output frame `i` is emitted at push time `i+L`; the attack average covers `e(i..i+L)`; every `h(u)` for `u∈[i,i+L]` is a minimum over a window `[u-L,u]` that contains `i` (eviction `limiter.rs:243`, wedge capacity `window+1` suffices); `e ≤ h` (`limiter.rs:335-340`). Guard clamp `limiter.rs:280-286`. Bypass `limiter.rs:309-312`. | At the limiter's **own output** the claim holds, including the first `L` frames after a reset (the delay line is zero) and NaN/inf (required gain 0, NaN·g → NaN → `guard` → 0). **At the ring it fails**: the band-limited resampler runs after the limiter and rebuilds inter-sample peaks (finding 3). A ceiling of exactly 0 cannot be reached (`ceiling` is a private constant). Bit-exact when disabled: holds, also for a partial block. Delayed bit-exact when quiet: holds after `reset`, and for partial blocks (tested with blocks 1/7/100/4096/333). Untested and unsafe edges: a trailing partial frame (`chunks_exact_mut` leaves the remainder neither delayed nor limited, `limiter.rs:314`); an unprepared stage has `channels == 0` and bypasses even when enabled (`limiter.rs:309`). Neither is reachable from `decode.rs` today. |
| 6 | Ring drain: `consumed <= written` per epoch, `buffered_frames() = written - consumed`, a run neither finishes early nor fails to finish, a seek drain aligns the counter | **Fails** under a legal interleaving | `decode.rs:649` (`request_epoch`) comes **before** `decode.rs:654` (`set_written(new)`); the render drain reads `written` once (`ring.rs:257-258`); finish `engine.rs:731-736`; `buffered_frames` saturates (`engine.rs:357-358`). | Ring unit test, single-threaded: `set_written(10); push 10; let e = request_epoch(); render(4); set_written(3);` → `consumed()==10 > written()==3`. Then `push 5; set_written(8)` → `buffered_frames()` would report 0 while 5 frames are in the ring, and `consumed >= written` holds, so the run **finishes early**. A forward seek (`set_written(1000)` instead of 3) leaves `consumed` 990 behind for good, so the run **never finishes**. See finding 2. |
| 7 | The tests actually pin what they claim | **Fails** for several tests | See the list under "Test audit". | — |

Verdict counts: **3 hold** (1, 2, 3), **2 partly fail** (4, 5), **2 fail** (6, 7).

Why the flush buffer cannot be short (claim 4c). For the FFT engine, with `FixedSync::Input`
and `chunk = 256`, the output still due at flush time is at most
`ceil(fifo·up/down) + saved·up/down + delay`. That is at most
`(chunk−1 + fft_in−1)·up/down + fft_out/2 + 1`, which is less than `latency_frames()·up/down`.
The buffer holds `ceil(2L/chunk)·block_max ≥ 2L·up/down` frames, so the margin is at least
twice the need. Example, 44.1 → 48 kHz: `chunk=256`, `fft_in=294`, `fft_out=320`,
`delay=160`, `L=697`. The worst-case need is about 757 frames, and both the buffer and the
`max_pumps` bound of 9 cover it. The sinc engine follows the same argument with
`block_max = output_frames_max()`. No reproduction exists.

## Findings (by severity)

### 1. HIGH — proven (analytically) — every gapless seam clicks when the device rate differs from the file rate

Every track gets a fresh resampler:
- `prepare_track` calls `conv.prepare`, which resets (`decode.rs:431`, `convert.rs:292`).
- The previous track's tail was produced by `flush`, which pads with **silence** and then
  resets (`convert.rs:400-424`).
- rubato's `reset` zeroes all history (`rubato/src/synchro.rs:700-713`).
- The start-up delay is trimmed after the reset (`convert.rs:343`).

So at the seam, the output is the band-limited version of "A followed by zeros" placed next to
the band-limited version of "zeros followed by B". Resampling is linear, so the sum of those two
equals the band-limited version of the joined signal "A then B" only if each half also kept its
ringing into the other's region. Both halves are cut off instead: A at `ceil(N·up/down)`, and B's
lead-in by the trim. The result:
- For a signal that is continuous across the seam (a live album, a DJ mix), the output at the
  seam drops to about half amplitude. A constant level `c` comes out as about `c/2` on both sides
  of the seam, rising back over about half the filter length (≈147 input frames ≈ 3.3 ms at
  44.1 kHz). That is an audible click or notch.
- The per-track `ceil` also adds up to one extra frame per seam.

This hits the most common Windows setup: 44.1 kHz files on a 48 kHz shared-mode device.
`gapless_join_has_no_silence_and_no_dropped_frames` runs at equal rates (`tests/common/mod.rs:22`,
`SPEC = 44100/2` for both source and device), so it cannot catch this.
`silence_stays_exact_and_dc_passes_at_unity` checks only the settled middle
(`convert.rs:787`), which skips exactly the edge roll-off.

Smallest reproduction (converter level): `a = resample(44_100, 48_000, &vec![0.5; 4_410], 1024)`,
`b` the same, and `ab = resample(44_100, 48_000, &vec![0.5; 8_820], 1024)`. Then
`a.len() + b.len() == ab.len()`, but `a.last()` and `b[0]` come out at about 0.25 where `ab`
has about 0.5 at indices 4799 and 4800. At engine level: queue two 44.1 kHz DC tracks on a
48 kHz `NullOutput` and look at the recorded seam.

Before this change, linear interpolation made the same seam error over at most one frame, so
this is a regression in audible terms.

### 2. MEDIUM — proven (by interleaving) — the seek drain can misalign `consumed`, so a run finishes early or never

`apply_seek` publishes the new epoch (`decode.rs:649`) before it publishes the new `written`
(`decode.rs:654`). The render callback sets `consumed = written` as soon as it sees the new
epoch (`ring.rs:240-259`). If a callback runs between those two stores, `consumed` keeps the
pre-seek `written`:
- **Backward seek:** `consumed > written`. `buffered_frames()` saturates to 0 while audio is
  still in the ring (`engine.rs:358`). The finish condition `consumed >= written`
  (`engine.rs:734`) becomes true at end of stream with up to one ring of audio unplayed, so the
  run finishes early.
- **Forward seek:** `consumed` stays behind by the size of the jump for the rest of the run. The
  run never finishes: the engine stays `Playing` on silence, `PlaylistEnded` never fires, and
  `pump_until_drained` spins.

The window is short: two stores with some float maths in between. Preemption of the decode
thread widens it. The initial seek (`start_at`, `decode.rs:383-386`) takes the same path.

A second, **suspected** variant: during a seek while paused, the decode thread may write
post-seek frames before the drain (`await_drain` returns when inactive, `decode.rs:697-698`). It
commits to the ring (`decode.rs:577`) **before** it calls `set_written` (`decode.rs:513`). A
drain landing between the two drops frames that are not yet in `written`, so `consumed` stays
behind for good. This needs the ring to have free space while paused.

Reproduction: see claim 6 in the table. It runs single-threaded, in the style of
`ring.rs::consumed_resets_to_written_after_epoch_drain`.

Fix direction (not applied): publish `written` before `request_epoch`, and make the drain read
`written` and the ring under one ordering. One option is to carry the post-seek base with the
epoch.

### 3. MEDIUM — proven (analytically) — with the limiter enabled, the ring can still hold samples above 0 dBFS

The limiter guarantees a **sample**-peak ceiling (-0.3 dBFS ≈ 0.9661) at its own output. The
resampler after it reconstructs the waveform between samples. Example: at 44.1 kHz, take
`x[n] = 1.366·sin(π n/2 + π/4)`. Its samples are exactly ±0.966, so the limiter passes them
untouched, bit for bit. At 48 kHz the 11.025 kHz tone (inside the passband) is sampled near its
true peak, which gives output samples of about 1.366 (+2.7 dBFS).

The module doc says the opposite at `limiter.rs:77-78`: "After it come only the resampler and
the volume/balance gains, which never raise the level above unity". The -0.3 dB margin
(`limiter.rs:61-64`) covers 0.3 dB, not the +3 dB of this simple case, and pathological
sequences go higher.

Reproduction: feed that sequence through `Limiter` (enabled, 44.1 kHz) and then
`FormatConverter` 44.1 → 48 kHz, and assert the peak is at most the ceiling. Expected: about
1.366.

Not wired yet (`wave39-limiter-wiring`), so this is latent. It matters for the claim "cannot
overshoot".

### 4. MEDIUM — proven (by code), latent until the limiter is wired — the limiter loses 5 ms per track and breaks gapless

`DspStage` has no flush, and every stage is re-prepared at every track start
(`decode.rs:428-430`). `Limiter::prepare` → `reset` zeroes the delay line
(`limiter.rs:299,362`). With the limiter enabled:
- At each gapless seam, the last `L` frames (240 at 48 kHz) of track A are dropped and `L`
  zeros are put in front of track B.
- At the end of the queue, the last `L` frames never come out.

Nothing consumes `DspStage::latency_frames()` (no caller in `crates/rustamp-audio/src`), so
the reported position is also off by `L`. `a_hot_signal_is_held_at_the_ceiling` works around
this with an appended silent tail (`limiter.rs:485-486`).

The same per-track `prepare` also clears the equalizer's biquad history (`eq.rs:416,425`), a
smaller transient at gapless seams with a non-flat EQ. That behaviour predates this change.

### 5. LOW-MEDIUM — proven (by code) — a mid-stream format change drops the resampler tail

`on_format_changed` calls `conv.prepare` without `flush` (`decode.rs:708-711`). The FIFO, the
FFT engine's saved frames and the delay are thrown away, which is up to `latency_frames()`
input frames (≈697 frames ≈ 16 ms at 44.1 → 48 kHz), and a click follows. Once the limiter is
wired, its delay line goes too.

Reproduction: a mock decoder that returns `Error::FormatChanged` after N frames, on a device at
a different rate. The frames reported by `TrackEnded` come out short by about the latency.

### 6. LOW — proven — group-delay trimming is only approximate for some rate pairs

See claim 4a: 32 → 44.1 kHz and 8 → 11.025 kHz lag by 0.5 frame; 44.1 → 32 kHz leads by 0.36
frame; sinc pairs are off by less than 1 frame because of truncation
(`rubato/src/asynchro.rs:748`). The error is inaudible, but it contradicts "no group delay" and
"exact" in `convert.rs:51-56`.

### 7. LOW — proven — the tail is not flushed on a decoder error

`decode.rs:497-500` returns `Ended` without calling `flush_tail`, so the last ≤16 ms before
the error are lost when resampling.

### 8. LOW — proven — edge paths of the limiter and of NaN handling

- A trailing partial frame passes the enabled limiter unlimited and undelayed
  (`limiter.rs:314`).
- An unprepared limiter bypasses even when enabled (`limiter.rs:309`).
- With the limiter disabled (the default), a NaN from a decoder or the EQ reaches the device on
  the equal-rate path. On the resampled path it spreads over a whole FFT block and the next
  overlap.

None of these is reachable from today's decode loop except the NaN case.

## Test audit (claim 7)

Tests that can pass while the property they name is broken:

- **`ring.rs::consumed_resets_to_written_after_epoch_drain`** (`ring.rs:391-419`): **vacuous.**
  `written` is 10, and 6 consumed plus 4 drained also make 10. An implementation that adds the
  drained frames instead of aligning to `written` passes. The test's own comment admits it
  (`ring.rs:412-413`). It should set `written` to a different value before the drain (for
  example 3 or 5000), which would also have exposed finding 2.
- **`engine.rs::f2_invariants_hold_across_underruns_and_seeks`** (`engine.rs:1303-1351`): the
  "must not underflow" check (`buffered_frames() == 0`) cannot fail, because
  `buffered_frames()` uses `saturating_sub` (`engine.rs:358`). `consumed > written` passes
  silently. Step 4's bound `<= 16384` is also met by 0.
- **`engine.rs::seek_while_paused_drains_consistently`** (`engine.rs:936-990`): the bound
  `buffered <= track_len` is loose and is met by 0. The `written` variable is computed and
  thrown away (`engine.rs:971-972`). This test proves no alignment.
- **`limiter.rs::a_hot_signal_is_held_at_the_ceiling`** (`limiter.rs:474-495`): it pins the
  **clamp**, not the construction. Every sample with gain ≠ 1 goes through
  `guard(..).clamp(±ceiling)` (`limiter.rs:350`), so a broken hold window (`L` instead of
  `L+1`) or a misaligned attack average still gives `peak <= ceiling`. `peak >= 0.999·ceiling`
  is also met by a plain clipper.
- **`limiter.rs::look_ahead_shapes_the_transient_instead_of_clipping_it`**: it catches gross
  clipping through the gain-step bound. An off-by-one that overshoots by a few ULP to a few
  percent on single frames is clamped and changes the measured gain by less than the step
  bound, so it passes. A test that counts how many samples `guard` actually changed by more
  than 1 ULP would pin "the clamp only absorbs rounding".
- **`convert.rs::output_is_time_aligned_with_the_exact_length`** and
  **`upsample_2x_reproduces_the_band_limited_waveform`**: they check alignment only for
  44.1 → 48 and 4 → 8 kHz, where the delay is an integer. The pairs that fail (finding 6) are
  not tested. The sinc test (`convert.rs:829-843`) checks no phase at all.
- **`convert.rs::silence_stays_exact_and_dc_passes_at_unity`**: the DC check uses only the
  settled middle half (`convert.rs:787`), which hides the edge roll-off behind finding 1.
- **The decode-worker flush (`fb7a68d`) has no engine-level test.** Every engine test runs with
  source rate = device rate (`tests/common/mod.rs:22`), so `flush_tail`, the out-buffer sizing
  in `prepare_track`/`on_format_changed` and the resampled gapless seam never run through the
  engine. If `flush_tail` were deleted, no engine test would fail.
- No test changes the format mid-stream, neither in `convert.rs` nor at engine level.

Tests that do pin their claim:
- `equal_rate_path_is_bit_exact_and_holds_nothing_back`
- `phase_carries_across_blocks`
- `reset_clears_the_history` (in both modules)
- `stereo_resampling_keeps_channels_apart_and_averages_to_mono`
- `disabled_is_a_bit_exact_bypass`
- `a_quiet_signal_passes_bit_exact_when_enabled` (partial blocks included)
- `switching_is_latched_until_reset`
- `consumed_counts_ring_frames_not_silence`
- `normal_run_finishes_when_ring_is_drained`
- `second_run_resets_consumed_and_drains_correctly`
- The quality tests at ±0.05 dB / −70 dB pin their own bounds.

## Not verifiable without hardware or execution

- None of the reproductions above was executed (no `cargo test` on this host, by rule). The
  half-amplitude size of the seam dip in finding 1 is an analytic estimate from the filter's
  step response. The exact figure needs the VM.
- How likely the race in finding 2 is under a real WASAPI or cpal callback period and real
  thread scheduling.
- Whether `output.pause()` (`engine.rs:545`, `:829`) is synchronous on WASAPI and cpal. If a
  render callback is still in flight when `stop()` or `start_run()` resets
  `consumed`/`written` to 0 (`engine.rs:554-555`, `:884-888`), its late `set_consumed(old+n)`
  would give the next run a stale, large `consumed`, and that run would finish early. This was
  not observable headless.
- Whether the WASAPI shared-mode mixer clips or soft-limits the +2.7 dBFS float samples of
  finding 3, and whether the click of finding 1 is audible on real material. Both need a
  listening test or a loopback capture.
