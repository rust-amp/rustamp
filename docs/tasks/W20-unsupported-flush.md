# W20 unsupported flush — handling endpoint buffer flush capability gaps honestly

**Lane:** W20 unsupported flush  
**Status:** complete  
**Owned paths:** `crates/rustamp-audio/**`, `crates/rustamp-app/**`, `docs/tasks/W20-unsupported-flush.md`

---

## 1. Defect Description and Evidence

### Verbatim Evidence
When running `the_cable_carries_the_fixture_tone` in `crates/rustamp-app/tests/loopback.rs` against a real audio device (VB-Audio Virtual Cable) inside the test environment:
```text
the decode failed: Some(ErrorInfo { kind: Output, message: "output error on Lautsprecher (VB-Audio Virtual Cable), output flush failed, unsupported operation, cpal endpoint-buffer flush, close and recreate the output stream instead, output closed, recreate the engine/output before playback" })
```
The test panicked at line 543 on `assert!(engine.last_error().is_none())`.

### Root Cause
1. At the completion of playback (1.5 s sine fixture), the decode loop reaches `EndOfStream`.
2. `Engine::poll` invokes `Engine::finish_run()`.
3. `finish_run()` calls `self.flush_output()`.
4. `flush_output()` calls `<CpalOutput as OutputDevice>::flush()`.
5. For `CpalOutput` (`crates/rustamp-output/src/cpal_out.rs:381`), `flush()` returns:
   `Err(Error::Unsupported("cpal endpoint-buffer flush; close and recreate the output stream instead"))`.
6. Under task A13's policy, `Engine::flush_output` treated any error from `self.output.flush()`, including `Error::Unsupported`, as fatal:
   it paused the output, closed the output device, latched `self.flush_failure`, and returned `Err(Error::Output(...))`.
7. `finish_run()` published `Event::Error(ErrorKind::Output)` and populated `self.error`.
8. Furthermore, `Engine::stop()` and `Engine::seek_frames()` also called `flush_output()`, meaning any seek or stop on a cpal endpoint permanently destroyed the engine output.
9. Sibling test `real_playback` passed on the exact same endpoint because it never checked `engine.last_error()`. The device opened and accepted the stream without issue; the failure was entirely caused by the player treating an unsupported endpoint flush as fatal.

---

## 2. Analysis of Candidates and Architectural Decision

### Candidate 1: Close and recreate the output stream
*Discarded as dishonest and unviable.*
- The `OutputDevice` trait has no stream recreation API. In `CpalOutput`, `start()` consumes `self.device.take()` and `self.config.take()`, and `close()` drops backend handles. Once closed, an `OutputDevice` cannot be reopened.
- Tearing down and reconstructing the OS audio stream on every seek, stop, or track end would introduce massive driver teardown/init latency, audible clicks and pops, reset the hardware audio clock (`played_frames`), destroy the real-time audio thread, and break gapless/timeline synchronization.
- Crucially, the audio endpoint is healthy and operational. Tearing down a working device because a buffer drop is unsupported is a destructive misinterpretation of a capability gap.

### Candidate 2: Skip endpoint flush when unsupported (`Error::Unsupported`)
*Selected as the honest, correct behavior.*
- `Error::Unsupported` is defined in `crates/rustamp-core/src/error.rs` to represent a capability gap, not an operational failure.
- During any seek, stop, or end-of-stream event, `rustamp-audio` already flushes and drains its internal lock-free ring buffer (`rtrb`) and DSP chain.
- The output device flush (`OutputDevice::flush`) is an additional request to drop any samples already rendered into the driver or hardware buffer. On backends like WASAPI, hardware buffer reset is supported; on backends like cpal, portable endpoint buffer resetting is not possible.
- Skipping endpoint flush on `Error::Unsupported` leaves the stream open, healthy, and usable. The small hardware backlog (e.g. 5-20 ms) drains naturally, but no old audio is rendered from the ring.
- **Real errors stay loud:** Any genuine device failure (`Error::Output`, driver disconnect, I/O errors) continues to close the output, latch `flush_failure`, block restart paths, and publish `Event::Error(ErrorKind::Output)`.

---

## 3. Required Changes in `crates/rustamp-core` and Contracts (Documented Only)

Per the task constraints, files outside the owned paths (`crates/rustamp-core` and existing RFC documents) were not modified. The exact required documentation updates are recorded below:

### `crates/rustamp-core/src/audio.rs` (lines 375–385)
**Current text:**
```rust
/// # What a caller must assume
///
/// The engine treats a refusal as terminal for the current output: it stops
/// the run and does not resume into an output it cannot clean, so the
/// caller (the application) has to recreate the engine and the output
/// before it can play again. There is no automatic recovery, and a
/// successful flush is not a promise that the hardware falls silent at that
/// instant - only that what was buffered will not be heard as the next
/// audio. (W10 finding F3, A13.)
```
**Required replacement:**
```rust
/// # What a caller must assume
///
/// An unsupported flush (`Error::Unsupported`) indicates a backend capability
/// gap: the engine does not treat this as fatal, leaves the stream open and
/// healthy, and continues playback without an endpoint buffer reset. Genuine
/// output failures (`Error::Output`, device loss, etc.) remain fatal: the engine
/// deactivates the ring, pauses and closes the output, latches a restart barrier,
/// and requires recreating the engine/output before playback can resume.
```

### `docs/tasks/A13-flush-contract.md`
Add a note referencing W20 clarifying that A13's engine policy (which treated `Error::Unsupported` identically to a fatal hardware failure) was refined to distinguish capability gaps from genuine device errors.

---

## 4. Code Changes Made

### `crates/rustamp-audio/src/engine.rs`
1. Updated `Engine::flush_output`:
   ```rust
   match self.output.flush() {
       Ok(()) => Ok(()),
       Err(Error::Unsupported(_)) => {
           // Capability gap: the backend cannot drop its hardware endpoint
           // backlog, but the stream is healthy. Proceed without endpoint flush.
           Ok(())
       }
       Err(err) => {
           // Genuine output failure: deactivate ring, pause, close, latch failure barrier.
           ...
       }
   }
   ```
2. Updated `Engine::stop()` doc comment to note that unsupported flushes leave the device open.

### `crates/rustamp-audio/tests/flush_contract.rs`
1. Enhanced `Endpoint` and `BufferedOutput` to support both `refuse_flush: bool` (`Error::Unsupported`) and `fail_flush: bool` (`Error::Output`).
2. Added `Harness::with_flags` to configure seek refusal, flush refusal (`Unsupported`), flush failure (`Output`), and track length.
3. Updated the three real-error contract tests to use `fail_flush: true` to guarantee that genuine output failures remain loud, close the output, and block restart paths:
   - `a_failed_flush_on_seek_closes_output_and_blocks_all_restart_paths`
   - `stop_and_stopped_seek_report_a_failed_flush_instead_of_silent_success`
   - `natural_end_keeps_its_position_and_reports_a_failed_flush`
4. Added three new contract tests pinning behavior on backends where flush is unsupported (`refuse_flush: true`):
   - `an_unsupported_flush_on_seek_continues_playback_without_error_or_close`: verifies that seek succeeds, output remains open (`closes == 0`), transport remains `Playing`, no error is published, and post-seek audio is rendered.
   - `stop_and_stopped_seek_succeed_on_unsupported_flush`: verifies that `stop()` and stopped seek return `Ok(())`, leave output open, report no error, and allow playback to restart.
   - `natural_end_on_unsupported_flush_finishes_cleanly_without_error`: verifies that natural end transitions to `Stopped`, leaves output open, sets no `last_error`, and allows subsequent tracks to play.

---

## 5. Verification

The following gates were run and passed cleanly:
- `cargo fmt --all -- --check` (Exit code 0)
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` (Exit code 0)
- `cargo build --workspace` (Exit code 0)

Per task instructions, `cargo test` was not run.
