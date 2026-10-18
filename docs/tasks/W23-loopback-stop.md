# W23 — Loopback stop must assert the terminal cpal flush refusal

## Scope and provenance

- Worktree: `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave115-loopbackstop`.
- Owned paths: `crates/rustamp-app/**` and this card; the only code change is
  `crates/rustamp-app/tests/loopback.rs`.
- Write only these owned files in this worktree, never under `D:\code\Rustamp`.
- Sources: `AGENTS.md`, `docs/HANDOFF.md`, `CLEANROOM.md`, `docs/PLAN.md`,
  `CONTRIBUTING.md`, W23-loopback-capture, W22-refused-flush-caller,
  W22-two-reds, and the project's audio contract and implementation.
- No Winamp source, SDK material, disassembly, or research repository was read.
  No third-party code, downloads, dependencies, production changes, or git writes.

## Reported VM evidence (not reproduced by this lane)

The orchestrator reports that `the_cable_carries_the_fixture_tone` now passes
all tone assertions after the capture-buffer repair. It fails only at the old
final `engine.stop().expect("stopping the engine after the run")`, with the
terminal cpal endpoint-buffer flush refusal naming output closure and required
engine/output recreation. This is progress, not evidence of a product defect.

## Contract and mistaken expectation

`OutputDevice::flush` in `crates/rustamp-core/src/audio.rs:355-374` requires
`Error::Unsupported` when a backend cannot guarantee that buffered frames will
not be heard as the next audio. The engine stops, closes that output, latches
the terminal failure, and never resumes into it. The caller must recreate both
engine and output before further playback; instantaneous hardware silence and
automatic recovery are not promised.

This test explicitly selects `CpalOutput`, whose `flush()` always refuses with
`Unsupported("cpal endpoint-buffer flush; close and recreate the output stream instead")`.
Natural completion already encounters that refusal. The test previously allowed
an absent completion error or any output error containing `output flush failed`,
then incorrectly demanded successful `stop()` on that same terminal engine.
Creating a separate fresh engine does not repair the old engine.

## Changes and stronger assertions

1. Require `last_error()` after natural completion to equal an `ErrorInfo` with
   `ErrorKind::Output` and the complete expected diagnostic: the selected device,
   unsupported cpal endpoint-buffer flush, output closure, and caller recreation.
   Missing errors, other unsupported operations, decoder/stream failures, generic
   flush failures, and appended close failures cannot satisfy that equality.
2. Require `is_flush_failed()` and `TransportState::Stopped` on the old engine.
3. Retain caller-side creation of a fresh `CpalOutput` and `Engine`, after capture
   is dropped. Require no error, no terminal flush latch, a cable endpoint by
   name (never a default-device fallback), and stopped transport. This checks
   fresh construction, not resumed playback; no second playback is attempted.
4. Retain the final explicit `stop()` call, but require `Err(Error::Output(...))`
   with exactly the same latched diagnostic. Require unchanged `last_error()`,
   a still-set `is_flush_failed()`, and stopped transport afterwards. Success
   or any unrelated error now fails rather than bypassing the assertion.
5. Correct the module documentation's former no-error completion claim.

All cable discovery, rendered/drained-frame checks, capture-health checks,
pre-roll, onset, contiguous duration, maximum duration, and per-channel tone
amplitude/frequency checks remain unchanged. Silence still fails. No sleep or
other timing-based ordering mechanism was introduced. No core/audio/output
change or RFC is needed.

## Verification

Local gates run offline/locked where applicable, with build artifacts confined
to this worktree and each exit code captured immediately after its command:

- `cargo fmt --all -- --check`: exit `0` (`$fmtExit`).
- `cargo clippy --offline --locked --workspace --all-targets --all-features -- -D warnings`:
  exit `0` (`$clippyExit`); includes compilation/checking of the integration test.
- `cargo build --offline --locked --workspace`: exit `0` (`$buildExit`).
- `cargo test`: deliberately not run; this lane did not run the VM or broker.

Only the orchestrator's VM interactive session can execute the actual cable
round trip and prove both the unchanged tone assertions and these lifecycle
assertions against the driver, including successful fresh endpoint creation.
Host formatting/lint/build success is not runtime or VM evidence. This lane
cannot independently reproduce the reported guest failure or claim a VM pass.
