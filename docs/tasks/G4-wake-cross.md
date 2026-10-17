# Task card G4 — Cross-bitness blocking wake check (RFC 0007 §14.9)

> **Orchestrator notes (2026-10-17).**
> Completes RFC 0007 acceptance criterion 9 (deferred from G2): cross-process,
> cross-bitness verification of the blocking wake across `i686` and `x86_64`.

**Goal**

Prove the RFC 0007 blocking wake across the 32-bit / 64-bit process boundary in
both directions (`i686 -> x86_64` and `x86_64 -> i686`) via a headless,
two-process check driven by `cargo xtask wake-cross`:
1. Prove that a blocked reader (`RingConsumer::read_into_blocking` in
   `WaitForSingleObject` on `Wake::Data`) is woken across the process boundary
   by a writer advancing `write_seq` and signaling `Wake::Data`.
2. Prove that a blocked writer (`RingProducer::write_from_blocking` in
   `WaitForSingleObject` on `Wake::Space`) is woken across the process boundary
   by a consumer advancing `read_seq` and signaling `Wake::Space`.
3. Prove that timeout precision is honoured within physical OS kernel timer
   bounds without premature returns (< 1 ms) or hangs.
4. Prove that `RingProducer::close()` wakes a blocked consumer immediately with
   `Err(RingError::Closed)`.
5. Prove that the 32-bit and 64-bit architectures agree on shared memory header
   layout, atomics, and sample slot payload values without loss or corruption.
6. Fail loudly (non-zero exit and message naming the failing step) if any wake
   is lost, timeout dishonoured, or layout mismatched.

**Lane** G — plugin host / IPC (`crates/rustamp-ipc/`, `xtask/`).

**Owned paths**
- `xtask/src/wake_cross.rs` (new module: orchestrator).
- `xtask/src/main.rs` (subcommand registration).
- `crates/rustamp-ipc/examples/ring_peer.rs` (`wake-produce` and `wake-consume` modes).
- `crates/rustamp-ipc/src/shm/**` (monitored; no changes required as G2/G3 wake held).
- `docs/tasks/G4-wake-cross.md` (this card).

**Allowed sources**
- `docs/rfc/0007-blocking-consumer.md` (RFC 0007 blocking consumer contract).
- `docs/rfc/0003-shm-rings.md` (shared memory ring protocol).
- `crates/rustamp-ipc/src/shm/**`.
- `xtask/src/ring_cross.rs` (cross-process test harness pattern).
- Public Win32 API documentation (`CreateEventW`, `OpenEventW`, `SetEvent`, `WaitForSingleObject`).

**Forbidden sources**
- Winamp source code, SDK headers, disassembly, dirty references.
- No network access, no downloads, no new dependencies.
- No GUI windows, no audio output.

**Decisions**
- Built as a dedicated sibling subcommand `cargo xtask wake-cross` rather than
  overloading `cargo xtask ring-cross`: `ring-cross` continues to verify pure
  non-blocking bulk throughput for PCM and Vis, while `wake-cross` verifies
  the five blocking wake phases without regression.
- Both directions are executed:
  1. `i686` (32-bit producer) -> `x86_64` (64-bit consumer)
  2. `x86_64` (64-bit producer) -> `i686` (32-bit consumer)
- Five distinct phases tested per direction:
  - `timeout`: verifies wait on empty ring returns `Ok(0)` in elapsed time >= 1 ms.
  - `reader-wake`: reader enters kernel wait on `Wake::Data`; writer writes 16 frames; reader wakes and verifies data.
  - `writer-wake`: writer fills ring to capacity (16 frames) and blocks on `Wake::Space` for 8 more; reader drains 16 frames; writer wakes and finishes; reader verifies data.
  - `bulk-lossless`: 256 frames transferred through the wake ring with `write_from_blocking` and `read_into_blocking`; verifies 0 frames dropped and mathematical sample equality.
  - `close-wake`: reader blocks on empty ring; writer calls `close()`; reader is woken immediately and returns `Err(RingError::Closed)`.
- Reused the existing `ring_peer` example binary in `rustamp-ipc` with new
  `wake-produce` and `wake-consume` subcommands to allow building for both targets
  via Cargo without modifying crate dependencies.

**What was implemented**
- `crates/rustamp-ipc/examples/ring_peer.rs`:
  Added `wake-produce` and `wake-consume` subcommands using `RingWake::create` and
  `RingWake::open`, with an interactive stdin/stdout protocol for driving discrete
  wake phases (`TEST_TIMEOUT`, `READ_BLOCK`, `WRITE`, `FILL_AND_BLOCK`, `DRAIN`,
  `BULK_WRITE`, `BULK_READ`, `WAIT_CLOSE`, `CLOSE`, `QUIT`).
- `xtask/src/wake_cross.rs`:
  Created orchestrator that builds `ring_peer` for `x86_64-pc-windows-msvc` and
  `i686-pc-windows-msvc`, launches them headless (`CREATE_NO_WINDOW`), drives the
  five phases for both directions, measures timeouts, validates all sample values,
  and prints a summary table.
- `xtask/src/main.rs`:
  Registered `wake-cross` in command dispatcher and usage message.
- `crates/rustamp-ipc/src/shm/**`:
  Reviewed and verified across bitness. Zero code modifications required — the
  implementation from G2/G3 functioned flawlessly across processes.

**Acceptance criteria**
- [x] Sibling subcommand `cargo xtask wake-cross` exists and runs headless on Windows.
- [x] Exercises blocking wake in both directions (`i686 -> x86_64` and `x86_64 -> i686`).
- [x] Blocked reader is woken across process boundary by writer.
- [x] Blocked writer is woken across process boundary by reader on space.
- [x] Timeout precision is verified and honoured (elapsed >= 1 ms, bounded <= 500 ms).
- [x] Peer `close()` wakes blocked consumer with `Err(RingError::Closed)`.
- [x] Fails loudly with non-zero exit code naming the exact failing phase upon failure.
- [x] Existing `cargo xtask ring-cross` remains green and unchanged.
- [x] `cargo fmt --all -- --check` clean.
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [x] `cargo build --workspace` clean.
- [x] No `cargo test` executed on host (Hyper-V test VM reserved).
- [x] No GUI windows opened, no audio played, no git commits made.

**Provenance**
All implementation developed clean-room from RFC 0007, RFC 0003, and public
Win32 synchronization APIs. Zero Winamp source code, SDK headers, or disassembly
consulted.
