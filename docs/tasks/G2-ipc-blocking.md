# Task card G2 — Blocking consumer and producer for the shm rings (RFC 0007)

**Goal**

Implement RFC 0007 (accepted 2026-10-15) in `rustamp-ipc`'s `shm` module: blocking
PCM ring operations backed by two named auto-reset events per ring, so RFC 0006's
lossless decoder and DSP delivery have a wait instead of a poll-plus-drop path. The
non-blocking API stays exactly as RFC 0003 left it; the additions are opt-in and
PCM-only.

**Lane** G — plugin host / IPC (`crates/rustamp-ipc/`, RFC 0007 in `docs/rfc/`).

**Owned paths**
- `crates/rustamp-ipc/src/shm/wake.rs` (new module).
- `crates/rustamp-ipc/src/shm/mod.rs` (exports, two `RingError` variants,
  `RingRefuseReason::WakeUnavailable`, `reason()` mapping).
- `crates/rustamp-ipc/src/shm/ring.rs` (blocking methods and tests on
  `RingProducer`/`RingConsumer`).
- `crates/rustamp-ipc/Cargo.toml` (`Win32_System_Threading` feature).
- `docs/tasks/G2-ipc-blocking.md` (this file).
- Do **not** edit `Cargo.toml` at the repository root and do not touch any other
  crate or spec. No `Message` change (RFC 0007 §10); `PROTOCOL_VERSION` and
  `RING_VERSION` are untouched.

**Allowed sources**
- `docs/rfc/0003-shm-rings.md` and acceptance RFC `docs/rfc/0007-blocking-consumer.md` (the spec to implement).
- `crates/rustamp-ipc/src/shm/**`, `crates/rustamp-ipc/src/message.rs`,
  `crates/rustamp-ipc/src/transport.rs`, `crates/rustamp-ipc/Cargo.toml`.
- Public documentation of the Win32 event API (`CreateEventW`, `OpenEventW`,
  `SetEvent`, `WaitForSingleObject`, `DuplicateHandle`, `CloseHandle`).
- `docs/PLAN.md` section 2 (process diagram and tech stack).

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`. Nothing from them.
- Do not download anything. Do not run a reference binary. No `cargo add`; the
  `windows` crate is already a dependency. No new dependencies.

**Decisions**
- Two-event design from RFC 0007 §4 (not `WaitOnAddress`/`WakeByAddress`): one
  `Wake::Data` event per ring (producer signals, consumer waits) and one
  `Wake::Space` event (consumer signals, producer waits).
- Dedicated `RingRefuseReason::WakeUnavailable` for a failed event open (owner
  decision, RFC 0007 open question O2), not a reuse of `MappingUnavailable`.
- Dedicated `RingError::WakeOpenFailed { op, code }` variant for event syscall
  failures (open question O10), returning `None` from `reason()` (RFC 0007 §11).
- Non-Windows backing: `Arc<(Mutex<bool>, Condvar)>` auto-reset latches for the
  informational CI job only — the real transport is Windows-only.
- `PROTOCOL_VERSION = 2` already in the codebase (RFC 0005/0006); RFC 0007 adds no
  `Message` change, so no further bump is needed. `RING_VERSION` stays 1.

**Interface contract**
- `shm::wake` (new module) owns the Win32 calls; `unsafe` stays inside `shm`;
  every new `unsafe` block carries a `// SAFETY:` comment. `RingWake` is `Send`
  (via `unsafe impl`) so it can cross threads, mirroring `NamedRegion`.
- The hot path stays allocation-free and lock-free: `SetEvent` is a syscall, not a
  sleep, and is called only on the five advance sites listed in RFC 0007 §8 after a
  `Release` store. Signalling is conditional (signals only when the write started
  from empty or the read started from full, per §6.2).
- Sleeping is allowed only inside `wait_readable`, `wait_writable`,
  `read_into_blocking` and `write_from_blocking` — the only calls that may invoke
  `WaitForSingleObject`.
- A blocking call on a wake-less ring returns `RingError::NoWakeObject`. A blocking
  call on a vis ring returns `RingError::WrongRingKind` (kind is checked before the
  wake), so a vis ring can never block.
- `RingError` does **not** implement `PartialEq`; tests that check an error variant
  use `matches!`.

**What was implemented (from `docs/HANDOFF.md` §4)**
- `shm/wake.rs` — `RingWake` with `create`/`open`/`close`/`Clone`/`Drop`,
  `RingWake::signal`/`wait`, `Wake`, `WaitOutcome`, `ring_wake_name`; a single
  `backend` module per platform (no dual `mod imp`, no dead code).
- `shm/mod.rs` — `pub mod wake`, new exports, `RingError::NoWakeObject` +
  `RingError::WakeOpenFailed`, `RingRefuseReason::WakeUnavailable`, `reason()`
  mapping, test updated.
- `shm/ring.rs` — `RingProducer`/`RingConsumer` carry `wake: Option<RingWake>`,
  gain `with_wake`, `wait_writable`/`wait_readable`, `write_from_blocking`/
  `read_into_blocking`, and signal on their non-blocking paths and on `close`.
- `Cargo.toml` — `Win32_System_Threading` feature added; `CloseHandle`,
  `DuplicateHandle`, `WAIT_OBJECT_0`, `WAIT_TIMEOUT`, `HANDLE` imported from
  `Win32::Foundation` (windows crate 0.62.2 moved them there).
- Tests rewritten: `blocking_delivers_every_frame_exactly_once`,
  `blocking_close_wakes_blocked_producer`, `blocking_no_lost_wakeup_under_jitter`,
  `blocking_two_thread_lossless_delivery`, `vis_ring_blocking_is_rejected`
  (corrected to expect `WrongRingKind`), `blocking_close_wakes_waiter_with_closed`
  (`matches!` for the `Closed` error), `pcm_ring_with_wake` helper.

**Acceptance criteria**
- [x] `shm::wake` exists with `RingWake::create`/`open`/`close`, `ring_wake_name`;
      `with_wake` on both ends; the four blocking methods with the signatures of
      RFC 0007 §7; `RingError::NoWakeObject` and `RingError::WakeOpenFailed`.
      Unsafe is confined to `shm`; every new block has a `// SAFETY:` comment.
- [x] Lossless delivery: `blocking_delivers_every_frame_exactly_once` and
      `blocking_two_thread_lossless_delivery` deliver every frame exactly once, in
      order, with `frames_dropped() == 0` and `received == sent`.
- [x] No lost wakeup at the check/wait boundary:
      `blocking_no_lost_wakeup_under_jitter` finishes under a watchdog timeout.
- [x] `close()` wakes both waiters: `blocking_close_wakes_blocked_producer`
      returns `Err(Closed)`; `blocking_close_wakes_waiter_with_closed` drains
      remaining data then returns `Err(Closed)`.
- [x] Timeout: `blocking_timeout_returns_timed_out` returns `Ok(0)`/`TimedOut`
      after ~the requested duration; `blocking_signal_after_timeout_is_not_lost`
      proves a racing signal is not lost.
- [x] Blocking on a wake-less ring returns `NoWakeObject`. Vis ring blocking is
      rejected with `WrongRingKind`.
- [x] Non-blocking path on a wake ring is unchanged: `wake_ring_non_blocking_path_is_unchanged`.
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo build --workspace --all-features` green.
- [ ] `cargo test --workspace --all-features` green (not run on this lane per
      constraint; the CI job covers it).
- [ ] `cargo test -p rustamp-ipc --target i686-pc-windows-msvc` green (i686 build
      of the permissive crate; `RingWake` uses `Arc` cloning, not `DuplicateHandle`).
- [ ] Cross-bitness wake-ring case added to `cargo xtask ring-cross` (32-bit
      producer → 64-bit consumer and the reverse). — *Follow-up, see OWNED PATHS:
      `xtask` is not in this lane's owned paths; the wake API is ready for it.*

**Out of scope / deferred**
- RFC 0006 integration (the hosting `Decoder`/`Dsp` wrappers that call these
  blocking methods) — that is the G2 follow-on / hosting lane.
- Access control for the events (RFC 0007 O6/O2): the default DACL matches the
  section; Q2 must apply whatever it decides to both atomically.
- `WaitOnAddress`/`WakeByAddress` (RFC 0007 O4): rejected by the accepted RFC's
  two-event design.
- Late joiner (RFC 0007 O12): uncovered by the wake.
- Linux/macOS backing primitive (RFC 0007 O11): M6 milestone, not this lane.

**Provenance notes**
- Implementation from `docs/rfc/0007-blocking-consumer.md` and our own RFC 0003
  only. Win32 API usage is documented behaviour cited like RFC 0003 §2 cites it.
- No third-party code was copied; no new dependency was added. The `windows` crate
  was already a dependency.

**Report back**
Files changed, which acceptance criteria are green (fmt/clippy/build verified in
this lane; tests run by CI), and what is deferred to the hosting lane
