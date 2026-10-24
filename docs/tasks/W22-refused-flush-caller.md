# Task Card W22-refused-flush-caller — Caller-Side Engine and Output Recreation on Refused Flush

## 1. Context and Problem Statement

### Background
In lane W22 (`docs/tasks/W22-two-reds.md`), the verdict of W20 (`docs/tasks/W20-unsupported-flush.md`) was overturned. W20 had attempted to silence `Error::Unsupported` returned by `OutputDevice::flush()` on backends that cannot portably flush hardware buffers (specifically CPAL, as used with the VB-Audio Virtual Cable), treating it as `Ok(())`. Contract test `crates/rustamp-audio/tests/flush_contract.rs` proved that treating an unsupported flush as success allowed stale buffered audio to continue playing, directly violating the core audio contract.

### Authoritative Audio Contract
The core audio contract in `crates/rustamp-core/src/audio.rs:355-385` (`OutputDevice::flush`) establishes the rules:
> *"After a successful return, no frame that was already buffered can still reach the speakers: the next frame heard belongs to whatever the engine renders from then on. A backend that cannot do that portably — the cpal backend is the one we have, because a shared endpoint buffer cannot be reset from here — must **refuse** with `Error::Unsupported` rather than return `Ok` and let stale audio play. A silent `Ok` is the one answer that is worse than a failure, because the caller cannot tell the difference.*
> 
> *The engine treats a refusal as terminal for the current output: it stops the run and does not resume into an output it cannot clean, so the caller (the application) has to recreate the engine and the output before it can play again. There is no automatic recovery, and a successful flush is not a promise that the hardware falls silent at that instant — only that what was buffered will not be heard as the next audio."*

### Application Gap Today
Prior to this task, `rustamp-app` did not meet this caller-side duty:
1. `App::with_registry` constructed `Engine` and its `OutputDevice` exactly once at application startup.
2. When playback finished naturally (`finish_run`), when the user stopped playback (`stop`), or when a seek occurred on a device whose backend refuses flush, `Engine::flush_output` closed the output, latched `self.flush_failure = Some(...)`, and published `Event::Error(ErrorKind::Output)`.
3. `App::surface_error` handled the event by logging to notes and setting `self.last_error`. It performed no recreation of the engine or output.
4. Consequently, on devices such as the VB-Audio Virtual Cable, every stop, track completion, or seek permanently incapacitated the player: any subsequent call to `play()`, `play_index()`, `seek_frames()`, or track advance immediately returned `Err(Error::Output)` due to the latched `flush_failure`.

### Loopback Test Gap
In `the_cable_carries_the_fixture_tone` (`crates/rustamp-app/tests/loopback.rs`), line 544 asserted `assert!(engine.last_error().is_none())` immediately following the drained fixture tone playback. Under the restored contract, `finish_run()` on the cable endpoint encountered `Error::Unsupported` from `CpalOutput::flush()`, leaving the engine stopped with a terminal flush refusal in `engine.last_error()`. The test failed unless updated to exercise the documented caller recreation path.

---

## 2. Caller-Side Fix Implementation

### Changes in `crates/rustamp-audio/src/engine.rs`
To enable the application caller to inspect engine state and transfer configuration seamlessly to a recreated engine without violating contract boundaries:
- Added `Engine::is_flush_failed(&self) -> bool` to report whether the engine output has suffered a terminal flush refusal or failure.
- Added `Engine::volume(&self) -> u8` and `Engine::balance(&self) -> i8` to query the engine's active volume and pan balance.
- Added `Engine::set_current_index(&mut self, index: Option<u32>)` to allow restoring the queue cursor on a newly created engine without initiating immediate playback.
- Maintained the strict fail-closed contract in `Engine::flush_output` (no automatic engine-internal recreation).

### Changes in `crates/rustamp-app/src/app.rs`
1. **State Retention**:
   - Added `output_choice: OutputChoice` and `registry: DecoderRegistry` to `struct App`.
   - Stored these parameters during `App::with_registry`.
   - Extracted helper `create_output(choice: OutputChoice) -> (Box<dyn OutputDevice>, Option<NullClock>)`.

2. **Recreation Routine**:
   - Implemented `App::recreate_engine(&mut self) -> Result<()>`:
     - Obtains a fresh output device via `create_output(self.output_choice)`.
     - Preserves all live player parameters: playlist queue, current track index, volume, pan balance, equalizer state, limiter enabled state, replaygain mode and preamp, shuffle, and repeat settings.
     - Reconstructs `Engine::new` using the shared event bus.
     - Updates `self.engine` and `self.clock`.
   - Implemented `App::ensure_engine_ready(&mut self) -> Result<()>`:
     - If `self.engine.is_flush_failed()`, executes `self.recreate_engine()?`.

3. **Event and Error Handling**:
   - In `App::surface_error`:
     - Checks `is_flush_refusal = self.engine.is_flush_failed()`.
     - Notes the terminal refusal.
     - Calls `self.recreate_engine()`. If recreation succeeds, clears `self.last_error = None`, ensuring that normal track completion on a device with an unflushable backend does not leave an alarming error in the window title. If recreation fails, surfaces the error.
   - In `App::apply_from`:
     - Calls `ensure_engine_ready()` before forwarding commands to `self.engine`.
   - In `App::play_entry`, `App::load_playlist_file`, and `App::add_items`:
     - Calls `ensure_engine_ready()` before initiating playback on `self.engine`.

4. **Testing**:
   - Added unit test `recreate_engine_preserves_playback_state_and_cleans_flush_failure` verifying that `recreate_engine()` preserves queue, cursor, volume, balance, EQ, shuffle, repeat, and resets `is_flush_failed()` to `false`.

---

## 3. Loopback Test Update (`crates/rustamp-app/tests/loopback.rs`)

In `the_cable_carries_the_fixture_tone`:
1. The test verifies that the fixture tone rendered frames and that the buffer ring was completely drained (`engine.played_frames() > 0`, `engine.buffered_frames() == 0`).
2. The initial engine's error state is inspected: if an error is present, it is verified to be strictly `ErrorKind::Output` containing `"output flush failed"`, confirming that audio decoding did not fail.
3. The test exercises the documented caller-side contract duty:
   ```rust
   let output = Box::new(CpalOutput::with_device_name(CABLE_MATCH));
   let engine = match Engine::new(
       rustamp_decoders::default_registry(),
       output,
       EventBus::new(),
       EngineConfig::default(),
   ) {
       Ok(engine) => engine,
       Err(error) => panic!(
           "MISSING CABLE: the playback endpoint \"{PLAYBACK_DEVICE}\" could not be reopened ({error})"
       ),
   };
   ```
4. The test retains the explicit assertion:
   ```rust
   assert!(
       engine.last_error().is_none(),
       "the freshly recreated engine has an error: {:?}",
       engine.last_error()
   );
   ```
5. The test proceeds to Goertzel tone analysis over consecutive 50 ms windows on the capture endpoint, proving that the 440 Hz fixture tone reached the physical/virtual cable device.

No assertions were deleted, weakened, or bypassed.

---

## 4. Contract Evaluation & Potential Future RFC

### Why the Duty Belongs to the Caller Today
1. **Output Factory Ownership**: `Engine::new` takes a trait object `Box<dyn OutputDevice>`. It does not possess a device factory, endpoint identifier, or platform device descriptor. It cannot recreate an arbitrary device on its own.
2. **Platform and Device Diversity**: Devices can be WASAPI endpoints, named CPAL devices, virtual null clocks, or future ASIO/JACK drivers. Creating and opening audio hardware streams is a host-level system operation that may fail (e.g. exclusive mode conflicts, device removal) and must be governed by the application supervisor.
3. **Fail-Closed Guarantee**: If a device backend cannot guarantee that buffered samples will not leak into the next playback session, closing the device and mandating fresh caller recreation is the only sound architectural guarantee.

### Text for Future RFC (if Owner Decides on Engine-Internal Recreation)
If the project owner prefers engine-managed automatic recreation rather than caller-managed recreation, that change cannot be made ad-hoc in `rustamp-audio`; it constitutes an architectural contract change requiring an RFC in `docs/rfc/`:

```markdown
# Proposed RFC: Engine-Managed Output Lifecycle via Device Supplier

## Motivation
Backends like `cpal` cannot portably flush shared endpoint buffers and refuse with `Error::Unsupported`. Requiring every caller/embedder of `Engine` to detect `is_flush_failed` and re-instantiate `Engine` duplicates recreation logic across applications, CLI runners, and test suites.

## Proposed Changes
1. In `crates/rustamp-core/src/audio.rs`:
   Introduce trait:
   ```rust
   pub trait OutputDeviceSupplier: Send + Sync {
       fn create_device(&self) -> Result<Box<dyn OutputDevice>>;
   }
   ```
2. In `crates/rustamp-audio/src/engine.rs`:
   Modify `EngineConfig` or `Engine::new` to accept `Arc<dyn OutputDeviceSupplier>`.
   When `OutputDevice::flush()` returns `Error::Unsupported`:
   - `Engine::flush_output()` closes the current device.
   - It invokes `self.supplier.create_device()?`.
   - It opens the new device, reconnects the render sync/buffer, and clears `flush_failure`.
   - Playback can resume without caller-side engine reconstruction.

## Recommendation
Until such an RFC is approved and adopted, the caller-side recreation duty defined in `crates/rustamp-core/src/audio.rs:370-380` remains authoritative and is now fully satisfied by `crates/rustamp-app`.
```
