# RFC 0007: Blocking consumer and producer for the shm rings (RFC 0003 Q1)

Status: **Accepted** on 2026-10-15 by the project owner. Implementation waits
for the RFC 0006 card (this RFC exists to unblock it).

### Correction (2026-10-18): §6.2's conditional-signal refinement is **withdrawn**

§6.2 used to recommend an edge-triggered refinement — the producer signals
`Wake::Data` only when its write started from an empty ring, the consumer
signals `Wake::Space` only when its read started from a full one — and gave a
short soundness proof for it. **The refinement is unsound and is withdrawn**;
the rule of record is the unconditional one that §6.1's argument already
assumes ("that end always signals when it advances that counter"). The proof,
the counterexample and the reason for withdrawing rather than repairing are in
§6.2; the acceptance criterion that was supposed to catch this is sharpened in
§14.3. Nothing on the wire, in the header or in the API changes: this is a
signalling rule inside `write_from`/`write_slot`/`read_into`/`read_slot_into`
and their blocking forms. Diagnosed in
[`docs/tasks/W19-twoproc-stall.md`](../tasks/W19-twoproc-stall.md) §1, fixed in
[`docs/tasks/W19-ring-wake-root.md`](../tasks/W19-ring-wake-root.md).

### Owner decisions (2026-10-15)

- **Accepted as proposed.** The five load-bearing open questions are settled with
  the recommended defaults and may be revisited at the implementation card:
  the wake mode is **not** announced on the wire (no `Message` change); a
  dedicated `WakeUnavailable` refusal reason rather than reusing
  `MappingUnavailable`; the hosting path waits with **no timeout** and relies on
  the liveness ping; Q2's access control must cover the **events** as well as the
  section.
- **O4 resolved — `WaitOnAddress`/`WakeByAddressSingle` is permitted.** The
  minimum supported Windows version is **Windows 10 25H2**; anything older is out
  of scope, which removes the OS-floor objection that made this RFC prefer two
  named events. The implementation card chooses between that mechanism and the
  two-event design of §4; the rejected-alternatives reasoning in §4.2 stands
  either way.
- **New open item O13 — a Linux/macOS wake mechanism for M6.** The cross-platform
  milestone needs an equivalent (futex/eventfd on Linux, `pthread_cond`/`kqueue`
  on macOS) behind the same check-then-wait contract. Not designed here.
- **New open item O14 — the owner accepts the §7 "or later" risk** (see the
  licensing note): the exception is worded for GPL-3.0 *or later*; if that ever
  fails for a recipient, staying on version 3 remains available.
Owner: Lane R (RFC planning). Written by: task card
[`docs/tasks/R6-rfc-blocking-consumer.md`](../tasks/R6-rfc-blocking-consumer.md).
Requires: RFC 0003 (shared-memory rings), RFC 0006 (out-of-process hosting).
Touches (later implementation card): `crates/rustamp-ipc` (`shm` only), the
RFC 0006 implementation.
Deliberately out of scope: any implementation, the acceptance of this RFC,
Q2/Q3/Q4/Q5 themselves (they are referenced, never answered here), the slot
layout, the ring header layout, and the publication ordering of RFC 0003 §6.

## 1. Why

RFC 0003 §7 makes the producer **never block and never overwrite unread data**:
on a full ring it drops whole frames and counts them in `dropped_seq`. That is
the right rule for live taps (a vis feed, an output mirror), and RFC 0003 §12
records it as a decision: "A back-pressure signal. There is none by design: the
producer drops." RFC 0003 §12 also records that the consumer polls, and that an
event "would be a separate RFC". This is that RFC.

RFC 0006 §4 makes the consequence explicit: a decoder feed and a DSP round-trip
are **not** live taps — "a dropped frame is an audible gap. Polling plus drops
cannot carry them." RFC 0006 therefore freezes the hosting message shapes but
**blocks its implementation card on Q1**, under one hard rule: *no PCM crosses a
process boundary under a drop policy, ever.* Q1 has to give both ends a way to
wait for the other:

- the **consumer** waits for data instead of polling for it, and
- the **producer** waits for space instead of dropping into it,

while the existing non-blocking calls stay exactly as they are, because the
audio render path and the vis path must never block (RFC 0006 §4; RFC 0003
§12).

This RFC proposes a mechanism, its lifecycle, its lost-wakeup rules, what it
costs the ring API, and what (if anything) it costs the protocol. It changes no
existing guarantee for a ring that does not opt in.

## 2. Scope

In scope: the mechanism choice and its rejected alternatives; who creates, owns
and closes the kernel object; how its name and lifetime tie to the ring
generation rule of RFC 0003 §8; the lost-wakeup-proof check-then-wait
sequencing; spurious wakeups; timeout and shutdown semantics (`close()` wakes a
waiter with `Closed`); which ring calls gain blocking variants and which must
never block; whether `RingOffer`/`RingReady` change; the version accounting
against RFC 0005 and RFC 0006; and acceptance criteria for the implementation
lane.

Out of scope: implementing anything; accepting this RFC; answering Q2 (access
control), Q3 (`RingError` vs `IpcError`), Q4 (late joiner) or Q5 (compact
counters) — each is referenced where it interacts; the slot layout, the ring
header layout and the publication ordering of RFC 0003 §6 (all unchanged here);
a back-pressure signal for rings that do not opt in (there still is none).

## 3. Facts and decisions

| Item | Kind | Source |
|---|---|---|
| A ring is SPSC, one-way, fixed `frame_size` at creation; the producer drops on overrun; the consumer polls | fact | RFC 0003 §§5–7, §12 |
| The consumer owns `read_seq`, the producer owns `write_seq`/`dropped_seq`; the handshake is one release/acquire pair per counter | fact | RFC 0003 §6, `crates/rustamp-ipc/src/shm/ring.rs` |
| `RingProducer` exposes `create`, `attach`, `kind`, `frame_size`, `capacity`, `generation`, `expect`, `mapping_size`, `frames_written`, `frames_dropped`, `frames_available`, `free_frames`, `is_closed`, `close`, `validate`, `write_from`, `write_slot` | fact | `shm/ring.rs` (a superset of the RFC 0003 §10 sketch: `free_frames`, the accessors and `frames_available` on the producer are in the code) |
| `RingConsumer` exposes `attach`, `kind`, `frame_size`, `capacity`, `generation`, `expect`, `frames_read`, `frames_published`, `frames_dropped`, `frames_available`, `is_closed`, `close`, `validate`, `read_into`, `read_slot_into` | fact | `shm/ring.rs` |
| `RingExpect` is `Copy` and carries `kind`, `frame_size`, `capacity`, `generation` — **not** the ring name; `Message::ring_expect()` builds it from a `RingOffer` and drops `name` | fact | `shm/ring.rs`, `message.rs` |
| `ring_name(kind, generation)` builds `rustamp-<kind>-<pid>-<generation hex>`; the name is "chosen by the creator and carried in `RingOffer`; it is an opaque string to the transport" | fact | RFC 0003 §3, `shm/region.rs` |
| `generation` is a non-zero `u64` epoch; every opener validates `header.generation == offer.generation` before touching a counter; a live participant that later sees a different generation treats the ring as stale and stops | fact | RFC 0003 §8 |
| `state` is an atomic flag (`RING_STATE_LIVE` 0 / `RING_STATE_CLOSED` 1); `close()` stores `RING_STATE_CLOSED` with `Release`; `Drop` releases the mapping and does **not** set the flag | fact | RFC 0003 §8, `shm/ring.rs`, `shm/region.rs` |
| `RingProducer`/`RingConsumer` are `Send`, not `Sync`; a blocking read/write takes `&mut self` | fact | `shm/ring.rs` |
| `RingError` is not a wire type (no serde); `reason()` maps the peer-visible cases onto `RingRefuseReason`, and local-only cases onto `None` | fact | `shm/mod.rs` |
| `PROTOCOL_VERSION = 1`; `Message::RingOffer`/`RingReady`/`RingRefused` exist at v1 | fact | `message.rs` |
| RFC 0005 §12 bumps to **2** (app commands); RFC 0006 §7 bumps to **2** (hosting messages) | fact | RFC 0005 §12, RFC 0006 §7 |
| `unsafe` is confined to `shm::region` and `shm::ring`; every block carries a `// SAFETY:` comment | fact | RFC 0003 §11, `shm/mod.rs` |
| The crate enables only `Win32_Foundation`, `Win32_Security`, `Win32_System_Memory` from the `windows` crate today | fact | `crates/rustamp-ipc/Cargo.toml` |
| `cargo xtask ring-cross` runs a 32-bit producer against a 64-bit consumer and the reverse | fact | `docs/HANDOFF.md` |
| A named object without a `\` prefix lives in the session namespace; its handle is per-process; the object is destroyed when the last handle closes | fact | Win32 documentation (same basis as RFC 0003 §3) |
| An auto-reset event that is set while no thread waits stays signaled; the next wait returns immediately and resets it | fact | Win32 documentation |
| `CreateEventW`/`OpenEventW` on an existing name return a handle to the **existing** object (the section's behaviour in RFC 0003 §3, repeated) | fact | Win32 documentation |
| Blocking is opt-in, PCM-only, and built on two auto-reset events per ring | decision | this RFC (§§4–7) |
| The ring's wake is a `rustamp-ipc`-local resource; no `Message` changes | decision | this RFC (§§10–11) |

## 4. The mechanism: two auto-reset events per ring

### 4.1 The shape

A **wake ring** is a ring whose creator created two **named, auto-reset events**
and whose consumer opened them:

- **`Wake::Data`** — signaled by the **producer** after it advances `write_seq`.
  The **consumer** waits on it when it observes no data.
- **`Wake::Space`** — signaled by the **consumer** after it advances `read_seq`.
  The **producer** waits on it when it observes no free space.

Each event has exactly **one waiter** (the ring is SPSC) and exactly **one
signaler**, so a signal can never be consumed by the wrong end. The card's
phrasing "one event per direction" is read here as *one per wait direction*
(two per ring); §4.2 explains why one per ring is not enough.

The events carry **no state about the data**. The condition is always re-derived
from the header counters, which are already the single source of truth and are
already generation-guarded. That is the decisive property of this mechanism:
a wake object can be lost, duplicated or spuriously signaled without ever
turning into a wrong frame or a wrong count, because the wake is only a hint to
re-read the counters.

### 4.2 Why one event per ring does not work

A single auto-reset event shared by both ends conflates "readable" and
"writable", and the latch can be consumed by the wrong end. A concrete
deadlock, with `capacity = 1`:

1. The ring is full. The producer enters `write_from_blocking`, observes
   `free == 0` and is preempted just before it waits.
2. The consumer reads the one frame, advances `read_seq`, and sets the event
   (meaning "space available"); no one is waiting, so the event latches.
3. The consumer loops, observes the ring empty, decides to wait, and **consumes
   the latch**; it re-checks, still empty, and waits again.
4. The producer resumes and waits — the latch is gone. The consumer sleeps for
   data, the producer sleeps for space, and the ring is empty: a deadlock that
   no future state change repairs.

Two events remove the conflation: the consumer only ever consumes a data latch,
the producer only ever consumes a space latch.

### 4.3 Rejected alternatives

| Alternative | Verdict | Why |
|---|---|---|
| One auto-reset event per ring | rejected | Conflates two conditions; the interleaving above deadlocks (§4.2). |
| One manual-reset event per ring, both ends waiting | rejected | A manual-reset event wakes *both* ends and stays signaled until reset; the reset races the waiters, and the conflation remains. |
| Semaphore with count = free slots (plus one for data) | rejected | A second source of truth that can drift from `write_seq`/`read_seq` (a crashed peer, a late joiner, a missed release) and cannot be reconstructed from the header; `ReleaseSemaphore` fails past its maximum, so a double release is a hard error; `close()` still needs an extra signal, and the waiter still has to distinguish it from data. |
| Pure credit messages on the control channel | rejected | The control channel is a serialized `Transport` stream: a credit can queue behind `Ping`/`PlayerStateUpdate`/commands, so the audio feed's pacing would depend on control-plane latency and ordering; it needs a demultiplexer on the receiving side; it duplicates the counters; it makes the audio path depend on the pipe staying healthy. |
| `WaitOnAddress`/`WakeByAddress*` on a new word in the ring header | strongest alternative; rejected **for now** | Technically cleaner (no kernel object, no name, no handle lifetime, no DACL interaction, no create/open pairing, and a conditional wake whose safety rests on the comparand check rather than on a latch argument) but it needs a new word in the cross-process header — the ABI shared with the 32-bit host — and a minimum-OS floor the repository does not state. Recorded as open question O4; if the owner sets the floor, this is the better mechanism. |
| Spin/poll with a backoff (the status quo) | rejected | RFC 0006 §4 forbids lossy polling for decode/DSP; a spin burns a core, and a sleep-based poll adds unbounded latency and jitter to a path that needs neither. |

## 5. Lifecycle

### 5.1 Names

The two events are named by a pure function of the ring mapping name, which the
creator already chose and which `RingOffer.name` already carries:

```rust
// rustamp-ipc::shm
pub enum Wake { Data, Space }

/// `<ring_name>-data` / `<ring_name>-space`, same session namespace as the
/// section (RFC 0003 §3: no `\` prefix).
pub fn ring_wake_name(ring_name: &str, which: Wake) -> String;
```

Consequences:

- **No wire change is needed to find the events.** Both ends have the ring name
  (`RingOffer.name` for the opener, its own choice for the creator), so the
  event names are derived, not announced.
- The names live in the same session namespace as the section, so the security
  limitation of RFC 0003 §3 applies unchanged: a null `SECURITY_ATTRIBUTES` means
  the default DACL, exactly as for the section. This RFC assumes **no** private
  DACL (Q2 is open) and must not be read as answering Q2. Whatever Q2 decides for
  the section must be applied to the events in the same change, or the two
  objects diverge (open question O6).
- Kernel object names are bounded by `MAX_PATH`. The recommended
  `ring_name` is ~40 characters, so the derived names fit with room to spare; a
  creator that invents a very long ring name must keep the derived names legal.
- The events are created for a **wake ring only**. A creator that wants drops
  (a vis feed, an output mirror) creates nothing and pays nothing.

### 5.2 Who creates, owns and closes

- **Creator.** The side that owns the ring header (RFC 0003 §8: "the creator is
  the only writer of the header metadata and must own the ring") calls
  `RingWake::create(&ring_name)` — two `CreateEventW` calls with
  `bManualReset = FALSE`, `bInitialState = FALSE`, and the derived names. It
  creates them **before** sending `RingOffer`, so an opener that receives the
  offer can always open them.
- **Opener.** It calls `RingWake::open(&offer.name)` after the ring header has
  validated (so a stale or foreign ring is rejected before any kernel object is
  touched).
- **Ownership.** Each side owns **its own handles**; `RingWake` closes them in
  `Drop`, and `close()` on the wake is idempotent. The kernel object itself
  disappears when the last handle closes. No handle is inherited, shared or
  duplicated across the boundary — there is no `DuplicateHandle` step and no
  handle crosses the wire.
- **Attaching.** `with_wake` hands the wake to the ring
  (`RingProducer::with_wake`, `RingConsumer::with_wake`), so the ring signals it
  itself and no caller can forget to.

Creation order, and why:

1. Creator: `NamedRegion::create(name, size)` (or an `AnonymousRegion` in tests).
2. Creator: `RingProducer::create(region, kind, frame_size, capacity)`.
3. Creator: `RingWake::create(&name)`.
4. Creator: `producer.with_wake(wake)`.
5. Creator: send `RingOffer { name, kind, frame_size, capacity, size, generation, direction }`.
6. Opener: `NamedRegion::open(name, size)` (or the shared region).
7. Opener: `RingConsumer::attach(region, expect)` (validates magic, version,
   size, kind, frame size, capacity, generation).
8. Opener: `RingWake::open(&name)`, then `consumer.with_wake(wake)`.
9. Opener: send `RingReady { kind, generation }`.

If step 8 fails, the opener refuses the ring with the existing
`RingRefused { kind, generation, reason: RingRefuseReason::MappingUnavailable }`
(the event is a mapping-like kernel object; see O2 for a dedicated reason).

### 5.3 Stale generations must not wake the wrong consumer

- The recommended `ring_name` embeds the pid and the generation, so a
  re-created ring gets **new** section *and* event names. A stale generation's
  consumer holds a handle to the stale event and can never be woken by the new
  generation's producer.
- RFC 0003 §8 allows a creator to reuse a name when it is *deliberately*
  replacing a ring. Then `CreateEventW` attaches to the **existing** event
  object, and an old consumer still waiting on it *can* be woken by the new
  producer. The safety net is therefore not the name but the rule below.
- **Rule.** After every wake, a waiter re-checks the header generation before it
  treats the wake as data (a cheap `generation` load or a full `validate()`; the
  blocking path is not the hot path, so RFC 0003 §8's "the per-frame hot path
  does not re-read it" does not apply). A generation change ends the wait with
  `RingError::Stale`. A wake from the wrong generation therefore costs at most
  one spurious wakeup and can never be mistaken for data.

## 6. Wake-up and loss rules

### 6.1 The rule (check-then-wait with the counter as the condition)

Both ends use the same shape: **check the condition from the counters; only if
it is unmet, wait; on return, re-check.** The generation is checked first
(§5.3), the state second, the counter third.

```rust
// consumer (read_into_blocking)
loop {
    validate()?;                                     // magic, version, generation (RFC 0003 §8)
    let write_seq = load_write_seq(Acquire);
    let read_seq  = load_read_seq(Relaxed);
    if write_seq.wrapping_sub(read_seq) > 0 {
        return self.read_into(out);                  // drains everything available
    }
    if self.is_closed() {
        return Err(RingError::Closed);
    }
    match wait(self.wake.data, timeout)? {           // WaitForSingleObject
        WaitOutcome::Signaled => continue,           // re-check; never assume data
        WaitOutcome::TimedOut => return Ok(0),       // "nothing within the timeout"
    }
}

// producer (write_from_blocking)
loop {
    validate()?;
    if self.is_closed() {
        return Err(RingError::Closed);
    }
    let write_seq = load_write_seq(Relaxed);
    let free = capacity - (write_seq.wrapping_sub(load_read_seq(Acquire)));
    if free > 0 {
        return self.write_available(samples);        // min(offered, free) whole frames, no drop
    }
    match wait(self.wake.space, timeout)? {
        WaitOutcome::Signaled => continue,
        WaitOutcome::TimedOut => return Ok(0),
    }
}
```

The argument is the standard one, and it rests on the auto-reset latch:

- A waiter only waits after observing its condition **unmet**.
- The condition can only become met through the *other* end advancing its own
  counter, and that end always signals when it advances that counter (§6.2).
- The event latches: a signal that lands between the check and the wait is not
  lost; the wait returns immediately.

There is exactly one waiter per event, so a signal cannot be stolen by the other
end (which is the failure mode of §4.2).

### 6.2 When the signaler signals

- **Producer** → `Wake::Data`, after the `write_seq` `Release` store.
- **Consumer** → `Wake::Space`, after the `read_seq` `Release` store.
- **Either end on `close()`** → both events, after the `state` `Release` store.

The rule is **unconditional**: every advance signals. The only guard an
implementation may apply is that its own store happened — a call that accepted
or read zero frames changed nothing and signals nothing. That costs one
`SetEvent` per advance, and §6.1's argument depends on it.

#### The withdrawn refinement (2026-10-18)

This section used to recommend an edge-triggered refinement as "safe and
cheap", and it is neither. For the record, the withdrawn text proposed to
signal only when the advance could have made the peer's condition true, i.e.

- the producer signals `Wake::Data` iff the write **started from empty**
  (`write_seq == read_seq` before the write), and
- the consumer signals `Wake::Space` iff the read **started from full**
  (`write_seq - read_seq == capacity` before the read),

with this proof: "the consumer waits only after observing empty, so its
observed `read_seq` does not change while it waits; the producer's next write
reads its *own* `write_seq` and **that same `read_seq`** […] so the producer's
check must observe empty too — so it signals."

**"That same `read_seq`" is the step the proof needs and the step it skips.**
Nothing orders the producer's load of `read_seq` after the consumer's last
store to it. Each side decides its edge from the counter the *peer* owns, read
before its own operation; a plain interleaving therefore lets it evaluate a
pre-drain (or pre-fill) snapshot. Capacity 3, one frame in the ring
(`write_seq = 10`, `read_seq = 9`), every step one atomic operation:

1. **P** enters `write_from_blocking`, loads `write_seq = 10`, loads
   `read_seq = 9` → occupancy 1 → "not from empty". P is descheduled before its
   `write_seq` store.
2. **C** reads the frame and stores `read_seq = 10`. Its own edge
   (`available == capacity`) is false, so it sends no `Wake::Space` either.
3. **C** is asked for more, finds `write_seq = 10`, `read_seq = 10` → empty,
   not closed → waits on `Wake::Data`.
4. **P** resumes, stores `write_seq = 12`, and sends **no** signal, because
   step 1 already decided the ring was not empty.

C sleeps on data the ring holds; P fills the ring, finds no space and sleeps on
a `Wake::Space` that C — not looking at the ring at all — will never send. The
ring is neither closed nor failed, so nothing else ends the wait. The
occupancy invariant the old proof leaned on holds throughout; it simply says
nothing about *which snapshot* each side used. The auto-reset latch does not
rescue it either: it buys C exactly one extra re-check at step 3, after which
the latch is clear and the next wait is unbounded.

**Withdrawn, not corrected.** Correcting the proof would mean changing the
rule, not the argument: the premise it needs ("the producer's load of
`read_seq` is ordered after the consumer's last store to it") is false for the
code the refinement describes, and no rewording makes it true. A *different*
rule could be made sound — store first, then re-read the peer's counter behind
a `SeqCst` fence and signal if the peer's condition now holds (the
store-then-recheck shape), or add a header waiter flag and signal only when it
is set — but both need a fence or a new header word on the hot path, both are
easy to get wrong in exactly the way that produced this defect, and neither was
what §6.2 claimed. Since the cost of being right is a single `SetEvent` per
advance on a path that already does a shared-memory copy, the unconditional
rule wins on merit and not merely on safety. Recorded as open item **O15** for
anyone who later wants the syscall-free path back and is prepared to prove it.

A wake is a **hint**: the waiter re-reads the counters and drains everything
available (`read_into` already returns `min(want, available)`), so N writes may
be served by one wake and one wake may serve zero frames if a race resolved
itself. That is what makes the unconditional rule free of risk in the other
direction — an extra signal costs one wait return and one re-check, never a
frame (§6.3).

### 6.3 Spurious wakeups

Windows auto-reset events do not wake spuriously, but the loop must tolerate a
wake that finds its condition unmet anyway: a signal that raced with the
condition already being met, a signal from a reused event name across
generations (§5.3), and the `close()` signal are all handled by the same
re-check. A wake is never evidence of data; only the counters are.

### 6.4 Timeout

`timeout: Option<Duration>` — `None` means "until signaled, closed or stale"
(`INFINITE`); `Some(d)` maps to `WaitForSingleObject(handle, ms)` with the
duration rounded up (a sub-millisecond duration becomes 1 ms). A timeout is
**not** an error:

- `wait_readable`/`wait_writable` return `Ok(WaitOutcome::TimedOut)`.
- `read_into_blocking` returns `Ok(0)` (the same "nothing available yet" meaning
  `read_into` already has), and `write_from_blocking` returns `Ok(0)` (nothing
  was written because the ring stayed full).

A signal that races the timeout is not lost: the auto-reset latch survives, so
the next wait returns immediately and the re-check sees the condition.

### 6.5 Shutdown: `close()` wakes the waiter with `Closed`

`RingProducer::close` and `RingConsumer::close` (existing signatures, `&mut
self`) gain one step on a wake ring:

```rust
state.store(RING_STATE_CLOSED, Release);   // first: the waiter must see it
SetEvent(wake.data);
SetEvent(wake.space);
```

The order is the contract: the state store is `Release` and happens **before**
the signals, so a waiter that is released by either signal observes
`RING_STATE_CLOSED` on its re-check and returns `Err(RingError::Closed)`. A
waiter that still finds data in the ring returns that data first (the loop
checks `available` before `is_closed`), which is exactly the drain-then-EOS
order RFC 0006 §5 step 6 needs; the next call returns `Err(Closed)`, and the
engine-side `Decoder` wrapper maps that to `Ok(0)`/EOS ("ring closed and
consumed"). On a ring without a wake, `close()` is unchanged.

`Drop` keeps RFC 0003 §8's behaviour: it releases the mapping and closes this
side's event handles, and it does **not** set `state` and does **not** signal.
An abrupt exit therefore still looks like a stalled peer, and a waiter's only
escape is its timeout (or a peer that closes).

### 6.6 Local shutdown vs peer shutdown

`RingProducer`/`RingConsumer` are `Send` and not `Sync`, and the blocking calls
take `&mut self`, so a second thread cannot call `close()` on the same object
while the first is blocked inside it. That is deliberate and it shapes shutdown:

- **Peer shutdown** (the normal case): the peer's `close()` (or its exit path)
  sets the state and signals, and the blocked waiter returns `Closed`. RFC 0006
  §5/§6 use this on seek, format change, reconfigure, EOS and error; RFC 0006 §8
  turns a missed `Pong` into exactly this close.
- **Local shutdown** of a blocked worker is *not* a ring call: the supervisor
  uses a finite timeout plus a shared shutdown flag that the worker re-checks
  between waits. An `Arc`-shared close handle that could set the state from
  outside the borrowing worker is possible but is more machinery than the
  consumers need today; recorded as open question O9.
- **Consequence for defaults:** an infinite wait is only safe where a liveness
  path (RFC 0006 §8 `Ping`/`Pong`, or a peer close on every exit path)
  guarantees a wake. The hosting code should use a finite timeout on the write
  side at least, so a mismatched pairing or a dead peer is a loud error, not a
  hang (open question O3).

### 6.7 Where sleeping is and is not allowed

- **Sleeping is allowed only inside** `wait_readable`, `wait_writable`,
  `read_into_blocking` and `write_from_blocking`. These are the only calls that
  may call `WaitForSingleObject`.
- **Sleeping is never allowed** in `create`, `attach`, `with_wake`,
  `RingWake::create`/`open`/`close`, `write_from`, `write_slot`, `read_into`,
  `read_slot_into`, `frames_*`, `free_frames`, `is_closed`, `validate`,
  `close`, `Drop`, or anywhere in `shm::header`/`shm::region`. `SetEvent` is a
  syscall, not a sleep; it may be called from `write_from`/`write_slot`/
  `read_into`/`read_slot_into`/`close` on a wake ring.
- **The audio render path and the vis path must never call a blocking method.**
  The vis ring is created without a wake at all (§9), so a blocking call on it
  is a typed error rather than a latent stall.
- The hot path stays **allocation-free and lock-free**: the new code allocates
  only in `RingWake::create`/`open` (name building, off the ring path) and takes
  no lock anywhere.

## 7. API sketch

Quoting the existing names exactly; the additions are `shm::wake` (new module)
and four methods plus two error variants in `shm::ring`/`shm::mod`.

```rust
// shm::wake — new module; the handles are #[cfg(windows)] like NamedRegion,
// with an in-process backing for the non-Windows (informational) CI job.
pub enum Wake { Data, Space }

pub fn ring_wake_name(ring_name: &str, which: Wake) -> String;

/// The two auto-reset events of a wake ring; owns this side's handles.
pub struct RingWake { /* HANDLE data, HANDLE space */ }

impl RingWake {
    pub fn create(ring_name: &str) -> Result<RingWake, RingError>;
    pub fn open(ring_name: &str) -> Result<RingWake, RingError>;
    pub fn close(&mut self);                     // idempotent; also in Drop
}

/// What ended a blocking wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome { Signaled, TimedOut }

impl<R: MappedRegion> RingProducer<R> {
    // existing: create, attach, kind, frame_size, capacity, generation, expect,
    //           mapping_size, frames_written, frames_dropped, frames_available,
    //           free_frames, is_closed, close, validate, write_from, write_slot
    pub fn with_wake(self, wake: RingWake) -> Self;
    pub fn wait_writable(&self, timeout: Option<Duration>) -> Result<WaitOutcome, RingError>;
    pub fn write_from_blocking(
        &mut self,
        samples: &[f32],
        timeout: Option<Duration>,
    ) -> Result<usize, RingError>;
}

impl<R: MappedRegion> RingConsumer<R> {
    // existing: attach, kind, frame_size, capacity, generation, expect,
    //           frames_read, frames_published, frames_dropped, frames_available,
    //           is_closed, close, validate, read_into, read_slot_into
    pub fn with_wake(self, wake: RingWake) -> Self;
    pub fn wait_readable(&self, timeout: Option<Duration>) -> Result<WaitOutcome, RingError>;
    pub fn read_into_blocking(
        &mut self,
        out: &mut [f32],
        timeout: Option<Duration>,
    ) -> Result<usize, RingError>;
}
```

Semantics of the additions:

- `with_wake` attaches the wake. From then on the ring signals on every counter
  advance (§6.2). A ring without a wake keeps today's behaviour and cost
  exactly.
- `wait_readable`/`wait_writable` are the primitive: they check the generation,
  the state and the counter, and wait only if the condition is unmet. They never
  copy data. `Signaled` means "re-check", `TimedOut` means "nothing changed in
  time".
- `read_into_blocking(out, timeout)` waits for at least one whole frame and then
  behaves like `read_into`: frame-alignment is checked as today
  (`RingError::NotFrameAligned`), `Ok(0)` only on timeout, `Err(Closed)` on
  shutdown, `Err(Stale)` on a generation change.
- `write_from_blocking(samples, timeout)` waits for at least one whole frame of
  space, then writes `min(offered, free)` whole frames and returns the accepted
  count. It **never drops and never advances `dropped_seq`**; the caller loops
  if it wants all frames in. `Ok(0)` means the timeout expired with the ring
  still full. On a wake ring driven only through the blocking calls,
  `frames_dropped()` stays 0 by construction — that is the lossless property
  RFC 0006 §4 needs.
- `RingError` gains `NoWakeObject` (a blocking call on a ring without a wake)
  and reuses `MappingFailed { op, code }` for a failing event syscall (the
  message says "mapping operation"; a rename or a `WakeFailed` variant is open
  question O10). Both map to `None` in `reason()`, so no wire reason is invented
  and Q3 is untouched.

## 8. What changes in the code

`shm::wake` (new) owns the four Win32 calls (`CreateEventW`, `OpenEventW`,
`SetEvent`, `WaitForSingleObject`) and the `CloseHandle` in `Drop`. The crate's
`windows` feature list gains `Win32_System_Threading`. `unsafe` stays inside
`shm`; `shm::ring` gains only safe calls into `shm::wake`, and every new
`unsafe` block carries a `// SAFETY:` comment (RFC 0003 §11). `RingWake` needs
`unsafe impl Send` for the same reason `NamedRegion` does.

Signalling is added only where a counter is stored, always after the existing
`Release` store (the publication ordering of RFC 0003 §6 is untouched):

| Place | After | Signals |
|---|---|---|
| `RingProducer::write_from` | `write_seq` `Release` store | `Wake::Data`, whenever the store happened |
| `RingProducer::write_slot` | `write_seq` `Release` store | `Wake::Data`, whenever the slot was accepted |
| `RingConsumer::read_into` | `read_seq` `Release` store | `Wake::Space`, whenever the store happened |
| `RingConsumer::read_slot_into` | `read_seq` `Release` store | `Wake::Space`, whenever a slot was delivered |
| `RingProducer::close` / `RingConsumer::close` | `state` `Release` store | both, unconditionally |

(The "if the write started from empty" / "if the read started from full"
conditions this table carried until 2026-10-18 are the withdrawn refinement of
§6.2. `write_from_blocking` and `read_into_blocking` carry the same rule at
their own stores, which makes seven signalling sites rather than five.)

Everything else is untouched: the slot layout, the header layout, the
`magic`-last initialisation, the wrap-around arithmetic, the overrun/underrun
rules of RFC 0003 §7, `RingExpect`, `ring_name`, `fresh_generation`,
`MappedRegion`, `AnonymousRegion`, `NamedRegion`, the framing, and the
`Message` enum.

## 9. What stays non-blocking

Blocking is a **PCM-ring** facility, and it is opt-in:

- `write_from`, `write_slot`, `read_into`, `read_slot_into` keep their exact
  signatures and semantics (drop on overrun, `Ok(0)` = nothing yet, never
  sleep). On a wake ring they additionally signal, which is a syscall, not a
  block.
- **No blocking variant exists for the vis path.** `read_slot_into` and
  `write_slot` gain nothing, and vis rings are created without a wake, so the
  vis consumer (30–60 Hz polling, RFC 0003 §12) and the vis tap (which runs on
  the audio render path) cannot block by construction.
- **No blocking variant exists for the audio render path.** The output puller
  keeps reading non-blocking; the lossless paths (the decoder feed, the DSP
  round trip) run on their own threads, which are the only ones allowed to
  wait.
- A blocking call on a wake-less ring returns `RingError::NoWakeObject`; a
  blocking call on a vis ring is therefore a programming error, not a stall.

## 10. `RingOffer` / `RingReady` do not change

Both stay exactly as RFC 0003 §9 defined them, for four reasons:

1. **The information is already there.** `RingOffer.name` is on the wire and the
   event names are a pure function of it (§5.1). A field would be redundant, and
   redundant wire data creates a consistency requirement (the field must equal
   the derivation) with no benefit.
2. **Changing an existing variant's shape is a positional postcard change.** RFC
   0005 §7 rejected exactly that for `OpenPrefs` ("a unit variant gains a
   payload; old peers mis-decode"). `RingOffer` is a struct variant, so a new
   field moves every following field.
3. **The wake is a transport-local resource**, like the mapping handle: it is
   discovered by name and validated by the header, not negotiated in the
   protocol. RFC 0003 §3 already treats the name as opaque.
4. **`RingReady` is not needed to make the pairing safe** in the design above:
   the producer's signals latch whether or not anyone waits, and the consumer's
   signals are what the producer needs — the pairing rule is a hosting-contract
   rule (RFC 0006 owns it), and a mismatch surfaces as a write timeout, not a
   wrong frame.

If the owner wants the pairing announced anyway (belt and braces), the two
options are a `wake: bool` on `RingReady` (a `Message` change: rides the v2
bump, must not become a v3) or a flag bit in the ring header (a ring-ABI change:
needs a `RING_VERSION` decision). Both are recorded in O1; neither is proposed
here.

## 11. Version accounting

| Version | Contents | Source |
|---|---|---|
| `PROTOCOL_VERSION = 0` | `Hello`, `Ping`/`Pong`, `LoadPlugin`, `PluginLoaded`/`PluginError`, `PlayerStateUpdate`, `CommandFromPlugin`, `Vis`, `Shutdown`, `WinampIpc` | RFC 0001, `message.rs` |
| `PROTOCOL_VERSION = 1` | adds `RingOffer`, `RingReady`, `RingRefused` | RFC 0003 §9 (in effect) |
| `PROTOCOL_VERSION = 2` | RFC 0005's app-level `Command` variants (+ `Message::CommandToHost(Command)` with M3) **and** RFC 0006's hosting messages (`DecoderOpen`…`DspError`) | RFC 0005 §12, RFC 0006 §7 |
| **This RFC** | **no `Message` change at all** | this RFC §10 |

Accounting:

- **This RFC needs no bump of its own.** It adds no `Message` variant, changes
  no existing variant, and changes no ring-header field: `PROTOCOL_VERSION`
  stays as it is, `RING_VERSION` stays 1, and `RingRefuseReason` is unchanged.
- **It rides the v2 bump.** Its only consumer is RFC 0006 (which is v2) and it
  must land in the same build, so the v2 history entry in `message.rs` should
  name it for completeness: "v2: RFC 0005 app commands; RFC 0006 hosting
  messages; RFC 0007 wake events (`rustamp-ipc`-local, no `Message` change)".
  Writing it into the history is bookkeeping, not a bump.
- **If O1 is decided the other way** (a `wake` field on `RingOffer`/`RingReady`,
  or a new `RingRefuseReason` for a failed event open), that *is* a `Message`
  change: it rides the same v2 bump and must never become a separate v3.
- **`RingError`** gains variants (`NoWakeObject`, and the reuse of
  `MappingFailed`); `RingError` is not a wire type and `reason()` returns `None`
  for them, so the wire is untouched. Q3 (`RingError` vs `IpcError`) is not
  answered and not affected.

## 12. Backwards compatibility

- **Rings without a wake are unchanged**, byte for byte and syscall for
  syscall: no event is created, `with_wake` is never called, and every existing
  call keeps its documented behaviour and its allocation-free/lock-free
  guarantee. All existing `shm` tests (round trip, wrap-around, full-ring drops,
  unpublished vis slot, stale/foreign regions, generation bump, close visibility,
  layout, soak) must stay green unchanged.
- **Rings with a wake** pay one `SetEvent` per state-advancing call — on
  *every* advance, per §6.2 as corrected on 2026-10-18; the "only when the
  advance could have made the peer's condition true" qualifier this bullet
  carried is the withdrawn refinement. The
  guarantee wording changes for them from "no blocking call on the read or write
  path" to "no allocation, no lock, no **sleeping** call, no logging; one
  `SetEvent` on an advance".
- **In-process API:** additions only (`RingWake`, `ring_wake_name`,
  `WaitOutcome`, `with_wake`, four methods, two `RingError` variants). No
  existing signature changes; `RingExpect`, `RingOffer` and the ring header are
  untouched.
- **32-bit / 64-bit:** `CreateEventW`/`OpenEventW`/`SetEvent`/
  `WaitForSingleObject` behave identically on i686 and x64; names are
  per-session, like the section. `cargo xtask ring-cross` must gain a wake-ring
  case (§14).
- **Q4 (late joiner):** a late joiner opens the same events by the announced
  name and blocks for the **next** frame; the wake changes neither where a
  joiner starts (`read_seq`, as today) nor Q4's open question. If anything the
  wake helps: a late joiner's `read_into` advances `read_seq` and therefore
  wakes a producer that is blocked on a full ring.
- **Q2 (access control):** the events get the default DACL, like the section;
  whatever Q2 decides must be applied to both (O6). **Q5 (compact counters):**
  the wake condition is counter-based, so a future counter change does not
  change the mechanism.

## 13. The permissive-crate invariant

`RingWake` lives in `rustamp-ipc` (MIT OR Apache-2.0, RFC 0004) and needs only
the `windows` crate already present in the target section; no new dependency and
no GPL crate enters the permissive set. No wire type is added or moved. RFC
0004's acceptance 4 ("no GPL crate in the permissive set's dependency graph")
must keep holding.

## 14. Acceptance criteria for the implementation lane

1. `shm::wake` exists with `RingWake::create`/`open`/`close` and
   `ring_wake_name`; `with_wake` exists on both ends; the four blocking methods
   exist with the signatures of §7; `RingError::NoWakeObject` exists. `unsafe`
   stays inside `shm`, every new block has a `// SAFETY:` comment, and the
   crate's module docs describe the wake ring and the sleeping rule.
2. **Lossless delivery:** a two-thread test (producer `write_from_blocking`,
   consumer `read_into_blocking`) delivers every frame exactly once and in
   order for a multi-million-frame run, with `frames_dropped() == 0` and
   `received == sent`. Cases: the consumer waits before any data exists; the
   producer waits on a full ring; a block larger than the ring (partial writes,
   caller loops).
3. **No lost wakeup at the check/wait boundary** — sharpened on 2026-10-18,
   because the original wording ("must finish under a watchdog timeout, not
   hang") was satisfied by a test that could not fail: it looped on `Ok(0)`, so
   a skipped signal only made it slower. A test for this criterion must
   **fail**, not merely slow down, when a signal is skipped. Two shapes
   qualify, and the implementation should have both:
   - the stress test (one frame per call, alternating ends,
     `std::hint::spin_loop`/`yield` jitter) with a bound that is an assertion:
     while the peer still has work, `Ok(0)` from a blocking call means the
     signal was skipped, so the test fails and prints the counters that prove
     the condition was already met; and
   - a deterministic test of the signalling rule itself (§6.2): with a third
     handle on the same two latches, drain a latch and assert that the next
     advance — one that starts from a ring that is neither empty nor full —
     sets it again. This needs no race and covers all six signalling sites.
4. **`close()` wakes both waiters:** with a consumer blocked in
   `read_into_blocking(out, None)` and a producer blocked in
   `write_from_blocking(&samples, None)` (two rings, or two phases), a `close()`
   from the peer makes both return `Err(RingError::Closed)` within a bounded
   time, and a `close()` with unread data still delivers the remaining frames
   before `Closed`.
5. **Timeout:** a waiter with `Some(d)` returns `Ok(0)`/`TimedOut` after
   approximately `d`; a signal that races the timeout is not lost (the following
   wait returns immediately).
6. **Generation:** a consumer waiting on generation N, with the mapping
   re-created as generation N+1 under a reused name, returns `Err(RingError::Stale)`
   (or `Closed`) and never returns N+1's frames as its own.
7. **Non-blocking path unchanged:** a wake-less ring behaves identically
   (existing tests green, including the counting-allocator test); a wake ring's
   non-blocking calls signal but never sleep (assert with the same generous
   wall-clock bound the existing "never blocks" test uses).
8. **Vis and render path:** no blocking method is reachable for a vis ring
   (`RingError::NoWakeObject`), and a wake ring is never created for the vis
   kind in the hosting code.
9. **Cross-bitness:** `cargo xtask ring-cross` gains a wake-ring case (32-bit
   producer → 64-bit consumer and the reverse), asserting lossless delivery.
10. **Handles:** create/close many wake rings and assert the process handle
    count is stable (a Windows-only test); `Drop` closes this side's handles and
    does not set `state`.
11. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
    --all-features -- -D warnings` and `cargo test --workspace --all-features`
    green (plus the i686 checks for the permissive crates per HANDOFF §7).
12. The permissive-crate invariant of §13 still holds.

## 15. Open questions

| ID | Question |
|---|---|
| O1 | Should the wake mode be announced at all? Options: nothing (this RFC), a `wake: bool` on `RingReady` (a `Message` change, rides v2), or a `RING_FLAG_WAKE` bit in the ring header (a ring-ABI change, needs a `RING_VERSION` decision). |
| O2 | A dedicated `RingRefuseReason::WakeUnavailable` for a failed event open (a `Message` change, rides v2), or reuse `MappingUnavailable` (this RFC's proposal)? |
| O3 | Timeout policy for the hosting path: infinite waits (relies on RFC 0006 §8 liveness) or a finite default (turns a dead peer or a mismatched pairing into a loud error)? Contract or engine policy? |
| O4 | `WaitOnAddress`/`WakeByAddress*` on a new header word instead of events: strictly cleaner, but needs a new word in the 32-bit/64-bit header ABI and a minimum-Windows floor the repository does not state. Adopt if the owner sets the floor. |
| O5 | Should `RingExpect` carry the ring name (so `attach` can open the wake itself) instead of the explicit `with_wake`? This RFC chose `with_wake` to avoid changing a `Copy` type and every existing call site. |
| O6 | Q2 (access control) must cover the events, not only the section: whatever Q2 decides (a private DACL, a `Global\` name) must be applied to both in one change. Not answered here. |
| O7 | Should a blocking `read_slot_into` ever exist (a bounded vis wait), or is "vis never blocks" permanent? This RFC proposes permanent. |
| O8 | Should `close()` signal unconditionally (proposed) or only when it can prove a waiter exists? |
| O9 | Local shutdown of a blocked worker: the timeout + shared-flag pattern (proposed) or an `Arc`-shared close handle that can set the state from outside the borrowing worker? |
| O10 | `RingError::NoWakeObject` plus reusing `MappingFailed` (proposed), or a distinct blocking-ring type, or a `WakeFailed { op, code }` variant? |
| O11 | Non-Windows backing for the wake: a Linux/macOS primitive (futex / `os_sync_wait_on_address`) for the cross-platform milestone. The RFC defines semantics, not the syscall. |
| O12 | If Q4's late joiner gets its own rule, does it need a "wake is fresh" signal (it currently just blocks for the next frame)? |
| O15 | Does the hot path ever need to be syscall-free again, and if so by which sound rule? §6.2's edge is withdrawn; the candidates are store-then-recheck behind a `SeqCst` fence, or a header waiter flag (a ring-ABI change, see O1/O4). Only with a proof that names the ordering edge the old one skipped, and a test of the shape §14.3 now requires. Added 2026-10-18. |

## 16. Provenance

Design from our own material only: RFC 0001, RFC 0003 (especially §§3, 6–8, 11,
12, Q1–Q5), RFC 0004, RFC 0005 §12, RFC 0006 (§§4, 5, 6, 7, 8, 9) and the
`rustamp-ipc` sources (`shm/header.rs`, `shm/region.rs`, `shm/ring.rs`,
`shm/mod.rs`, `message.rs`, `transport.rs`, `Cargo.toml`). Win32 facts are
documented API behaviour, cited like RFC 0003 §2 cites it. No Winamp source,
SDK material, documentation or disassembly was read, no reference binary was
run, and nothing was downloaded.
