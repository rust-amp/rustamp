# RFC 0012: Output Device Mid-Run Error Hook

Status: **Accepted** (owner approval, 2026-10-18; Proposed until then)
Owner: Lane I (contracts / architecture) & Lane H (output robustness).
Depends on: RFC 0001 (core contracts), RFC 0002 (audio contracts), RFC 0004 (licensing and permissive wire crates).

---

## 1. Why & Problem Statement

### 1.1 The Problem
During audio playback, an output device can experience fatal mid-run failures:
1. Physical device removal (e.g. USB DAC or Bluetooth headphones disconnected).
2. Format invalidation or exclusive-mode revocation (e.g. Windows OS renegotiating the endpoint mix format or an exclusive application seizing the hardware).
3. Continuous buffer write failures or endpoint hangs.

Currently, the `rustamp-output` crate catches such failures and latches them locally in the backend, but the engine and the wider application have no contract-level mechanism to detect or react to this state during playback. As a result:
- The device stops consuming frames from the lock-free audio ring buffer.
- The ring buffer fills up, causing the decode thread to block waiting for space.
- The engine remains indefinitely in `TransportState::Playing`.
- The audio position clock (`OutputDevice::played_frames`) freezes or halts.
- The player hangs silently with no error event published, no notification to the UI, and no notification to plugins.

### 1.2 Code Evidence
- **`crates/rustamp-core/src/audio.rs:320-353`**: The [`OutputDevice`] trait specifies the lifecycle:
  `open(spec) -> Result<OutputFormat>` -> `start(render) -> Result<()>` -> (`pause` / `resume` / `flush`)* -> `close() -> Result<()>`.
  The trait provides no method to query mid-run device health or stream validity.
- **`crates/rustamp-core/src/audio.rs:304`**: [`RenderFn`] is defined as `Box<dyn FnMut(&mut [f32]) + Send + 'static>`. It is a pure pull callback executed on the audio thread that returns `()`. It has no channel to bubble up mid-run fatal device failures.
- **`crates/rustamp-output/src/wasapi_out.rs:75-123,323-325,570,702`**: Task H3 added a failure streak counter (`FailureStreak`, limit `MAX_CONSECUTIVE_FAILURES = 64`) and an atomic error latch (`shared.error: AtomicU8`, `ERR_GONE = 5`). It exposed an inherent method `WasapiOutput::stream_error(&self) -> Option<&'static str>` at lines 323–325. However, because this is an inherent method on `WasapiOutput` rather than an `OutputDevice` trait method, it cannot be called through the trait object (`Box<dyn OutputDevice>`).
- **`crates/rustamp-output/src/cpal_out.rs:40-44,99-104,325-331`**: cpal's error callback latches `shared.failed.store(true, Ordering::Release)`. An inherent method `CpalOutput::has_failed(&self) -> bool` is exposed at lines 102–104, but it is likewise inaccessible via `OutputDevice`.
- **`crates/rustamp-output/src/null.rs:110-215`**: The virtual test device `NullOutput` has no error latch or error injection mechanism, preventing deterministic testing of mid-run device failures in automated test suites.
- **`crates/rustamp-audio/src/engine.rs:161,794-820`**: `Engine` holds `output: Box<dyn OutputDevice>`. In `Engine::poll(&mut self)`, the engine drains messages from the decode thread and checks end-of-stream counters, but never inspects `output` for mid-run failures.
- **`crates/rustamp-core/src/bus.rs:8,70-96`**: `EventBus::publish` acquires a `std::sync::Mutex` and sends over unbounded channels that allocate heap memory per subscriber. It explicitly documents: *"do not publish from the audio thread"*.

A contract change in `rustamp-core` is required before any code can wire mid-run error detection into the audio engine.

---

## 2. Options Considered & Trade-Offs

We evaluated three architectural approaches:

### Option A: Polled Accessor on `OutputDevice` (Engine checks in `poll()`)
The `OutputDevice` trait gains a non-blocking, lock-free accessor:
```rust
fn stream_error(&self) -> Option<Error> { None }
```
The engine calls `self.output.stream_error()` during its existing control-thread `Engine::poll()` loop (which runs every few milliseconds).

- **Audio-thread impact**: Zero. The audio thread only stores an atomic integer or boolean (`Ordering::Release`) when a fatal failure streak occurs. No locks, no heap allocations, no I/O.
- **Thread safety**: Safe. The engine performs a single atomic load (`Ordering::Acquire`) on the control thread.
- **Layering & ownership**: Clean. Output backends remain pure audio sinks and know nothing about `EventBus`, channels, or player state. The engine retains full ownership of transport transitions and event dispatch.
- **Detection latency**: Sub-frame to a few milliseconds (bounded by the engine's polling interval, typically ~10 ms), well within human perception and significantly faster than the hardware streak detection timeout (~320 ms in WASAPI).

### Option B: Asynchronous Error Callback Passed to the Backend
The backend is given an error callback during `open` or `start`:
```rust
fn start(&mut self, render: RenderFn, on_error: Box<dyn Fn(Error) + Send + 'static>) -> Result<()>;
```
When a fatal error occurs, the backend's audio/error thread invokes `on_error(err)`.

- **Audio-thread impact**: High violation risk. The callback executes on the OS audio thread or cpal stream error thread. If the callback attempts to allocate heap memory, acquire mutexes, or perform I/O, it violates the real-time audio thread contract.
- **Deadlock & Reentrancy**: If the callback attempts to call back into `Engine` while the engine is executing a command (e.g. `Engine::stop` calling `OutputDevice::pause`), deadlocks or race conditions are likely.
- **Degeneration**: If the callback is restricted to only signaling an atomic flag or writing to a lock-free queue for the engine to read later, it degenerates into a more complex, error-prone variation of Option A.

### Option C: Backend Publishes Directly to `EventBus`
The backend is given an `EventBus` instance during initialization and calls `bus.publish(Event::Error(...))` directly upon failure.

- **Contract violation**: RFC 0001 §Decisions and `rustamp-core::bus` explicitly forbid publishing from the audio thread. `EventBus::publish` locks a `Mutex<Vec<Sender<Event>>>` and clones messages into unbounded crossbeam channels, which allocate heap nodes dynamically.
- **State desynchronization**: If the backend publishes `Event::Error` directly to subscribers, the UI and plugins receive an error notification while the `Engine`'s internal state machine still believes it is in `TransportState::Playing`. The decode thread continues waiting on the ring buffer, and the player enters an incoherent zombie state.
- **Layering violation**: Low-level audio device drivers in `rustamp-output` become tightly coupled to the application event bus.

---

## 3. Recommendation

We strongly recommend **Option A (Polled Accessor)**.

### Why:
1. **Preserves Real-Time Purity**: The audio thread remains completely lock-free, allocation-free, and I/O-free. It only updates an atomic latch.
2. **Upholds Single-Owner Architecture**: The `Engine` is the single source of truth for transport state transitions. It coordinates stopping the decode thread, freezing the timeline clock, resetting the ring buffer, and publishing state events in a deterministic sequence.
3. **Simplicity & Safety**: Zero risk of reentrancy or deadlock between the audio callback and the engine control thread.
4. **Deterministic Testing**: Enables virtual test devices (`NullOutput`) to simulate mid-run hardware failures deterministically by flipping an atomic latch.

---

## 4. Proposed Design & Exact API

### 4.1 Trait Extension in `crates/rustamp-core/src/audio.rs`
Add `stream_error` with a default implementation to [`OutputDevice`]:

```rust
pub trait OutputDevice: Send {
    // ... existing methods ...

    /// Check if a fatal mid-stream error occurred on the output device.
    ///
    /// Real backends latch stream failures (e.g. endpoint removal, hardware
    /// format revocation, buffer write streak exhaustion) asynchronously from
    /// their audio/render callbacks.
    ///
    /// This method is called from the control/engine thread (e.g. in
    /// `Engine::poll`). It must be non-blocking and lock-free.
    ///
    /// Returns `None` if the device is healthy or stopped, or `Some(err)`
    /// describing the fatal failure that terminated output.
    fn stream_error(&self) -> Option<Error> {
        None
    }
}
```

### 4.2 Backend Implementations in `crates/rustamp-output`

1. **`WasapiOutput` (`crates/rustamp-output/src/wasapi_out.rs`)**:
   Implement `stream_error` on `OutputDevice for WasapiOutput`:
   ```rust
   fn stream_error(&self) -> Option<Error> {
       let code = self.shared.error.load(Ordering::Acquire);
       error_text(code).map(|msg| {
           Error::Output(format!("{}: {msg}", self.device_name))
       })
   }
   ```
   *Note: Inherent `stream_error` method can be deprecated or delegate to the trait method.*

2. **`CpalOutput` (`crates/rustamp-output/src/cpal_out.rs`)**:
   Implement `stream_error` on `OutputDevice for CpalOutput`:
   ```rust
   fn stream_error(&self) -> Option<Error> {
       if self.shared.failed.load(Ordering::Acquire) {
           Some(Error::Output(format!(
               "{}: audio stream encountered a fatal error or disconnected",
               self.device_name
           )))
       } else {
           None
       }
   }
   ```

3. **`NullOutput` (`crates/rustamp-output/src/null.rs`)**:
   Add an atomic error latch to `Shared`:
   ```rust
   struct Shared {
       // ... existing fields ...
       failed: AtomicBool,
   }
   ```
   Add a test helper on `NullClock`:
   ```rust
   impl NullClock {
       /// Simulate a fatal hardware/stream failure mid-run.
       pub fn fail(&self) {
           self.shared.failed.store(true, Ordering::Release);
       }
   }
   ```
   Implement `stream_error` on `OutputDevice for NullOutput`:
   ```rust
   fn stream_error(&self) -> Option<Error> {
       if self.shared.failed.load(Ordering::Acquire) {
           Some(Error::Output("null: simulated device failure".into()))
       } else {
           None
       }
   }
   ```
   `NullOutput::open()` resets `self.shared.failed` to `false`.

### 4.3 Engine Integration in `crates/rustamp-audio/src/engine.rs`

1. **Poll Hook**:
   In `Engine::poll(&mut self)` (`crates/rustamp-audio/src/engine.rs:794`):
   ```rust
   pub fn poll(&mut self) -> Result<()> {
       if self.transport == TransportState::Playing {
           if let Some(err) = self.output.stream_error() {
               self.handle_output_error(err);
               return Ok(());
           }
       }
       // ... remaining poll logic (decode out draining, end-of-stream, position) ...
   }
   ```

2. **Error Recovery & Teardown Policy**:
   What should the engine do on device loss?
   - **Do NOT keep playing into nothing**: Silently discarding audio drains the playlist, wastes CPU, desynchronizes timeline position, and confuses the user.
   - **Do NOT silently reopen or switch device**: Silently switching to another device (e.g. laptop speakers) violates the explicit rule established in H3 and `lib.rs:51` ("the chosen backend never silently switches devices"). Reopening a physically disconnected USB/Bluetooth device will fail in a rapid loop.
   - **POLICY: Stop the run cleanly and report the error.**
     When `self.output.stream_error()` returns an error:
     1. Stop the decode thread immediately (`self.sync.set_active(false); self.join_decode();`).
     2. Pause the device (`let _ = self.output.pause();`).
     3. Transition transport to `TransportState::Stopped`.
     4. Freeze the timeline position at the point of failure:
        `self.timeline_origin = self.timeline_frames(); self.clock_origin = self.output.played_frames();`.
     5. Publish `Event::StateChanged(TransportState::Stopped)`.
     6. Publish `Event::PositionChanged` at the frozen position.
     7. Publish `Event::Error(ErrorInfo { kind: ErrorKind::Output, message: err.to_string() })`.
     8. If the user presses Play later, `Engine::play()` initiates a clean `open()` sequence, which will either succeed if the device is back or return a clear `Error::Output` naming the missing device and listing alternatives.

### 4.4 Observable Behavior for UI & Plugins

- **User Interface (UI)**:
  - Receives `Event::Error(ErrorInfo)` -> displays error in the main window status bar / marquee or modal alert.
  - Receives `Event::StateChanged(TransportState::Stopped)` -> Play button unpresses, visualization stops, marquee stops scrolling.
  - Receives `Event::PositionChanged` -> position display freezes at the exact second where output failed.
- **Out-of-Process Plugins (`rustamp-plughost` / `rustamp-ipc`)**:
  - Receives `Message::PlayerStateUpdate` reflecting `TransportState::Stopped`.
  - Classic `IPC_ISPLAYING` returns `0` (stopped).
  - Classic `IPC_GETOUTPUTTIME` returns the frozen position in milliseconds.
  - Vis plugins cease receiving new `VisFrame` packets.

---

## 5. Compatibility & Wire Impact

- **`rustamp-core` Contract**: Fully backwards compatible. The addition of `stream_error(&self) -> Option<Error>` to `OutputDevice` includes a default implementation returning `None`. Existing trait implementors, mocks, and test stubs compile without changes.
- **`rustamp-proto` & Wire Formats**: Zero changes. `ErrorKind::Output` and `Event::Error(ErrorInfo)` already exist in `rustamp-proto::error` and `rustamp-proto::event`.
- **`rustamp-ipc`**: Zero changes. `PROTOCOL_VERSION` remains untouched (v2).
- **Public Plugin ABI**: Zero changes.

---

## 6. Licensing Reasoning

- `rustamp-core`, `rustamp-audio`, and `rustamp-output` are licensed **GPL-3.0-or-later** with the `PLUGIN-EXCEPTION.md` permission.
- The permissive crates (`rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost`, `rustamp-mediahost`) remain completely untouched.
- No third-party dependencies are introduced. The license boundaries defined in RFC 0004 remain strictly preserved.

---

## 7. Test Plan

All tests must run without host audio or windows, respecting the Hyper-V test VM constraint:

1. **Virtual Null Device Unit Tests (`crates/rustamp-output/src/null.rs`)**:
   - `stream_error_returns_none_initially`: A freshly created `NullOutput` returns `None` from `stream_error()`.
   - `stream_error_reflects_injected_failure`: Calling `clock.fail()` causes `output.stream_error()` to return `Some(Error::Output(...))`.
   - `open_clears_injected_failure`: Reopening a failed `NullOutput` resets the failure latch.

2. **Backend Unit Tests (`crates/rustamp-output`)**:
   - `wasapi_stream_error_reflects_failure_streak`: Verify that after `FailureStreak` reaches its limit, `OutputDevice::stream_error()` returns `Some(Error::Output(...))` containing the device name.
   - `cpal_stream_error_reflects_error_callback`: Verify that setting `shared.failed` causes `OutputDevice::stream_error()` to return `Some(Error::Output(...))` containing the device name.

3. **Engine Transport Tests (`crates/rustamp-audio/tests/output_error.rs`)**:
   - `engine_stops_and_publishes_error_when_output_fails_mid_run`:
     1. Build an `Engine` with `NullOutput::pair()` and subscribe to its `EventBus`.
     2. Start playback of a test tone source (`Command::Play`).
     3. Pump 2048 frames via `NullClock::pump`; assert `transport == TransportState::Playing`.
     4. Trigger `clock.fail()`.
     5. Call `engine.poll()`.
     6. Assert that `engine.state().transport` transitioned to `TransportState::Stopped`.
     7. Assert that `Event::StateChanged(TransportState::Stopped)` was published.
     8. Assert that `Event::Error(ErrorInfo { kind: ErrorKind::Output, .. })` was published.
     9. Assert that the decode thread was joined and the position is frozen.

---

## 8. Open Questions for the Owner

| ID | Question | Recommendation & Context |
|---|---|---|
| **Q1** | **Error recovery vs. Fail-to-stop**: Should Rustamp ever support an optional auto-reconnect setting (e.g. waiting up to 3 seconds for a disconnected USB DAC to return before failing), or should it strictly stay fail-to-stop? | **Recommendation: Fail-to-stop.** Matches Winamp 2.95 behavior. Auto-reconnect risks audio device thrashing and accidental playback through unwanted default fallbacks (e.g. laptop speakers). If auto-reconnect is ever desired, it should be an explicit app-level policy, not an engine/driver behavior. |
| **Q2** | **Error payload precision**: Should `stream_error(&self)` return `Option<Error>` or a specialized `Option<OutputError>` enum (e.g. distinguishing `DeviceLost` from `FormatInvalidated` or `BufferUnderflowStreak`)? | **Recommendation: `Option<Error>`.** `Error::Output(String)` already maps cleanly to `ErrorKind::Output` on the wire and fits the existing `rustamp-core::error::Error` hierarchy. A specialized enum adds contract complexity without actionable behavioral differences at the engine level (both require stopping playback and notifying the user). |
| **Q3** | **Pre-command polling**: Should `output.stream_error()` only be checked inside `Engine::poll()`, or also at the beginning of commands like `Engine::play()`, `Engine::resume()`, and `Engine::seek_frames()`? | **Recommendation: In `Engine::poll()` and at the start of `Engine::resume()`.** `Engine::poll()` is the steady-state detector. Checking in `resume()` prevents resuming a stream whose hardware was yanked while paused. |
| **Q4** | **cpal severe underruns**: On some platforms (e.g. Linux ALSA/PulseAudio), cpal's error callback can occasionally fire on severe underruns rather than unrecoverable device loss. Should cpal's error callback treat all errors as fatal? | **Recommendation: Yes for now.** On Windows, cpal errors represent unrecoverable stream terminations. Distinguishing platform-specific transient underruns from fatal device disconnects can be refined in the Linux milestone (M6) without breaking this contract. |
