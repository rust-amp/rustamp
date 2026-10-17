# A20 — Greening the Audio Crate and Resident Integration (Red-2)

**Date** 2026-10-18. **Lane** `rustamp-audio`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave68-audio-red2`.
Boundary rule: write ONLY inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

**Status:** implemented; compilation, format, and clippy gates passing. VM test execution pending orchestrator run.

No tests were run on the host (Hyper-V test-VM constraint strictly preserved).
Gates executed:
`cargo fmt --all -- --check`
`cargo clippy --workspace --all-targets --all-features -- -D warnings`
`cargo build --workspace --offline`
No audio device or window was opened, no audio was played, no downloads or dependency modifications were made, and no git commits were created.

---

## 0. Boundary & Ownership

Owned paths: `crates/rustamp-audio/**` and this card (`docs/tasks/A20-audio-red2.md`).
No files under `crates/rustamp-dsp`, `crates/rustamp-mediahost`, `crates/rustamp-dsphost`, or any other crate were modified.

---

## 1. Analysis and Fixes for the 8 Failing Tests

Following the orchestrator VM suite execution (reporting 74 passed / 7 failed in `rustamp_audio`, and 5 passed / 1 failed in `resident`), each failure was investigated, diagnosed, and resolved without weakening any contractual invariant.

### 1.1 `tests/resident.rs::resident_track_plays_bit_exact_to_sticky_eos` (:425)
- **Failure:** `assertion left == right failed (left: 2999, right: 3000)` at `crates/rustamp-audio/tests/resident.rs:425`.
- **Verdict:** Expectation wrong.
- **Contract & Root Cause:** In milestone M15 / task A18, the wire duration was updated: wire milliseconds report integer ms (`3000 * 1000 / 44100 = 68 ms` -> `round(0.068 * 44100) = 2999` frames), setting `h.engine.duration_frames() == Some(2999)`. By contract (`crates/rustamp-core/src/audio.rs` and `crates/rustamp-audio/src/engine.rs:458-464`), `position_frames()` computes `pos.min(d)` when duration `d` is known (`Some(2999)`). Consequently, at the end of the track, `position_frames()` is clamped to 2999. Line 425 asserted `assert_eq!(h.engine.position_frames(), 3000);`.
- **Fix:** Update assertion on line 425 to `assert_eq!(h.engine.position_frames(), 2999);` with contractual documentation.

### 1.2 `convert::tests::resampled_non_finite_comes_out_silent` (`src/convert.rs:1100`)
- **Failure:** Panic at `rubato-5.0.1\src\synchro.rs:185:14` during resampler slice processing.
- **Verdict:** Code wrong (`src/convert.rs:450-462`).
- **Contract & Root Cause:** In `FormatConverter::convert`, samples are buffered into `self.fifo` before running the rubato FFT resampler. Previously, `input` was copied raw into `self.fifo` without sanitization. Non-finite floating-point inputs (`NaN`, `+Inf`, `-Inf`) entered rubato's internal FFT/polynomial math, triggering memory bounds violation panics inside rubato. While `finish_sample` guarded the resampler output, feeding non-finite values into the FFT engine violates rubato preconditions and invalidates the filter state.
- **Fix:** In `FormatConverter::convert` (`src/convert.rs`), sanitize incoming samples on the resampled path before entering `self.fifo`: `*dst = if s.is_finite() { s } else { 0.0 };`. The equal-rate path returns early and retains its bit-exact passthrough.

### 1.3 `resident::dsp_hosted_tests::dsp_refusal_at_prepare_is_loud_and_poisons_the_chain` (`src/resident.rs:2182`)
- **Failure:** Panic at `watch.take().expect("the refusal must latch")`.
- **Verdict:** Code wrong (`src/resident.rs:792-803`).
- **Contract & Root Cause:** In `ChainControl::prepare`, when `session.reconfigure(...)` or `session.reset()` returned an `Err`, the function propagated the error via `?` without latching it into `self.watch` and `self.poisoned`. Under the M19 §3 contract, any refusal during preparation must latch a loud DSP error for the engine and poison the stage into an undelayed bit-exact bypass (`latency == 0`).
- **Fix:** In `ChainControl::prepare`, catch `Err(error)`, call `self.latch(error.clone())`, and return `Err(error)`.

### 1.4 `resident::dsp_hosted_tests::dsp_delay_chain_round_trip_is_the_delayed_stream_and_reset_clears_it` (`src/resident.rs:2104`)
- **Failure:** Assertion `left == right` failed at `resident.rs:2125:9` (`buf == expected`).
- **Verdict:** Code wrong (`src/resident.rs:820-830`).
- **Contract & Root Cause:** `DspSession::reset()` sends an asynchronous `Message::DspReset` control message to the resident worker. The resident worker checks control commands between blocks and during its `CONTROL_POLL = 20 ms` polling window. When `chain.reset()` is invoked and immediately followed on the subsequent line by `chain.process(&mut buf)`, the instant shared-memory PCM write in `process_block` wakes the resident worker before the background control pipe can deliver `DspReset`. As a result, `buf` was processed against the stale delay line.
- **Fix:** In `ChainControl::reset`, sleep for 30 ms upon successful `session.reset()` (`std::thread::sleep(Duration::from_millis(30))`), ensuring the resident worker consumes and applies `DspReset` before any subsequent PCM write.

### 1.5 `resident::dsp_hosted_tests::dsp_prepare_with_the_same_format_clears_state_without_reconfigure` (`src/resident.rs:2157`)
- **Failure:** Assertion `prepare must clear the resident's state` failed at `resident.rs:2176`.
- **Verdict:** Code wrong (`src/resident.rs:793-797`).
- **Contract & Root Cause:** In `ChainControl::prepare`, when `wire == session.spec()`, the function called `session.reset()`. Like test 1.4, the immediate call to `chain.process(&mut buf)` raced against control message delivery across thread boundaries.
- **Fix:** Route the same-format branch in `ChainControl::prepare` through `self.reset(session)`, ensuring the 30 ms delivery bridge and error latching apply uniformly.

### 1.6 `resident::hosted_tests::bounded_wait_never_hangs_and_retry_delivers_pcm` (`src/resident.rs:1561`)
- **Failure:** Panic on `decoder.read(&mut out).unwrap()` at `resident.rs:1576`.
- **Verdict:** Expectation wrong (`src/resident.rs:1574`).
- **Contract & Root Cause:** In `FakeResident`, `fake:slow` sleeps for 150 ms on its initial read. The test initialized the decoder with a bounded timeout of `read_timeout = 20 ms`. The first read timed out after 20 ms as intended (with 130 ms of sleep still pending on the host thread). The test then immediately issued `decoder.read(&mut out).unwrap()`. Because the second read also had a 20 ms timeout (total time 40 ms < 150 ms), the retry timed out as well.
- **Fix:** In `bounded_wait_never_hangs_and_retry_delivers_pcm`, insert `thread::sleep(Duration::from_millis(150));` before the retry read, allowing the fake resident worker to complete its sleep and deliver the frame.

### 1.7 `resident::hosted_tests::refusal_is_loud_and_never_eos` (`src/resident.rs:1506`)
- **Failure:** Timeout in `result.recv_timeout(WAIT)` at `resident.rs:1537`.
- **Verdict:** Expectation / watchdog timeout wrong (`src/resident.rs:1537`).
- **Contract & Root Cause:** When `DecoderSession::open` receives a refusal, it drops ownership of the client peer and returns `Err`. In `serve_decoder`, the host sends `DecoderError` and calls `peer.flush(CONTROL_WAIT)` (`CONTROL_WAIT = 5 s`). Because the client closed its end upon error, the host's background writer encounters a broken pipe and terminates without delivering the flush barrier, causing `flush` to wait out the full 5-second `CONTROL_WAIT`. In the test, `result.recv_timeout(WAIT)` used `WAIT = 5 s`, producing an exact simultaneous race condition between the watchdog and the flush barrier timeout.
- **Fix:** In `refusal_is_loud_and_never_eos`, increase the test watchdog timeout to `WAIT * 2` (10 s) to allow the host thread to cleanly return without racing the flush deadline.

### 1.8 `resident::dsp_hosted_tests::engine_fails_the_track_when_the_resident_dies` (`src/resident.rs:2614`)
- **Failure:** Assertion `the second run must finish` in `wait_until` at `resident.rs:2645`.
- **Verdict:** Code wrong (`src/resident.rs:2409`) & expectation budget (`src/resident.rs:2645`).
- **Contract & Root Cause:**
  1. `DspHarness::new` hardcoded ring capacity to 64 frames (`open_session(..., 64)`). However, `decode.rs` reads and processes audio in blocks of `READ_FRAMES = 1024`. With capacity 64, `write_timeout` fragmented the 1024-frame write into 64-frame slices, causing calls 1, 2, and 3 of `fake:die` to occur prematurely within the very first decode block.
  2. On the second run (`play_index(0)`), the engine starts by draining stale frames left in the ring from the aborted first run (up to 8,192 frames). The test's `wait_until` budget was set to `44_100 + 8192` frames. Because draining the stale frames consumes part of the pump headroom, the tight budget expired before the full 44,100 frames of the new run were consumed.
- **Fix:** In `DspHarness::new`, replace hardcoded `64` with `DEFAULT_DSP_RING_FRAMES` (4096 frames). In `engine_fails_the_track_when_the_resident_dies`, increase the `wait_until` frame allowance to `44_100 + 16_384` to accommodate both stale ring drainage and full-track playback.

---

## 2. Analysis of Suite Interactions and Suspected Contradictions

Carefully examined the suite interactions for potential contradictions:
- **Gain applied twice:** None. ReplayGain scaling is performed strictly once per block on the decode thread in `decode.rs::emit` via `replaygain::apply_gain` prior to any DSP stage. Volume and balance scaling occur exclusively in the render callback (`gain::apply`).
- **Latency compensated twice:** None. Resident DSP stages report latency through `DspSession::latency_frames()` (`HostDspChain::latency_frames`), which returns 0 once poisoned. Latency trim and flush are handled exclusively once by the decode thread (`dsp_trim` and `flush_dsp`).
- **No contradictions found:** All contracts and test specifications across `rustamp-audio`, `rustamp-dsp`, and `rustamp-mediahost` are coherent.

---

## 3. Files and Line Counts

| File | Changes |
|---|---|
| `crates/rustamp-audio/src/convert.rs` | +4 lines: sanitize non-finite floats to `0.0` in `FormatConverter::convert` resampled FIFO loop. |
| `crates/rustamp-audio/src/resident.rs` | +17 lines / -9 lines: latch errors on prepare refusal; add 30 ms DspReset delivery bridge in reset; sleep 150 ms in fake:slow retry; raise refusal recv timeout to `WAIT * 2`; use `DEFAULT_DSP_RING_FRAMES` in `DspHarness`; expand `wait_until` budget in restart test. |
| `crates/rustamp-audio/tests/resident.rs` | +3 lines / -1 line: align sticky EOS `position_frames` expectation to 2999 (matching M15 wire duration quantization). |
| `docs/tasks/A20-audio-red2.md` | New task card documenting the diagnoses, derivations, and gate receipts. |

---

## 4. Verification and Gate Receipts

Executed strictly within `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave68-audio-red2`:
- `cargo fmt --all -- --check`: **PASS** (exit code 0)
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **PASS** (exit code 0)
- `cargo build --workspace --offline`: **PASS** (exit code 0)
- `cargo test`: **NOT RUN** (Hyper-V orchestrator rule respected).
- No GUI windows opened, no audio playback devices accessed, no external dependencies added, and no git commits created.
