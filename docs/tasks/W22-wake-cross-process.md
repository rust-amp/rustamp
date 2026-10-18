# W22: the wake across the process and the 32/64-bit boundary

Ready-queue item 4 ("**The wake, cross-process** — prove the RFC 0007 wake
across the 32/64-bit boundary, the way `ring-cross` does"). Sources:
[`G2-ipc-blocking.md`](G2-ipc-blocking.md) (its last, unticked acceptance box),
[`../rfc/0007-blocking-consumer.md`](../rfc/0007-blocking-consumer.md) §14.3 and
§14.9, and [`W19-ring-wake-root.md`](W19-ring-wake-root.md), which removed the
edge this card has to avoid re-proving.

Owned: `crates/rustamp-ipc/**`, `xtask/**`, this file. RFC 0007 is **not**
touched; §7 of it is one sentence short of describing what I added and the exact
wording is in §7 of this card, for whoever owns the RFC next.

---

## 1. A harness existed. It proved the wrong ring.

`cargo xtask wake-cross` (`xtask/src/wake_cross.rs`, commit `e2ad229`,
2026-10-17) already drives the RFC 0007 wake between a real 64-bit process and a
real 32-bit process, in both directions, through the `wake-produce` /
`wake-consume` modes of the `ring_peer` example — the same two-process,
two-target shape `ring-cross` uses, with the same stdout line protocol, the same
bounded waits and the same `RingMeta` offer line. It had five phases per
direction: the timeout contract, a blocked reader woken by the writer, a blocked
writer woken by the reader, 256 frames of lossless bulk transfer, and `close()`
releasing a blocked waiter with `Closed`. There is nothing wrong with any of
them and I kept all five.

What was missing is not a direction and not a bitness. It is the **property**.

`e2ad229` lands one day before `c96935a`, the commit that removed the
edge-triggered signal and withdrew RFC 0007 §6.2. Every one of those five phases
wakes a waiter across an **edge**, and that is not an accident of how they were
written — it is forced:

- a consumer blocks on `Wake::Data` only after it observed the ring **empty**,
  so the write that wakes it starts from empty;
- a producer blocks on `Wake::Space` only after it observed the ring **full**,
  so the read that wakes it starts from full.

Those are exactly the two cases the withdrawn refinement still signalled. So the
whole cross-process suite passes unchanged under the unsound rule that caused
the thirty-second stall, and it did: it was green on 2026-10-17 against the
edge-triggered ring. It could not have caught the defect, which means it was not
evidence for the ring the tree now has — it was evidence for a ring that no
longer exists.

**And no waiter can close that gap.** The deterministic shape RFC 0007 §14.3
demands is "drain a latch, then require that the next advance — one that starts
from a ring that is neither empty nor full — sets it again", and the public ring
API cannot express it:

- draining `Wake::Data` needs the consumer to reach its `wake.wait` call, which
  it only does when the ring is **empty** (`read_into_blocking` and
  `wait_readable` both return on the counter check when data is there);
- draining `Wake::Space` needs the producer to reach its wait, which it only
  does when the ring is **full**.

The drain needs the condition unmet; the advance under test needs it partly met.
The two cannot be arranged through blocking calls, in one process or two. The
in-process tests escape this by holding a **third handle** on the latches and
asking the events directly (`every_advance_signals_not_only_the_edge` and its
two siblings, W19 §3). A cross-process peer is a separate crate and could not:
`RingWake::signal` and `RingWake::wait` are `pub(crate)`.

## 2. What I added

### 2.1 `RingWake::take_signal` — a poll, not a wait

`crates/rustamp-ipc/src/shm/wake.rs` **444**:

```rust
pub fn take_signal(&self, which: Wake) -> Result<WaitOutcome, RingError>
```

It takes one direction's pending signal if there is one, and never blocks: it is
`self.wait(which, Some(Duration::ZERO))`, and `backend::timeout_ms` already maps
`Duration::ZERO` to the `0` millisecond argument that polls a Win32 event
without waiting — a documented pure function with its own test
(`timeout_ms_rounds_the_request_up_to_whole_milliseconds`, which has pinned
`ZERO -> 0` since G5). `Signaled` means a signal was pending and this call
consumed it; `TimedOut` means none was.

Why this and not a public `wait`: RFC 0007 §6.7 names the four ring methods that
may sleep, and a public `RingWake::wait` would let any caller sleep on a latch
without re-deriving its condition from the counters — the exact mistake class
the RFC exists to prevent. A zero-timeout poll adds no sleeping surface at all,
so §6.7 stands untouched; the module docs of `shm::wake` were corrected to say
"the only thing that may **block**" and to name `take_signal` as the poll.

New unit test, `take_signal_reports_and_consumes_one_pending_signal`
(**524–562**): clear latch reads `TimedOut`; after one `signal` it reads
`Signaled`; immediately after that it reads `TimedOut` again (auto-reset); and
taking `Data` leaves `Space` alone.

### 2.2 Both peers keep a probe

`crates/rustamp-ipc/examples/ring_peer.rs`. `wake_produce` (**654**) and
`wake_consume` (**914**) each clone their `RingWake` before handing it to the
ring, so each process holds a second handle on the same two kernel events.
`create_and_clone_share_the_same_latches` in `shm::wake` already pins that a
clone is the same object. The consumer's probe is a clone of a handle it
**opened by name**, so a `Wake::Data` it reports was set by the other process,
across the bitness boundary.

Four new commands on both halves, each answered by one line:

| command | answer | what it is for |
| --- | --- | --- |
| `DRAIN_LATCH data\|space` | `LATCH_DRAINED <dir> taken=<n>` | poll the latch clear (**617**) |
| `EXPECT_NO_LATCH data\|space` | `LATCH_CLEAR <dir>` | the negative control (**857**, **1212**) |
| `EXPECT_LATCH data\|space` | `LATCH_SET <dir>` | the assertion (**837**, **1192**) |
| `REPORT` | `STATE available=… free=… capacity=… …` | the premise, from the counters (**877**, **1227**) |

plus `WRITE_NB <n>` (**803**, `write_from`) and `READ_NB <n>` (**1151**,
`read_into`, with the same per-sample verification the other read commands do),
so the two non-blocking signalling sites are reachable as single advances.

A failed expectation prints `FAIL <diagnostic>` on stdout **and** exits
non-zero (`fail`, **586**): the runner treats a `FAIL` line as an immediate
failure of the step it is waiting for, so the message the run prints is the
peer's own sentence with its own counters in it.

### 2.3 Two new phases, four new assertions per direction

`xtask/src/wake_cross.rs`, phases 5 and 6 (**337** and **387**), between the
bulk phase and the close phase — the bulk phase leaves the ring empty and both
sides in step, and the close phase needs an empty ring, which the last step of
phase 6 re-establishes and asserts.

`prove_mid_advance` (**586**) is the shape, used four times:

1. the probe side drains its latch;
2. **negative control**: require the latch now reads *clear*;
3. the probe side's own `REPORT` must show `0 < available < capacity` (consumer)
   or `0 < free < capacity` (producer) — the ring is neither empty nor full;
4. the **other process** performs one advance, acknowledged only after the ring
   call returned;
5. the latch must be set again.

| phase | advance under test | signaller | prober |
| --- | --- | --- | --- |
| mid-advance | `write_from` (`WRITE_NB 1`) | producer process | consumer process, `Wake::Data` |
| mid-advance | `read_into` (`READ_NB 1`) | consumer process | producer process, `Wake::Space` |
| mid-advance-blk | `write_from_blocking` (`WRITE 1`) | producer process | consumer process, `Wake::Data` |
| mid-advance-blk | `read_into_blocking` (`DRAIN 1`) | consumer process | producer process, `Wake::Space` |

That is all four PCM signalling sites that carried the withdrawn edge, each
proved from the *other* process and, because `RUNS` runs both directions, each
proved once with a 64-bit signaller and a 32-bit prober and once the other way
round. Neither blocking call ever waits in phase 6: each finds its condition
already met, so what is under test there is the store-and-signal step, which had
its own copy of the edge, and not the wait.

The ring runs at capacity 16 with 8-byte frames (**82**), and the occupancy walks
1 → 2 → 1 → 2 → 1 → 0 across the two phases, so every advance happens strictly
between the boundaries and the final `REPORT` proves the ring is empty again.

### 2.4 The three sleeps that ordered the old phases are gone

`wake_cross.rs` used `std::thread::sleep(SLEEP_BEFORE_SIGNAL)` (25 ms) three
times: after `READER_BLOCKING`, after `WRITER_BLOCKING` and after
`WAITING_CLOSE`, to "make sure" the peer was really inside its wait before the
runner told the other side to signal. That is a sleep as an ordering mechanism,
and it bought nothing: it could not establish where the peer was, and if it had
been too short the phase would still have passed, because the auto-reset latch
holds a signal sent while nobody waits.

The announcement is now **earned**. Before it prints `READER_BLOCKING`, the
consumer runs `read_into_blocking(buf, Some(ANNOUNCE_WAIT))` and requires
`Ok(0)`; before `WRITER_BLOCKING`, the producer runs
`write_from_blocking(block, Some(ANNOUNCE_WAIT))` and requires `Ok(0)`; before
`WAITING_CLOSE`, the consumer does the same read. `ANNOUNCE_WAIT` is 50 ms
(**112**). That outcome is positive evidence: the call entered the kernel wait,
found the ring empty (or full) with the latch clear, and came back untouched. It
is a wait on the mechanism under test whose `TimedOut` is an assertion, not a
sleep, and it cannot flake — nothing is signalling during it, and a stale latch
only costs one further pass of the check-then-wait loop before the same `Ok(0)`.

The wait that follows the announcement now uses `BLOCK_WAIT`, 5 s (**119**),
deliberately shorter than the runner's 10 s `STEP_TIMEOUT`, so the side that can
print the counters is the side that reports the failure.

Sleeps left in the two runners, and why they are not ordering mechanisms:
`wait_timeout`'s 2 ms backoff polls a child **process** for exit; `OPEN_RETRY`
retries opening the section or the events while the peer is still starting, under
a hard `OPEN_TIMEOUT`; and the wake-less `produce` path's 1 ms poll waits for the
consumer to close a ring that has no event to wait on. None of them stands
between an advance and a signal.

---

## 3. How I convinced myself each new proof can fail

A test that cannot fail is not a test, so for each new assertion: what is the
unique code that makes it pass, and what does the run look like without it.

1. **The latch is written by exactly one thing.** The only code that sets a
   ring's `Wake::Data` event is `wake.signal(Wake::Data)` in `write_from`,
   `write_slot`, `write_from_blocking` and `close`; for `Wake::Space` it is
   `read_into`, `read_slot_into`, `read_into_blocking` and `close` (the
   signalling table of RFC 0007 §8 and the six call sites W19 §1 lists). In
   phases 5 and 6 no vis call runs and no `close` has happened yet, the probe
   has just drained the latch, and the peer process is blocked reading stdin.
   So the single advance the runner commands is the **only** possible writer of
   that latch between the drain and the assertion. Delete its `signal` — or
   restore the `was_empty` / `was_full` guard — and `EXPECT_LATCH` observes a
   clear latch, prints `FAIL`, and exits non-zero. There is no retry to absorb
   it, no timeout to wait out, and no other path to the same observation.

2. **Under the withdrawn edge specifically, all four fail.** Step 3 of
   `prove_mid_advance` requires `0 < available < capacity` (or `0 < free <
   capacity`) from the peer's own counters immediately before the advance. That
   is precisely the region where `was_empty` and `was_full` are both false, so
   the restored guard skips the `SetEvent` and the assertion in step 5 fails.
   The same four assertions were observed red under that code in-process — W19
   §3 reports all six of its deterministic assertions failing under the old
   ring — and these are the same assertions over the same latches, with the
   process boundary added.

3. **The assertion is not vacuous.** Step 2, `EXPECT_NO_LATCH`, requires the
   same probe over the same handle to answer `TimedOut` a moment earlier in the
   same phase. A probe that answered `Signaled` unconditionally, or a drain that
   silently did nothing, fails there. So the pair of steps discriminates rather
   than asserting a constant — which is the thing `blocking_no_lost_wakeup_under_jitter`
   was missing before W19 §3 sharpened it.

4. **The two announce-waits are assertions, not tolerances.** `Ok(0)` from the
   announce wait is required (the ring must be empty/full), and `Ok(0)` from the
   wait *after* the announcement is a failure, with `available` /
   `frames_published` / `frames_read` (consumer) or `free_frames` /
   `frames_written` (producer) printed, the same discriminator W19 §3 chose.
   Remove the signal from the waking side and both of those phases fail inside
   5 s with the counters that prove the condition was already met. Before this
   card they would have passed by sleeping 25 ms first and then waiting 30 s.

5. **`take_signal` itself is pinned.** Its unit test fails if the poll ever
   reports a signal that was not sent, never reports one that was, or fails to
   consume it. Phases 5 and 6 rest on that behaviour, so it is tested where it
   is cheap to test.

What I did **not** convince myself of by running it: nothing in this card was
executed. `cargo xtask wake-cross` spawns the two peer processes, which is a
test run and belongs to the orchestrator's VM (§6). The four points above are a
code argument plus the in-process observation W19 already recorded; the run is
what turns them into an observation here.

---

## 4. Files

| file | change | +/− |
| --- | --- | --- |
| `crates/rustamp-ipc/src/shm/wake.rs` | `take_signal` (**444**) and its unit test (**524**); the module docs' sleeping rule now says "block" and names the poll | +68 / −4 |
| `crates/rustamp-ipc/examples/ring_peer.rs` | the wake-mode doc block; `ANNOUNCE_WAIT`/`BLOCK_WAIT`; `fail`, `wake_direction`, `direction_name`, `drain_latch`; a probe handle on both halves; six new commands; the two earned announcements and the `Ok(0)`-is-a-failure bounds | +370 / −13 |
| `xtask/src/wake_cross.rs` | phases 5 and 6 with `prove_mid_advance`, `Side`, `Advance`, `step`, `assert_strictly_between`; `CAPACITY`/`FRAME_SIZE`/`PHASES_PER_DIRECTION`; the three `thread::sleep` ordering steps removed; the "why phases 5 and 6 exist" and "no sleep orders anything here" sections | +315 / −10 |
| `docs/tasks/W22-wake-cross-process.md` | **new** (this card) | +— |

File sizes after the change: `wake.rs` 602, `ring_peer.rs` 1245,
`wake_cross.rs` 785 lines.

Not touched, and why:

- `docs/rfc/0007-blocking-consumer.md` — out of this lane's owned paths. The one
  change it wants is in §7 below.
- The vis signalling sites (`write_slot`, `read_slot_into`). They carried the
  same withdrawn edge and are covered in-process by
  `every_vis_advance_signals_not_only_the_edge`, but a vis ring is created
  **without** a wake in production (RFC 0007 §9), so a cross-bitness vis wake
  ring would be machinery that exists only for the check. The cross-boundary
  question — do the derived event names resolve to the same kernel objects from
  a 32-bit and a 64-bit process — is answered by the PCM phases and is not
  per-kind.
- `crates/rustamp-mediahost/**`, `crates/rustamp-core/**` and the other
  forbidden crates. `rustamp-mediahost/src/pcm.rs` **26–31** still describes the
  edge as live code; W19 §4 already names that one sentence as its owner's
  follow-up and it is not a behaviour change.

---

## 5. Gates

Run on the host, from the worktree:

| gate | exit |
| --- | --- |
| `cargo fmt --all -- --check` | **0** |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **0** |
| `cargo build --workspace` | **0** |
| `cargo build -p rustamp-ipc --example ring_peer` (x86_64) | **0** |
| `cargo build -p rustamp-ipc --example ring_peer --target i686-pc-windows-msvc` | **0** |

The last two are not formally gates but they are the two binaries the check
runs, and the new code has to compile for **both** bitnesses or the check cannot
start; `cargo build --workspace` does not build examples, so it would not have
noticed.

`cargo test` was **not** run, and neither was `cargo xtask wake-cross`: both are
runs of the suite and belong to the orchestrator's VM.

---

## 6. What only the virtual machine can prove

1. **Do phases 5 and 6 pass, in both directions?** This is the claim of the
   card. Eight new assertions per run (four per direction), each milliseconds
   long and race-free. A failure names the ring call, the direction of the
   latch, and which side was probing.
2. **Does the negative control hold?** If `EXPECT_NO_LATCH` ever fails, the
   drain or the probe is wrong and the positive assertion next to it is worth
   nothing — read that before reading anything else in the output.
3. **Do the three re-worked announcements still reach their waits?** Phases 2, 3
   and 7 now require `Ok(0)` from a 50 ms wait before they announce. If the ring
   turns out not to be empty (or full) at that point, the phase fails where it
   used to sleep through the discrepancy — that would be a real finding about
   the phase's own bookkeeping, not about the wake.
4. **How long does a direction take now?** The three 25 ms sleeps are gone
   and three 50 ms announce-waits replaced them, so the floor rises by about
   75 ms per direction, plus roughly twenty short command round-trips. A direction that takes
   materially longer than before is worth a look: the only mechanism that could
   cost time here is a wait that is actually waiting, which in phases 5 and 6
   would mean a signal arriving late rather than before the acknowledgement.
5. **Does the rest of the `rustamp-ipc` suite stay green?** `take_signal` adds
   one public method and one test and changes no existing path; the
   non-Windows backend reaches it through the same `do_wait` with a zero
   timeout. Nothing else in the crate calls it. That is a host read of the code,
   not a run.
6. **Does `cargo xtask ring-cross` still pass?** It shares `ring_peer` with
   `wake-cross` but not a single wake code path; the `produce`/`consume` modes
   are untouched. Still worth one run, because the file changed.

---

## 7. The one change RFC 0007 wants (not made here)

§7's API sketch lists `RingWake` as `create` / `open` / `close`, and §2.1 above
adds a fourth public method. The RFC is `Accepted` and out of this lane's owned
paths, so the exact text to add, for whoever owns it next — in §7, in the
`impl RingWake` block of the sketch:

```rust
    pub fn close(&mut self);                     // idempotent; also in Drop
    /// Poll one direction's latch (zero timeout): takes a pending signal if
    /// there is one. Never blocks, so §6.7 is unaffected.
    pub fn take_signal(&self, which: Wake) -> Result<WaitOutcome, RingError>;
```

and one sentence at the end of §7's "Semantics of the additions" list:

> - `take_signal(which)` polls a latch instead of waiting on it (`Duration::ZERO`
>   per §6.4) and consumes the signal it reports. It is not a wait, so §6.7's
>   list of sleeping calls is unchanged; it exists so that a third handle on a
>   ring's latches can assert §6.2's signalling rule directly, which §14.3
>   requires and which no waiter can observe (see
>   `docs/tasks/W22-wake-cross-process.md` §1).

§14.9 ("`cargo xtask ring-cross` gains a wake-ring case") is satisfied by
`cargo xtask wake-cross` as a sibling command rather than by a case inside
`ring-cross`; that was already true before this card and needs no RFC change,
but §14.9's wording names the wrong command and could say `wake-cross`.

No other part of RFC 0007 needs a change. Nothing on the wire, in the header, in
`PROTOCOL_VERSION` or in `RING_VERSION` moves, and §6.2's rule of record — every
advance signals — is what this card proves rather than anything it alters.

---

## 8. Provenance

From `docs/rfc/0007-blocking-consumer.md`, `docs/rfc/0003-shm-rings.md`, the
`rustamp-ipc` sources, the existing `xtask/src/ring_cross.rs` and
`xtask/src/wake_cross.rs`, and the cards `G2-ipc-blocking.md`,
`W19-twoproc-stall.md` and `W19-ring-wake-root.md`. Win32 event behaviour used
here (an auto-reset event latches a signal sent with nobody waiting; a zero
timeout polls without blocking; a named object in the session namespace is the
same object for every opener) is documented API behaviour, cited as RFC 0003 §2
cites it. No Winamp source, SDK material, documentation or disassembly was read,
no reference binary was run, nothing was downloaded and no dependency was added.
