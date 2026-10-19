# G6 — The wake across the process boundary, in the test suite

Ready-queue item 4 ("**The wake, cross-process** — prove the RFC 0007 wake
across the 32/64-bit boundary, the way `ring-cross` does"). Sources:
[`G2-ipc-blocking.md`](G2-ipc-blocking.md) (its last, unticked acceptance box),
[`../rfc/0007-blocking-consumer.md`](../rfc/0007-blocking-consumer.md) §1, §4.3,
§6.1, §14.3 and §14.9, and
[`W22-wake-cross-process.md`](W22-wake-cross-process.md).

Owned: `crates/rustamp-ipc/**`, `xtask/**`, this file. RFC 0007 is **not**
touched: the wake mechanism, the signalling rule and the public API are
unchanged, and this lane adds **no** public item to `rustamp-ipc`.

---

## 1. The audit: what was already there

| what | where | state |
| --- | --- | --- |
| the wake primitive: `RingWake`, `Wake`, `WaitOutcome`, `ring_wake_name`, `take_signal` | `crates/rustamp-ipc/src/shm/wake.rs` | there (G2, G3, W22) |
| the blocking ring methods: `with_wake`, `wait_readable`, `wait_writable`, `read_into_blocking`, `write_from_blocking` | `crates/rustamp-ipc/src/shm/ring.rs` | there (G2) |
| the in-process proof: lossless delivery, `close()` wakes, the timeout, the jitter stress test, the three "every advance signals" tests | `crates/rustamp-ipc/src/shm/ring.rs` | there (G2, W19) |
| the two-process harness: `cargo xtask wake-cross` with seven phases per direction, both bitnesses | `xtask/src/wake_cross.rs`, `crates/rustamp-ipc/examples/ring_peer.rs` | there (G4, W22) |
| the cross-bitness **wake** proof | `cargo xtask wake-cross` | there (G4, W22) |

**What G2 still listed as open** was its last acceptance box: *"Cross-bitness
wake-ring case added to `cargo xtask ring-cross` (32-bit producer → 64-bit
consumer and the reverse)"*. G4 closed it as a **sibling** command
(`cargo xtask wake-cross`) rather than as a case inside `ring-cross`; W22
strengthened it with the mid-advance phases, the withdrawn edge of §6.2.

**What was still missing** was not a direction and not a bitness. It was three
things:

1. **The cross-process proof was not in the test suite.** `cargo xtask
   wake-cross` is a *tool*: no `cargo test` run executes it, and the guest's
   `--ignored` pass never sees it. A regression in the wake would leave the
   ordinary suite green.
2. **No latency claim.** Every phase proved that the wake *happens* (bounded
   by the runner's 10 s step timeout). Nothing said how long the blocked side
   took to be released.
3. **No busy-spin claim.** RFC 0007 §1 and §4.3 make the consumer *wait*
   instead of polling the counter - the mechanism's whole point against the
   spin/poll alternative of §4.3. Nothing measured it: the blocked side could
   have burned a core and every phase would still have passed.

## 2. What this lane added

### 2.1 The claims, measured in one process (headless, on the host)

`crates/rustamp-ipc/src/shm/ring.rs`, five new tests:

| test | what it pins |
| --- | --- |
| `a_blocking_read_sleeps_instead_of_spinning` | a consumer blocked in an empty-ring read burns **no CPU** over a 300 ms window (the thread's own user+kernel time, from `GetThreadTimes`) |
| `a_blocking_write_sleeps_instead_of_spinning` | the same for a producer blocked on a full ring |
| `the_wait_state_machine_reports_the_state_instead_of_waiting` | a condition that is already met returns `Signaled` at once; a closed ring reports `Closed` instead of waiting |
| `the_blocking_path_rejects_a_stale_generation` | a blocking call on a ring whose generation moved on reports `Stale`, not a wait on a ring that is not its own (§5.3) |
| `a_blocking_write_times_out_on_a_full_ring_without_dropping` | `Ok(0)` on timeout, `frames_dropped() == 0`: a blocking write never drops (§6.4, §7) |

The measurement is the point: Windows accounts thread time on the system timer
tick, so a window of a few milliseconds could not tell a sleeping thread from
a spinning one - hence 300 ms, and hence the bound `cpu * 4 < wall` plus an
absolute `cpu < 50 ms`. A polling consumer would burn the whole window, three
orders of magnitude more, and the test would fail.

### 2.2 The two-process proof, where the ordinary suite can see it

`crates/rustamp-ipc/tests/wake_cross_process.rs` (new, `#[ignore]`, VM-only):

* `cross_process_consumer_is_woken_by_the_remote_producer` - a consumer blocks
  in an empty-ring read; a producer **in the other process** publishes and
  wakes it.
* `cross_process_producer_is_woken_by_the_remote_consumer` - the mirror: a
  producer blocks on a full ring; a consumer in the other process drains it
  and wakes it.

Both assert the three things the queue item asks for: the frame that crosses
the process boundary is the frame that was published (verified sample by
sample), the **wake latency is bounded** (the two processes stamp the wall
clock, the only clock they share, and the difference is compared against a
2 s bound), and the blocked thread **slept** (its CPU time over the window is
a fraction of the wall clock).

The peer is **this same test binary**, started again with a role in its
environment (`RUSTAMP_WAKE_PEER_ROLE`). That keeps the check self-contained:
no second program to build, stage or find, and the guest's runner stages the
test binary itself, so the peer is always next to it. `RUSTAMP_WAKE_PEER_BIN`
points the check at a different executable - the i686 build of this very test
binary - and the proof then runs across the bitness boundary as well.

The blocked side proves that it blocks before the peer is allowed to act: a
bounded `read_into_blocking` on the empty ring that returns `Ok(0)` entered the
kernel wait with the latch clear and came back untouched. Nothing is ordered
by a sleep.

### 2.3 The same two claims across the 32/64-bit boundary

`xtask/src/wake_cross.rs` and `crates/rustamp-ipc/examples/ring_peer.rs`: the
two phases that already woke a blocked reader and a blocked writer now also
measure what the queue item asks for, in both bitness directions:

* the peers stamp the wall clock (`started_us`, `done_us`, `at_us`), and the
  runner compares the producer's publish against the consumer's release across
  the process boundary;
* the blocked peer reports how much CPU **its own thread** burned across the
  blocked window, and the runner requires that number to be a fraction of the
  wall clock.

The window has to be wide enough for the accounting to mean anything, so the
peer that publishes (or drains) waits 300 ms first. That delay orders
nothing: the blocked side has already proved that it blocks, and the signal
follows the command, not the delay.

## 3. The gates

| gate | where | result |
| --- | --- | --- |
| `cargo fmt --all -- --check` | host | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | host | clean |
| `cargo test -p rustamp-ipc --all-features` | host | 50 + 11 + 21 + 1 + 3 + 1 green, 2 ignored (the two-process checks) |
| `cargo test -p rustamp-ipc --all-features --target i686-pc-windows-msvc` | host | the same, green on i686 |
| `cargo xtask wake-cross` | host | 14 of 14 green, both directions |
| `cargo test -p rustamp-ipc --all-features --test wake_cross_process -- --ignored` | host, gate temporarily bypassed | 2 of 2 green |
| the same with `RUSTAMP_WAKE_PEER_BIN` = the i686 build of the test binary | host, gate temporarily bypassed | 2 of 2 green |

The last two rows are the two-process checks. They are `#[ignore]`d and
gated on the test VM, so **the delivered file refuses to run on the host**;
they were exercised during development with the gate's assertion temporarily
relaxed, and the gate was restored before the lane finished. The peer is
started with `CREATE_NO_WINDOW`, so no window was raised either way.

## 4. What only the virtual machine can prove

1. **The two-process checks as the guest runs them** - the peer is the staged
   test binary, and the VM's path to it is the guest's, not the host's.
   `docs/notes/vm-run-tests.ps1 -Filter '--ignored'` covers them (the binary
   is `wake_cross_process`; no other binary shares the stem). The ignored-test
   inventory ([`W12-ignored-inventory.md`](W12-ignored-inventory.md)) counts
   20 ignored tests; this lane adds two, and they belong to its Group C
   ("VM two-process"): two real processes, VM-only, no fixture beyond the
   staged tree.
2. **The VM gate itself**: the check refuses unless the machine is
   `DESKTOP-UL3J9H6` or carries the runner's staged tree
   (`C:\rustamp\code\Rustamp`). The host has neither, so the check is inert
   there - which is the point.
3. **The cross-bitness run of the *test*** needs the i686 build of the test
   binary in the guest. The runner stages the i686 **plughost** test binaries
   today; staging this one is the orchestrator's call, not this lane's. Until
   then the cross-bitness proof of the new claims is
   `cargo xtask wake-cross`, which builds both bitnesses itself.

## 5. A note for the next peer protocol

The peer's stdout belongs to the **test harness**, not to the peer: libtest
prints `test <name> ... ` without a newline before the test's own output runs,
so the peer's first line arrives glued to that prefix. A runner that looks for
its marker at the *start* of a line therefore waits forever. The check looks
for the marker **anywhere** in the line and takes the fields from there.
(`ring_peer` does not have this problem: it is an example, not a test
binary, so it owns its stdout completely.)

## 6. Files

| file | change |
| --- | --- |
| `crates/rustamp-ipc/src/shm/ring.rs` | five headless tests and the `thread_cpu_micros` helper |
| `crates/rustamp-ipc/tests/wake_cross_process.rs` | **new**: the two-process checks (VM-only) |
| `crates/rustamp-ipc/examples/ring_peer.rs` | the wall-clock stamps, the thread's CPU time and the optional publish/drain delay |
| `xtask/src/wake_cross.rs` | the latency bound and the no-busy-spin assertions in phases 2 and 3 |
| `docs/tasks/G6-wake-cross-process-proof.md` | **new**: this card |

Not touched: `docs/rfc/0007-blocking-consumer.md` (the mechanism, the
signalling rule and the public API are unchanged), `crates/rustamp-mediahost`,
`crates/rustamp-plughost`, `crates/rustamp-audio` (other lanes), and the VM
runners under `docs/notes/` (the orchestrator's).

## 7. Provenance

From `docs/rfc/0007-blocking-consumer.md`, `docs/rfc/0003-shm-rings.md`, the
`rustamp-ipc` sources, the existing `xtask/src/{ring,wake}_cross.rs`, the
`ring_peer` example and the cards `G2-ipc-blocking.md`, `G3-wake-timeout.md`,
`G4-wake-cross.md`, `G5-wake-honesty.md`, `W19-ring-wake-root.md` and
`W22-wake-cross-process.md`. Win32 behaviour used here - an auto-reset event
latches a signal sent with nobody waiting, a thread blocked in a kernel wait
does not accrue CPU time, `GetThreadTimes` accounts a thread on the system
timer tick, a named object in the session namespace is the same object for
every opener - is documented API behaviour, cited the way RFC 0003 §2 cites
it. No Winamp source, SDK material, documentation or disassembly was read, no
reference binary was run, nothing was downloaded and no dependency was added.
