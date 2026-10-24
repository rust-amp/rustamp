# RFC 0003: Shared-memory PCM and visualization rings

Status: **Accepted** on 2026-10-14 by the integration lead (layout, publication
protocol, overrun rules, stale-ring rule and the `RingOffer`/`RingReady`/
`RingRefused` variants with `PROTOCOL_VERSION = 1` are in effect; open questions
Q1–Q5 stay open as follow-ups).
Owner: Lane G (plugin host / IPC). Implementation: `crates/rustamp-ipc`.
Depends on: RFC 0001 (core contracts), spec S09 (visualization data sizes).

RFC 0001 froze `rustamp-ipc`'s `Message` enum at v0 and said that "shared-memory
rings for PCM/vis come later (M3)". This RFC adds them: a lock-free,
allocation-free single-producer/single-consumer (SPSC) byte ring that both
processes map into their address space, and the protocol messages that announce
and acknowledge such a ring. Adding `Message` variants and bumping
`PROTOCOL_VERSION` is a contract change, which is why this document exists.

## 1. Scope

In scope: the mapping, the byte layout of the ring, the publication protocol,
the overrun/underrun rules, the stale-ring rules, and the protocol messages that
carry a ring from one process to the other.

Out of scope (and deliberately not decided here):

- IPC **message semantics**: which `WM_WA_IPC` ids exist and what a plugin
  expects from them. That is spec S10. This RFC carries bytes.
- Plugin call order and lifetime (spec S09) and embed windows (spec S11).
- A **mid-stream format change** inside a ring. The frame size is fixed when the
  ring is created; a new format means a new ring (new name, new generation).
- Resampling, buffering policy, latency compensation and which frames a
  consumer *chooses* to render. S09 section 3.8 describes time-matching for the
  classic analyzer; that logic sits above this transport.
- Crash detection. A dead peer leaves counters frozen, which is indistinguishable
  from a slow peer at this layer. Liveness is the control channel's job
  (`Ping`/`Pong`).

## 2. Facts and decisions

| Item | Kind | Source |
|---|---|---|
| Vis data is 2 channels x 576 `f32` waveform + 2 x 576 `f32` spectrum + a timestamp | fact | `rustamp-core::vis` (RFC 0001), S09 section 5.2/9 |
| All 2.95 plugins are 32-bit x86; the player is x64 | fact | S09 section 0, RFC 0001 |
| `i686-pc-windows-msvc` provides lock-free 64-bit atomics (`cmpxchg8b`) | fact | Rust target definition (`target_has_atomic = "64"`) |
| One process may map the same named section at a different address than the other | fact | Win32 documentation |
| Named sections live in the session namespace; a name without a `\` prefix is per-session | fact | Win32 documentation |
| Ring layout, counters, arithmetic, drop rule, generation rule | decision | this RFC |

## 3. The mapping

- The **creator** allocates the section and maps it read/write:
  Windows `CreateFileMappingW(INVALID_HANDLE_VALUE, PAGE_READWRITE, size, name)`
  followed by `MapViewOfFile`.
- The **opener** maps the same section by name: `OpenFileMappingW` +
  `MapViewOfFile`.
- An in-process **anonymous** region (`AnonymousRegion`) backs the tests. It is
  the same byte layout; only the backing store differs.
- The section size is exactly `RING_HEADER_SIZE + capacity * frame_size` bytes
  (Windows rounds the mapping up to a page internally; `len()` reports the
  requested size). `MAX_RING_BYTES = 1 GiB` is a sanity cap; a larger offer is
  refused.
- A view must be at least `RING_MIN_ALIGN = 8` byte aligned. Both
  `MapViewOfFile` (64 KiB aligned) and `AnonymousRegion` (allocated with
  `align(64)`) satisfy this. `RING_DATA_OFFSET = 0xC0` is a multiple of 64, so
  the data area inherits the base alignment.
- Recommended name: `rustamp-<kind>-<pid>-<generation hex>`, no namespace
  prefix. The name is chosen by the creator and carried in `RingOffer`; it is an
  opaque string to the transport. `rustamp_ipc::shm::ring_name` builds the
  recommended form. The pid and generation components matter: if a *stale*
  section with the same name still exists (a peer still holds a handle) and it
  is smaller than the new one, `CreateFileMappingW` attaches to the old section
  and `MapViewOfFile` for the larger size fails. A unique name avoids that
  entirely; a reused name is only correct when the old ring is being replaced
  deliberately (then the generation bump in section 8 is what makes the old
  consumer notice).
- **Security note (limitation).** A named section in the session namespace is
  reachable by every process of the same user session with the default DACL. The
  ring carries PCM/vis data only, never credentials; but a hostile process in
  the session could read or corrupt a ring. A private DACL or a
  `Global\`-scoped name is *not* implemented here; see open question Q2.

## 4. Header layout (facts after this RFC)

`RING_HEADER_SIZE = 0xC0 = 192` bytes, on x86 and on x86_64. `#[repr(C)]`,
explicit padding, no compiler-dependent layout. `RING_MAGIC = 0x52545352`
(the bytes `R S T R` little-endian), `RING_VERSION = 1`.

| Offset | Size | Type | Name | Written by | Meaning |
|---|---|---|---|---|---|
| 0x00 | 4 | u32 | `magic` | creator, **last** | `RING_MAGIC` |
| 0x04 | 2 | u16 | `version` | creator | `RING_VERSION` |
| 0x06 | 2 | u16 | `kind` | creator | `1` = PCM, `2` = vis |
| 0x08 | 4 | u32 | `header_size` | creator | `0xC0` |
| 0x0C | 4 | u32 | `frame_size` | creator | bytes per slot (see 5) |
| 0x10 | 4 | u32 | `capacity` | creator | number of slots, `> 0` |
| 0x14 | 4 | u32 | `data_offset` | creator | `0xC0` |
| 0x18 | 4 | u32 | `flags` | creator | bit 0: creator is a 32-bit process |
| 0x1C | 4 | atomic u32 | `state` | both | `0` = live, `1` = closed |
| 0x20 | 8 | u64 | `generation` | creator | epoch, see 8 |
| 0x28 | 24 | — | reserved | — | zero |
| 0x40 | 8 | atomic u64 | `write_seq` | producer | frames published |
| 0x48 | 8 | atomic u64 | `dropped_seq` | producer | frames dropped |
| 0x50 | 48 | — | reserved | — | zero |
| 0x80 | 8 | atomic u64 | `read_seq` | consumer | frames consumed |
| 0x88 | 56 | — | reserved | — | zero |
| 0xC0 | | | **slot 0** | producer | data area |

Size on x86: 192 bytes. Size on x86_64: 192 bytes. Every offset above is
identical on both. The three 64-byte cache lines are: metadata (0x00-0x3F),
producer-owned counters (0x40-0x7F) and the consumer-owned counter (0x80-0xBF),
so the two hot counters never share a line.

All three counters are 64-bit and 8-byte aligned in both architectures. On
`i686-pc-windows-msvc` `AtomicU64` is lock-free, so a 32-bit producer and a
64-bit consumer agree on the values; the layout does not depend on
`align_of::<u64>()` because every field position is pinned by explicit padding
and asserted at compile time.

**Initialisation order.** The creator fills every metadata field except `magic`,
zeroes the counters, and writes `magic` **last**. Every header field is a whole
atomic, and the final write is `magic.store(RING_MAGIC, Release)`; an opener
reads `magic` with `Acquire` first. A peer that sees a valid `magic` therefore
sees a complete, consistently initialised header, and no access to a shared
header is a data race even if two processes race on creating it. `generation` is
written before `magic` too.

## 5. Slot layout

`frame_size` is the number of bytes in one slot. The data area is a plain array
of `capacity` slots; a slot never straddles the end of the array, so a slot is
always a single contiguous copy.

**PCM ring (`kind = 1`).** One slot is one audio frame: interleaved `f32`
samples, `frame_size = channels * 4`. The channel count is fixed at creation. A
format change re-creates the ring. `frame_size % 4 == 0` is required.

**Vis ring (`kind = 2`).** One slot is one visualization frame. `VisSlot` is
`#[repr(C)]`, 8-byte aligned, 9232 bytes, no padding:

| Offset | Size | Type | Name |
|---|---|---|---|
| 0x00 | 8 | u64 | `seq` |
| 0x08 | 8 | u64 | `timestamp_us` |
| 0x10 | 4608 | f32[2][576] | `waveform` |
| 0x1210 | 4608 | f32[2][576] | `spectrum` |
| 0x2410 | | | end (9232 bytes) |

`seq` is the frame's **offered index** (see 7) and is written by the producer;
a caller-supplied value is overwritten. `timestamp_us` has the meaning of
`VisFrame::timestamp_us`. The arrays are laid out exactly like
`rustamp_core::vis::VisFrame`, channel-major, so a slot converts to and from a
`VisFrame` with a plain copy. `VIS_SLOT_SIZE = 9232` is asserted at compile
time; if core changes `VIS_SAMPLES`/`VIS_CHANNELS`, the build fails instead of
silently changing the ABI.

## 6. Publication protocol and memory ordering

The ring is single-producer/single-consumer and is **not** internally
synchronised beyond these two atomic handshakes:

1. The producer writes the whole slot body (plain stores / `memcpy`), then
   `write_seq.store(new, Release)`.
2. The consumer does `write_seq.load(Acquire)` and only then reads slot bodies
   with index `< write_seq`. It therefore never observes a half-written slot.
3. The consumer reads the bodies, then `read_seq.store(new, Release)`.
4. The producer does `read_seq.load(Acquire)` before computing free space, so it
   never overwrites a slot the consumer has not finished reading.

`dropped_seq` is stored `Release` by the producer and loaded `Acquire` by
anyone who reads it; it is a statistics counter, not part of the safety
argument.

No allocation, no lock, no blocking call and no logging happens on the read or
write path. Every access to the header is a single atomic load or store.

## 7. Counters, wrap-around and the overrun/underrun rule

Counters are **frame counts**, not byte offsets, and are monotonic `u64`.

- `write_seq` — frames the producer has **published**.
- `dropped_seq` — frames the producer has **dropped**.
- `read_seq` — frames the consumer has **consumed**.

Occupancy is `write_seq - read_seq` and is always `<= capacity`; the invariant
`read_seq <= write_seq` holds because only the consumer advances `read_seq` and
only up to what the producer published. Free space is
`capacity - (write_seq - read_seq)`.

**Wrap-around arithmetic.** With an arbitrary (not necessarily power-of-two)
capacity the slot index is `(seq % capacity)`, computed in `u64` and cast to
`usize`. That is one integer division per operation. We chose it over a
power-of-two mask because the card requires any capacity, and a mask would
silently round a caller's capacity up. A multi-frame copy is split into at most
two contiguous runs (up to the end of the array, then from slot 0), so the data
move is `memcpy`-like, not a per-frame loop.

**Overrun (producer, `write_from` / `write_slot`).** The producer never blocks
and never overwrites unread data.

- It writes `min(offered, free)` whole frames, starting at slot
  `write_seq % capacity`, and drops the remainder **as whole frames**.
- `dropped_seq` increases by the number of dropped frames.
- The call returns how many frames were accepted. It never sleeps, never spins
  and never grows a buffer.
- A partial frame offered to `write_from` (a sample count that is not a multiple
  of the channel count) is a programming error and returns
  `RingError::NotFrameAligned`; it is not silently truncated.
- `write_slot` accepts exactly one whole slot; if the ring is full it returns
  "not accepted" and counts one drop.

**Offered index.** Before a write, `w = write_seq` and `d = dropped_seq`. The
global index of the i-th frame in the offered block is `w + d + i`. Accepted
frames are published with consecutive indices and dropped frames are the *tail*
of the block, so after the call `write_seq + dropped_seq` equals the total
number of frames ever offered. Because accepted frames always come first, the
`seq` values of published slots are strictly increasing; gaps appear exactly
where a slot was dropped. That is what lets a vis consumer report "N frames were
skipped".

**Underrun (consumer, `read_into` / `read_slot_into`).** The consumer reads what
is available and never blocks.

- `read_into` copies `min(want, available)` whole frames and returns the count;
  `Ok(0)` means "nothing available yet", which is normal and not an error.
- `read_slot_into` returns `true` (one whole slot filled) or `false`.
- A frame that has not been published is **not** data. The consumer decides what
  is available purely from `write_seq - read_seq`, which the producer advances
  only after the complete slot body is in memory, so a partially written frame
  is reported as "not yet complete", never as data.
- A read buffer that is not a whole number of frames is a programming error
  (`RingError::NotFrameAligned`).

## 8. Stale, foreign and closed rings

`generation` is a `u64` epoch in the header. It exists so that a ring left over
from a crashed run, or a mapping that happens to have the right name but the
wrong contents, can never be mistaken for live data.

- The creator derives it as: `old.generation + 1` if the region already contains
  a header with `RING_MAGIC` and `RING_VERSION`, otherwise a fresh non-zero
  value built from the process id and the wall clock. It is never `0`.
- `RingOffer` carries the generation, and every opener validates
  `header.generation == offer.generation` **before** touching any counter.
- Validation order and failure reasons (all produce a typed error, never
  undefined behaviour, never a partial read):

  | Check | Failure |
  |---|---|
  | `region.len() >= RING_HEADER_SIZE` | `RegionTooSmall` |
  | `magic == RING_MAGIC` | `BadMagic` |
  | `version == RING_VERSION` | `BadVersion` |
  | `header_size == 0xC0` and `data_offset == 0xC0` | `BadHeaderSize` |
  | `kind` is a known value | `UnknownKind` |
  | `kind == expected` | `WrongKind` |
  | `frame_size == expected` | `FrameSizeMismatch` |
  | `capacity == expected` | `CapacityMismatch` |
  | `generation == expected` | `Stale` |
  | the header's own layout is usable | `InvalidLayout` |
  | `region.len() >= data_offset + capacity * frame_size` | `RegionTooSmall` |

- A live participant that later sees a **different** generation (a new run
  re-created the ring under the same name while it still held a handle) must
  treat the ring as stale and stop. `validate()` re-checks the generation
  cheaply; the per-frame hot path does not re-read it, so a caller that wants
  the guarantee calls `validate()` on its own cadence.
- Creating a ring over a region that a live peer is currently using is a
  protocol error: the generation would change under that peer. The creator is
  the only writer of the header metadata and must own the ring.
- **Closed state.** `state` is an atomic flag: `0` live, `1` closed. `close()`
  on either side sets it. A producer that writes into a closed ring gets
  `RingError::Closed` instead of silently writing data nobody will read; a
  consumer uses `is_closed()` to tell "no data yet" from "peer is done". `Drop`
  only releases the mapping and does **not** set the flag, so an abrupt exit
  looks like a stalled peer; the generation rule covers the next run.

## 9. Protocol changes

`PROTOCOL_VERSION` becomes **1**. Version 0 has no ring variants, so a peer that
only speaks 0 would mis-decode a ring message; the bump makes the mismatch a
clean `Hello` rejection instead of a decode error. RFC 0001's `Message` list is
extended by three variants (additive, but a version change for the reason
above):

```rust
/// M or H: I created a shared-memory ring; open it by this name.
RingOffer {
    name: String,          // mapping name, opaque to the transport
    kind: RingKind,        // Pcm | Vis
    frame_size: u32,       // bytes per slot
    capacity: u32,         // slots
    size: u64,             // total mapping size in bytes
    generation: u64,       // epoch written into the header
    direction: RingDirection, // MainToHost | HostToMain
},
/// The peer opened and validated the ring.
RingReady { kind: RingKind, generation: u64 },
/// The peer could not open it, or it is stale/foreign.
RingRefused { kind: RingKind, generation: u64, reason: RingRefuseReason },
```

`direction` is what tells the opener which role to take: the end named by
`MainToHost`/`HostToMain` builds the consumer view, the other end the producer
view. Without it the role would have to be inferred from `kind`, which would
bake an S10-level convention into the transport.

`RingRefuseReason` mirrors the validation table in section 8
(`MappingUnavailable`, `BadMagic`, `BadVersion`, `BadHeaderSize`, `WrongKind`
— which also covers an unknown kind value —, `FrameSizeMismatch`,
`CapacityMismatch`, `Stale`, `RegionTooSmall`, `InvalidLayout`). `IpcError` is
**not** changed; ring failures use `shm::RingError`, which converts to
`RingRefuseReason`.

## 10. API

```rust
// Transport-level types
pub enum RingKind { Pcm, Vis }
pub enum RingDirection { MainToHost, HostToMain }
pub struct RingExpect { kind, frame_size, capacity, generation }
pub struct VisSlot { seq, timestamp_us, waveform, spectrum }

// Mapping
pub trait MappedRegion: Send { unsafe fn base(&self) -> *mut u8; fn len(&self) -> usize; }
pub struct AnonymousRegion;                       // in-process, for tests
#[cfg(windows)] pub struct NamedRegion;           // named file mapping

// Ring
pub struct RingProducer<R: MappedRegion>;
pub struct RingConsumer<R: MappedRegion>;

impl<R> RingProducer<R> {
    pub fn create(region: R, kind: RingKind, frame_size: u32, capacity: u32) -> Result<Self, RingError>;
    pub fn attach(region: R, expect: RingExpect) -> Result<Self, RingError>;
    pub fn generation(&self) -> u64;
    pub fn expect(&self) -> RingExpect;
    pub fn write_from(&mut self, samples: &[f32]) -> Result<usize, RingError>;  // PCM
    pub fn write_slot(&mut self, slot: &VisSlot) -> Result<bool, RingError>;    // vis
    pub fn frames_written(&self) -> u64;
    pub fn frames_dropped(&self) -> u64;
    pub fn validate(&self) -> Result<(), RingError>;
    pub fn close(&mut self);
}

impl<R> RingConsumer<R> {
    pub fn attach(region: R, expect: RingExpect) -> Result<Self, RingError>;
    pub fn read_into(&mut self, out: &mut [f32]) -> Result<usize, RingError>;   // PCM
    pub fn read_slot_into(&mut self, out: &mut VisSlot) -> Result<bool, RingError>; // vis
    pub fn frames_read(&self) -> u64;
    pub fn frames_published(&self) -> u64;
    pub fn frames_available(&self) -> u64;
    pub fn frames_dropped(&self) -> u64;
    pub fn is_closed(&self) -> bool;
    pub fn validate(&self) -> Result<(), RingError>;
    pub fn close(&mut self);
}
```

The vis slot is filled through `&mut VisSlot` rather than returned by value so
that no 9 KiB value crosses the API boundary by accident.

## 11. Unsafe surface

`unsafe` is confined to the `shm` module subtree, and inside it to two places:

- `shm::region` — creating and releasing the mapping (aligned `alloc_zeroed`,
  `MapViewOfFile`, `UnmapViewOfFile`, `CloseHandle`) and exposing the base
  pointer.
- `shm::ring` — dereferencing the validated header pointer for the atomic
  accesses, and copying slot bytes with `ptr::copy_nonoverlapping`.

`shm::header` is pure layout and contains no `unsafe`. Every `unsafe` block
carries a `// SAFETY:` comment naming the invariant. Above `shm`, the crate sees
only `RingProducer`/`RingConsumer`, which own their region and cannot outlive it.

## 12. What is deliberately not decided here

- Which end creates which ring in which situation (that follows from the
  direction fields the *caller* passes; the transport does not enforce it).
- The channel count, sample rate and buffer sizes to use. Those are policy.
- A back-pressure signal. There is none by design: the producer drops.
- Interrupt/event notification. The consumer polls. An event (a semaphore or a
  named event) would be a separate RFC; polling is enough for a vis consumer
  that runs at 30-60 Hz and for an audio puller that is driven by the device.

## 13. Open questions (for later RFCs, not blockers)

| ID | Question |
|---|---|
| Q1 | Should the ring carry an event handle so a consumer can block instead of poll? Needs a second kernel object and a lifecycle rule. |
| Q2 | Access control: a private DACL on the section, or a machine-wide `Global\` name for a host that runs at a different integrity level. |
| Q3 | `RingError` is not part of `IpcError`. If a later milestone wants one error type at the IPC boundary, that is a small additive RFC. |
| Q4 | Should a consumer be able to re-attach mid-stream (a "late joiner" that starts at the newest frame)? Today it starts at `read_seq`, i.e. at the oldest unread frame. |
| Q5 | 64-bit counters mean a ring can never wrap its index space in practice; the arithmetic uses `wrapping_*` anyway. A future RFC could shrink the counters if a compact header is ever needed. |

## 14. Verification

The implementation is checked by: a single-threaded round trip with wrap-around
at non-power-of-two capacities, a multi-million-frame two-thread soak test that
asserts exactly-once/in-order delivery and `received + dropped == sent`, a
full-ring test that asserts whole-frame drops and no overwrite, a vis test that
asserts a not-yet-published slot is never delivered, a counting-allocator test
that asserts zero allocations on the ring path, compile-time and runtime layout
assertions on both architectures, a Windows named-mapping test that re-opens by
name, and a stale/foreign-region test for every failure reason in section 8.
