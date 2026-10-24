# Task card H4 — Output device mid-run error hook RFC (RFC 0012)

**Goal**  
Draft the contract RFC `docs/rfc/0012-output-error-hook.md` for `rustamp-core` and the audio engine to handle mid-run output device failures (device disconnected, format revoked, buffer write failures). This addresses the gap identified in task H3: *"the OutputDevice trait and mid-run error hook needs an RFC"*. Design only — no implementation or behavior change.

**Lane**  
Lane I (contracts) / Lane H (output robustness).

**Owned paths**  
- `docs/rfc/0012-output-error-hook.md` (new RFC draft)
- `docs/tasks/H4-output-error-rfc.md` (this task card)  
*No other file may be created or modified.*

**Allowed sources**  
- `specs/**`
- `docs/rfc/0001-core-contracts.md`, `docs/rfc/0002-audio-contracts.md`, `docs/rfc/0004-plugin-licensing.md`
- `docs/tasks/H3-output-robustness.md`
- `crates/rustamp-core/src/audio.rs` (the `OutputDevice` trait and `RenderFn`)
- `crates/rustamp-core/src/bus.rs` (`EventBus` and publish rules)
- `crates/rustamp-core/src/error.rs` and `crates/rustamp-proto/src/event.rs`
- `crates/rustamp-output/src/**` (`wasapi_out.rs`, `cpal_out.rs`, `null.rs`, `lib.rs`)
- `crates/rustamp-audio/src/engine.rs`

**Clean room**  
Never read Winamp source, SDK headers, disassembly, `rustamp-research`, or `rustamp-refbin`.

---

## Findings & Architectural Options

Task H3 added backend-local error latches:
- WASAPI: `FailureStreak` with `MAX_CONSECUTIVE_FAILURES = 64` latches `ERR_GONE` into `shared.error: AtomicU8` and exposes `WasapiOutput::stream_error(&self) -> Option<&'static str>`.
- cpal: Stream error callback latches `shared.failed: AtomicBool` and exposes `CpalOutput::has_failed(&self) -> bool`.

However, the `OutputDevice` trait has no method to query this health status, and `Engine::poll` never inspects the output device. A mid-run failure leaves the engine in `TransportState::Playing` forever with the decode thread blocked on a full ring.

The RFC evaluates three options:
1. **Polled accessor on `OutputDevice` (Recommended)**:  
   `fn stream_error(&self) -> Option<Error> { None }` added to `OutputDevice`. The engine calls it on the control thread during `Engine::poll()`. Audio thread only stores an atomic integer/bool; no locks, no heap allocations, no I/O.
2. **Asynchronous callback passed to backend**:  
   Backend invokes callback from audio thread. Fails real-time purity constraints (heap allocations, lock acquisition, or logging), and risks deadlocks with control thread commands.
3. **Backend publishes to `EventBus`**:  
   Directly violates RFC 0001 and `bus.rs` invariant: *"do not publish from the audio thread"*. Causes state desynchronization between engine and bus subscribers.

---

## Deliverable Summary: RFC 0012

RFC 0012 specifies:
1. **Contract extension**: Add `stream_error(&self) -> Option<Error>` with default `None` to `rustamp_core::audio::OutputDevice`.
2. **Backend implementations**:
   - `WasapiOutput`: translates `shared.error` into `Some(Error::Output(format!("{}: {msg}", self.device_name)))`.
   - `CpalOutput`: translates `shared.failed` into `Some(Error::Output(format!("{}: device lost or stream error", self.device_name)))`.
   - `NullOutput`: reads an atomic latch controlled by a new test helper `NullClock::fail()`.
3. **Engine policy**: On device loss, the engine stops the run immediately (`join_decode()`), pauses output, transitions transport to `TransportState::Stopped`, freezes timeline position, and publishes `Event::StateChanged(TransportState::Stopped)` and `Event::Error(ErrorInfo)`. It does **not** silently play into nothing, and does **not** silently switch devices.
4. **Wire & compatibility impact**: Zero wire impact. Additive method on `OutputDevice` preserves backwards compatibility.
5. **Licensing**: Permissive crates remain untouched; license boundaries intact.
6. **Test plan**: Fully deterministic unit tests using `NullOutput::pair()` and `NullClock::fail()` without requiring audio hardware.

---

## Verification & Acceptance

- [x] RFC 0012 written in house style at `docs/rfc/0012-output-error-hook.md`.
- [x] Task card written at `docs/tasks/H4-output-error-rfc.md`.
- [x] Strict boundary rule honoured: no files touched outside the worktree.
- [x] No code modified; no commit made.
- [x] `cargo fmt --all -- --check` clean.
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [x] `cargo build --workspace` succeeds.
- [x] `cargo test` NOT run (host test execution forbidden; VM-only rule).
