//! The lock-free SPSC ring (RFC 0003 sections 6 to 8).
//!
//! One [`RingProducer`] and one [`RingConsumer`], possibly in different
//! processes, share a [`MappedRegion`]. Everything here is allocation-free and
//! lock-free on the read and write path; the only synchronisation is the
//! release/acquire pair on the two sequence counters.
//!
//! Raw pointers are used in exactly two ways: dereferencing the header pointer
//! for whole-atomic accesses, and copying slot bytes. Both are confined to the
//! private helpers at the top of this module; the public API is safe.
//!
//! # The signalling rule (RFC 0007 §6.2, as corrected on 2026-10-18)
//!
//! On a wake ring, **every** advance of a counter signals the matching event:
//! the producer signals `Wake::Data` after each `write_seq` store, the consumer
//! signals `Wake::Space` after each `read_seq` store, and `close()` signals
//! both after the `state` store. The only guard is that this side's store
//! actually happened — a call that accepted or read zero frames changed
//! nothing and signals nothing.
//!
//! RFC 0007 §6.2 used to recommend an edge: signal only when the write started
//! from an empty ring, or the read from a full one. That refinement is
//! withdrawn, because each side had to decide its edge from the counter the
//! *peer* owns, loaded before the operation and never ordered after the peer's
//! last store to it. A plain interleaving then lets both sides evaluate a stale
//! edge, neither signals, and both ends sleep on a ring that is neither closed
//! nor failed — the stall diagnosed in `docs/tasks/W19-twoproc-stall.md` §1 and
//! fixed here (`docs/tasks/W19-ring-wake-root.md`). The RFC's own escape clause
//! is this rule: unconditional signalling "costs one extra `SetEvent` per
//! advance and is always correct".
//!
//! A signal is only a hint, so the store it announces must be visible to the
//! peer when the signal is delivered: the peer re-reads the counters after
//! every wake, and a re-read that still sees the old counters would send it
//! back to sleep with the wakeup lost. `shm::wake` therefore puts a barrier
//! before every signal and after every wait; see the module docs there.
//!
//! # The reset rule (RFC 0007 §5.3)
//!
//! A ring that is **reset** — the creator re-creates the section, so the
//! generation moves on — signals the wake once, in both directions. The peers
//! of the old generation are waiting on the same event objects when the
//! section and its events are reused, and they have to learn that the
//! generation moved on: RFC 0007 §5.3 ends such a wait with
//! `RingError::Stale`. Without the signal they would wait for their whole
//! timeout, and an unbounded wait (`timeout: None`) would never end.
//!
//! The reset is the *only* event that changes the ring's identity, so it is
//! the only one that has to say so out loud. Everything else — a frame
//! written, a frame read, a close — is already visible in the counters or in
//! the state, and the waiter re-reads them after every wake.

use core::sync::atomic::Ordering;
use std::time::Duration;

use super::RingError;
use super::Wake;
use super::header::{
    MAX_RING_BYTES, RING_DATA_OFFSET, RING_FLAG_CREATOR_32BIT, RING_HEADER_SIZE, RING_MAGIC,
    RING_STATE_CLOSED, RING_STATE_LIVE, RING_VERSION, RingHeader, RingKind, VIS_SLOT_SIZE, VisSlot,
};
use super::region::{MappedRegion, fresh_generation};
use super::wake::{RingWake, WaitOutcome};

/// What an opener expects to find in a ring header.
///
/// This mirrors the fields of the `RingOffer` message (RFC 0003 section 9) that
/// describe the ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingExpect {
    /// Ring kind.
    pub kind: RingKind,
    /// Bytes per slot.
    pub frame_size: u32,
    /// Number of slots.
    pub capacity: u32,
    /// Generation (epoch) the creator announced.
    pub generation: u64,
}

impl RingExpect {
    /// Build an expectation from its parts.
    pub fn new(kind: RingKind, frame_size: u32, capacity: u32, generation: u64) -> RingExpect {
        RingExpect {
            kind,
            frame_size,
            capacity,
            generation,
        }
    }
}

/// Header metadata, cached at construction. It never changes while a ring is
/// live, so the hot path never re-reads it.
#[derive(Debug, Clone, Copy)]
struct RingMeta {
    kind: RingKind,
    frame_size: u32,
    capacity: u32,
    generation: u64,
}

/// The header pointer of a ring region.
///
/// This is safe because every ring is constructed through `create` or `attach`,
/// which both proved `region.len() >= RING_DATA_OFFSET >= size_of::<RingHeader>()`
/// and that the base address is at least [`RING_MIN_ALIGN`] aligned.
fn header_ptr<R: MappedRegion>(region: &R) -> *mut RingHeader {
    // SAFETY: `MappedRegion::base` is valid and writable for `region.len()`
    // bytes and aligned to at least RING_MIN_ALIGN; `RingHeader` is
    // RING_HEADER_SIZE (0xC0) bytes with alignment <= RING_MIN_ALIGN, and
    // `region.len() >= RING_DATA_OFFSET == RING_HEADER_SIZE` by the
    // constructor invariant, so the header lies inside the mapping.
    unsafe { region.base().cast::<RingHeader>() }
}

/// The first byte of the slot area.
///
/// # Safety
///
/// `region` must satisfy the ring constructor invariant (see [`header_ptr`]).
unsafe fn data_ptr<R: MappedRegion>(region: &R) -> *mut u8 {
    // SAFETY: the base pointer is valid for `region.len()` bytes, and the
    // constructor invariant guarantees `len >= RING_DATA_OFFSET`.
    unsafe { region.base().add(RING_DATA_OFFSET) }
}

/// Validate a ring layout and return the number of bytes it needs.
fn check_layout(kind: RingKind, frame_size: u32, capacity: u32) -> Result<u64, RingError> {
    if frame_size == 0 || capacity == 0 {
        return Err(RingError::InvalidLayout);
    }
    match kind {
        RingKind::Pcm => {
            if !frame_size.is_multiple_of(4) {
                return Err(RingError::InvalidLayout);
            }
        }
        RingKind::Vis => {
            if frame_size as usize != VIS_SLOT_SIZE {
                return Err(RingError::InvalidLayout);
            }
        }
    }
    let needed = RING_DATA_OFFSET as u64 + u64::from(capacity) * u64::from(frame_size);
    if needed > MAX_RING_BYTES {
        return Err(RingError::InvalidLayout);
    }
    Ok(needed)
}

/// Validate an existing header against an expectation and read its metadata.
///
/// The header is the source of truth: identity (magic, version, size) is checked
/// first, then the expectation is compared field by field, and only then is the
/// header's layout validated against the mapping size. Every failure returns a
/// typed error before any counter is read (RFC 0003 section 8).
fn check_region<R: MappedRegion>(region: &R, expect: RingExpect) -> Result<RingMeta, RingError> {
    let have = region.len() as u64;
    if have < RING_HEADER_SIZE as u64 {
        return Err(RingError::RegionTooSmall {
            needed: RING_HEADER_SIZE as u64,
            have,
        });
    }
    let h = header_ptr(region);
    // The creator publishes the header with a release store of `magic`; this
    // acquire load therefore makes every other metadata field visible.
    // SAFETY: `h` points to a live header inside the mapping (see `header_ptr`).
    let magic = unsafe { (*h).magic.load(Ordering::Acquire) };
    if magic != RING_MAGIC {
        return Err(RingError::BadMagic { found: magic });
    }
    // SAFETY: see above.
    let (version, kind_raw, header_size, data_offset) = unsafe {
        (
            (*h).version.load(Ordering::Relaxed),
            (*h).kind.load(Ordering::Relaxed),
            (*h).header_size.load(Ordering::Relaxed),
            (*h).data_offset.load(Ordering::Relaxed),
        )
    };
    if version != RING_VERSION {
        return Err(RingError::BadVersion { found: version });
    }
    if header_size as usize != RING_HEADER_SIZE || data_offset as usize != RING_DATA_OFFSET {
        return Err(RingError::BadHeaderSize);
    }
    let Some(kind) = RingKind::from_u16(kind_raw) else {
        return Err(RingError::UnknownKind { found: kind_raw });
    };
    if kind != expect.kind {
        return Err(RingError::WrongKind {
            expected: expect.kind,
            found: kind,
        });
    }
    // SAFETY: see above.
    let (frame_size, capacity, generation) = unsafe {
        (
            (*h).frame_size.load(Ordering::Relaxed),
            (*h).capacity.load(Ordering::Relaxed),
            (*h).generation.load(Ordering::Relaxed),
        )
    };
    if frame_size != expect.frame_size {
        return Err(RingError::FrameSizeMismatch {
            expected: expect.frame_size,
            found: frame_size,
        });
    }
    if capacity != expect.capacity {
        return Err(RingError::CapacityMismatch {
            expected: expect.capacity,
            found: capacity,
        });
    }
    if generation != expect.generation {
        return Err(RingError::Stale {
            expected: expect.generation,
            found: generation,
        });
    }
    // The header describes a layout this build can use, and the mapping is big
    // enough for it.
    let needed = check_layout(kind, frame_size, capacity)?;
    if have < needed {
        return Err(RingError::RegionTooSmall { needed, have });
    }
    Ok(RingMeta {
        kind,
        frame_size,
        capacity,
        generation,
    })
}

/// The writing end of a ring. Single producer only.
///
/// The producer owns `write_seq` and `dropped_seq` and only ever reads
/// `read_seq`. It never blocks and never overwrites unread data (RFC 0003
/// section 7). When a [`RingWake`] is attached via [`with_wake`], the producer
/// signals the consumer on every publish (RFC 0007).
///
/// [`with_wake`]: RingProducer::with_wake
pub struct RingProducer<R: MappedRegion> {
    region: R,
    meta: RingMeta,
    wake: Option<RingWake>,
    /// This producer took over a section that already held a ring: the
    /// generation moved on, so the peers of the old generation are stale and
    /// have to be woken to notice (RFC 0007 §5.3).
    replaced: bool,
}

/// The reading end of a ring. Single consumer only.
///
/// The consumer owns `read_seq` and only ever reads `write_seq` and
/// `dropped_seq`. When a [`RingWake`] is attached via [`with_wake`], the
/// consumer signals the producer on every consume (RFC 0007).
///
/// [`with_wake`]: RingConsumer::with_wake
pub struct RingConsumer<R: MappedRegion> {
    region: R,
    meta: RingMeta,
    wake: Option<RingWake>,
}

impl<R: MappedRegion> RingProducer<R> {
    /// Create a ring in `region`, initialising the header.
    ///
    /// The generation is derived from the region's current contents (RFC 0003
    /// section 8): an existing valid header is bumped by one, a fresh region
    /// gets a new value. The creator must own the ring; creating over a ring a
    /// live peer is using is a protocol error.
    pub fn create(
        region: R,
        kind: RingKind,
        frame_size: u32,
        capacity: u32,
    ) -> Result<RingProducer<R>, RingError> {
        let needed = check_layout(kind, frame_size, capacity)?;
        let have = region.len() as u64;
        if have < needed {
            return Err(RingError::RegionTooSmall { needed, have });
        }
        let h = header_ptr(&region);
        // The creator owns the ring, so nothing else writes the metadata while
        // this runs. Read the old generation to bump a reused mapping.
        // SAFETY: `h` points to a live header inside the mapping.
        let (old_valid, old_generation) = unsafe {
            (
                (*h).magic.load(Ordering::Relaxed) == RING_MAGIC
                    && (*h).version.load(Ordering::Relaxed) == RING_VERSION,
                (*h).generation.load(Ordering::Relaxed),
            )
        };
        let generation = if old_valid {
            let bumped = old_generation.wrapping_add(1);
            if bumped == 0 { 1 } else { bumped }
        } else {
            fresh_generation()
        };

        // SAFETY: `h` points to a live, exclusively owned header inside the
        // mapping. Every field is written as a whole atomic, and `magic` is
        // stored last with `Release`, which publishes the complete header to
        // any opener that acquires it.
        unsafe {
            (*h).version.store(RING_VERSION, Ordering::Relaxed);
            (*h).kind.store(kind.as_u16(), Ordering::Relaxed);
            (*h).header_size
                .store(RING_HEADER_SIZE as u32, Ordering::Relaxed);
            (*h).frame_size.store(frame_size, Ordering::Relaxed);
            (*h).capacity.store(capacity, Ordering::Relaxed);
            (*h).data_offset
                .store(RING_DATA_OFFSET as u32, Ordering::Relaxed);
            (*h).flags.store(
                if cfg!(target_pointer_width = "32") {
                    RING_FLAG_CREATOR_32BIT
                } else {
                    0
                },
                Ordering::Relaxed,
            );
            (*h).reserved0 = [0; 3];
            (*h).reserved1 = [0; 6];
            (*h).reserved2 = [0; 7];
            (*h).write_seq.store(0, Ordering::Relaxed);
            (*h).dropped_seq.store(0, Ordering::Relaxed);
            (*h).read_seq.store(0, Ordering::Relaxed);
            (*h).state.store(RING_STATE_LIVE, Ordering::Relaxed);
            // The generation is stored last, with `Release`, and it publishes
            // everything above it. That matters for the *reset* path: the
            // magic never changes there, so a peer that waits on the ring and
            // then re-reads the generation (RFC 0007 §5.3) has to see the new
            // value and the restarted counters with it. The `Acquire` load in
            // `validate_header` pairs with this store.
            (*h).generation.store(generation, Ordering::Release);
            (*h).magic.store(RING_MAGIC, Ordering::Release);
        }

        Ok(RingProducer {
            region,
            meta: RingMeta {
                kind,
                frame_size,
                capacity,
                generation,
            },
            wake: None,
            replaced: old_valid,
        })
    }

    /// Attach to an existing ring and validate it against `expect`.
    pub fn attach(region: R, expect: RingExpect) -> Result<RingProducer<R>, RingError> {
        let meta = check_region(&region, expect)?;
        Ok(RingProducer {
            region,
            meta,
            wake: None,
            replaced: false,
        })
    }

    /// Attach a [`RingWake`] to this ring (RFC 0007 §5.2).
    ///
    /// After this call the ring signals on every counter advance and supports
    /// the blocking methods. Called by the creator before sending `RingOffer`.
    ///
    /// If this producer **replaced** a ring — the creator re-created a section
    /// that already held one, so the generation moved on — the attached wake is
    /// signaled once, in both directions. The peers of the old generation are
    /// waiting on the same event objects when the section and its events are
    /// reused (RFC 0007 §5.3), and they must learn that the generation moved
    /// on: the wake ends their wait, the re-check finds the new generation and
    /// the wait ends with `RingError::Stale`. Without the signal they would
    /// wait for their whole timeout, and an unbounded wait would never end.
    ///
    /// A peer whose event objects are *not* shared (a re-created ring gets new
    /// names, RFC 0007 §5.3) cannot be reached this way; its timeout ends the
    /// wait with `RingError::Stale` as well.
    pub fn with_wake(mut self, wake: RingWake) -> Self {
        self.wake = Some(wake);
        if self.replaced
            && let Some(wake) = &self.wake
        {
            // The generation was already stored (with `Release`) before
            // this call, so a peer that wakes on this signal sees the new
            // generation and the restarted counters.
            let _ = wake.signal(Wake::Data);
            let _ = wake.signal(Wake::Space);
        }
        self
    }

    /// Ring kind.
    pub fn kind(&self) -> RingKind {
        self.meta.kind
    }

    /// Bytes per slot.
    pub fn frame_size(&self) -> u32 {
        self.meta.frame_size
    }

    /// Number of slots.
    pub fn capacity(&self) -> u32 {
        self.meta.capacity
    }

    /// Generation (epoch) of this ring instance.
    pub fn generation(&self) -> u64 {
        self.meta.generation
    }

    /// What an opener should expect; hand this to [`RingConsumer::attach`].
    pub fn expect(&self) -> RingExpect {
        RingExpect::new(
            self.meta.kind,
            self.meta.frame_size,
            self.meta.capacity,
            self.meta.generation,
        )
    }

    /// Total mapping size in bytes for this layout.
    pub fn mapping_size(&self) -> u64 {
        RING_DATA_OFFSET as u64 + u64::from(self.meta.capacity) * u64::from(self.meta.frame_size)
    }

    /// Frames published so far.
    pub fn frames_written(&self) -> u64 {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `write_seq` is producer-owned.
        unsafe { (*h).write_seq.load(Ordering::Relaxed) }
    }

    /// Frames dropped so far.
    pub fn frames_dropped(&self) -> u64 {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `dropped_seq` is producer-owned.
        unsafe { (*h).dropped_seq.load(Ordering::Relaxed) }
    }

    /// Frames currently in the ring (occupancy).
    pub fn frames_available(&self) -> u64 {
        self.frames_written()
            .wrapping_sub(self.read_seq(Ordering::Acquire))
    }

    /// Slots that can be written without dropping.
    pub fn free_frames(&self) -> u64 {
        u64::from(self.meta.capacity).saturating_sub(self.frames_available())
    }

    /// Whether the peer (or this side) closed the ring.
    pub fn is_closed(&self) -> bool {
        self.state() == RING_STATE_CLOSED
    }

    /// Mark the ring closed so the peer stops treating "no data" as "not yet".
    ///
    /// On a wake ring this also signals both events so a blocked peer returns
    /// `Err(RingError::Closed)` (RFC 0007 §6.5). On a wake-less ring `close`
    /// is unchanged.
    pub fn close(&mut self) {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `state` is a whole atomic. The
        // `Release` store happens-before the signals below.
        unsafe { (*h).state.store(RING_STATE_CLOSED, Ordering::Release) };
        if let Some(wake) = &self.wake {
            let _ = wake.signal(super::Wake::Data);
            let _ = wake.signal(super::Wake::Space);
        }
    }

    /// Re-check the header (magic, version, generation) after a long run.
    pub fn validate(&self) -> Result<(), RingError> {
        validate_header(&self.region, self.meta.generation)
    }

    /// Write interleaved `f32` PCM frames (PCM rings only).
    ///
    /// `samples.len()` must be a multiple of the channel count. Writes as many
    /// whole frames as fit, drops the rest and counts them, and returns how many
    /// frames were accepted. Never blocks, never allocates, never overwrites
    /// unread data.
    ///
    /// On a wake ring, signals `Wake::Data` after **every** write that advanced
    /// `write_seq`, so a blocked consumer is woken (RFC 0007 §6.2 as corrected
    /// on 2026-10-18; see the signalling rule in the module docs).
    pub fn write_from(&mut self, samples: &[f32]) -> Result<usize, RingError> {
        self.require_kind(RingKind::Pcm)?;
        self.require_live()?;
        let samples_per_frame = (self.meta.frame_size / 4) as usize;
        if !samples.len().is_multiple_of(samples_per_frame) {
            return Err(RingError::NotFrameAligned {
                samples: samples.len(),
                samples_per_frame,
            });
        }
        let offered = samples.len() / samples_per_frame;
        if offered == 0 {
            return Ok(0);
        }

        let write_seq = self.write_seq(Ordering::Relaxed);
        let dropped_seq = self.dropped_seq(Ordering::Relaxed);
        let read_seq = self.read_seq(Ordering::Acquire);
        let occupancy = write_seq.wrapping_sub(read_seq);
        let free = u64::from(self.meta.capacity).saturating_sub(occupancy);
        let accepted = (offered as u64).min(free) as usize;
        let dropped = offered - accepted;

        if accepted > 0 {
            self.copy_into_slots(write_seq, samples.as_ptr(), accepted);
            self.store_write_seq(write_seq + accepted as u64);
            // Every advance signals (module docs). The withdrawn `was_empty`
            // edge was read off `read_seq`, which the consumer owns, so it
            // could be stale and the signal lost. `accepted > 0` survives as
            // a guard because it is a fact about this side's own store.
            if let Some(wake) = &self.wake {
                let _ = wake.signal(Wake::Data);
            }
        }
        if dropped > 0 {
            self.store_dropped_seq(dropped_seq + dropped as u64);
        }
        Ok(accepted)
    }

    /// Write one complete visualization slot (vis rings only).
    ///
    /// The ring overwrites `slot.seq` with the frame's offered index. Returns
    /// `true` when the slot was accepted, `false` when the ring was full (one
    /// drop is counted). Never blocks and never overwrites unread data.
    ///
    /// On a wake ring, signals `Wake::Data` after every accepted slot
    /// (RFC 0007 §6.2 as corrected; see the signalling rule in the module docs).
    pub fn write_slot(&mut self, slot: &VisSlot) -> Result<bool, RingError> {
        self.require_kind(RingKind::Vis)?;
        self.require_live()?;

        let write_seq = self.write_seq(Ordering::Relaxed);
        let dropped_seq = self.dropped_seq(Ordering::Relaxed);
        let read_seq = self.read_seq(Ordering::Acquire);
        let occupancy = write_seq.wrapping_sub(read_seq);
        if occupancy >= u64::from(self.meta.capacity) {
            self.store_dropped_seq(dropped_seq + 1);
            return Ok(false);
        }

        let slot_index = (write_seq % u64::from(self.meta.capacity)) as usize;
        self.copy_slot_in(slot_index, slot);
        // The slot body is in place; stamp the offered index before publishing.
        // SAFETY: the slot area holds `capacity` `VisSlot`s and `slot_index` is
        // in range; the pointer is 8-byte aligned because the data offset and
        // `VIS_SLOT_SIZE` are both multiples of 8.
        unsafe {
            let target = data_ptr(&self.region).cast::<VisSlot>().add(slot_index);
            (*target).seq = write_seq.wrapping_add(dropped_seq);
        };
        self.store_write_seq(write_seq + 1);
        // Every advance signals (module docs); the slot was accepted, so this
        // side did store.
        if let Some(wake) = &self.wake {
            let _ = wake.signal(Wake::Data);
        }
        Ok(true)
    }

    /// Wait for at least one free frame of space, or for the ring to close.
    ///
    /// Returns `Ok(WaitOutcome::Signaled)` when space is available (the caller
    /// should re-check), `Ok(WaitOutcome::TimedOut)` when the timeout elapsed,
    /// or `Err(Closed)` / `Err(Stale)` for a closed or stale ring. Never
    /// succeeds on a ring without a wake ([`RingError::NoWakeObject`]).
    pub fn wait_writable(&self, timeout: Option<Duration>) -> Result<WaitOutcome, RingError> {
        let wake = self.wake.as_ref().ok_or(RingError::NoWakeObject)?;
        loop {
            self.validate()?;
            if self.is_closed() {
                return Err(RingError::Closed);
            }
            let write_seq = self.write_seq(Ordering::Relaxed);
            let read_seq = self.read_seq(Ordering::Acquire);
            let free =
                u64::from(self.meta.capacity).saturating_sub(write_seq.wrapping_sub(read_seq));
            if free > 0 {
                return Ok(WaitOutcome::Signaled);
            }
            match wake.wait(Wake::Space, timeout)? {
                WaitOutcome::Signaled => continue,
                WaitOutcome::TimedOut => return Ok(WaitOutcome::TimedOut),
            }
        }
    }

    /// Write PCM frames, waiting for space instead of dropping (wake rings only).
    ///
    /// Behaves like [`write_from`] but blocks (up to `timeout`) when the ring is
    /// full instead of dropping. Writes `min(offered, free)` whole frames and
    /// returns the accepted count; it **never drops** and never advances
    /// `dropped_seq`. `Ok(0)` means the timeout elapsed with the ring still full.
    ///
    /// On a ring without a wake this returns [`RingError::NoWakeObject`].
    ///
    /// [`write_from`]: RingProducer::write_from
    pub fn write_from_blocking(
        &mut self,
        samples: &[f32],
        timeout: Option<Duration>,
    ) -> Result<usize, RingError> {
        self.require_kind(RingKind::Pcm)?;
        self.require_live()?;
        let samples_per_frame = (self.meta.frame_size / 4) as usize;
        if !samples.len().is_multiple_of(samples_per_frame) {
            return Err(RingError::NotFrameAligned {
                samples: samples.len(),
                samples_per_frame,
            });
        }
        let offered = samples.len() / samples_per_frame;
        if offered == 0 {
            return Ok(0);
        }

        let wake = self.wake.as_ref().ok_or(RingError::NoWakeObject)?;

        // The check-then-wait loop (RFC 0007 §6.1): re-derive the condition from
        // the counters after every wake. A signal is never evidence of data;
        // only the counters are.
        loop {
            self.validate()?;
            if self.is_closed() {
                return Err(RingError::Closed);
            }
            let write_seq = self.write_seq(Ordering::Relaxed);
            let read_seq = self.read_seq(Ordering::Acquire);
            let occupancy = write_seq.wrapping_sub(read_seq);
            let free = u64::from(self.meta.capacity).saturating_sub(occupancy);
            if free > 0 {
                // `offered > 0` and `free > 0`, so this branch always stores.
                let accepted = (offered as u64).min(free) as usize;
                self.copy_into_slots(write_seq, samples.as_ptr(), accepted);
                self.store_write_seq(write_seq + accepted as u64);
                // Every advance signals (module docs).
                let _ = wake.signal(Wake::Data);
                return Ok(accepted);
            }
            match wake.wait(Wake::Space, timeout)? {
                WaitOutcome::Signaled => continue,
                WaitOutcome::TimedOut => return Ok(0),
            }
        }
    }

    fn require_kind(&self, expected: RingKind) -> Result<(), RingError> {
        if self.meta.kind == expected {
            Ok(())
        } else {
            Err(RingError::WrongRingKind {
                expected,
                found: self.meta.kind,
            })
        }
    }

    fn require_live(&self) -> Result<(), RingError> {
        if self.is_closed() {
            Err(RingError::Closed)
        } else {
            Ok(())
        }
    }

    fn state(&self) -> u32 {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `state` is a whole atomic.
        //
        // The load is `Acquire` because the store in `close` is `Release`, and
        // the release/acquire pair is what makes the close visible to a peer
        // that was woken by the signal (RFC 0007 section 6.5). A `Relaxed`
        // load would leave the store without a partner.
        unsafe { (*h).state.load(Ordering::Acquire) }
    }

    fn write_seq(&self, order: Ordering) -> u64 {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `write_seq` is producer-owned.
        unsafe { (*h).write_seq.load(order) }
    }

    fn dropped_seq(&self, order: Ordering) -> u64 {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `dropped_seq` is producer-owned.
        unsafe { (*h).dropped_seq.load(order) }
    }

    fn read_seq(&self, order: Ordering) -> u64 {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; the peer only ever stores whole
        // values into `read_seq`.
        unsafe { (*h).read_seq.load(order) }
    }

    fn store_write_seq(&self, value: u64) {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `write_seq` is producer-owned.
        // The `Release` publishes every slot byte written before this call.
        unsafe { (*h).write_seq.store(value, Ordering::Release) };
    }

    fn store_dropped_seq(&self, value: u64) {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `dropped_seq` is producer-owned.
        unsafe { (*h).dropped_seq.store(value, Ordering::Release) };
    }

    /// Copy `frames` frames from `src` into the slot area, splitting at most
    /// once at the end of the array.
    fn copy_into_slots(&self, start_seq: u64, src: *const f32, frames: usize) {
        let frame_bytes = self.meta.frame_size as usize;
        let capacity = self.meta.capacity as usize;
        let start_slot = (start_seq % u64::from(self.meta.capacity)) as usize;
        let first = frames.min(capacity - start_slot);
        // SAFETY: `data_ptr` is valid for `capacity * frame_size` bytes (proved
        // by the constructor invariant) and `src` is valid for
        // `frames * frame_size` bytes because `frames <= offered` and `offered`
        // is derived from the caller's slice length. `start_slot + first <=
        // capacity` and the tail write stays inside the array, so source and
        // destination never overlap (the source is a caller slice, the
        // destination is the ring's own mapping).
        unsafe {
            let dst = data_ptr(&self.region);
            core::ptr::copy_nonoverlapping(
                src.cast::<u8>(),
                dst.add(start_slot * frame_bytes),
                first * frame_bytes,
            );
            if first < frames {
                let rest = frames - first;
                core::ptr::copy_nonoverlapping(
                    src.cast::<u8>().add(first * frame_bytes),
                    dst,
                    rest * frame_bytes,
                );
            }
        }
    }

    /// Copy one whole slot into the slot area.
    fn copy_slot_in(&self, slot_index: usize, slot: &VisSlot) {
        // SAFETY: the slot area holds `capacity` `VisSlot`s, `slot_index` is in
        // range, and the source is a live `VisSlot` of exactly `VIS_SLOT_SIZE`
        // bytes. `VisSlot` has no padding, so a byte copy is a value copy.
        unsafe {
            core::ptr::copy_nonoverlapping(
                core::ptr::from_ref(slot).cast::<u8>(),
                data_ptr(&self.region).add(slot_index * VIS_SLOT_SIZE),
                VIS_SLOT_SIZE,
            );
        }
    }
}

impl<R: MappedRegion> RingConsumer<R> {
    /// Attach to an existing ring and validate it against `expect`.
    ///
    /// Every failure mode in RFC 0003 section 8 returns a typed error before any
    /// counter is touched; nothing here can read unmapped memory.
    pub fn attach(region: R, expect: RingExpect) -> Result<RingConsumer<R>, RingError> {
        let meta = check_region(&region, expect)?;
        Ok(RingConsumer {
            region,
            meta,
            wake: None,
        })
    }

    /// Attach a [`RingWake`] to this ring (RFC 0007 §5.2).
    ///
    /// After this call the ring signals on every counter advance and supports
    /// the blocking methods. Called by the opener after the header validates.
    pub fn with_wake(mut self, wake: RingWake) -> Self {
        self.wake = Some(wake);
        self
    }

    /// Ring kind.
    pub fn kind(&self) -> RingKind {
        self.meta.kind
    }

    /// Bytes per slot.
    pub fn frame_size(&self) -> u32 {
        self.meta.frame_size
    }

    /// Number of slots.
    pub fn capacity(&self) -> u32 {
        self.meta.capacity
    }

    /// Generation (epoch) of this ring instance.
    pub fn generation(&self) -> u64 {
        self.meta.generation
    }

    /// What this consumer expects; useful to re-attach or to diagnose.
    pub fn expect(&self) -> RingExpect {
        RingExpect::new(
            self.meta.kind,
            self.meta.frame_size,
            self.meta.capacity,
            self.meta.generation,
        )
    }

    /// Frames this consumer has consumed.
    pub fn frames_read(&self) -> u64 {
        self.read_seq(Ordering::Relaxed)
    }

    /// Frames the producer has published.
    pub fn frames_published(&self) -> u64 {
        self.write_seq(Ordering::Acquire)
    }

    /// Frames the producer has dropped.
    pub fn frames_dropped(&self) -> u64 {
        self.dropped_seq(Ordering::Acquire)
    }

    /// Frames ready to be read right now.
    pub fn frames_available(&self) -> u64 {
        self.frames_published().wrapping_sub(self.frames_read())
    }

    /// Whether the peer (or this side) closed the ring.
    ///
    /// `read_into`/`read_slot_into` returning "nothing" is normal; this tells
    /// "no data yet" from "the producer is done".
    pub fn is_closed(&self) -> bool {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `state` is a whole atomic.
        //
        // The load is `Acquire` because the store in `close` is `Release`, and
        // the release/acquire pair is what makes the close visible to a peer
        // that was woken by the signal (RFC 0007 section 6.5). A `Relaxed`
        // load would leave the store without a partner.
        unsafe { (*h).state.load(Ordering::Acquire) == RING_STATE_CLOSED }
    }

    /// Mark the ring closed.
    ///
    /// On a wake ring this also signals both events so a blocked peer returns
    /// `Err(RingError::Closed)` (RFC 0007 §6.5). On a wake-less ring `close`
    /// is unchanged.
    pub fn close(&mut self) {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `state` is a whole atomic. The
        // `Release` store happens-before the signals below.
        unsafe { (*h).state.store(RING_STATE_CLOSED, Ordering::Release) };
        if let Some(wake) = &self.wake {
            let _ = wake.signal(Wake::Data);
            let _ = wake.signal(Wake::Space);
        }
    }

    /// Re-check magic, version and generation (RFC 0003 section 8).
    pub fn validate(&self) -> Result<(), RingError> {
        validate_header(&self.region, self.meta.generation)
    }

    /// Read interleaved `f32` PCM frames (PCM rings only).
    ///
    /// `out.len()` must be a multiple of the channel count. Reads
    /// `min(want, available)` whole frames, returns how many were read and never
    /// blocks. `Ok(0)` means "nothing available yet", not an error.
    ///
    /// On a wake ring, signals `Wake::Space` after **every** read that advanced
    /// `read_seq`, so a blocked producer is woken (RFC 0007 §6.2 as corrected
    /// on 2026-10-18; see the signalling rule in the module docs).
    pub fn read_into(&mut self, out: &mut [f32]) -> Result<usize, RingError> {
        self.require_kind(RingKind::Pcm)?;
        let samples_per_frame = (self.meta.frame_size / 4) as usize;
        if !out.len().is_multiple_of(samples_per_frame) {
            return Err(RingError::NotFrameAligned {
                samples: out.len(),
                samples_per_frame,
            });
        }
        let want = out.len() / samples_per_frame;
        if want == 0 {
            return Ok(0);
        }

        let write_seq = self.write_seq(Ordering::Acquire);
        let read_seq = self.read_seq(Ordering::Relaxed);
        let available = write_seq.wrapping_sub(read_seq);
        let count = (want as u64).min(available) as usize;
        if count == 0 {
            return Ok(0);
        }

        self.copy_out_of_slots(read_seq, out.as_mut_ptr(), count);
        self.store_read_seq(read_seq + count as u64);
        // Every advance signals (module docs). The withdrawn `was_full` edge
        // was read off `write_seq`, which the producer owns, so it could be
        // stale and the signal lost. `count > 0` survives as a guard because
        // it is a fact about this side's own store.
        if let Some(wake) = &self.wake {
            let _ = wake.signal(Wake::Space);
        }
        Ok(count)
    }

    /// Read one complete visualization slot (vis rings only).
    ///
    /// Returns `true` and fills `out` when a whole slot is available. Returns
    /// `false` when nothing is published; a slot that is not (yet) published is
    /// never delivered as data (RFC 0003 section 7). The slot's `seq` lets the
    /// caller detect frames skipped since the previous read.
    ///
    /// On a wake ring, signals `Wake::Space` after every delivered slot
    /// (RFC 0007 §6.2 as corrected; see the signalling rule in the module docs).
    pub fn read_slot_into(&mut self, out: &mut VisSlot) -> Result<bool, RingError> {
        self.require_kind(RingKind::Vis)?;

        let write_seq = self.write_seq(Ordering::Acquire);
        let read_seq = self.read_seq(Ordering::Relaxed);
        let available = write_seq.wrapping_sub(read_seq);
        if available == 0 {
            return Ok(false);
        }

        let slot_index = (read_seq % u64::from(self.meta.capacity)) as usize;
        // SAFETY: the slot area holds `capacity` `VisSlot`s and `slot_index` is
        // in range; the source slot was published before `write_seq` was
        // advanced, so it is fully written. `VisSlot` has no padding, so a byte
        // copy is a value copy.
        unsafe {
            core::ptr::copy_nonoverlapping(
                data_ptr(&self.region)
                    .add(slot_index * VIS_SLOT_SIZE)
                    .cast::<VisSlot>(),
                core::ptr::from_mut(out).cast::<VisSlot>(),
                1,
            );
        }
        self.store_read_seq(read_seq + 1);
        // Every advance signals (module docs); a slot was delivered, so this
        // side did store.
        if let Some(wake) = &self.wake {
            let _ = wake.signal(Wake::Space);
        }
        Ok(true)
    }

    /// Wait for at least one published frame, or for the ring to close.
    ///
    /// Returns `Ok(WaitOutcome::Signaled)` when data is available,
    /// `Ok(WaitOutcome::TimedOut)` when the timeout elapsed, or `Err(Closed)` /
    /// `Err(Stale)` for a closed or stale ring. Never succeeds on a ring without
    /// a wake ([`RingError::NoWakeObject`]).
    pub fn wait_readable(&self, timeout: Option<Duration>) -> Result<WaitOutcome, RingError> {
        let wake = self.wake.as_ref().ok_or(RingError::NoWakeObject)?;
        loop {
            self.validate()?;
            if self.is_closed() {
                return Err(RingError::Closed);
            }
            let write_seq = self.write_seq(Ordering::Acquire);
            let read_seq = self.read_seq(Ordering::Relaxed);
            if write_seq.wrapping_sub(read_seq) > 0 {
                return Ok(WaitOutcome::Signaled);
            }
            match wake.wait(Wake::Data, timeout)? {
                WaitOutcome::Signaled => continue,
                WaitOutcome::TimedOut => return Ok(WaitOutcome::TimedOut),
            }
        }
    }

    /// Read PCM frames, waiting for data instead of polling (wake rings only).
    ///
    /// Behaves like [`read_into`] but blocks (up to `timeout`) when the ring is
    /// empty instead of returning `Ok(0)`. Reads `min(want, available)` whole
    /// frames and returns the count. `Ok(0)` means the timeout elapsed with no
    /// data. `Err(Closed)` ends the stream (after any remaining frames have been
    /// drained).
    ///
    /// On a ring without a wake this returns [`RingError::NoWakeObject`].
    ///
    /// [`read_into`]: RingConsumer::read_into
    pub fn read_into_blocking(
        &mut self,
        out: &mut [f32],
        timeout: Option<Duration>,
    ) -> Result<usize, RingError> {
        self.require_kind(RingKind::Pcm)?;
        let samples_per_frame = (self.meta.frame_size / 4) as usize;
        if !out.len().is_multiple_of(samples_per_frame) {
            return Err(RingError::NotFrameAligned {
                samples: out.len(),
                samples_per_frame,
            });
        }
        let want = out.len() / samples_per_frame;
        if want == 0 {
            return Ok(0);
        }

        let wake = self.wake.as_ref().ok_or(RingError::NoWakeObject)?;

        // Drain-then-close ordering: data available is checked *before* closed,
        // so a peer that closes after publishing still hands its remaining frames
        // over first, then yields Closed on the next call (RFC 0007 §6.5).
        loop {
            self.validate()?;
            let write_seq = self.write_seq(Ordering::Acquire);
            let read_seq = self.read_seq(Ordering::Relaxed);
            let available = write_seq.wrapping_sub(read_seq);
            if available > 0 {
                // `want > 0` and `available > 0`, so this branch always stores.
                let count = (want as u64).min(available) as usize;
                self.copy_out_of_slots(read_seq, out.as_mut_ptr(), count);
                self.store_read_seq(read_seq + count as u64);
                // Every advance signals (module docs).
                let _ = wake.signal(Wake::Space);
                return Ok(count);
            }
            if self.is_closed() {
                return Err(RingError::Closed);
            }
            match wake.wait(Wake::Data, timeout)? {
                WaitOutcome::Signaled => continue,
                WaitOutcome::TimedOut => return Ok(0),
            }
        }
    }

    fn require_kind(&self, expected: RingKind) -> Result<(), RingError> {
        if self.meta.kind == expected {
            Ok(())
        } else {
            Err(RingError::WrongRingKind {
                expected,
                found: self.meta.kind,
            })
        }
    }

    fn write_seq(&self, order: Ordering) -> u64 {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; the producer only ever stores
        // whole values into `write_seq`.
        unsafe { (*h).write_seq.load(order) }
    }

    fn dropped_seq(&self, order: Ordering) -> u64 {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; the producer only ever stores
        // whole values into `dropped_seq`.
        unsafe { (*h).dropped_seq.load(order) }
    }

    fn read_seq(&self, order: Ordering) -> u64 {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `read_seq` is consumer-owned.
        unsafe { (*h).read_seq.load(order) }
    }

    fn store_read_seq(&self, value: u64) {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `read_seq` is consumer-owned.
        // The `Release` publishes the completed slot reads to the producer.
        unsafe { (*h).read_seq.store(value, Ordering::Release) };
    }

    /// Copy `frames` frames out of the slot area, splitting at most once.
    fn copy_out_of_slots(&self, start_seq: u64, dst: *mut f32, frames: usize) {
        let frame_bytes = self.meta.frame_size as usize;
        let capacity = self.meta.capacity as usize;
        let start_slot = (start_seq % u64::from(self.meta.capacity)) as usize;
        let first = frames.min(capacity - start_slot);
        // SAFETY: `data_ptr` is valid for `capacity * frame_size` bytes and
        // `dst` is valid for `frames * frame_size` bytes because `frames` is
        // bounded by the caller's slice length. `start_slot + first <= capacity`
        // and the tail read stays inside the array; the source and destination
        // never overlap (the destination is a caller slice).
        unsafe {
            let src = data_ptr(&self.region);
            core::ptr::copy_nonoverlapping(
                src.add(start_slot * frame_bytes),
                dst.cast::<u8>(),
                first * frame_bytes,
            );
            if first < frames {
                let rest = frames - first;
                core::ptr::copy_nonoverlapping(
                    src,
                    dst.cast::<u8>().add(first * frame_bytes),
                    rest * frame_bytes,
                );
            }
        }
    }
}

/// Re-check magic, version and generation of a live ring.
fn validate_header<R: MappedRegion>(region: &R, expected_generation: u64) -> Result<(), RingError> {
    let h = header_ptr(region);
    // SAFETY: `h` points to a live header inside the mapping (see `header_ptr`).
    let magic = unsafe { (*h).magic.load(Ordering::Acquire) };
    if magic != RING_MAGIC {
        return Err(RingError::BadMagic { found: magic });
    }
    // SAFETY: see above.
    let (version, generation) = unsafe {
        (
            (*h).version.load(Ordering::Relaxed),
            (*h).generation.load(Ordering::Acquire),
        )
    };
    if version != RING_VERSION {
        return Err(RingError::BadVersion { found: version });
    }
    if generation != expected_generation {
        return Err(RingError::Stale {
            expected: expected_generation,
            found: generation,
        });
    }
    Ok(())
}

// The ring is a single-producer/single-consumer object: it can be moved to
// another thread but must not be shared between threads.
//
// SAFETY: `MappedRegion: Send` guarantees the mapping may be used from another
// thread, the metadata is immutable after construction, and the counters are
// accessed only through atomics.
unsafe impl<R: MappedRegion> Send for RingProducer<R> {}
// SAFETY: see above.
unsafe impl<R: MappedRegion> Send for RingConsumer<R> {}

#[cfg(test)]
mod tests {
    /// Every blocking wait in a test is bounded: a test must fail loudly
    /// rather than hang the suite when a wake is lost (RFC 0007 §6.1).
    const TEST_WAIT: Option<Duration> = Some(Duration::from_secs(10));

    use super::*;
    use crate::shm::header::{RING_DATA_OFFSET, RING_HEADER_SIZE, VIS_SLOT_SIZE};
    use crate::shm::region::{AnonymousRegion, ring_name};
    use rustamp_proto::vis::VisFrame;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::AtomicU64;
    use std::time::{Duration, Instant};

    /// Region big enough for a ring of this layout.
    fn region_for(frame_size: u32, capacity: u32) -> AnonymousRegion {
        let size = RING_DATA_OFFSET + capacity as usize * frame_size as usize;
        AnonymousRegion::new(size).unwrap()
    }

    /// A PCM ring with `channels` interleaved `f32` channels.
    fn pcm_ring(
        capacity: u32,
        channels: u32,
    ) -> (RingProducer<AnonymousRegion>, RingConsumer<AnonymousRegion>) {
        let frame_size = channels * 4;
        let region = region_for(frame_size, capacity);
        let producer =
            RingProducer::create(region.clone(), RingKind::Pcm, frame_size, capacity).unwrap();
        let consumer = RingConsumer::attach(region, producer.expect()).unwrap();
        (producer, consumer)
    }

    /// A vis ring.
    fn vis_ring(capacity: u32) -> (RingProducer<AnonymousRegion>, RingConsumer<AnonymousRegion>) {
        let region = region_for(VIS_SLOT_SIZE as u32, capacity);
        let producer = RingProducer::create(
            region.clone(),
            RingKind::Vis,
            VIS_SLOT_SIZE as u32,
            capacity,
        )
        .unwrap();
        let consumer = RingConsumer::attach(region, producer.expect()).unwrap();
        (producer, consumer)
    }

    /// The CPU time the calling thread has burned, in microseconds.
    ///
    /// Windows accounts a thread's user and kernel time in 100 ns units, on the
    /// system timer tick, so the number only means something over a window of
    /// a few ticks. That is why the tests below block for hundreds of
    /// milliseconds: a shorter window could not tell a sleeping thread from a
    /// spinning one.
    ///
    /// A thread that is blocked in the kernel wait does not accrue CPU time -
    /// it is not running - so the number stays near zero for a wait and grows
    /// with the wall clock for a spin.
    #[cfg(windows)]
    fn thread_cpu_micros() -> u64 {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};

        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: all four out-parameters are live locals, and
        // `GetCurrentThread` returns the pseudo-handle of the calling thread,
        // which is always valid for `GetThreadTimes`.
        unsafe {
            GetThreadTimes(
                GetCurrentThread(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        }
        .expect("GetThreadTimes on the current thread");
        let micros = |ft: FILETIME| -> u64 {
            ((u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)) / 10
        };
        micros(kernel) + micros(user)
    }

    #[test]
    fn pcm_round_trip_is_byte_exact_across_two_wraps() {
        // Capacity 7 is deliberately not a power of two.
        let capacity = 7u32;
        let (mut producer, mut consumer) = pcm_ring(capacity, 2);

        let mut expected: Vec<f32> = Vec::new();
        let mut got: Vec<f32> = Vec::new();
        let total = 3 * capacity + 2;
        let mut next = 0u32;
        let mut block = [0.0f32; 6];
        let mut out = [0.0f32; 4];

        while next < total {
            let frames = 3.min(total - next);
            for i in 0..frames {
                let index = (next + i) as f32;
                block[(i * 2) as usize] = index;
                block[(i * 2 + 1) as usize] = -index;
                expected.push(index);
                expected.push(-index);
            }
            let wrote = producer
                .write_from(&block[..(frames * 2) as usize])
                .unwrap();
            assert_eq!(wrote, frames as usize);
            next += frames;

            loop {
                let read = consumer.read_into(&mut out).unwrap();
                if read == 0 {
                    break;
                }
                got.extend_from_slice(&out[..read * 2]);
            }
        }

        assert_eq!(got, expected);
        assert_eq!(producer.frames_dropped(), 0);
        assert_eq!(consumer.frames_read(), u64::from(total));
        assert!(
            u64::from(total) > 2 * u64::from(capacity),
            "must wrap twice"
        );
    }

    #[test]
    fn wrap_around_works_for_any_capacity() {
        for capacity in [1u32, 2, 3, 5, 8, 9, 16, 17] {
            let (mut producer, mut consumer) = pcm_ring(capacity, 2);
            let total = 5 * capacity;
            let mut next = 0u32;
            let mut out = vec![0.0f32; 2 * capacity as usize];
            let mut seen: Vec<f32> = Vec::new();
            while next < total {
                let frames = 2.min(total - next);
                let mut block = Vec::with_capacity(frames as usize * 2);
                for i in 0..frames {
                    block.push((next + i) as f32);
                    block.push(-((next + i) as f32));
                }
                let wrote = producer.write_from(&block).unwrap();
                assert!(wrote <= frames as usize, "capacity {capacity}");
                next += frames;
                loop {
                    let read = consumer.read_into(&mut out).unwrap();
                    if read == 0 {
                        break;
                    }
                    seen.extend_from_slice(&out[..read * 2]);
                }
            }
            // Whatever survived the wrap-around arrives in order, intact, and
            // exactly once: received + dropped == offered.
            let received = seen.len() as u64 / 2;
            assert_eq!(
                received + producer.frames_dropped(),
                u64::from(total),
                "capacity {capacity}"
            );
            let mut previous: Option<f32> = None;
            for pair in seen.chunks(2) {
                assert_eq!(pair[1], -pair[0], "capacity {capacity}");
                if let Some(previous) = previous {
                    assert!(pair[0] > previous, "capacity {capacity}: out of order");
                }
                previous = Some(pair[0]);
            }
        }
    }

    #[test]
    fn full_ring_drops_whole_frames_and_never_overwrites() {
        let capacity = 8u32;
        let (mut producer, mut consumer) = pcm_ring(capacity, 2);
        let source: Vec<f32> = (0..40).map(|i| i as f32).collect();

        let started = Instant::now();
        let wrote = producer.write_from(&source).unwrap();
        let elapsed = started.elapsed();

        assert_eq!(wrote, capacity as usize);
        assert_eq!(producer.frames_dropped(), 12);
        assert_eq!(wrote + 12, 20, "accepted + dropped == offered");
        // "Never blocks": a generous bound, not a tight timing assertion.
        assert!(elapsed < Duration::from_secs(1));

        // Read half of what is stored; the rest must survive the next write,
        // which cannot fit in the space that is still occupied.
        let mut small = [0.0f32; 8];
        assert_eq!(consumer.read_into(&mut small).unwrap(), 4);
        assert_eq!(&small[..], &source[..8], "unread data intact");

        let more: Vec<f32> = (100..116).map(|i| i as f32).collect();
        assert_eq!(producer.write_from(&more).unwrap(), 4, "only 4 slots free");
        assert_eq!(producer.frames_dropped(), 16);

        let mut tail = vec![0.0f32; 40];
        let read = consumer.read_into(&mut tail).unwrap();
        assert_eq!(read, 8);
        assert_eq!(&tail[..8], &source[8..16], "the stored frames, in order");
        assert_eq!(&tail[8..16], &more[..8], "then the newly written frames");
        assert_eq!(consumer.read_into(&mut tail).unwrap(), 0);
        assert_eq!(producer.frames_written(), 12);
        assert_eq!(consumer.frames_available(), 0);
    }

    #[test]
    fn unpublished_vis_slot_is_never_delivered() {
        let capacity = 4u32;
        let region = region_for(VIS_SLOT_SIZE as u32, capacity);
        let mut producer = RingProducer::create(
            region.clone(),
            RingKind::Vis,
            VIS_SLOT_SIZE as u32,
            capacity,
        )
        .unwrap();
        let mut consumer = RingConsumer::attach(region.clone(), producer.expect()).unwrap();
        let mut out = VisSlot::zeroed();

        assert!(!consumer.read_slot_into(&mut out).unwrap());

        // A producer that has written a whole slot body but has not published it
        // yet (no `write_seq` advance) must not be observed as data.
        // SAFETY: the region is alive, `RING_DATA_OFFSET + capacity *
        // VIS_SLOT_SIZE` bytes are mapped, and no other thread is using them.
        unsafe {
            std::ptr::write_bytes(
                region.base().add(RING_DATA_OFFSET),
                0xAB,
                VIS_SLOT_SIZE * capacity as usize,
            );
        }
        assert!(!consumer.read_slot_into(&mut out).unwrap());
        assert_eq!(consumer.frames_available(), 0);
        assert_eq!(out.seq, 0);

        // Publishing it makes it visible in full.
        let slot = VisSlot::from_frame(&VisFrame::default());
        assert!(producer.write_slot(&slot).unwrap());
        assert!(consumer.read_slot_into(&mut out).unwrap());
        assert_eq!(out.seq, 0);
    }

    #[test]
    fn vis_round_trip_reports_sequence_and_detects_skips() {
        let capacity = 4u32;
        let (mut producer, mut consumer) = vis_ring(capacity);

        let mut frame = VisFrame {
            timestamp_us: 123_456,
            ..VisFrame::default()
        };
        frame.waveform[0][0] = 0.5;
        frame.spectrum[1][575] = 0.25;
        let slot = VisSlot::from_frame(&frame);

        assert!(producer.write_slot(&slot).unwrap());
        let mut out = VisSlot::zeroed();
        assert!(consumer.read_slot_into(&mut out).unwrap());
        assert_eq!(out.seq, 0);
        assert_eq!(out.timestamp_us, 123_456);
        assert_eq!(out.to_frame(), frame);

        // Fill the ring, then drop one frame, then drain it and write one more.
        for _ in 0..capacity {
            assert!(producer.write_slot(&slot).unwrap());
        }
        assert!(!producer.write_slot(&slot).unwrap(), "ring full: one drop");
        assert_eq!(producer.frames_dropped(), 1);

        let mut seqs = vec![out.seq];
        while consumer.read_slot_into(&mut out).unwrap() {
            seqs.push(out.seq);
        }
        assert!(producer.write_slot(&slot).unwrap());
        assert!(consumer.read_slot_into(&mut out).unwrap());
        seqs.push(out.seq);

        assert_eq!(seqs, vec![0, 1, 2, 3, 4, 6]);
        let skipped: u64 = seqs.windows(2).map(|w| w[1] - w[0] - 1).sum();
        assert_eq!(skipped, 1, "the consumer can see the dropped frame");
        assert_eq!(consumer.frames_read(), 6);
        assert_eq!(consumer.frames_published(), 6);
    }

    #[test]
    fn stale_and_foreign_regions_are_rejected() {
        let capacity = 8u32;
        let frame_size = 8u32;
        let region = region_for(frame_size, capacity);

        // An all-zero region is not a ring.
        assert!(matches!(
            RingConsumer::attach(
                region.clone(),
                RingExpect::new(RingKind::Pcm, frame_size, capacity, 1)
            ),
            Err(RingError::BadMagic { found: 0 })
        ));

        let producer =
            RingProducer::create(region.clone(), RingKind::Pcm, frame_size, capacity).unwrap();
        let expect = producer.expect();
        assert_eq!(producer.generation(), expect.generation);

        // Wrong kind, frame size, capacity and generation.
        assert!(matches!(
            RingConsumer::attach(
                region.clone(),
                RingExpect::new(RingKind::Vis, frame_size, capacity, expect.generation)
            ),
            Err(RingError::WrongKind { .. })
        ));
        assert!(matches!(
            RingConsumer::attach(
                region.clone(),
                RingExpect::new(RingKind::Pcm, 4, capacity, expect.generation)
            ),
            Err(RingError::FrameSizeMismatch {
                expected: 4,
                found: 8
            })
        ));
        assert!(matches!(
            RingConsumer::attach(
                region.clone(),
                RingExpect::new(RingKind::Pcm, frame_size, 9, expect.generation)
            ),
            Err(RingError::CapacityMismatch {
                expected: 9,
                found: 8
            })
        ));
        assert!(matches!(
            RingConsumer::attach(
                region.clone(),
                RingExpect::new(RingKind::Pcm, frame_size, capacity, expect.generation + 1)
            ),
            Err(RingError::Stale { .. })
        ));

        // Corrupted version and kind.
        // SAFETY: the region is alive and the header lies inside it.
        unsafe {
            let h = header_ptr(&region);
            (*h).version.store(99, Ordering::Relaxed);
        }
        assert!(matches!(
            RingConsumer::attach(region.clone(), expect),
            Err(RingError::BadVersion { found: 99 })
        ));
        // SAFETY: see above.
        unsafe {
            let h = header_ptr(&region);
            (*h).version.store(RING_VERSION, Ordering::Relaxed);
            (*h).kind.store(9, Ordering::Relaxed);
        }
        assert!(matches!(
            RingConsumer::attach(region.clone(), expect),
            Err(RingError::UnknownKind { found: 9 })
        ));
        // SAFETY: see above; restore the good header so the rest of the test can
        // use the ring.
        unsafe {
            let h = header_ptr(&region);
            (*h).kind.store(RingKind::Pcm.as_u16(), Ordering::Relaxed);
        }

        // A region that is smaller than a ring header at all.
        let tiny = AnonymousRegion::new(64).unwrap();
        assert!(matches!(
            RingConsumer::attach(tiny, expect),
            Err(RingError::RegionTooSmall {
                needed: 192,
                have: 64
            })
        ));

        // A valid header in a mapping that is too small for the slots it
        // describes: defensive, the creator always maps the full size.
        let truncated = AnonymousRegion::new(RING_HEADER_SIZE).unwrap();
        // SAFETY: both regions are alive and `RING_HEADER_SIZE` bytes of the big
        // one hold the initialised header; the small one is that big.
        unsafe {
            core::ptr::copy_nonoverlapping(region.base(), truncated.base(), RING_HEADER_SIZE);
        }
        assert!(matches!(
            RingConsumer::attach(truncated, expect),
            Err(RingError::RegionTooSmall {
                needed: 256,
                have: 192
            })
        ));

        // Nothing above has run any ring operation, so the good producer still
        // works: the rejections never touched a counter.
        let mut consumer = RingConsumer::attach(region, expect).unwrap();
        let mut out = [0.0f32; 2];
        assert_eq!(consumer.read_into(&mut out).unwrap(), 0);
        assert_eq!(producer.frames_written(), 0);
    }

    #[test]
    fn recreating_a_ring_bumps_the_generation() {
        let region = region_for(8, 8);
        let first = RingProducer::create(region.clone(), RingKind::Pcm, 8, 8).unwrap();
        let first_expect = first.expect();
        let second = RingProducer::create(region.clone(), RingKind::Pcm, 8, 8).unwrap();

        assert_eq!(second.generation(), first.generation() + 1);
        assert_ne!(second.generation(), 0);
        assert_eq!(second.frames_written(), 0);
        assert!(matches!(
            RingConsumer::attach(region, first_expect),
            Err(RingError::Stale { .. })
        ));
        assert!(second.validate().is_ok());
    }

    #[test]
    fn close_is_visible_to_the_peer() {
        let (mut producer, mut consumer) = pcm_ring(4, 2);
        assert!(!consumer.is_closed());
        assert!(!producer.is_closed());
        producer.close();
        assert!(consumer.is_closed());
        assert!(matches!(
            producer.write_from(&[0.0f32; 2]),
            Err(RingError::Closed)
        ));
        assert!(consumer.validate().is_ok());
        consumer.close();
        assert!(consumer.is_closed());
    }

    #[test]
    fn kind_and_alignment_errors_are_reported() {
        let (mut pcm_producer, mut pcm_consumer) = pcm_ring(4, 2);
        assert!(matches!(
            pcm_producer.write_slot(&VisSlot::zeroed()),
            Err(RingError::WrongRingKind {
                expected: RingKind::Vis,
                found: RingKind::Pcm
            })
        ));
        assert!(matches!(
            pcm_consumer.read_slot_into(&mut VisSlot::zeroed()),
            Err(RingError::WrongRingKind { .. })
        ));
        // Three samples is not a whole number of stereo frames.
        assert!(matches!(
            pcm_producer.write_from(&[0.0, 1.0, 2.0]),
            Err(RingError::NotFrameAligned {
                samples: 3,
                samples_per_frame: 2
            })
        ));
        let mut out = [0.0f32; 3];
        assert!(matches!(
            pcm_consumer.read_into(&mut out),
            Err(RingError::NotFrameAligned { .. })
        ));

        let (mut vis_producer, mut vis_consumer) = vis_ring(2);
        assert!(matches!(
            vis_producer.write_from(&[0.0f32; 2]),
            Err(RingError::WrongRingKind {
                expected: RingKind::Pcm,
                found: RingKind::Vis
            })
        ));
        let mut out = [0.0f32; 2];
        assert!(matches!(
            vis_consumer.read_into(&mut out),
            Err(RingError::WrongRingKind { .. })
        ));
    }

    #[test]
    fn layout_arguments_are_validated() {
        let region = region_for(8, 8);
        assert!(matches!(
            RingProducer::create(region.clone(), RingKind::Pcm, 0, 8),
            Err(RingError::InvalidLayout)
        ));
        assert!(matches!(
            RingProducer::create(region.clone(), RingKind::Pcm, 8, 0),
            Err(RingError::InvalidLayout)
        ));
        assert!(matches!(
            RingProducer::create(region.clone(), RingKind::Pcm, 6, 8),
            Err(RingError::InvalidLayout)
        ));
        assert!(matches!(
            RingProducer::create(region.clone(), RingKind::Vis, 8, 8),
            Err(RingError::InvalidLayout)
        ));
        assert!(matches!(
            RingProducer::create(region, RingKind::Pcm, 8, 1 << 30),
            Err(RingError::InvalidLayout)
        ));
    }

    #[test]
    fn soak_two_threads_deliver_exactly_once_in_order() {
        const TOTAL: u64 = 4_000_000;
        const CHANNELS: u32 = 2;
        const CAPACITY: u32 = 2048;
        const BLOCK: usize = 256;

        let (mut producer, mut consumer) = pcm_ring(CAPACITY, CHANNELS);
        // Approximate per-frame residence time: the producer stamps the slot it
        // is about to write before publishing, the consumer compares with its
        // own clock. Accurate as long as the ring does not wrap between a write
        // and the matching read, which is the normal case here.
        let stamps: Vec<AtomicU64> = (0..CAPACITY).map(|_| AtomicU64::new(0)).collect();
        let done = AtomicBool::new(false);
        let started = Instant::now();

        let (sent, dropped, received, latency_nanos, worst_nanos) = std::thread::scope(|scope| {
            let stamps_ref = &stamps;
            let done_ref = &done;
            let producer_thread = scope.spawn(move || {
                let mut block = vec![0.0f32; BLOCK * CHANNELS as usize];
                let mut next = 0u64;
                while next < TOTAL {
                    let frames = (TOTAL - next).min(BLOCK as u64) as usize;
                    for i in 0..frames {
                        let index = (next + i as u64) as f32;
                        block[i * 2] = index;
                        block[i * 2 + 1] = -index;
                    }
                    let before = producer.frames_written();
                    let now = started.elapsed().as_nanos() as u64;
                    for i in 0..frames {
                        let slot = ((before + i as u64) % u64::from(CAPACITY)) as usize;
                        stamps_ref[slot].store(now, Ordering::Relaxed);
                    }
                    let wrote = producer
                        .write_from(&block[..frames * CHANNELS as usize])
                        .unwrap();
                    assert!(wrote <= frames);
                    next += frames as u64;
                }
                let written = producer.frames_written();
                let dropped = producer.frames_dropped();
                // Published after the last `write_seq` store, so an `Acquire`
                // load of `done` also makes the final frames visible.
                done_ref.store(true, Ordering::Release);
                (written, dropped)
            });

            let consumer_thread = scope.spawn(move || {
                let mut out = vec![0.0f32; BLOCK * CHANNELS as usize];
                let mut last: Option<u64> = None;
                let mut received = 0u64;
                let mut latency_nanos = 0u128;
                let mut worst_nanos = 0u64;
                loop {
                    if done_ref.load(Ordering::Acquire) {
                        // Drain what is left, then stop.
                        let read = consumer.read_into(&mut out).unwrap();
                        if read == 0 {
                            break;
                        }
                        account(
                            &consumer,
                            &out,
                            read,
                            stamps_ref,
                            started.elapsed().as_nanos() as u64,
                            &mut last,
                            &mut received,
                            &mut latency_nanos,
                            &mut worst_nanos,
                        );
                        continue;
                    }
                    let read = consumer.read_into(&mut out).unwrap();
                    if read == 0 {
                        std::hint::spin_loop();
                        continue;
                    }
                    account(
                        &consumer,
                        &out,
                        read,
                        stamps_ref,
                        started.elapsed().as_nanos() as u64,
                        &mut last,
                        &mut received,
                        &mut latency_nanos,
                        &mut worst_nanos,
                    );
                }
                (received, latency_nanos, worst_nanos)
            });

            let (sent, dropped) = producer_thread.join().unwrap();
            let (received, latency_nanos, worst_nanos) = consumer_thread.join().unwrap();
            (sent, dropped, received, latency_nanos, worst_nanos)
        });

        let elapsed = started.elapsed();
        assert_eq!(received + dropped, TOTAL, "received + dropped == sent");
        assert_eq!(sent, TOTAL - dropped);
        assert_eq!(received, TOTAL - dropped);

        let fps = TOTAL as f64 / elapsed.as_secs_f64();
        let average_us = latency_nanos as f64 / received as f64 / 1000.0;
        println!(
            "soak: {TOTAL} frames in {:.3}s = {fps:.0} frames/s ({:.1} MiB/s), dropped {dropped}, \
             average residence {average_us:.2} us, worst {} us",
            elapsed.as_secs_f64(),
            fps * f64::from(CHANNELS) * 4.0 / (1024.0 * 1024.0),
            worst_nanos / 1000
        );
        assert!(elapsed < Duration::from_secs(120));
    }

    /// Verify one block of received frames and accumulate statistics.
    #[allow(clippy::too_many_arguments)]
    fn account(
        consumer: &RingConsumer<AnonymousRegion>,
        out: &[f32],
        read: usize,
        stamps: &[AtomicU64],
        now: u64,
        last: &mut Option<u64>,
        received: &mut u64,
        latency_nanos: &mut u128,
        worst_nanos: &mut u64,
    ) {
        let first_read_seq = consumer.frames_read() - read as u64;
        for i in 0..read {
            let index = out[i * 2];
            assert_eq!(index, index.trunc(), "torn float in frame {i}");
            let index = index as u64;
            if let Some(prev) = *last {
                assert!(index > prev, "frame {index} arrived after {prev}");
            }
            *last = Some(index);
            assert_eq!(out[i * 2 + 1], -(index as f32), "channel mismatch");
            let slot = ((first_read_seq + i as u64) % stamps.len() as u64) as usize;
            let written_at = stamps[slot].load(Ordering::Relaxed);
            let delta = now.saturating_sub(written_at);
            *latency_nanos += u128::from(delta);
            *worst_nanos = (*worst_nanos).max(delta);
        }
        *received += read as u64;
    }

    /// A PCM ring with a wake attached to both ends (for blocking tests).
    fn pcm_ring_with_wake(
        capacity: u32,
        channels: u32,
    ) -> (RingProducer<AnonymousRegion>, RingConsumer<AnonymousRegion>) {
        let (producer, consumer, _probe) = pcm_ring_with_wake_probe(capacity, channels);
        (producer, consumer)
    }

    /// Like [`pcm_ring_with_wake`], plus a third handle on the same two
    /// latches so a test can observe the signals themselves.
    ///
    /// The probe is a peer waiter, not an observer: an auto-reset event
    /// releases exactly one waiter, so a test that uses the probe must be the
    /// only thing waiting on that direction.
    fn pcm_ring_with_wake_probe(
        capacity: u32,
        channels: u32,
    ) -> (
        RingProducer<AnonymousRegion>,
        RingConsumer<AnonymousRegion>,
        RingWake,
    ) {
        let frame_size = channels * 4;
        let region = region_for(frame_size, capacity);
        let producer =
            RingProducer::create(region.clone(), RingKind::Pcm, frame_size, capacity).unwrap();
        let wake = RingWake::create(&ring_name(RingKind::Pcm, producer.generation())).unwrap();
        let producer = producer.with_wake(wake.clone());
        let consumer = RingConsumer::attach(region, producer.expect())
            .unwrap()
            .with_wake(wake.clone());
        (producer, consumer, wake)
    }

    /// Consume whatever is already latched on one direction of a wake, so that
    /// the next wait reports only what was signaled after this call. Bounded
    /// by the 1 ms floor of RFC 0007 §6.4 and only ever used on a direction
    /// nothing else waits on.
    fn drain_latch(wake: &RingWake, which: Wake) {
        while wake.wait(which, Some(Duration::from_millis(1))).unwrap() == WaitOutcome::Signaled {}
    }

    /// Bound for a probe wait that must already be satisfied: the signal is a
    /// `SetEvent` the ring made before returning, so the latch is set before
    /// the wait starts and a correct ring returns immediately. The bound only
    /// decides how fast a regression fails.
    const PROBE_WAIT: Option<Duration> = Some(Duration::from_millis(250));

    /// `write_from_blocking` + `read_into_blocking` deliver every frame exactly
    /// once, in order, with zero drops (RFC 0007 acceptance criterion 2).
    #[test]
    fn blocking_delivers_every_frame_exactly_once() {
        const CHANNELS: u32 = 2;
        const CAPACITY: u32 = 16;
        const TOTAL: u64 = 10_000;
        const SPF: usize = CHANNELS as usize;

        let (producer, consumer) = pcm_ring_with_wake(CAPACITY, CHANNELS);
        let out_len = CAPACITY as usize * SPF;

        let (written, dropped, received) = std::thread::scope(|scope| {
            let p = scope.spawn(move || {
                let mut producer = producer;
                let mut buf = vec![0.0f32; CAPACITY as usize * SPF];
                let mut index = 0u64;
                while index < TOTAL {
                    let frames = (TOTAL - index).min(CAPACITY as u64) as usize;
                    for i in 0..frames {
                        let v = (index + i as u64) as f32;
                        buf[i * 2] = v;
                        buf[i * 2 + 1] = -v;
                    }
                    let written = producer
                        .write_from_blocking(&buf[..frames * SPF], TEST_WAIT)
                        .unwrap();
                    assert!(written > 0, "producer stalled with no data and no close");
                    index += written as u64;
                }
                (producer.frames_written(), producer.frames_dropped())
            });

            let c = scope.spawn(move || {
                let mut consumer = consumer;
                let mut out = vec![0.0f32; out_len];
                let mut received = 0u64;
                let mut previous: Option<f32> = None;
                while received < TOTAL {
                    let read = consumer.read_into_blocking(&mut out, TEST_WAIT).unwrap();
                    for i in 0..read {
                        let v = out[i * 2];
                        if let Some(prev) = previous {
                            assert!(v > prev, "out of order: {v} after {prev}");
                        }
                        previous = Some(v);
                        assert_eq!(out[i * 2 + 1], -v, "channel mismatch");
                    }
                    received += read as u64;
                }
                received
            });

            let (written, dropped) = p.join().unwrap();
            let received = c.join().unwrap();
            (written, dropped, received)
        });

        assert_eq!(written, TOTAL, "frames_written == TOTAL");
        assert_eq!(dropped, 0, "frames_dropped == 0");
        assert_eq!(received, TOTAL, "frames_read == TOTAL");
    }

    /// `close()` from the peer wakes a blocked `read_into_blocking` with
    /// `Err(Closed)`, but only after delivering any in-flight data
    /// (RFC 0007 §6.5, acceptance criterion 4).
    #[test]
    fn blocking_close_wakes_waiter_with_closed() {
        let (mut producer, mut consumer) = pcm_ring_with_wake(4, 2);

        // Producer writes one frame, then closes. The consumer should read it,
        // then get `Closed` on the next call.
        producer.write_from(&[1.0f32, -1.0]).unwrap();
        producer.close();

        let mut out = [0.0f32; 2];
        assert_eq!(consumer.read_into_blocking(&mut out, TEST_WAIT).unwrap(), 1);
        assert_eq!(out[0], 1.0);
        assert!(matches!(
            consumer.read_into_blocking(&mut out, TEST_WAIT),
            Err(RingError::Closed)
        ));
    }

    /// A producer blocked on a full ring is woken by a consumer `close()`
    /// (RFC 0007 §6.5, acceptance criterion 4).
    #[test]
    fn blocking_close_wakes_blocked_producer() {
        let (mut producer, mut consumer) = pcm_ring_with_wake(1, 2);

        // Fill the ring (capacity 1).
        producer
            .write_from_blocking(&[0.0f32, 0.0], TEST_WAIT)
            .unwrap();

        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let producer_handle = std::thread::spawn(move || {
            // Signal that the blocking call is about to start, then block.
            let _ = started_tx.send(());
            let result = producer.write_from_blocking(&[1.0f32, 1.0], TEST_WAIT);
            let _ = result_tx.send(result);
        });

        // Wait for the producer to start. Once it has started, `write_from_blocking`
        // will either be in or about to enter its wait. A `close()` from the
        // consumer wakes it either way:
        //   - if the producer hasn't entered the wait yet, it sees `is_closed()`
        //     on its first loop iteration;
        //   - if it's already waiting, `close()` signals the event and the
        //     waiter re-checks `is_closed()`.
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        consumer.close();

        let result = result_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap_err();
        assert!(matches!(result, RingError::Closed));
        producer_handle.join().unwrap();
    }

    /// A timed-out wait returns `TimedOut` / `Ok(0)`, not `Err` or a hang
    /// (RFC 0007 §6.4, acceptance criterion 5).
    ///
    /// The `>= 1 ms` lower bound is a smoke floor, not a precision
    /// measurement: it proves the call entered the kernel wait and did not
    /// return instantly. The earlier `>= 8 ms` bound failed in the VM, and
    /// the documented Win32 caveat allows a wait shorter than the system
    /// clock resolution to time out early, so no tighter wall-clock bound
    /// follows from the mechanism. The tick-boundary explanation this
    /// comment used to give (a tick 7–8 ms after the call expiring a 10 ms
    /// timer) was self-inconsistent and is withdrawn; how early the kernel
    /// actually expires a sub-tick wait is unmeasured in this tree. What the
    /// mechanism does control exactly — the millisecond value handed to
    /// `WaitForSingleObject` — is pinned by the `timeout_ms` test in
    /// `wake.rs`. Task G5 records the rest.
    #[test]
    fn blocking_timeout_returns_timed_out() {
        let (_producer, mut consumer) = pcm_ring_with_wake(8, 2);
        let mut out = [0.0f32; 8];
        let start = std::time::Instant::now();
        let result = consumer.read_into_blocking(&mut out, Some(Duration::from_millis(10)));
        let elapsed = start.elapsed();
        assert_eq!(result.unwrap(), 0, "timeout should return Ok(0)");
        assert!(
            elapsed >= Duration::from_millis(1),
            "should block for at least 1ms, took {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(200),
            "should not take too long, took {elapsed:?}"
        );
    }

    /// A timed-out `wait_readable` returns `TimedOut`. The bounds are the same
    /// smoke floor and hang guard as `blocking_timeout_returns_timed_out`
    /// (task G5): `>= 1 ms` proves kernel entry, not precision.
    #[test]
    fn wait_readable_returns_timeout() {
        let (_producer, consumer) = pcm_ring_with_wake(8, 2);
        let start = std::time::Instant::now();
        let result = consumer.wait_readable(Some(Duration::from_millis(10)));
        let elapsed = start.elapsed();
        assert_eq!(result.unwrap(), WaitOutcome::TimedOut);
        assert!(
            elapsed >= Duration::from_millis(1),
            "should block for at least 1ms, took {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(200),
            "should not take too long, took {elapsed:?}"
        );
    }

    /// The blocking wait honours its bounds across several durations and
    /// multiple runs (task G3, made honest by task G5).
    ///
    /// This is a bounds check, not a precision measurement, and the old name
    /// (`blocking_timeout_accuracy_across_durations`) overclaimed. The code
    /// itself never returns early — `Ok(0)` is returned only when the kernel
    /// reports `WAIT_TIMEOUT` — but the documented Win32 caveat allows a wait
    /// shorter than the system clock resolution (~15.625 ms) to expire early,
    /// so the only wall-clock assertion that follows from the mechanism is a
    /// kernel-entry smoke floor. For the 10 ms case the lower bound below
    /// proves the call entered the kernel and did not return instantly; it
    /// cannot catch an early expiry of up to 9 ms, and no in-tree wall-clock
    /// bound can — that would measure the OS, not the code. The mechanism's
    /// own timing contract (the exact millisecond value passed to
    /// `WaitForSingleObject`) is pinned deterministically by the `timeout_ms`
    /// test in `wake.rs`; the OS behaviour itself is what the
    /// `wake_timeout_probe` example measures (task G5).
    #[test]
    fn blocking_timeout_bounds_hold_across_durations() {
        const RUNS: usize = 5;
        let durations = [
            Duration::from_millis(10),
            Duration::from_millis(30),
            Duration::from_millis(60),
        ];

        let (_producer, mut consumer) = pcm_ring_with_wake(8, 2);
        let mut out = [0.0f32; 8];

        for &requested in &durations {
            for run in 0..RUNS {
                let start = std::time::Instant::now();
                let result = consumer.read_into_blocking(&mut out, Some(requested));
                let elapsed = start.elapsed();

                assert_eq!(
                    result.unwrap(),
                    0,
                    "expected Ok(0) on timeout for {requested:?}, run {run}"
                );

                // Lower bound, derived rather than measured: the code returns
                // Ok(0) only on the kernel's WAIT_TIMEOUT, and the documented
                // Win32 caveat lets a sub-clock-resolution wait expire early,
                // so the bound allows one tick of early expiry (requested − 16 ms)
                // and clamps to the 1 ms kernel-entry floor for sub-tick requests.
                // For 10 ms this catches an immediate return but not an early
                // expiry of up to 9 ms; the OS behaviour itself is what
                // `wake_timeout_probe` measures (task G5).
                let min_expected = requested
                    .saturating_sub(Duration::from_millis(16))
                    .max(Duration::from_millis(1));

                assert!(
                    elapsed >= min_expected,
                    "wait returned earlier than the loosest safe bound for {requested:?} on run {run}: elapsed {elapsed:?} < min {min_expected:?}"
                );

                // Upper bound: a hang guard, bounded so the test cannot stall.
                // Generous tolerance (requested + 500ms) to tolerate loaded VM scheduling jitter.
                assert!(
                    elapsed < requested + Duration::from_millis(500),
                    "wait took too long for {requested:?} on run {run}: elapsed {elapsed:?}"
                );
            }
        }
    }

    /// A signal that races with the timeout is not lost: the next wait returns
    /// immediately (RFC 0007 §6.4).
    #[test]
    fn blocking_signal_after_timeout_is_not_lost() {
        let (mut producer, mut consumer) = pcm_ring_with_wake(8, 2);

        // First: a wait that times out (ring is empty, producer hasn't written).
        let mut out = [0.0f32; 16];
        assert_eq!(
            consumer
                .read_into_blocking(&mut out, Some(Duration::from_millis(10)))
                .unwrap(),
            0
        );

        // Producer writes; the signal should have latched. The next wait should
        // return immediately with data.
        producer.write_from(&[42.0f32, -42.0]).unwrap();
        let result = consumer.read_into_blocking(&mut out, Some(Duration::from_secs(1)));
        assert_eq!(result.unwrap(), 1);
        assert_eq!(out[0], 42.0);
    }

    /// Blocking on a ring without a wake returns `NoWakeObject` (RFC 0007 §9,
    /// acceptance criterion 6).
    #[test]
    fn blocking_on_wake_less_ring_returns_no_wake_object() {
        let (mut producer, mut consumer) = pcm_ring(8, 2);

        let mut out = [0.0f32; 8];
        assert!(matches!(
            consumer.read_into_blocking(&mut out, TEST_WAIT),
            Err(RingError::NoWakeObject)
        ));
        assert!(matches!(
            producer.write_from_blocking(&[0.0f32; 2], TEST_WAIT),
            Err(RingError::NoWakeObject)
        ));
        assert!(matches!(
            consumer.wait_readable(None),
            Err(RingError::NoWakeObject)
        ));
        assert!(matches!(
            producer.wait_writable(None),
            Err(RingError::NoWakeObject)
        ));
    }

    /// The non-blocking path on a wake ring still works correctly — signaling
    /// is a side effect, not a behavior change (RFC 0007 §12, acceptance
    /// criterion 7).
    #[test]
    fn wake_ring_non_blocking_path_is_unchanged() {
        let (mut producer, mut consumer) = pcm_ring_with_wake(8, 2);
        let mut out = [0.0f32; 8];

        producer.write_from(&[1.0f32, -1.0, 2.0, -2.0]).unwrap();
        assert_eq!(consumer.read_into(&mut out).unwrap(), 2);
        assert_eq!(&out[..4], &[1.0, -1.0, 2.0, -2.0]);
        assert_eq!(producer.frames_dropped(), 0);
        assert_eq!(consumer.frames_read(), 2);
        // No wake-related allocation on the hot path.
    }

    /// A vis ring rejects PCM-only blocking methods with `WrongRingKind`
    /// (checked before the wake check), so a vis ring can never block
    /// (RFC 0007 §9, acceptance criterion 8).
    #[test]
    fn vis_ring_blocking_is_rejected() {
        let (mut vis_producer, mut vis_consumer) = vis_ring(4);

        let mut out = [0.0f32; 4];
        // Vis ring → WrongRingKind (kind is checked before the wake check).
        assert!(matches!(
            vis_consumer.read_into_blocking(&mut out, TEST_WAIT),
            Err(RingError::WrongRingKind { .. })
        ));
        assert!(matches!(
            vis_producer.write_from_blocking(&[0.0f32; 2], TEST_WAIT),
            Err(RingError::WrongRingKind { .. })
        ));
    }

    /// No lost wakeup at the check/wait boundary: a stress test with one frame
    /// per call and jitter must not deadlock (RFC 0007 §6.1, acceptance
    /// criterion 3).
    ///
    /// **The bound is the test.** Both sides treat `Ok(0)` as a failure, not
    /// as a retry. While the peer thread still has work, a wait here can only
    /// end in data (or space) or in a skipped signal: the ring cannot be empty
    /// for the consumer and full for the producer at the same time, and with
    /// every advance signalling (module docs) a live peer always sets the
    /// latch the waiter is on. So a `TEST_WAIT` that expires is a lost wake,
    /// and the message prints the counters that prove it — the condition was
    /// already met while the waiter slept.
    ///
    /// Until 2026-10-18 this test looped on `Ok(0)` on both sides, so the
    /// defect it is named after (`docs/tasks/W19-twoproc-stall.md` §1) only
    /// made it ten seconds slower per occurrence and it could not fail at all.
    /// The race-free half of the same criterion is
    /// `every_advance_signals_not_only_the_edge`.
    #[test]
    fn blocking_no_lost_wakeup_under_jitter() {
        const CHANNELS: u32 = 2;
        const CAPACITY: u32 = 4;
        const FRAMES: u64 = 5_000;

        let (producer, consumer) = pcm_ring_with_wake(CAPACITY, CHANNELS);
        let out_len = CAPACITY as usize * CHANNELS as usize;

        std::thread::scope(|scope| {
            let p = scope.spawn(move || {
                let mut producer = producer;
                for index in 0..FRAMES {
                    let v = index as f32;
                    let buf = [v, -v];
                    let written = producer.write_from_blocking(&buf, TEST_WAIT).unwrap();
                    assert_eq!(
                        written,
                        1,
                        "lost `Space` wake at frame {index}: the wait expired with {} of \
                         {CAPACITY} frames free (written {}, consumed {})",
                        producer.free_frames(),
                        producer.frames_written(),
                        producer.frames_written() - producer.frames_available(),
                    );
                }
            });

            let c = scope.spawn(move || {
                let mut consumer = consumer;
                let mut out = vec![0.0f32; out_len];
                let mut received = 0u64;
                let mut last: Option<f32> = None;
                while received < FRAMES {
                    let read = consumer.read_into_blocking(&mut out, TEST_WAIT).unwrap();
                    assert!(
                        read > 0,
                        "lost `Data` wake after {received} frames: the wait expired with {} \
                         frames available (published {}, read {})",
                        consumer.frames_available(),
                        consumer.frames_published(),
                        consumer.frames_read(),
                    );
                    for i in 0..read {
                        let v = out[i * 2];
                        if let Some(prev) = last {
                            assert!(v > prev, "out of order at {v} (prev {prev})");
                        }
                        last = Some(v);
                    }
                    received += read as u64;
                    // Jitter: yield to increase the chance of the
                    // check/wait race between the condition and the wait.
                    std::thread::yield_now();
                }
            });

            p.join().unwrap();
            c.join().unwrap();
        });
    }

    /// Every advance signals, including one that did not start from an empty
    /// or a full ring (RFC 0007 §6.2 as corrected, acceptance criterion 3).
    ///
    /// This is the race-free half of criterion 3: rather than trying to hit
    /// the interleaving of `docs/tasks/W19-twoproc-stall.md` §1, it asks the
    /// events directly. `probe` is a third handle on the same two auto-reset
    /// latches, so the test can drain a latch and then require that the next
    /// advance set it again. Nothing else waits on either direction here, so a
    /// drained latch stays drained and no signal can be stolen.
    ///
    /// Under the withdrawn edge both asserted signals were simply not sent,
    /// which is what let a waiter that had already observed its condition
    /// unmet sleep on a ring that was neither closed nor failed.
    #[test]
    fn every_advance_signals_not_only_the_edge() {
        let (mut producer, mut consumer, probe) = pcm_ring_with_wake_probe(4, 2);

        // `Data`: a write into a ring that is *not* empty must still signal.
        assert_eq!(producer.write_from(&[1.0f32, -1.0]).unwrap(), 1);
        drain_latch(&probe, Wake::Data);
        assert_eq!(producer.frames_available(), 1, "the ring is not empty");
        assert_eq!(producer.write_from(&[2.0f32, -2.0]).unwrap(), 1);
        assert_eq!(
            probe.wait(Wake::Data, PROBE_WAIT).unwrap(),
            WaitOutcome::Signaled,
            "a write into a non-empty ring did not signal Data"
        );

        // `Space`: a read from a ring that is *not* full must still signal.
        drain_latch(&probe, Wake::Space);
        let mut out = [0.0f32; 2];
        assert_eq!(consumer.frames_available(), 2, "2 of 4 frames, not full");
        assert_eq!(consumer.read_into(&mut out).unwrap(), 1);
        assert_eq!(out, [1.0, -1.0]);
        assert_eq!(
            probe.wait(Wake::Space, PROBE_WAIT).unwrap(),
            WaitOutcome::Signaled,
            "a read from a non-full ring did not signal Space"
        );
    }

    /// The blocking paths signal on every advance too: `write_from_blocking`
    /// and `read_into_blocking` carried their own copy of the withdrawn edge
    /// (RFC 0007 §6.2 as corrected, acceptance criterion 3).
    ///
    /// Same probe discipline as `every_advance_signals_not_only_the_edge`:
    /// neither call below ever waits, because each one finds its condition
    /// already met.
    #[test]
    fn every_blocking_advance_signals_not_only_the_edge() {
        let (mut producer, mut consumer, probe) = pcm_ring_with_wake_probe(4, 2);

        assert_eq!(
            producer
                .write_from_blocking(&[1.0f32, -1.0], TEST_WAIT)
                .unwrap(),
            1
        );
        drain_latch(&probe, Wake::Data);
        assert_eq!(producer.frames_available(), 1, "the ring is not empty");
        assert_eq!(
            producer
                .write_from_blocking(&[2.0f32, -2.0], TEST_WAIT)
                .unwrap(),
            1
        );
        assert_eq!(
            probe.wait(Wake::Data, PROBE_WAIT).unwrap(),
            WaitOutcome::Signaled,
            "write_from_blocking into a non-empty ring did not signal Data"
        );

        drain_latch(&probe, Wake::Space);
        let mut out = [0.0f32; 2];
        assert_eq!(consumer.frames_available(), 2, "2 of 4 frames, not full");
        assert_eq!(consumer.read_into_blocking(&mut out, TEST_WAIT).unwrap(), 1);
        assert_eq!(out, [1.0, -1.0]);
        assert_eq!(
            probe.wait(Wake::Space, PROBE_WAIT).unwrap(),
            WaitOutcome::Signaled,
            "read_into_blocking from a non-full ring did not signal Space"
        );
    }

    /// A vis ring with a wake signals on every advance as well; the vis path
    /// carried the same two guards (RFC 0007 §6.2 as corrected). Vis rings are
    /// created without a wake in production (RFC 0007 §9), so this pins the
    /// rule for the shape, not for a live caller.
    #[test]
    fn every_vis_advance_signals_not_only_the_edge() {
        let region = region_for(VIS_SLOT_SIZE as u32, 4);
        let producer =
            RingProducer::create(region.clone(), RingKind::Vis, VIS_SLOT_SIZE as u32, 4).unwrap();
        let wake = RingWake::create(&ring_name(RingKind::Vis, producer.generation())).unwrap();
        let mut producer = producer.with_wake(wake.clone());
        let mut consumer = RingConsumer::attach(region, producer.expect())
            .unwrap()
            .with_wake(wake.clone());

        let slot = VisSlot::from_frame(&VisFrame::default());
        assert!(producer.write_slot(&slot).unwrap());
        drain_latch(&wake, Wake::Data);
        assert!(producer.write_slot(&slot).unwrap());
        assert_eq!(
            wake.wait(Wake::Data, PROBE_WAIT).unwrap(),
            WaitOutcome::Signaled,
            "write_slot into a non-empty ring did not signal Data"
        );

        drain_latch(&wake, Wake::Space);
        let mut out = VisSlot::zeroed();
        assert!(consumer.read_slot_into(&mut out).unwrap());
        assert_eq!(
            wake.wait(Wake::Space, PROBE_WAIT).unwrap(),
            WaitOutcome::Signaled,
            "read_slot_into from a non-full ring did not signal Space"
        );
    }

    /// Two-thread lossless delivery via blocking calls: every frame arrives
    /// exactly once, in order, with zero drops (RFC 0007 acceptance criterion 2).
    /// Larger run than `blocking_delivers_every_frame_exactly_once`.
    #[test]
    fn blocking_two_thread_lossless_delivery() {
        const CHANNELS: u32 = 2;
        const CAPACITY: u32 = 8;
        const TOTAL: u64 = 50_000;
        const SPF: usize = CHANNELS as usize;

        let (producer, consumer) = pcm_ring_with_wake(CAPACITY, CHANNELS);
        let out_len = CAPACITY as usize * SPF;

        let (written, dropped, received) = std::thread::scope(|scope| {
            let p = scope.spawn(move || {
                let mut producer = producer;
                let mut buf = vec![0.0f32; CAPACITY as usize * SPF];
                let mut index = 0u64;
                while index < TOTAL {
                    let frames = (TOTAL - index).min(CAPACITY as u64) as usize;
                    for i in 0..frames {
                        let v = (index + i as u64) as f32;
                        buf[i * 2] = v;
                        buf[i * 2 + 1] = -v;
                    }
                    let written = producer
                        .write_from_blocking(&buf[..frames * SPF], TEST_WAIT)
                        .unwrap();
                    assert!(written > 0, "producer stalled");
                    index += written as u64;
                }
                (producer.frames_written(), producer.frames_dropped())
            });

            let c = scope.spawn(move || {
                let mut consumer = consumer;
                let mut out = vec![0.0f32; out_len];
                let mut received = 0u64;
                let mut last: Option<f32> = None;
                while received < TOTAL {
                    let read = consumer.read_into_blocking(&mut out, TEST_WAIT).unwrap();
                    for i in 0..read {
                        let v = out[i * 2];
                        if let Some(prev) = last {
                            assert!(v > prev, "out of order at {v} (prev {prev})");
                        }
                        last = Some(v);
                        assert_eq!(out[i * 2 + 1], -v, "channel mismatch at {v}");
                    }
                    received += read as u64;
                }
                received
            });

            let (written, dropped) = p.join().unwrap();
            let received = c.join().unwrap();
            (written, dropped, received)
        });

        assert_eq!(written, TOTAL);
        assert_eq!(dropped, 0);
        assert_eq!(received, TOTAL);
    }

    /// A blocked consumer sleeps in the kernel; it does not spin (RFC 0007
    /// §1 and §4.3: the consumer waits for data instead of polling for it).
    ///
    /// This is the RFC's core claim, measured rather than argued: the thread
    /// that waits burns no CPU while it waits. A polling consumer would burn
    /// the whole wall clock, so the two cases are an order of magnitude apart
    /// and the bound below separates them.
    ///
    /// The window is 300 ms on purpose: Windows accounts thread time on the
    /// system timer tick, so a shorter window could not tell the two apart.
    /// This is an observation window, not a strict minimum for one OS timeout
    /// (RFC 0007 §6.4 and §14.5). Early expiry must not shorten the CPU sample.
    #[cfg(windows)]
    #[test]
    fn a_blocking_read_sleeps_instead_of_spinning() {
        const WINDOW: Duration = Duration::from_millis(300);

        let (_producer, consumer) = pcm_ring_with_wake(8, 2);

        let (cpu_micros, wall) = std::thread::scope(|scope| {
            let worker = scope.spawn(move || {
                let mut consumer = consumer;
                let mut out = [0.0f32; 8];
                // The ring is empty and nothing signals it, so this call can
                // only end in the timeout.
                observe_blocked_thread(WINDOW, |remaining| {
                    let read = consumer
                        .read_into_blocking(&mut out, Some(remaining))
                        .unwrap();
                    assert_eq!(read, 0, "the empty ring must time out, not deliver");
                })
            });
            worker.join().unwrap()
        });

        assert!(
            wall >= WINDOW,
            "the wait returned after {wall:?}, before the requested {WINDOW:?}"
        );
        println!(
            "blocked read: {wall:?} of wall clock, {cpu_micros} us of CPU ({:.3} % of a core)",
            100.0 * cpu_micros as f64 / wall.as_micros() as f64
        );
        assert!(
            cpu_micros * 4 < wall.as_micros() as u64,
            "the blocked thread burned {cpu_micros} us of CPU over {wall:?} of wall clock: \
             it polled instead of sleeping"
        );
        assert!(
            cpu_micros < 50_000,
            "the blocked thread burned {cpu_micros} us of CPU; a wait in the kernel costs \
             next to nothing"
        );
    }

    /// The same claim for the other end of the ring: a producer that waits for
    /// space sleeps in the kernel instead of spinning on the free counter
    /// (RFC 0007 §1, §4.3).
    #[cfg(windows)]
    #[test]
    fn a_blocking_write_sleeps_instead_of_spinning() {
        const WINDOW: Duration = Duration::from_millis(300);

        let (mut producer, _consumer) = pcm_ring_with_wake(1, 2);
        // Fill the ring, so the producer has to wait for space.
        producer.write_from(&[1.0f32, -1.0]).unwrap();
        assert_eq!(producer.free_frames(), 0);

        let (cpu_micros, wall) = std::thread::scope(|scope| {
            let worker = scope.spawn(move || {
                let mut producer = producer;
                observe_blocked_thread(WINDOW, |remaining| {
                    let written = producer
                        .write_from_blocking(&[2.0f32, -2.0], Some(remaining))
                        .unwrap();
                    assert_eq!(written, 0, "the full ring must time out, not accept frames");
                })
            });
            worker.join().unwrap()
        });

        assert!(
            wall >= WINDOW,
            "the wait returned after {wall:?}, before the requested {WINDOW:?}"
        );
        println!(
            "blocked write: {wall:?} of wall clock, {cpu_micros} us of CPU ({:.3} % of a core)",
            100.0 * cpu_micros as f64 / wall.as_micros() as f64
        );
        assert!(
            cpu_micros * 4 < wall.as_micros() as u64,
            "the blocked thread burned {cpu_micros} us of CPU over {wall:?} of wall clock: \
             it polled instead of sleeping"
        );
        assert!(
            cpu_micros < 50_000,
            "the blocked thread burned {cpu_micros} us of CPU; a wait in the kernel costs \
             next to nothing"
        );
    }

    /// Observe the same blocked thread for the entire CPU-accounting window.
    /// A kernel timeout can expire early; continue with only the remaining
    /// budget, without resetting either the wall clock or the CPU baseline.
    /// Even immediate-return polling therefore accrues CPU throughout the
    /// window and still fails the callers' unchanged CPU limits.
    #[cfg(windows)]
    fn observe_blocked_thread(window: Duration, mut wait: impl FnMut(Duration)) -> (u64, Duration) {
        let cpu_before = thread_cpu_micros();
        let started = Instant::now();
        let mut remaining = window;
        let wall = loop {
            wait(remaining);
            let elapsed = started.elapsed();
            if elapsed >= window {
                break elapsed;
            }
            remaining = window - elapsed;
        };
        let cpu_after = thread_cpu_micros();
        (cpu_after.saturating_sub(cpu_before), wall)
    }

    /// The state machine of a wait: a condition that is already met returns at
    /// once, and a ring that is closed or stale is reported instead of waited
    /// on (RFC 0007 §6.1, §5.3 and §6.5).
    #[test]
    fn the_wait_state_machine_reports_the_state_instead_of_waiting() {
        let (mut producer, consumer) = pcm_ring_with_wake(4, 2);

        // Data is already there: the wait must not block. `Signaled` is only
        // returned when the counter check found the condition met, so the
        // outcome itself is the evidence; the elapsed bound is a smoke floor.
        producer.write_from(&[1.0f32, -1.0]).unwrap();
        let started = Instant::now();
        assert_eq!(
            consumer
                .wait_readable(Some(Duration::from_secs(5)))
                .unwrap(),
            WaitOutcome::Signaled
        );
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "a satisfied wait must return at once, took {:?}",
            started.elapsed()
        );

        // Space is free: the same on the producer side.
        let started = Instant::now();
        assert_eq!(
            producer
                .wait_writable(Some(Duration::from_secs(5)))
                .unwrap(),
            WaitOutcome::Signaled
        );
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "a satisfied wait must return at once, took {:?}",
            started.elapsed()
        );

        // A closed ring is reported, not waited on.
        producer.close();
        assert!(matches!(
            consumer.wait_readable(TEST_WAIT),
            Err(RingError::Closed)
        ));
        assert!(matches!(
            producer.wait_writable(TEST_WAIT),
            Err(RingError::Closed)
        ));
    }

    /// A blocking call on a ring whose generation moved on reports `Stale`
    /// instead of waiting on a ring that is not its own (RFC 0007 §5.3,
    /// acceptance criterion 6).
    #[test]
    fn the_blocking_path_rejects_a_stale_generation() {
        let region = region_for(8, 8);
        let producer = RingProducer::create(region.clone(), RingKind::Pcm, 8, 8).unwrap();
        let wake = RingWake::create(&ring_name(RingKind::Pcm, producer.generation())).unwrap();
        let expect = producer.expect();
        let mut consumer = RingConsumer::attach(region.clone(), expect)
            .unwrap()
            .with_wake(wake);

        // The creator re-creates the ring under the same mapping: the
        // generation moves on and the old consumer is stale.
        let _second = RingProducer::create(region, RingKind::Pcm, 8, 8).unwrap();

        let mut out = [0.0f32; 8];
        assert!(
            matches!(
                consumer.read_into_blocking(&mut out, TEST_WAIT),
                Err(RingError::Stale { .. })
            ),
            "a blocking read on a stale ring must report Stale"
        );
        assert!(
            matches!(
                consumer.wait_readable(TEST_WAIT),
                Err(RingError::Stale { .. })
            ),
            "a wait on a stale ring must report Stale"
        );
    }

    /// A PCM ring whose wake a later creator can also hold, so the "the
    /// creator replaced the ring" case can be driven: the peers of the old
    /// generation wait on the same event objects (RFC 0007 §5.3) and have to
    /// be woken when the generation moves on.
    ///
    /// The wake is handed on by clone, which is the same kernel object that
    /// `RingWake::open` on a reused name yields on Windows, and the same
    /// latch the in-process backend shares on every platform.
    fn pcm_ring_with_reusable_wake(
        capacity: u32,
        channels: u32,
    ) -> (
        RingProducer<AnonymousRegion>,
        RingConsumer<AnonymousRegion>,
        AnonymousRegion,
        RingWake,
    ) {
        let frame_size = channels * 4;
        let region = region_for(frame_size, capacity);
        let producer =
            RingProducer::create(region.clone(), RingKind::Pcm, frame_size, capacity).unwrap();
        let wake = RingWake::create(&ring_name(RingKind::Pcm, producer.generation())).unwrap();
        let producer = producer.with_wake(wake.clone());
        let consumer = RingConsumer::attach(region.clone(), producer.expect())
            .unwrap()
            .with_wake(wake.clone());
        (producer, consumer, region, wake)
    }

    /// How long the woken peer may take to report the new generation. The
    /// wake is a `SetEvent`, so the peer is up in microseconds; the bound only
    /// decides how fast a regression fails.
    const RESET_WAKE_WAIT: Duration = Duration::from_secs(2);

    /// How long the peer is given to reach its kernel wait before the ring is
    /// reset.
    ///
    /// The peer cannot *prove* that it is asleep: the only way to see a
    /// waiter is to wake it, and a wake would let it re-check the counters
    /// on its own — which is exactly the thing under test. So the peer
    /// announces that it is about to block, and the test gives it time to get
    /// there. A peer that is still on its way would report the new
    /// generation from its first counter check, so the wait below is a
    /// *false pass* at worst, never a false failure.
    const RESET_SETTLE: Duration = Duration::from_millis(50);

    /// A ring that is **reset** — the creator re-creates the section, so the
    /// generation moves on — wakes the peer that is blocked in an empty-ring
    /// read (RFC 0007 §5.3).
    ///
    /// Before the fix the peer waited for its whole timeout and only then
    /// noticed that its ring was gone; with `timeout: None` it would have
    /// waited forever. The wake ends the wait, the re-check finds the new
    /// generation, and the wait ends with `RingError::Stale` — the outcome the
    /// wait state machine already defines (RFC 0007 §6.1).
    #[test]
    fn a_reset_wakes_a_blocked_reader() {
        const CHANNELS: u32 = 2;
        let (_producer, consumer, region, wake) = pcm_ring_with_reusable_wake(4, CHANNELS);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut consumer = consumer;
            let mut out = [0.0f32; 4];
            let _ = started_tx.send(());
            let result = consumer.read_into_blocking(&mut out, TEST_WAIT);
            let _ = result_tx.send(result);
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the reader never started");
        // The reader is blocked in an empty-ring read before the reset.
        std::thread::sleep(RESET_SETTLE);

        // The creator replaces the ring: the generation moves on and the
        // counters restart.
        let _second = RingProducer::create(region, RingKind::Pcm, CHANNELS * 4, 4)
            .unwrap()
            .with_wake(wake);

        // The blocked reader must wake and report the stale generation, long
        // before its own timeout would have expired.
        let result = result_rx
            .recv_timeout(RESET_WAKE_WAIT)
            .expect("the blocked reader was not woken by the reset");
        assert!(
            matches!(result, Err(RingError::Stale { .. })),
            "a reset must end the wait with the stale generation, got {result:?}"
        );
        reader.join().unwrap();
    }

    /// The same hole on the other side of the ring: a producer that waits for
    /// space on a full ring is woken by a reset as well, and reports the
    /// stale generation instead of writing into the ring that took its place.
    #[test]
    fn a_reset_wakes_a_blocked_writer() {
        const CHANNELS: u32 = 2;
        let (mut producer, _consumer, region, wake) = pcm_ring_with_reusable_wake(1, CHANNELS);
        // Fill the ring: the writer below has to wait for space.
        producer.write_from(&[1.0f32, -1.0]).unwrap();
        assert_eq!(producer.free_frames(), 0, "the ring is full");

        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            let mut producer = producer;
            let _ = started_tx.send(());
            let result = producer.write_from_blocking(&[2.0f32, -2.0], TEST_WAIT);
            let _ = result_tx.send(result);
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the writer never started");
        // The writer is blocked on a full ring before the reset.
        std::thread::sleep(RESET_SETTLE);

        // The creator replaces the ring while the writer waits for space.
        let _second = RingProducer::create(region, RingKind::Pcm, CHANNELS * 4, 1)
            .unwrap()
            .with_wake(wake);

        let result = result_rx
            .recv_timeout(RESET_WAKE_WAIT)
            .expect("the blocked writer was not woken by the reset");
        assert!(
            matches!(result, Err(RingError::Stale { .. })),
            "a reset must end the wait with the stale generation, got {result:?}"
        );
        writer.join().unwrap();
    }

    /// The peers of the old generation are gone; the new generation is a
    /// normal ring again: a read/write cycle over it delivers every frame
    /// exactly once, in order, with no drop.
    #[test]
    fn a_reset_leaves_the_new_generation_usable() {
        const CHANNELS: u32 = 2;
        let (_producer, _consumer, region, wake) = pcm_ring_with_reusable_wake(4, CHANNELS);
        let mut second = RingProducer::create(region.clone(), RingKind::Pcm, CHANNELS * 4, 4)
            .unwrap()
            .with_wake(wake.clone());
        let mut consumer = RingConsumer::attach(region, second.expect())
            .unwrap()
            .with_wake(wake);

        assert_eq!(second.write_from(&[1.0f32, -1.0, 2.0f32, -2.0]).unwrap(), 2);
        let mut out = [0.0f32; 4];
        assert_eq!(consumer.read_into(&mut out).unwrap(), 2);
        assert_eq!(out, [1.0, -1.0, 2.0, -2.0]);
        assert_eq!(second.frames_dropped(), 0);
        assert_eq!(consumer.frames_read(), 2);
    }

    /// A blocked write that runs out of time writes nothing and drops nothing
    /// (RFC 0007 §6.4 and §7: `Ok(0)` means the ring stayed full).
    #[test]
    fn a_blocking_write_times_out_on_a_full_ring_without_dropping() {
        let (mut producer, _consumer) = pcm_ring_with_wake(1, 2);
        producer.write_from(&[1.0f32, -1.0]).unwrap();
        assert_eq!(producer.free_frames(), 0, "the ring is full");

        let started = Instant::now();
        let written = producer
            .write_from_blocking(&[2.0f32, -2.0], Some(Duration::from_millis(50)))
            .unwrap();
        let elapsed = started.elapsed();

        assert_eq!(written, 0, "a timed-out write accepts nothing");
        assert_eq!(producer.frames_dropped(), 0, "a blocking write never drops");
        assert_eq!(producer.frames_written(), 1, "the ring is unchanged");
        assert!(
            elapsed >= Duration::from_millis(1),
            "the wait returned at once after {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "the wait took too long: {elapsed:?}"
        );
    }
}
