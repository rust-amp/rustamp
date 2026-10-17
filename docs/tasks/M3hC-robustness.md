# Task card M3hC — Close Group C CODEX review findings (6 to 12)

**Background**
An independent review (OpenAI CODEX, review-only) of the Rustamp out-of-process plugin host implementation identified robustness gaps in `rustamp-plughost` and `rustamp-app`. This card documents the resolution of Group C (findings 6 through 12).

**Allowed sources**
`specs/S10-ipc.md`, `specs/S09-plugin-abi.md`, `docs/rfc/0006-out-of-process-hosting.md`, `docs/rfc/0009-ipc-version-constant.md`.
**Clean room:** Never read Winamp source, SDK headers, disassembly, 2.95 binary, `rustamp-research` or `rustamp-refbin`.

---

## Findings and Implementations

### Finding 6 — Pong on reader thread and UI-independent parent-death termination
- **Defect (`crates/rustamp-plughost/src/control.rs:60`, `crates/rustamp-plughost/src/main.rs:188`):**
  Previously, `run_reader` in `control.rs` answered `Message::Ping(token)` immediately on the background reader thread by sending `Message::Pong(token)` directly back over the transport writer. Consequently, if the UI thread was hung in a third-party plugin window procedure or infinite loop, the reader thread still responded to parent pings, making a completely frozen host look healthy to the parent supervisor. Additionally, if the parent died or closed the stdin pipe while the UI thread was blocked, there was no UI-independent termination mechanism to guarantee the orphan host process exited.
- **Fix:**
  1. In `crates/rustamp-plughost/src/control.rs`: Introduced `ControlInbound` enum distinguishing `State(PlayerState)` updates from `Ping(u64)` queries. Added `run_reader_events` which forwards incoming `Ping` tokens to the caller rather than auto-answering them on the reader thread.
  2. In `crates/rustamp-plughost/src/main.rs`: Split I/O so that stdout writes are handled by a dedicated writer thread. Incoming `ControlInbound::Ping(token)` messages are sent across a bounded sync channel to the UI message loop. The UI loop answers with `Message::Pong(token)` only when actively pumping messages. If the UI thread hangs, pings are not answered, properly tripping the parent supervisor's liveness deadline (`PONG_TIMEOUT`).
  3. Parent-death watchdog: When the reader thread encounters EOF (`ChannelClosed`) on stdin, it immediately posts `WM_QUIT` to the host window and spawns an independent watchdog thread that waits 300 ms and then calls `std::process::exit(0)`. Even if the UI thread is deadlocked inside a plugin, the orphaned process terminates cleanly.
- **Tests:**
  `crates/rustamp-plughost/src/control.rs`: `run_reader_events_forwards_pings_and_states_without_auto_answering_pong`.

### Finding 7 — WM_QUIT stranded join on blocked stdin reader and exit reason distinction
- **Defect (`crates/rustamp-plughost/src/main.rs:218-223`):**
  When `WM_QUIT` was received by the UI message loop, `main` attempted `reader.join()`. However, the background reader thread was blocked in a synchronous `ReadFile` / `std::io::stdin().read()` on the anonymous pipe. Because anonymous pipes on Windows do not cancel blocking reads merely upon dropping handles in user space, `reader.join()` stranded indefinitely. Furthermore, the loop did not distinguish exit reasons (`ReaderDone` vs `WmQuit`).
- **Fix:**
  1. In `crates/rustamp-plughost/src/main.rs`: Introduced `enum UiLoopExit { ReaderDone, WmQuit }` returned by `run_ui_loop`.
  2. On Windows, the reader thread's native OS thread handle is duplicated via `DuplicateHandle` / `GetCurrentThread`.
  3. When `run_ui_loop` exits with `UiLoopExit::WmQuit`, `main` calls `CancelSynchronousIo` on the duplicated thread handle to abort any pending blocking `ReadFile` on stdin before invoking `reader.join()`.
  4. If `run_ui_loop` exits with `UiLoopExit::ReaderDone`, the reader thread has already reached EOF and exited, so `reader.join()` completes immediately without stranded I/O.
- **Tests:**
  `run_reader_events_forwards_pings_and_states_without_auto_answering_pong` verifies clean exit on channel close; `main.rs` unblock cancellation logic guarded by `CancelSynchronousIo`.

### Finding 8 — Hello is not a barrier: greeting requirement and handshake deadline
- **Defect (`crates/rustamp-app/src/supervise.rs:596-608`):**
  In `Supervisor::supervise`, upon spawning a host child, the link was immediately attached (`link.attach(transport)`) and the child marked live before any greeting had been received. Subsequent code could immediately publish `PlayerStateUpdate` before the host had announced itself, violating protocol handshaking. Furthermore, an incompatible protocol version was not rejected prior to link attachment.
- **Fix:**
  1. In `crates/rustamp-app/src/supervise.rs`: Added `PendingHost` holding `(transport, process, deadline)` and `DEFAULT_HANDSHAKE_TIMEOUT` (3 seconds).
  2. Spawning a new host places it into `self.pending` while keeping `link` detached.
  3. The supervisor enforces a strict Hello barrier: `link.attach()` is called ONLY after a valid `Message::Hello` with matching `protocol_version == rustamp_ipc::PROTOCOL_VERSION` is received.
  4. If an incompatible protocol version arrives, an unexpected message arrives, or the handshake deadline expires before Hello, the pending host is dropped, failure is recorded, and the link remains detached.
- **Tests:**
  - `crates/rustamp-app/src/supervise.rs`: `hello_barrier_prevents_attach_until_compatible_greeting`
  - `crates/rustamp-app/src/supervise.rs`: `hello_barrier_rejects_incompatible_protocol_version`

### Finding 9 — Unbounded inbound queues and unbounded drain-until-empty
- **Defect (`crates/rustamp-app/src/supervise.rs:260`, `crates/rustamp-plughost/src/main.rs:188`):**
  `spawn_host_reader` in `supervise.rs` used unbounded `mpsc::channel()`, which could grow without limit if the host flooded frames. Similarly, `rustamp-plughost/src/main.rs` had unbounded queues and ran unbounded drain loops.
- **Fix:**
  1. Host side (`crates/rustamp-plughost/src/main.rs`):
     - Inbound channel bounded by `INBOUND_QUEUE_BOUND = 16`.
     - Outbox channel bounded by `OUTBOX_QUEUE_BOUND = 16`.
     - UI message loop enforces `PUMP_WORK_BUDGET = 16` per pump iteration.
     - Snapshot coalescing: when draining inbound updates during a pump, snapshot updates are coalesced so that only the latest `PlayerState` is applied to the bridge, preventing backlog processing storms.
  2. App side (`crates/rustamp-app/src/supervise.rs`):
     - `spawn_host_reader` bounded by `HOST_RECV_QUEUE = 16` (`mpsc::sync_channel(HOST_RECV_QUEUE)`).
- **Tests:**
  - `crates/rustamp-app/src/supervise.rs`: `a_stalled_host_fills_the_outbox_without_stalling_the_send`
  - `crates/rustamp-plughost/src/control.rs`: `run_reader_events_forwards_pings_and_states_without_auto_answering_pong`

### Finding 10 — Gating real-spawn and end-to-end tests with VM predicate
- **Defect (`crates/rustamp-app/src/supervise.rs:1097`, `crates/rustamp-app/tests/plughost_e2e.rs:210`):**
  Real-spawn and end-to-end tests relied solely on `#[ignore]`. Running `cargo test -- --ignored` off-VM on a host machine would spawn real processes and create top-level Win32 windows, violating the hard project GUI rule.
- **Fix:**
  1. In `crates/rustamp-app/src/supervise.rs`: Added `crate::window::require_test_vm()` check at the start of `spawns_the_real_host_without_a_window`.
  2. Note on scope boundary: `crates/rustamp-app/tests/plughost_e2e.rs` is outside the strictly assigned file ownership list for this lane (`crates/rustamp-plughost/src/**`, `crates/rustamp-plughost/tests/**`, `crates/rustamp-app/src/supervise.rs`, `docs/tasks/M3hC-robustness.md`). The gate requirement for `plughost_e2e.rs` is noted in this report for follow-up by the lane owning integration tests.
- **Tests:**
  `spawns_the_real_host_without_a_window` in `crates/rustamp-app/src/supervise.rs`.

### Finding 11 — Binary discovery architecture validation (enforcing i686)
- **Defect (`crates/rustamp-app/src/supervise.rs:135-180`):**
  `find_plugin_host` searched for files named `rustamp-plughost32.exe`, but did not inspect the PE header. An x64 binary with that filename (e.g. from an erroneous target build or manual placement) would be selected and spawned, failing 32-bit plugin loading.
- **Fix:**
  1. In `crates/rustamp-app/src/supervise.rs`: Defined `IMAGE_FILE_MACHINE_I386 = 0x014c`.
  2. Implemented `is_i686_reader<R: Read + Seek>` and `is_i686_executable(path: &Path) -> bool` validating:
     - DOS `MZ` signature and PE header offset at `0x3C`.
     - PE signature `PE\0\0`.
     - COFF machine field matching `IMAGE_FILE_MACHINE_I386` (0x014c).
  3. Integrated `is_i686_executable` check into `find_plugin_host()` and `ProcessBackend::spawn()`.
- **Tests:**
  `crates/rustamp-app/src/supervise.rs`: `pe_architecture_validation_accepts_i686_and_rejects_other`.

### Finding 12 — Stop state persistence and persistent-failure policy
- **Defect (`crates/rustamp-app/src/supervise.rs:580-590`):**
  Calling `Supervisor::stop(&mut link)` killed the child and detached the link, but did not record a stopped state. The very next call to `Supervisor::supervise(&mut link)` (e.g. on the next audio pump) treated the detached link as a host failure and immediately re-spawned the child. Additionally, repeated spawn failures would spin indefinitely with only the restart cooldown.
- **Fix:**
  1. In `crates/rustamp-app/src/supervise.rs`: Introduced `SupervisorState` enum (`Running`, `Stopped`, `PersistentFailure`).
  2. Added `DEFAULT_MAX_CONSECUTIVE_FAILURES = 3`.
  3. `Supervisor::stop(&mut link)` transitions to `SupervisorState::Stopped`. While in `Stopped`, `supervise` returns immediately without attempting any spawn.
  4. Added `Supervisor::resume()` to explicitly reset failure counters and return to `Running`.
  5. If consecutive spawn/handshake failures reach `DEFAULT_MAX_CONSECUTIVE_FAILURES`, the supervisor transitions to `SupervisorState::PersistentFailure`, halting further spawn attempts until `resume()` is called.
- **Tests:**
  - `crates/rustamp-app/src/supervise.rs`: `stop_sticks_and_subsequent_supervise_does_not_restart_until_resumed`
  - `crates/rustamp-app/src/supervise.rs`: `persistent_failure_halts_retries_after_max_failures`

---

## Verification
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo build --workspace`
- Zero external worktree modifications confirmed via `git status --short`.
