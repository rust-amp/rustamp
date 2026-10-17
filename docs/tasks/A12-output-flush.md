# A12: Flush output device on stop, seek, and natural end (A10 F3)

**Date** 2026-10-17. **Lane** `rustamp-audio` + `rustamp-output`.
**Addresses** Finding F3 from `docs/tasks/A10-start-and-default-review.md`.

Nothing was executed or tested on the host (Hyper-V VM rule). Only
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
and `cargo build --workspace` were used.

---

## 1. Evidence that the defect is real

1. **Trait contract:** `crates/rustamp-core/src/audio.rs:330-337` documents:
   - `pause(&mut self) -> Result<()>`: *"Pause output (stops pulling; device buffer retained)."*
   - `flush(&mut self) -> Result<()>`: *"Drop everything buffered in the device (used on seek / stop)."*

2. **Missing engine calls:** In `crates/rustamp-audio/src/engine.rs`:
   - `Engine::stop()` (`engine.rs:682-707`) called `self.output.pause()` at line 685, but never called `self.output.flush()`.
   - `Engine::finish_run()` (`engine.rs:987-1021`) called `self.output.pause()` at line 996, but never called `self.output.flush()`.
   - `Engine::on_decode_out()` under `DecodeOut::Seeked` (`engine.rs:915-932`) re-based the clock at line 928, but never called `self.output.flush()`.
   - Indeed, prior to this change, no call to `OutputDevice::flush` existed anywhere in `crates/rustamp-audio/` or the wider repository.

3. **WASAPI endpoint buffer retention:** In `crates/rustamp-output/src/wasapi_out.rs`:
   - `WasapiOutput::pause()` (`wasapi_out.rs:396-399`) sets `shared.paused = true`.
   - In `render_loop` (`wasapi_out.rs:612-619`), `client.stop_stream()` freezes the stream. Per Microsoft WASAPI specification for `IAudioClient::Stop`:
     *"The Stop method stops the audio stream... Stopping the stream leaves the stream at its current position and does not reset the buffer. To reset the buffer, call the IAudioClient::Reset method."*
   - The endpoint buffer holds up to 200 ms of audio (`BUFFER_DURATION_HNS = 2_000_000`, `wasapi_out.rs:59`).
   - `IAudioClient::Reset` (`client.reset_stream()`) was only hooked up to the atomic `shared.flush` flag (`wasapi_out.rs:659-662`). Because `Engine::stop()` never called `OutputDevice::flush()`, `reset_stream()` was never invoked on stop.
   - When playback resumed (`Engine::play()` -> `wasapi_out.rs:620-625` `client.start_stream()`), up to 200 ms of the previous run was replayed by WASAPI from the endpoint buffer below the ring buffer, where the ring invalidation from A8/A11 could not reach it.

4. **Defective flush handling inside `wasapi_out.rs`:**
   - While paused, `wasapi_out.rs:618` called `continue;`, so line 659 was never reached while paused.
   - On resume, line 621 called `client.start_stream()`, making `streaming = true`, before line 659 called `client.reset_stream()`.
   - Per MSDN `IAudioClient::Reset`: *"This method fails if it is called on a stream that is not stopped. (AUDCLNT_E_NOT_STOPPED)"*.
   - Thus, even if the engine had called `flush()`, `client.reset_stream()` was executed while streaming and failed silently.

5. **Seek pre-buffer leak:** On seek (`DecodeOut::Seeked`), failing to flush meant that up to 200 ms of pre-seek audio remained in the hardware buffer while the displayed timeline position jumped immediately to the seek target.

---

## 2. What was changed

### `crates/rustamp-audio/src/engine.rs`
- **`Engine::stop()`:** Added `let _ = self.output.flush();` immediately following `self.output.pause();`. Drops the device buffer when the user stops playback.
- **`Engine::finish_run()`:** Added `let _ = self.output.flush();` immediately following `self.output.pause();`. Drops any residual audio in the device buffer when playback reaches the end of the queue or stops after current track.
- **`Engine::seek_frames()`:** Added `let _ = self.output.flush();` on the stopped-seek branch (`self.transport == TransportState::Stopped`).
- **`Engine::on_decode_out()` under `DecodeOut::Seeked`:** Added `let _ = self.output.flush();` before reading `self.output.played_frames()` and rebasing `clock_origin`. Drops pre-seek audio from the device buffer so post-seek playback begins cleanly at the target frame.

### `crates/rustamp-output/src/wasapi_out.rs`
- Fixed `render_loop` flush handling:
  - While paused (`shared.paused == true`), `shared.flush.swap(false, Ordering::AcqRel)` is checked and `client.reset_stream()` is invoked while the stream is stopped (`streaming == false`).
  - Before starting the stream (`if !streaming`), any pending flush is executed before `client.start_stream()`.
  - While running (`streaming == true`), if a flush arrives (e.g. during a running seek), the stream is stopped, reset, and restarted: `client.stop_stream(); client.reset_stream(); streaming = client.start_stream().is_ok();`.

### `crates/rustamp-output/src/null.rs`
- Added `flushes: AtomicU64` to `Shared`.
- In `NullOutput::open()`, resets `flushes` to 0.
- In `NullOutput::flush()`, increments `flushes`.
- Added public accessor `pub fn flushes(&self) -> u64` on `NullClock` so headless tests can inspect device flush occurrences.

### `crates/rustamp-audio/tests/engine.rs`
- Added 5 integration tests:
  1. `stopped_run_flushes_output_device`: Verifies `h.engine.stop()` invokes `flush()` on the device.
  2. `seek_flushes_output_device`: Verifies `Seeked` invokes `flush()` on the device during seek drain.
  3. `seek_while_stopped_flushes_output_device`: Verifies seeking while stopped flushes the device.
  4. `natural_end_flushes_output_device`: Verifies natural track completion flushes the device on transition to `Stopped`.
  5. `buffered_device_drops_tail_on_stop_preventing_stale_replay`: Simulates a device with an internal hardware/driver buffer (retained on pause like WASAPI) and proves headlessly that `stop()` followed by `play()` clears the device buffer via `flush()` and prevents stale replay.

---

## 3. Invariants, their tests, and what only hardware can prove

### Invariants & Headless Pins
- **`stop()` flushes the device:** Pinned by `stopped_run_flushes_output_device` (`flushes() >= 1`).
- **`seek` flushes the device:** Pinned by `seek_flushes_output_device` (`flushes() > flushes_before`).
- **`finish_run` flushes the device:** Pinned by `natural_end_flushes_output_device` (`flushes() >= 1`).
- **No stale audio replays below the ring:** Pinned by `buffered_device_drops_tail_on_stop_preventing_stale_replay`, where track 1 audio is loaded into a mock hardware buffer, `stop()` is called, and track 2 audio is verified to play with zero leakage from track 1.

### What only a real device can prove
- **Exact stale replay duration:** On real WASAPI hardware, the audible replay length without flush depends on the endpoint driver buffer size and how full it was when `IAudioClient::Stop` was called (up to 200 ms).
- **Physical audio / loopback capture:** Verifying on physical DACs/soundcards via loopback capture that playing a tone, stopping, and immediately playing silence yields zero audible or captured tone packets.
- **Driver `Reset` compliance:** Confirming that specific third-party audio drivers handle `Stop -> Reset -> Start` transitions without driver-level glitches or pops in shared mode.
- **Audio service event timing:** The latency and scheduling of WASAPI buffer-ready events immediately after `IAudioClient::Reset`.

---

## 4. Verification

- `cargo fmt --all -- --check`: **PASS** (clean exit 0)
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **PASS** (0 warnings, clean exit 0)
- `cargo build --workspace`: **PASS** (clean exit 0)
- `cargo test`: **NOT RUN** (Hyper-V test VM rule respected; no tests executed on host)
- No window opened, no audio played, no commit created.
