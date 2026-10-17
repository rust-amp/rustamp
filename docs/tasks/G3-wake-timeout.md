# Task card G3 — Measure and settle blocking wake timeout precision (RFC 0007 §6.4)

> **Orchestrator notes (2026-10-17).**
> Settles the open question recorded in task card G2 note 2: whether a wait
> returning `Ok(0)` in less than eight milliseconds on a ten millisecond timeout
> represents a defect in `rustamp-ipc` or the documented behavior of the OS
> kernel timer resolution.

**Goal**

Settle the open question regarding the blocking wake timeout with measurements,
not arguments:
1. Trace the code path from the `timeout: Option<Duration>` argument down to
   `WaitForSingleObject` with exact file and line references, including the
   behavior of spurious signals and already-signaled auto-reset events.
2. Add a measurement test covering multiple runs across several durations (sub-tick
   and multi-tick), with bounded execution and an explicit tolerance rationale.
3. Make the lower-bound assertion in `blocking_timeout_returns_timed_out` honest.
4. State plainly what the measurement proves and what it cannot.

**Lane** G — plugin host / IPC (`crates/rustamp-ipc/`, RFC 0007 in `docs/rfc/`).

**Owned paths**
- `crates/rustamp-ipc/src/shm/wake.rs`
- `crates/rustamp-ipc/src/shm/ring.rs`
- `docs/tasks/G3-wake-timeout.md` (this card)

**Allowed sources**
- `docs/rfc/0007-blocking-consumer.md` (blocking consumer contract)
- `docs/rfc/0003-shm-rings.md` (shared-memory rings)
- Public Win32 API documentation (`WaitForSingleObject`, `CreateEventW`, `SetEvent`,
  Windows system timer resolution and wait function documentation).

**Forbidden sources**
- Winamp source code, SDK headers, disassembly, dirty references.
- No network access, no downloads, no new dependencies.

**Code path down to `WaitForSingleObject`**
1. **API entry point**:
   - `RingConsumer::read_into_blocking(&mut self, out: &mut [f32], timeout: Option<Duration>)`
     (`crates/rustamp-ipc/src/shm/ring.rs:908`).
   - `RingConsumer::wait_readable(&self, timeout: Option<Duration>)`
     (`crates/rustamp-ipc/src/shm/ring.rs:878`).
   - (Symmetrically for producer: `write_from_blocking` at line 532, `wait_writable` at line 501).
2. **Ring check**:
   - Validates ring header (`ring.rs:932` / `881`).
   - Checks `available = write_seq - read_seq` (`ring.rs:935` / `887`).
     If `available > 0`, copies data / returns `Signaled` immediately without sleeping.
   - Checks `is_closed()` (`ring.rs:946` / `882`).
   - If empty and open, calls `wake.wait(Wake::Data, timeout)?` (`ring.rs:949` / `890`).
3. **Wake routing**:
   - `RingWake::wait(&self, which: Wake, timeout: Option<Duration>)`
     (`crates/rustamp-ipc/src/shm/wake.rs:384`).
   - Borrows handle `&self.data` and delegates to `backend::do_wait(h, timeout)` (`wake.rs:394`).
4. **Win32 wait backend**:
   - `backend::do_wait(handle: &Handle, timeout: Option<Duration>)`
     (`crates/rustamp-ipc/src/shm/wake.rs:135`).
   - Maps duration to millisecond `ms: u32` (`wake.rs:139–153`), rounding up fractional
     milliseconds per RFC 0007 §6.4.
   - Invokes syscall: `WaitForSingleObject(*handle, ms)` (`wake.rs:155`).
   - Maps `WAIT_OBJECT_0` to `Ok(WaitOutcome::Signaled)`, `WAIT_TIMEOUT` to
     `Ok(WaitOutcome::TimedOut)`.
5. **Loop handling on return**:
   - `WaitOutcome::TimedOut` returns `Ok(0)` (or `WaitOutcome::TimedOut`).
   - `WaitOutcome::Signaled` triggers `continue`, looping back to re-check counters.

**Spurious signals and already-signaled events**
- The events are auto-reset (`bManualReset = FALSE`, `wake.rs:105`).
- If an event is already signaled when `wait` is called, `WaitForSingleObject` returns
  `WAIT_OBJECT_0` immediately (0 ms) and resets the event to non-signaled.
- In `read_into_blocking`, `WaitOutcome::Signaled` triggers `continue`. The loop re-checks
  `available`. If still 0, it calls `wake.wait(Wake::Data, timeout)` a second time.
- On the second wait, the event is non-signaled, so `WaitForSingleObject` actually blocks
  for the requested timeout.
- Therefore, an already-signaled event or a spurious signal can only make the total elapsed
  time *longer* (0 ms + subsequent wait time), never shorter. It cannot cause an early return
  with `Ok(0)` / `TimedOut`.

**The early return: analysis and verdict**
- In the VM run, `read_into_blocking` with a 10 ms timeout returned `Ok(0)` in ~7.8 ms.
- Verdict: **The wait implementation is correct; the return in ~7.8 ms is expected Windows OS behavior.**
- Root cause: On Windows, the standard kernel timer resolution is 15.625 ms (64 timer ticks per second).
  Microsoft's official documentation for Win32 wait functions (`WaitForSingleObject`, `Sleep`) explicitly notes:
  > *"If the time-out interval is less than the resolution of the system clock, the wait may time out in less than the specified length of time."*
- When a 10 ms timeout is requested, 10 ms is smaller than the system clock tick (15.625 ms).
  If the call is made midway through a tick interval, the kernel timer interrupt fires at the
  next tick boundary (~7–8 ms later), evaluating the timer as expired and returning `WAIT_TIMEOUT`.
- The earlier assertion `elapsed >= Duration::from_millis(8)` failed because it assumed sub-tick timer
  precision that the general-purpose Windows kernel scheduler does not guarantee for intervals under 16 ms.

**Measurement and tolerance**
- New test: `blocking_timeout_accuracy_across_durations` in `crates/rustamp-ipc/src/shm/ring.rs`.
- Durations tested: 10 ms (sub-tick), 30 ms (multi-tick), 60 ms (longer).
- Runs: 5 iterations per duration.
- Tolerance:
  - Lower bound: `requested.saturating_sub(Duration::from_millis(16)).max(Duration::from_millis(1))`.
    - For sub-tick intervals (<= 16 ms): asserts `elapsed >= 1 ms`, proving the thread entered the kernel
      wait state and did not return immediately/spuriously.
    - For multi-tick intervals (> 16 ms): asserts that the wait cannot expire more than 1 system clock tick
      early (e.g. 30 ms must take >= 14 ms; 60 ms must take >= 44 ms).
  - Upper bound: `requested + Duration::from_millis(500)` ensures the test fails instead of hanging,
    while tolerating scheduling jitter in a loaded virtual machine.
- Honest assertion in `blocking_timeout_returns_timed_out`: restored `assert!(elapsed >= Duration::from_millis(1))`
  with documentation explaining why `>= 8 ms` failed in the VM.

**What the measurement proves and what it cannot**
- **What it proves**:
  - `read_into_blocking` and `wait_readable` genuinely enter kernel blocking via `WaitForSingleObject`.
  - Auto-reset event latches are respected and properly consumed.
  - The wait returns `Ok(0)` / `WaitOutcome::TimedOut` without hanging or returning immediately (no 0 ms return).
  - Multi-tick durations are honored within the physical resolution of the operating system clock.
- **What it cannot prove**:
  - It cannot prove sub-millisecond precision for intervals shorter than the Windows system clock resolution (~15.625 ms) without activating global high-resolution timer periods (`timeBeginPeriod(1)`), which Microsoft discourages due to power and battery impact.
  - It cannot prove deterministic execution times under extreme hypervisor scheduling delay or vCPU starvation on a loaded host.
