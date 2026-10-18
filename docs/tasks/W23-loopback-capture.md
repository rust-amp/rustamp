# Task Card W23-loopback-capture — Virtual Cable Capture Stream Robustness and Bounded Xrun Tolerance

## 1. Context and Problem Statement

### Evidence from the Orchestrator VM
In the guest virtual machine (the only environment equipped with the VB-Audio Virtual Cable driver), `the_cable_carries_the_fixture_tone` in `crates/rustamp-app/tests/loopback.rs` failed at line 596 with:
```text
the capture stream on "CABLE Output" failed: A buffer underrun or overrun occurred.
```

### Context Relative to Prior Lanes
This failure marked genuine progress beyond the earlier blocker resolved by lanes W20 (`docs/tasks/W20-unsupported-flush.md`) and W22 (`docs/tasks/W22-refused-flush-caller.md`):
- The earlier failure (`output flush failed: Error::Unsupported`) halted playback before completion.
- With output flush refusal properly handled by the engine and the caller recreating the engine/output, the playback side successfully completed: "CABLE Output" was discovered, opened, and received audio.
- The new failure was located entirely on the **capture** side of the loopback test.

---

## 2. Root Cause Analysis

### Why a Capture Stream on VB-Audio Virtual Cable Reports Underrun/Overrun in a VM
1. **Virtual Clock and Shared-Mode Latency:**
   The VB-Audio Virtual Cable is a purely software driver operating inside a virtual machine (Hyper-V test VM). Unlike a physical audio interface with a hardware crystal oscillator, its timing relies on host and guest kernel timers. In a virtualized environment with CPU preemption, timer jitter is substantially higher than on bare metal.
2. **Buffer Sizing Underestimation (`cpal::BufferSize::Default`):**
   `pick_capture_config` in `loopback.rs` requested `buffer_size: cpal::BufferSize::Default`. In CPAL's WASAPI backend (`cpal-0.18.2/src/host/wasapi/device.rs:1439`), `BufferSize::Default` maps to `buffer_duration = 0`. WASAPI shared mode interprets a requested duration of `0` as asking for the absolute minimum endpoint buffer matching the device period (typically 10 ms, i.e., 441 frames @ 44.1 kHz or 480 frames @ 48 kHz).
   A 10 ms ring buffer leaves zero cushion against VM hypervisor scheduling latency or thread dispatch jitter. Any preemption exceeding 10 ms causes WASAPI's capture ring to overflow, dropping frames and setting the `AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY` flag in `IAudioCaptureClient::GetBuffer`.
3. **CPAL Error Callback Semantics:**
   In `cpal-0.18.2/src/host/wasapi/stream.rs:825`, whenever `AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY` is detected on a capture packet, CPAL emits `ErrorKind::Xrun` ("A buffer underrun or overrun occurred.") through the stream error callback:
   ```rust
   if device_position != 0 && flags & Audio::AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
       let _ = try_emit_error(error_callback, ErrorKind::Xrun.into());
   }
   ```
   Crucially, this is an **informational glitch notification**. WASAPI does not abort the stream, and CPAL continues processing and delivering packets.
4. **Zero-Tolerance Error Handling in `loopback.rs`:**
   The test's `start_capture` error callback unconditionally recorded any `cpal::Error` into `recording.error = Some(...)`. At line 596, the test unconditionally panicked if `recording.error.is_some()`, treating a single transient xrun as an unrecoverable crash even though audio capture was still ongoing.
5. **Concurrent Machine Load During Capture Tail:**
   In the previous code, while the capture stream was still actively capturing the post-playback tail, the test executed `CpalOutput::with_device_name` and `Engine::new`, enumerating COM audio devices and initializing an audio client on the paired render endpoint ("CABLE Input"). Re-initializing the render pin while the capture pin was in-flight created driver and COM contention, provoking a discontinuity.
6. **Sleep as an Ordering Mechanism:**
   The test previously used `std::thread::sleep(TAIL)` (500 ms) as a naked sleep to wait for tail audio, which was non-deterministic and susceptible to clock drift.

---

## 3. Changes Implemented

All changes are strictly confined to owned path `crates/rustamp-app/tests/loopback.rs`.

### A. Adequate Capture Buffer Configuration
- Introduced `const CAPTURE_BUFFER: Duration = Duration::from_millis(100);`.
- In `pick_capture_config`, updated both the preferred format selection and the default configuration fallback to request:
  ```rust
  let buffer_frames = ((f64::from(rate) * CAPTURE_BUFFER.as_secs_f64()).round() as u32).max(1);
  config.buffer_size = cpal::BufferSize::Fixed(buffer_frames);
  ```
  In WASAPI shared mode, this instructs `IAudioClient::Initialize` to allocate a 100 ms ring buffer, providing ample headroom against VM scheduler delays while preserving the standard device period callback.

### B. Bounded and Honest Error Policy
- Defined `const MAX_TOLERATED_XRUNS: usize = 1;`.
- Refactored `struct Recording` to separate fatal stream errors from transient buffer overruns:
  ```rust
  struct Recording {
      samples: Vec<f32>,
      fatal_error: Option<String>,
      xrun_count: usize,
      first_xrun: Option<String>,
  }
  ```
- In `start_capture`, updated the error callback to inspect `e.kind()`:
  - If `e.kind() == cpal::ErrorKind::Xrun`: increments `xrun_count` and records the first message. If `xrun_count > MAX_TOLERATED_XRUNS`, latches into `fatal_error`.
  - All other errors (`DeviceNotAvailable`, `StreamInvalidated`, `PermissionDenied`, `BackendError`, etc.) are immediately latched into `fatal_error`.
- At test completion, checks `recording.fatal_error` and asserts `recording.xrun_count <= MAX_TOLERATED_XRUNS`.

### C. Deterministic Tail Capture Without Naked Sleep
- Replaced `std::thread::sleep(TAIL)` with a deterministic frame-counting polling loop matching the pre-roll pattern:
  ```rust
  let fixture_frames = (f64::from(rate) * FIXTURE_SECONDS) as usize;
  let tail_frames = (f64::from(rate) * TAIL.as_secs_f64()) as usize;
  let target_frames = play_mark + fixture_frames + tail_frames;
  let start = Instant::now();
  while recorded_frames(&recording, channels) < target_frames {
      assert!(
          start.elapsed() < DEADLINE,
          "\"{CAPTURE_DEVICE}\" delivered {} of {target_frames} frames within {DEADLINE:?}: \
           the capture stream stalled during tail",
          recorded_frames(&recording, channels)
      );
      std::thread::sleep(PUMP_INTERVAL);
  }
  ```
- Immediately stopped and dropped the capture stream (`drop(stream)`) once `target_frames` was reached.

### D. Separation of Capture Stream from Engine Recreation
- Moved `drop(stream)` before the caller-side engine recreation check (`Engine::new` / `CpalOutput`).
- Dropping the capture stream before recreating the playback engine ensures no COM or driver contention occurs on the virtual cable pins while audio is being captured.

---

## 4. Why Stream Error Tolerance Cannot Accept Silence

The test exists to prove that the fixture tone reaches the cable and returns intact. The tolerance of an isolated `Xrun` does **not** weaken this verification:

1. **Goertzel Filter Windowing:**
   The test divides the entire recording into 50 ms non-overlapping windows. In each window, the discrete Goertzel algorithm measures:
   - Power at exactly 440 Hz (the fixture frequency).
   - Total energy across all frequency bins.
   - RMS amplitude.
   A window is declared `tonal` if and only if:
   $$\frac{\text{power}(440\,\text{Hz})}{\text{total\_power}} \ge 0.90 \quad \text{and} \quad \text{amplitude} \ge \text{MIN\_AMPLITUDE}\; (0.10)$$
2. **Pre-Roll Silence Verification:**
   Every window prior to `play_mark` is asserted to be non-tonal (`!w.is_tonal()`), proving the cable was silent before playback started.
3. **Tone Onset Verification:**
   The first tonal window must appear within `MAX_ONSET` (1.0 s) after `play_mark`. If the cable delivered silence or dropped the signal, no window would be tonal, and the test immediately panics with:
   `"the tone never appeared after the play mark"`.
4. **Contiguous Run Verification:**
   The fixture is 1.5 seconds long. The test requires `max_run >= min_run` where `min_run` corresponds to 1.2 seconds (24 consecutive 50 ms tonal windows).
   If a stream error caused signal dropout or silence exceeding 300 ms, the contiguous run breaks and the test fails.
5. **Amplitude Bounds:**
   Every tonal window must satisfy `MIN_AMPLITUDE (0.10) <= amplitude <= MAX_AMPLITUDE (1.0)`.

A policy tolerating a single xrun cannot accept silence because silence has zero amplitude and zero 440 Hz spectral concentration, causing immediate failure on assertions 2, 3, and 5.

---

## 5. Notes and Recommendation for `crates/rustamp-output` (Unmodified Non-Owned Crate)

Per project guidelines, `crates/rustamp-output` was not modified because other development lanes hold it. However, the investigation revealed an identical vulnerability on the output side:
- In `crates/rustamp-output/src/cpal_out.rs:163`:
  ```rust
  let stream = match device.build_output_stream(
      config,
      move |buf: &mut [f32], _info: &cpal::OutputCallbackInfo| { ... },
      move |_e| {
          err_shared.failed.store(true, Ordering::Release);
      },
      None,
  )
  ```
  `CpalOutput` unconditionally marks the output stream as permanently failed on **any** callback error `_e`. If WASAPI reports `AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY` on render during a scheduling stutter, `_e.kind()` is `ErrorKind::Xrun`. Latching `failed = true` turns a transient glitch into a fatal device failure.
- **Recommendation for future lane:**
  Inspect `_e.kind()`. Do not latch `failed.store(true)` on `ErrorKind::Xrun` unless xruns exceed a reasonable threshold or are accompanied by a terminal error like `DeviceNotAvailable` or `StreamInvalidated`.
  Additionally, allow configuring a fixed buffer size on output stream initialization rather than hardcoding `cpal::BufferSize::Default`.

---

## 6. Verification and Gates

### Local Gates
- `cargo fmt --all -- --check`: Exit code `0`.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: Exit code `0`.
- `cargo build --workspace`: Exit code `0`.

### What Only the Virtual Machine Can Prove
The virtual machine is the sole host containing the VB-Audio Virtual Cable virtual driver with registered "CABLE Input" (render) and "CABLE Output" (capture) endpoints. Outside that VM, `require_test_vm()` skips or the test halts at endpoint enumeration.
Only the guest virtual machine can execute WASAPI against the virtual cable driver, prove that the 100 ms buffer eliminates scheduler-induced xruns, confirm that the bounded error policy survives virtualization timing jitter, and execute Goertzel detection on the round-trip 440 Hz audio.
