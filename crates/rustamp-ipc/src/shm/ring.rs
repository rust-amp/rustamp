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

use core::sync::atomic::Ordering;

use super::RingError;
use super::header::{
    MAX_RING_BYTES, RING_DATA_OFFSET, RING_FLAG_CREATOR_32BIT, RING_HEADER_SIZE, RING_MAGIC,
    RING_STATE_CLOSED, RING_STATE_LIVE, RING_VERSION, RingHeader, RingKind, VIS_SLOT_SIZE, VisSlot,
};
use super::region::{MappedRegion, fresh_generation};

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
/// section 7).
pub struct RingProducer<R: MappedRegion> {
    region: R,
    meta: RingMeta,
}

/// The reading end of a ring. Single consumer only.
///
/// The consumer owns `read_seq` and only ever reads `write_seq` and
/// `dropped_seq`.
pub struct RingConsumer<R: MappedRegion> {
    region: R,
    meta: RingMeta,
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
            (*h).generation.store(generation, Ordering::Relaxed);
            (*h).reserved0 = [0; 3];
            (*h).reserved1 = [0; 6];
            (*h).reserved2 = [0; 7];
            (*h).write_seq.store(0, Ordering::Relaxed);
            (*h).dropped_seq.store(0, Ordering::Relaxed);
            (*h).read_seq.store(0, Ordering::Relaxed);
            (*h).state.store(RING_STATE_LIVE, Ordering::Relaxed);
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
        })
    }

    /// Attach to an existing ring and validate it against `expect`.
    pub fn attach(region: R, expect: RingExpect) -> Result<RingProducer<R>, RingError> {
        let meta = check_region(&region, expect)?;
        Ok(RingProducer { region, meta })
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
    pub fn close(&mut self) {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `state` is a whole atomic.
        unsafe { (*h).state.store(RING_STATE_CLOSED, Ordering::Release) };
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
        let occupancy = write_seq.wrapping_sub(self.read_seq(Ordering::Acquire));
        let free = u64::from(self.meta.capacity).saturating_sub(occupancy);
        let accepted = (offered as u64).min(free) as usize;
        let dropped = offered - accepted;

        if accepted > 0 {
            self.copy_into_slots(write_seq, samples.as_ptr(), accepted);
            self.store_write_seq(write_seq + accepted as u64);
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
    pub fn write_slot(&mut self, slot: &VisSlot) -> Result<bool, RingError> {
        self.require_kind(RingKind::Vis)?;
        self.require_live()?;

        let write_seq = self.write_seq(Ordering::Relaxed);
        let dropped_seq = self.dropped_seq(Ordering::Relaxed);
        let occupancy = write_seq.wrapping_sub(self.read_seq(Ordering::Acquire));
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
        }
        self.store_write_seq(write_seq + 1);
        Ok(true)
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
        unsafe { (*h).state.load(Ordering::Relaxed) }
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
        Ok(RingConsumer { region, meta })
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
        unsafe { (*h).state.load(Ordering::Relaxed) == RING_STATE_CLOSED }
    }

    /// Mark the ring closed.
    pub fn close(&mut self) {
        let h = header_ptr(&self.region);
        // SAFETY: `h` points to a live header; `state` is a whole atomic.
        unsafe { (*h).state.store(RING_STATE_CLOSED, Ordering::Release) };
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
        Ok(count)
    }

    /// Read one complete visualization slot (vis rings only).
    ///
    /// Returns `true` and fills `out` when a whole slot is available. Returns
    /// `false` when nothing is published; a slot that is not (yet) published is
    /// never delivered as data (RFC 0003 section 7). The slot's `seq` lets the
    /// caller detect frames skipped since the previous read.
    pub fn read_slot_into(&mut self, out: &mut VisSlot) -> Result<bool, RingError> {
        self.require_kind(RingKind::Vis)?;

        let write_seq = self.write_seq(Ordering::Acquire);
        let read_seq = self.read_seq(Ordering::Relaxed);
        if write_seq.wrapping_sub(read_seq) == 0 {
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
        Ok(true)
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
    use super::*;
    use crate::shm::header::{RING_DATA_OFFSET, RING_HEADER_SIZE, VIS_SLOT_SIZE};
    use crate::shm::region::AnonymousRegion;
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
}
