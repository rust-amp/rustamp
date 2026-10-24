//! Shared-memory rings for bulk PCM and visualization data (RFC 0003).
//!
//! This is the transport that carries audio and vis frames between
//! `rustamp.exe` (x64) and `rustamp-plughost32.exe` (i686) without a lock and
//! without an allocation per frame. It is deliberately semantics-free: it moves
//! bytes, and the caller decides what they mean (spec S10 owns the message
//! catalogue, spec S09 the plugin call order).
//!
//! # Shape of the API
//!
//! * A [`MappedRegion`] is the memory: [`AnonymousRegion`] for one process
//!   (tests), [`NamedRegion`] for a Windows named file mapping (the real
//!   cross-process path).
//! * [`RingProducer`] and [`RingConsumer`] are the two ends. They own their
//!   region, so a ring can never outlive its mapping.
//! * [`VisSlot`] is the fixed-size slot of a vis ring; a PCM ring's slot is one
//!   interleaved `f32` audio frame.
//!
//! ```rust
//! use rustamp_ipc::shm::{
//!     AnonymousRegion, RING_DATA_OFFSET, RingConsumer, RingKind, RingProducer, VisSlot,
//! };
//!
//! // A stereo PCM ring: 64 frames of 8 bytes.
//! let region = AnonymousRegion::new(RING_DATA_OFFSET + 64 * 8).unwrap();
//! let mut producer = RingProducer::create(region.clone(), RingKind::Pcm, 8, 64).unwrap();
//! let mut consumer = RingConsumer::attach(region, producer.expect()).unwrap();
//!
//! let mut out = [0.0f32; 8];
//! let mut sent = 0usize;
//! let mut received = 0usize;
//! while received < 100 {
//!     if sent < 100 {
//!         sent += producer.write_from(&[0.5f32; 8]).unwrap();
//!     }
//!     received += consumer.read_into(&mut out).unwrap();
//! }
//! assert_eq!(received, 100);
//!
//! // The vis ring carries whole `VisSlot`s instead of raw samples.
//! let _slot = VisSlot::zeroed();
//! ```
//!
//! # Guarantees (RFC 0003 section 7)
//!
//! * The producer **never blocks** and **never overwrites unread data**. On a
//!   full ring it drops whole frames and counts them in `dropped_seq`.
//! * The consumer reads what is available and reports an unpublished frame as
//!   "not yet complete", never as data.
//! * Neither path allocates, locks, blocks or logs.
//! * The layout is byte-identical on x86 and x86_64, and the counters are 64-bit
//!   atomics, so a 32-bit producer and a 64-bit consumer agree.
//!
//! # Raw pointers
//!
//! `unsafe` is confined to this module subtree: [`region`] owns the mapping's
//! lifetime and [`ring`] dereferences the validated header and copies slot
//! bytes. [`header`] is pure layout with no `unsafe`. Every `unsafe` block has a
//! `// SAFETY:` comment naming the invariant.

pub mod header;
pub mod region;
pub mod ring;
pub mod wake;

pub use header::{
    MAX_RING_BYTES, RING_DATA_OFFSET, RING_FLAG_CREATOR_32BIT, RING_HEADER_SIZE, RING_MAGIC,
    RING_MIN_ALIGN, RING_STATE_CLOSED, RING_STATE_LIVE, RING_VERSION, RingDirection, RingHeader,
    RingKind, VIS_SLOT_SIZE, VisSlot,
};
#[cfg(windows)]
pub use region::NamedRegion;
pub use region::{AnonymousRegion, MappedRegion, ring_name};
pub use ring::{RingConsumer, RingExpect, RingProducer};
pub use wake::{RingWake, WaitOutcome, Wake, ring_wake_name};

/// Why a ring could not be used.
///
/// Every variant is produced by a check that runs before any counter is
/// touched, so a rejected ring can never be read as data (RFC 0003 section 8).
#[derive(Debug, thiserror::Error)]
pub enum RingError {
    /// Zero frame size or capacity, a frame size that is not a whole number of
    /// `f32` samples, a vis frame size that is not [`VIS_SLOT_SIZE`], or a ring
    /// larger than [`MAX_RING_BYTES`].
    #[error("ring layout is invalid")]
    InvalidLayout,
    /// The backing allocation for an [`AnonymousRegion`] failed.
    #[error("out of memory for the ring region")]
    OutOfMemory,
    /// A Win32 mapping call failed.
    #[error("mapping operation {op} failed (code {code})")]
    MappingFailed {
        /// Name of the failing operation.
        op: &'static str,
        /// `HRESULT`/Win32 error code.
        code: i32,
    },
    /// No mapping with the announced name exists (yet).
    #[error("no shared-memory mapping with that name")]
    MappingUnavailable,
    /// The mapping is smaller than the announced ring needs.
    #[error("ring region too small: need {needed} bytes, have {have}")]
    RegionTooSmall {
        /// Bytes the layout needs.
        needed: u64,
        /// Bytes the mapping has.
        have: u64,
    },
    /// The header does not start with [`RING_MAGIC`].
    #[error("not a ring: bad magic 0x{found:08x}")]
    BadMagic {
        /// Value found in the header.
        found: u32,
    },
    /// The header was written by an incompatible ring layout version.
    #[error("unsupported ring layout version {found}")]
    BadVersion {
        /// Value found in the header.
        found: u16,
    },
    /// The header size or data offset is not the one this build implements.
    #[error("unexpected ring header size or data offset")]
    BadHeaderSize,
    /// The header carries a kind value this build does not know.
    #[error("unknown ring kind {found}")]
    UnknownKind {
        /// Value found in the header.
        found: u16,
    },
    /// The ring carries a different kind than the opener expected.
    #[error("ring kind mismatch: expected {expected:?}, found {found:?}")]
    WrongKind {
        /// Kind the opener expected.
        expected: RingKind,
        /// Kind found in the header.
        found: RingKind,
    },
    /// The ring's frame size differs from the announced one.
    #[error("frame size mismatch: expected {expected}, found {found}")]
    FrameSizeMismatch {
        /// Bytes per slot the opener expected.
        expected: u32,
        /// Bytes per slot found in the header.
        found: u32,
    },
    /// The ring's capacity differs from the announced one.
    #[error("capacity mismatch: expected {expected}, found {found}")]
    CapacityMismatch {
        /// Slots the opener expected.
        expected: u32,
        /// Slots found in the header.
        found: u32,
    },
    /// The header generation differs from the announced one: the ring was
    /// re-created (stale) or is a different ring with the same name.
    #[error("stale ring: expected generation {expected}, found {found}")]
    Stale {
        /// Generation the opener expected.
        expected: u64,
        /// Generation found in the header.
        found: u64,
    },
    /// The call needs a different ring kind (e.g. `write_from` on a vis ring).
    #[error("operation needs a {expected:?} ring but this is a {found:?} ring")]
    WrongRingKind {
        /// Kind the operation needs.
        expected: RingKind,
        /// Kind of this ring.
        found: RingKind,
    },
    /// The buffer is not a whole number of frames.
    #[error(
        "buffer of {samples} samples is not a whole number of frames of {samples_per_frame} samples"
    )]
    NotFrameAligned {
        /// Samples in the buffer.
        samples: usize,
        /// Samples per frame of this ring.
        samples_per_frame: usize,
    },
    /// The ring was closed by one of the participants.
    #[error("ring is closed")]
    Closed,
    /// A blocking call was made on a ring that has no wake object attached.
    ///
    /// Only PCM rings that opt in via [`RingWake`] support blocking; vis rings
    /// and rings without `with_wake` are always non-blocking.
    #[error("ring has no wake object: blocking is not available on this ring")]
    NoWakeObject,
    /// A wake event syscall failed (create, open, set, or wait).
    #[error("wake event {op} failed (code {code})")]
    WakeOpenFailed {
        /// Name of the failing operation.
        op: &'static str,
        /// `HRESULT`/Win32 error code.
        code: i32,
    },
}

impl RingError {
    /// The wire reason for a refusal, if this error can be reported to the peer
    /// (`RingRefused` in RFC 0003 section 9).
    ///
    /// Local-only failures (out of memory, a misaligned caller buffer, a closed
    /// ring) return `None`; they are not the peer's fault.
    pub fn reason(&self) -> Option<RingRefuseReason> {
        Some(match self {
            RingError::InvalidLayout => RingRefuseReason::InvalidLayout,
            RingError::MappingFailed { .. } | RingError::MappingUnavailable => {
                RingRefuseReason::MappingUnavailable
            }
            RingError::RegionTooSmall { .. } => RingRefuseReason::RegionTooSmall,
            RingError::BadMagic { .. } => RingRefuseReason::BadMagic,
            RingError::BadVersion { .. } => RingRefuseReason::BadVersion,
            RingError::BadHeaderSize => RingRefuseReason::BadHeaderSize,
            RingError::UnknownKind { .. } | RingError::WrongKind { .. } => {
                RingRefuseReason::WrongKind
            }
            RingError::FrameSizeMismatch { .. } => RingRefuseReason::FrameSizeMismatch,
            RingError::CapacityMismatch { .. } => RingRefuseReason::CapacityMismatch,
            RingError::Stale { .. } => RingRefuseReason::Stale,
            RingError::OutOfMemory
            | RingError::WrongRingKind { .. }
            | RingError::NotFrameAligned { .. }
            | RingError::Closed
            | RingError::NoWakeObject => return None,
            RingError::WakeOpenFailed { .. } => RingRefuseReason::WakeUnavailable,
        })
    }
}

/// Why a peer refused a ring offer; carried by `Message::RingRefused`.
///
/// This is a closed enum on purpose: a peer can always tell the other side
/// *why* it refused without parsing an error string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RingRefuseReason {
    /// The mapping could not be created or opened.
    MappingUnavailable,
    /// The header magic is wrong.
    BadMagic,
    /// The header layout version is not supported.
    BadVersion,
    /// The header size or data offset is wrong.
    BadHeaderSize,
    /// The header kind is unknown or different from the offer.
    WrongKind,
    /// The header frame size differs from the offer.
    FrameSizeMismatch,
    /// The header capacity differs from the offer.
    CapacityMismatch,
    /// The header generation differs from the offer: the ring is stale.
    Stale,
    /// The mapping is too small for the announced layout.
    RegionTooSmall,
    /// The announced layout itself is invalid.
    InvalidLayout,
    /// The wake events for a wake ring could not be opened (RFC 0007).
    WakeUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuse_reasons_are_closed_and_stable() {
        // A refused ring always tells the peer why.
        assert_eq!(
            RingError::Stale {
                expected: 1,
                found: 2
            }
            .reason(),
            Some(RingRefuseReason::Stale)
        );
        assert_eq!(RingError::Closed.reason(), None);
        assert_eq!(
            RingError::NotFrameAligned {
                samples: 3,
                samples_per_frame: 2
            }
            .reason(),
            None
        );
        // RFC 0007: a blocking call on a wake-less ring is a local error.
        assert_eq!(RingError::NoWakeObject.reason(), None);
        // RFC 0007: a failed event syscall is reported to the peer as WakeUnavailable.
        assert_eq!(
            RingError::WakeOpenFailed {
                op: "OpenEventW",
                code: 0
            }
            .reason(),
            Some(RingRefuseReason::WakeUnavailable)
        );
        assert_eq!(RING_MAGIC, 0x5254_5352);
        assert_eq!(RING_HEADER_SIZE, 0xC0);
        assert_eq!(RING_DATA_OFFSET, RING_HEADER_SIZE);
        assert_eq!(VIS_SLOT_SIZE, 9232);
    }
}
