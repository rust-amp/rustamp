# Task card H5 — Output device mid-run error hook implementation (RFC 0012)

**Goal**  
Implement RFC 0012 (Option A: Polled Accessor) across `rustamp-core`, `rustamp-output`, and `rustamp-audio`:
a contract-level way for the engine to learn that an output device failed mid-run, stop playback cleanly, freeze the timeline position, and publish the error event so the UI and plugins can observe it.

**Lane**  
Lane I (contracts) / Lane H (output robustness) / Lane A (audio engine).

**Owned paths**  
- `crates/rustamp-core/src/audio.rs`
- `crates/rustamp-output/src/null.rs`
- `crates/rustamp-output/src/wasapi_out.rs`
- `crates/rustamp-output/src/cpal_out.rs`
- `crates/rustamp-audio/src/engine.rs`
- `crates/rustamp-audio/tests/output_error.rs`
- `docs/tasks/H5-output-error-hook.md` (this task card)  
*No other file was touched or modified.*

**Allowed sources**  
- `docs/rfc/0012-output-error-hook.md` (accepted design, Option A)
- `docs/rfc/0001-core-contracts.md`, `docs/rfc/0002-audio-contracts.md`, `docs/rfc/0004-plugin-licensing.md`
- `docs/tasks/H3-output-robustness.md`, `docs/tasks/H4-output-error-rfc.md`, `docs/tasks/A12-output-flush.md`
- `crates/rustamp-core/src/audio.rs`, `crates/rustamp-core/src/event.rs`, `crates/rustamp-core/src/error.rs`
- `crates/rustamp-output/src/**`, `crates/rustamp-audio/src/**`

**Clean room**  
Never read Winamp source, SDK headers, disassembly, `rustamp-research`, or `rustamp-refbin`.

---

## 1. Mechanism Implemented (RFC 0012 §3 & §4)

As recommended in RFC 0012 §3, **Option A (Polled Accessor on `OutputDevice`)** was implemented:

1. **`rustamp-core` Contract Extension (`crates/rustamp-core/src/audio.rs`)**:
   Added `OutputDevice::stream_error(&self) -> Option<Error>` with a default implementation returning `None`.
   This is non-blocking, lock-free, and backwards-compatible with existing mocks and test devices.

2. **Backend Implementations (`crates/rustamp-output`)**:
   - `NullOutput` (`null.rs`): Added `failed: AtomicBool` to `Shared`. `NullOutput::open()` resets `failed` to false. `stream_error()` returns `Some(Error::Output("null: simulated device failure".into()))` when failed. Added `NullClock::fail(&self)` and `NullClock::is_failed(&self)` for deterministic headless testing. Updated `drive()` to return 0 when failed.
   - `WasapiOutput` (`wasapi_out.rs`): Implemented `stream_error()` on `OutputDevice for WasapiOutput`, reading `self.shared.error` (`ERR_GONE`, `ERR_WRITE`, etc.) and formatting `Error::Output(format!("{}: {msg}", self.device_name))`. The inherent `stream_error` method delegates to the trait method.
   - `CpalOutput` (`cpal_out.rs`): Implemented `stream_error()` on `OutputDevice for CpalOutput`, reading `self.shared.failed` and formatting `Error::Output(format!("{}: audio stream encountered a fatal error or disconnected", self.device_name))`. Added inherent `stream_error` delegating to the trait method.

3. **Engine Integration (`crates/rustamp-audio/src/engine.rs`)**:
   - **Steady-state poll hook**: In `Engine::poll(&mut self)`, when `self.transport == TransportState::Playing`, the engine checks `self.output.stream_error()`.
   - **Pre-command resume check**: In `Engine::play(&mut self)` under `TransportState::Paused`, the engine checks `self.output.stream_error()` before resuming to prevent resuming into dead hardware.
   - **Pre-run check**: In `Engine::start_run(&mut self)`, checks `self.output.stream_error()` to error immediately if the device is already in a failed state.
   - **Teardown & reporting policy (`handle_output_error`)**:
     1. Sends `DecodeMsg::Stop` and deactivates ring sync (`self.sync.set_active(false)`).
     2. Pauses and flushes output (`self.output.pause()`, `self.output.flush()`).
     3. Joins the decode thread cleanly (`self.join_decode()`).
     4. Transitions transport to `TransportState::Stopped`.
     5. Freezes timeline position at the point of failure:
        `self.timeline_origin = self.timeline_frames(); self.clock_origin = self.output.played_frames();`.
     6. Clears pending seek/drain/counters.
     7. Updates `self.error = Some(info.clone())`.
     8. Publishes `Event::StateChanged(TransportState::Stopped)`.
     9. Publishes `Event::PositionChanged` at the frozen position.
     10. Publishes `Event::Error(ErrorInfo { kind: ErrorKind::Output, message })`.
     11. Because transport transitioned to `Stopped`, subsequent `poll()` calls do not re-poll or re-publish the error (failure reported once, not per poll).

---

## 2. Headless Test Suite Added

All tests execute headlessly without audio hardware or windows:

1. **`crates/rustamp-core/src/audio.rs`**:
   - `stream_error_defaults_to_none`: Proves the default trait method returns `None` on a minimal implementor.

2. **`crates/rustamp-output/src/null.rs`**:
   - `stream_error_returns_none_initially`: Fresh `NullOutput` returns `None`.
   - `stream_error_reflects_injected_failure`: `clock.fail()` causes `output.stream_error()` to report `Error::Output("null: simulated device failure")`.
   - `open_clears_injected_failure`: Reopening a failed `NullOutput` resets the failure latch.

3. **`crates/rustamp-output/src/wasapi_out.rs`**:
   - `wasapi_stream_error_reflects_failure_streak`: Verifies that a failure streak exceeding `MAX_CONSECUTIVE_FAILURES` latches `ERR_GONE` and `OutputDevice::stream_error()` returns `Some(Error::Output(...))` naming the device.

4. **`crates/rustamp-output/src/cpal_out.rs`**:
   - `cpal_stream_error_reflects_error_callback`: Verifies that latching `shared.failed` causes `OutputDevice::stream_error()` to return `Some(Error::Output(...))` naming the device.

5. **`crates/rustamp-audio/tests/output_error.rs`**:
   - `engine_stops_and_publishes_error_when_output_fails_mid_run`: Tests the complete RFC 0012 teardown sequence: `NullClock::fail()` -> `poll()` -> transport transitions to `Stopped`, `Event::StateChanged(Stopped)`, `Event::PositionChanged`, and `Event::Error(ErrorKind::Output)` are emitted, decode thread is joined, and position freezes.
   - `failure_is_reported_once_not_per_poll`: Verifies that multiple subsequent calls to `engine.poll()` do not emit duplicate `Event::Error` or `Event::StateChanged` events.
   - `healthy_device_is_unaffected`: Proves a normal playback run across multiple pumps and polls runs cleanly without errors.
   - `failure_observed_within_one_poll`: Proves that a mid-run failure is detected on the exact next single invocation of `engine.poll()`.
   - `device_failure_while_paused_prevents_resume`: Tests that triggering device loss while paused prevents resume and triggers clean teardown to `Stopped`.
   - `device_failure_before_play_errors_immediately`: Tests that starting playback on an already-failed device returns `Err` immediately and publishes `Event::Error`.

---

## 3. What Only a Real Device Can Prove

Headless unit and integration tests prove the contract state machine, atomic latching, and event dispatch deterministically. However, physical hardware is required to prove:
1. **Physical endpoint disconnection timing**: USB DAC or Bluetooth headphone detachment triggering OS device invalidation events and latency of the WASAPI/cpal event loop to notice endpoint detachment.
2. **OS mix format renegotiation**: Windows audio engine renegotiating shared endpoint mix formats when another process claims exclusive mode, and verifying that the `FailureStreak` counter reaches `MAX_CONSECUTIVE_FAILURES = 64` (~320 ms) cleanly without host thread starvation.
3. **Driver crash recovery**: Verifying third-party soundcard driver crashes do not hang the OS MTA thread or deadlock COM release during `close()`.

---

## 4. Open Questions from RFC 0012

1. **Q1 (Auto-reconnect vs. Fail-to-stop)**: RFC recommends fail-to-stop (matching Winamp 2.95). If an auto-reconnect policy is ever desired, it should remain an app-level setting, not an engine/driver behavior.
2. **Q2 (Error payload precision)**: Using `Option<Error>` mapping to `ErrorKind::Output` maintains simplicity. If UI later needs structured sub-types (e.g. `DeviceRemoved` vs `ExclusiveRevocation`), an additive error enum can be introduced without breaking the trait signature.
3. **Q3 (Pre-command polling)**: Checking `stream_error` in `poll()`, at the start of `play()` from `Paused`, and in `start_run()` fully covers steady-state and transition paths.
4. **Q4 (cpal underrun handling on Linux)**: On Windows, cpal stream errors represent fatal stream termination. Distinguishing platform-specific transient underruns from fatal device disconnects will be refined during the Linux milestone (M6).

---

## 5. Verification

- `cargo fmt --all -- --check`: **PASS** (clean exit 0)
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **PASS** (clean exit 0)
- `cargo build --workspace`: **PASS** (clean exit 0)
- `cargo test`: **NOT RUN** (Hyper-V test VM rule respected; no tests executed on host)
- No window opened, no audio played, no commit created.
- Boundary rule honoured: all writes strictly inside the worktree.
