# Task card G3 — Measure and settle blocking wake timeout precision (RFC 0007 §6.4)

> **Orchestrator notes (2026-10-17).**
> Settles the open question recorded in task card G2 note 2: whether a wait
> returning `Ok(0)` in less than eight milliseconds on a ten millisecond timeout
> represents a defect in `rustamp-ipc` or the documented behavior of the OS
> kernel timer resolution.
>
> **G5 note (2026-10-18).** W10 finding F7 showed this card's measurement does
> not settle the question it claims to settle: the sub-tick lower bound is a
> 1 ms smoke floor, the tick mechanism in "The early return" below was
> self-inconsistent, and the ~7.8 ms VM figure has no artefact in the tree.
> The claim is narrowed in this card. Task card [G5](G5-wake-honesty.md)
> records what is measured instead (a deterministic test of the mechanism's
> own timing contract, plus a reproducible host probe), what the tests now
> assert, and what remains unmeasured.

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
     (`crates/rustamp-ipc/src/shm/wake.rs:409`).
   - Borrows handle `&self.data` and delegates to `backend::do_wait(h, timeout)` (`wake.rs:419`).
4. **Win32 wait backend**:
   - `backend::do_wait(handle: &Handle, timeout: Option<Duration>)`
     (`crates/rustamp-ipc/src/shm/wake.rs:166`).
   - Maps duration to the millisecond `ms: u32` argument via `backend::timeout_ms`
     (`wake.rs:145`), rounding up fractional milliseconds per RFC 0007 §6.4. The
     mapping is pinned exactly by the
     `timeout_ms_rounds_the_request_up_to_whole_milliseconds` test (`wake.rs:507`).
   - Invokes syscall: `WaitForSingleObject(*handle, ms)` (`wake.rs:172`).
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

- In the VM run, `read_into_blocking` with a 10 ms timeout returned `Ok(0)` in ~7.8 ms
  (reported 2026-10-17). **No run log, capture or manifest for this figure exists in
  the tree**: it is a single, unevidenced observation that cannot be reproduced or
  checked from the repository (W10 finding F7, G3-5).
- Verdict: **the wait implementation is correct; whether the early return is expected
  OS behaviour is plausible but unproven in this tree.** The implementation adds no
  early return of its own: `Ok(0)` / `WaitOutcome::TimedOut` is returned only when the
  kernel reports `WAIT_TIMEOUT` (see "Spurious signals" above).
- The documented behaviour supports the conclusion: Microsoft's official documentation
  for Win32 wait functions (`WaitForSingleObject`, `Sleep`) explicitly notes:
  > *"If the time-out interval is less than the resolution of the system clock, the wait may time out in less than the specified length of time."*
  A 10 ms request is below the standard 15.625 ms kernel timer resolution.
- The mechanistic explanation this card previously gave is **withdrawn**: it said the
  kernel timer interrupt fires at the next tick boundary (~7–8 ms after a mid-tick
  call) and evaluates the 10 ms timer as expired there. That is self-inconsistent —
  a tick boundary 7–8 ms after the call *precedes* the 10 ms expiry, so the timer
  cannot be evaluated as expired at that boundary (W10 finding F7, G3-4). No in-tree
  measurement shows how early the kernel actually expires a sub-tick wait, on the
  host or in the VM.
- What is measured instead: the mechanism's own timing contract — the exact
  millisecond value handed to `WaitForSingleObject` (RFC 0007 §6.4) — is pinned
  deterministically by the `timeout_ms` test in `crates/rustamp-ipc/src/shm/wake.rs:507`.
  The OS behaviour itself is measured by the `wake_timeout_probe` example
  (`crates/rustamp-ipc/examples/wake_timeout_probe.rs`) on any Windows machine; task
  card [G5](G5-wake-honesty.md) records a host run and states what only the VM can
  still add.

**Measurement and tolerance**

- The mechanism's timing contract is exact and deterministic: `backend::timeout_ms`
  (`wake.rs:145`) maps `Some(d)` to the `WaitForSingleObject` millisecond argument,
  rounding up to whole milliseconds (a positive sub-millisecond duration becomes 1 ms;
  `ZERO` polls; `None` is `INFINITE`). The
  `timeout_ms_rounds_the_request_up_to_whole_milliseconds` test (`wake.rs:507`)
  pins every case. This is the only part of wait timing the implementation controls.
- `blocking_timeout_bounds_hold_across_durations` in
  `crates/rustamp-ipc/src/shm/ring.rs:1892` (renamed from
  `blocking_timeout_accuracy_across_durations` — it checks bounds, it does not measure
  accuracy) runs 5 iterations at 10 ms (sub-tick), 30 ms and 60 ms (multi-tick) and
  asserts, per run:
  - the outcome is `Ok(0)`: the kernel reported `WAIT_TIMEOUT`;
  - `elapsed >= max(requested − 16 ms, 1 ms)` — the loosest bound the documented OS
    contract supports. One tick of early expiry is allowed for tick-quantized timers,
    clamped to the 1 ms kernel-entry floor for sub-tick requests. For the 10 ms case
    this proves the call entered the kernel and did not return instantly; it cannot
    catch an early expiry of up to 9 ms, and no in-tree wall-clock bound can — that
    would measure the OS, not the code;
  - `elapsed < requested + 500 ms` — a hang guard, tolerating scheduling jitter in a
    loaded virtual machine.
- Honest assertion in `blocking_timeout_returns_timed_out` (`ring.rs:1837`):
  `elapsed >= 1 ms` (kernel-entry smoke floor), with a doc comment that says exactly
  that. The bound was weakened from `>= 8 ms` after the VM failed it; the weakening
  is now documented as a property of the documented OS contract, not presented as a
  tolerance the measurement earned.

**What the tests prove and what they cannot**

- **What they prove**:
  - The exact millisecond value the mechanism hands to `WaitForSingleObject`
    (deterministic, `wake.rs:507`).
  - `read_into_blocking` and `wait_readable` genuinely enter kernel blocking via
    `WaitForSingleObject` (the 1 ms smoke floor).
  - A timeout returns `Ok(0)` / `WaitOutcome::TimedOut` without hanging, returning
    instantly, or returning an error.
  - Auto-reset event latches are respected and properly consumed.
  - A wait never returns earlier than the loosest bound the documented contract
    supports (one tick of early expiry), and never hangs.
- **What they cannot prove**:
  - How early the OS actually expires a sub-tick wait, on any given system. The
    ~7.8 ms VM figure is an unevidenced single observation; the host probe in task
    G5 measured the host (no early expiry beyond 0.115 ms in 250 runs), and only a
    VM run of the same probe, with its per-run output committed, can measure the VM.
  - Sub-millisecond precision for intervals shorter than the Windows system clock
    resolution without activating global high-resolution timer periods
    (`timeBeginPeriod(1)`), which Microsoft discourages due to power and battery
    impact.
  - Deterministic execution times under extreme hypervisor scheduling delay or vCPU
    starvation on a loaded host.
