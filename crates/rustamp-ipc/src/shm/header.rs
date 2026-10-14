//! Byte layout of a shared-memory ring (RFC 0003).
//!
//! This module is **pure layout**: it contains no `unsafe` and no platform
//! calls. It defines the header a producer and a consumer agree on, the slot
//! layout for the two ring kinds, and compile-time assertions that pin every
//! size and offset on both architectures.
//!
//! Offsets, sizes and constants here are the contract from RFC 0003 section 4
//! and 5. Changing any of them changes the wire format and needs a new RFC.

use core::mem::{align_of, offset_of, size_of};
use core::sync::atomic::{AtomicU16, AtomicU32, AtomicU64};

use rustamp_core::vis::{VIS_CHANNELS, VIS_SAMPLES, VisFrame};

/// Header magic, the bytes `R S T R` read as a little-endian `u32`.
pub const RING_MAGIC: u32 = 0x5254_5352;

/// Ring layout version. Independent of the IPC `PROTOCOL_VERSION`.
pub const RING_VERSION: u16 = 1;

/// Size of [`RingHeader`] in bytes, identical on x86 and x86_64.
pub const RING_HEADER_SIZE: usize = 0xC0;

/// Byte offset of slot 0 from the start of the mapping.
pub const RING_DATA_OFFSET: usize = RING_HEADER_SIZE;

/// Minimum alignment of a mapping view that the ring accepts.
pub const RING_MIN_ALIGN: usize = 8;

/// Upper bound for the total mapping size, to reject absurd offers.
pub const MAX_RING_BYTES: u64 = 1 << 30;

/// `state` value: the ring is live.
pub const RING_STATE_LIVE: u32 = 0;

/// `state` value: one of the participants has closed the ring.
pub const RING_STATE_CLOSED: u32 = 1;

/// `flags` bit 0: the creator runs as a 32-bit process (informational).
pub const RING_FLAG_CREATOR_32BIT: u32 = 1;

/// Size of one visualization slot in bytes.
pub const VIS_SLOT_SIZE: usize = size_of::<VisSlot>();

/// Which payload a ring carries. The value is part of the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u16)]
pub enum RingKind {
    /// Interleaved `f32` PCM frames; one slot is one audio frame.
    Pcm = 1,
    /// One [`VisSlot`] per slot; the consumer always sees a whole frame.
    Vis = 2,
}

impl RingKind {
    /// Header value of this kind.
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    /// Kind for a header value, if it is known.
    pub fn from_u16(value: u16) -> Option<RingKind> {
        match value {
            1 => Some(RingKind::Pcm),
            2 => Some(RingKind::Vis),
            _ => None,
        }
    }

    /// Human-readable name, for error messages.
    pub fn name(self) -> &'static str {
        match self {
            RingKind::Pcm => "pcm",
            RingKind::Vis => "vis",
        }
    }
}

/// Which way data flows through a ring (RFC 0003 section 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RingDirection {
    /// `rustamp.exe` (x64) produces, `rustamp-plughost32.exe` (i686) consumes.
    MainToHost,
    /// The host produces, the main process consumes.
    HostToMain,
}

/// Fixed header of a shared-memory ring.
///
/// The layout is asserted below and is identical on x86 and x86_64. The three
/// 64-byte cache lines are: metadata (`0x00..0x40`), producer-owned counters
/// (`0x40..0x80`) and the consumer-owned counter (`0x80..0xC0`).
///
/// Only the creator writes the metadata fields, and it writes `magic` last with
/// a `Release` store; an opener that `Acquire`s a valid `magic` therefore sees
/// the complete header. `write_seq`/`dropped_seq` are owned by the producer,
/// `read_seq` by the consumer, and `state` by either side.
///
/// Every field is a whole atomic, so no access to a shared header is a data
/// race even while another process is creating the ring.
#[repr(C)]
#[derive(Debug)]
pub struct RingHeader {
    /// [`RING_MAGIC`]; written last, so a valid magic implies a complete header.
    pub magic: AtomicU32,
    /// [`RING_VERSION`].
    pub version: AtomicU16,
    /// [`RingKind`] as a `u16`.
    pub kind: AtomicU16,
    /// [`RING_HEADER_SIZE`] as a `u32`.
    pub header_size: AtomicU32,
    /// Bytes per slot.
    pub frame_size: AtomicU32,
    /// Number of slots; always `> 0`.
    pub capacity: AtomicU32,
    /// [`RING_DATA_OFFSET`] as a `u32`.
    pub data_offset: AtomicU32,
    /// Creator flags, see `RING_FLAG_*`.
    pub flags: AtomicU32,
    /// [`RING_STATE_LIVE`] or [`RING_STATE_CLOSED`]; atomic, written by both.
    pub state: AtomicU32,
    /// Epoch of this ring instance; see RFC 0003 section 8.
    pub generation: AtomicU64,
    /// Reserved; always zero.
    pub reserved0: [u64; 3],
    /// Frames the producer has published.
    pub write_seq: AtomicU64,
    /// Frames the producer has dropped.
    pub dropped_seq: AtomicU64,
    /// Reserved; always zero.
    pub reserved1: [u64; 6],
    /// Frames the consumer has consumed.
    pub read_seq: AtomicU64,
    /// Reserved; always zero.
    pub reserved2: [u64; 7],
}

/// One visualization frame as it sits in a vis ring slot (RFC 0003 section 5).
///
/// `seq` is written by the producer and is the frame's *offered index*; a value
/// set by a caller is overwritten. The arrays are laid out exactly like
/// [`VisFrame`], so converting is a plain copy.
#[repr(C)]
#[derive(Clone)]
pub struct VisSlot {
    /// Offered index of this frame; strictly increasing between slots.
    pub seq: u64,
    /// See [`VisFrame::timestamp_us`].
    pub timestamp_us: u64,
    /// Flattened waveform, `[channel][sample]`.
    pub waveform: [[f32; VIS_SAMPLES]; VIS_CHANNELS],
    /// Flattened spectrum, `[channel][bin]`.
    pub spectrum: [[f32; VIS_SAMPLES]; VIS_CHANNELS],
}

impl VisSlot {
    /// An all-zero slot (`seq` and `timestamp_us` zero).
    pub fn zeroed() -> VisSlot {
        VisSlot {
            seq: 0,
            timestamp_us: 0,
            waveform: [[0.0; VIS_SAMPLES]; VIS_CHANNELS],
            spectrum: [[0.0; VIS_SAMPLES]; VIS_CHANNELS],
        }
    }

    /// Fill a slot from a [`VisFrame`]; `seq` is left at zero for the producer
    /// to overwrite.
    pub fn from_frame(frame: &VisFrame) -> VisSlot {
        VisSlot {
            seq: 0,
            timestamp_us: frame.timestamp_us,
            waveform: frame.waveform,
            spectrum: frame.spectrum,
        }
    }

    /// Copy the payload into a [`VisFrame`], dropping `seq`.
    pub fn to_frame(&self) -> VisFrame {
        VisFrame {
            timestamp_us: self.timestamp_us,
            waveform: self.waveform,
            spectrum: self.spectrum,
        }
    }
}

impl Default for VisSlot {
    fn default() -> Self {
        VisSlot::zeroed()
    }
}

impl core::fmt::Debug for VisSlot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VisSlot")
            .field("seq", &self.seq)
            .field("timestamp_us", &self.timestamp_us)
            .field("waveform", &"[[f32; 576]; 2]")
            .field("spectrum", &"[[f32; 576]; 2]")
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Compile-time layout assertions (RFC 0003 sections 4 and 5).
// ---------------------------------------------------------------------------

const _: () = {
    assert!(size_of::<RingHeader>() == RING_HEADER_SIZE);
    assert!(align_of::<RingHeader>() >= RING_MIN_ALIGN);
    assert!(RING_HEADER_SIZE % 64 == 0);

    assert!(offset_of!(RingHeader, magic) == 0x00);
    assert!(offset_of!(RingHeader, version) == 0x04);
    assert!(offset_of!(RingHeader, kind) == 0x06);
    assert!(offset_of!(RingHeader, header_size) == 0x08);
    assert!(offset_of!(RingHeader, frame_size) == 0x0C);
    assert!(offset_of!(RingHeader, capacity) == 0x10);
    assert!(offset_of!(RingHeader, data_offset) == 0x14);
    assert!(offset_of!(RingHeader, flags) == 0x18);
    assert!(offset_of!(RingHeader, state) == 0x1C);
    assert!(offset_of!(RingHeader, generation) == 0x20);
    assert!(offset_of!(RingHeader, reserved0) == 0x28);
    assert!(offset_of!(RingHeader, write_seq) == 0x40);
    assert!(offset_of!(RingHeader, dropped_seq) == 0x48);
    assert!(offset_of!(RingHeader, reserved1) == 0x50);
    assert!(offset_of!(RingHeader, read_seq) == 0x80);
    assert!(offset_of!(RingHeader, reserved2) == 0x88);

    // The 64-bit counters must be 8-byte aligned on both architectures.
    assert!(offset_of!(RingHeader, write_seq) % 8 == 0);
    assert!(offset_of!(RingHeader, dropped_seq) % 8 == 0);
    assert!(offset_of!(RingHeader, read_seq) % 8 == 0);

    assert!(size_of::<VisSlot>() == 9232);
    assert!(align_of::<VisSlot>() >= RING_MIN_ALIGN);
    assert!(offset_of!(VisSlot, seq) == 0x00);
    assert!(offset_of!(VisSlot, timestamp_us) == 0x08);
    assert!(offset_of!(VisSlot, waveform) == 0x10);
    assert!(offset_of!(VisSlot, spectrum) == 0x1210);
    assert!(size_of::<VisSlot>() == offset_of!(VisSlot, spectrum) + 4608);
    assert!(VIS_SAMPLES == 576);
    assert!(VIS_CHANNELS == 2);
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The offset table from RFC 0003 section 4, as runtime values.
    const HEADER_OFFSETS: [(usize, usize); 16] = [
        (offset_of!(RingHeader, magic), 4),
        (offset_of!(RingHeader, version), 2),
        (offset_of!(RingHeader, kind), 2),
        (offset_of!(RingHeader, header_size), 4),
        (offset_of!(RingHeader, frame_size), 4),
        (offset_of!(RingHeader, capacity), 4),
        (offset_of!(RingHeader, data_offset), 4),
        (offset_of!(RingHeader, flags), 4),
        (offset_of!(RingHeader, state), 4),
        (offset_of!(RingHeader, generation), 8),
        (offset_of!(RingHeader, write_seq), 8),
        (offset_of!(RingHeader, dropped_seq), 8),
        (offset_of!(RingHeader, read_seq), 8),
        (offset_of!(RingHeader, reserved0), 24),
        (offset_of!(RingHeader, reserved1), 48),
        (offset_of!(RingHeader, reserved2), 56),
    ];

    fn assert_header_table() {
        assert_eq!(size_of::<RingHeader>(), 192);
        assert_eq!(RING_HEADER_SIZE, 0xC0);
        assert_eq!(RING_DATA_OFFSET, 0xC0);
        assert_eq!(HEADER_OFFSETS[0], (0x00, 4));
        assert_eq!(HEADER_OFFSETS[1], (0x04, 2));
        assert_eq!(HEADER_OFFSETS[2], (0x06, 2));
        assert_eq!(HEADER_OFFSETS[3], (0x08, 4));
        assert_eq!(HEADER_OFFSETS[4], (0x0C, 4));
        assert_eq!(HEADER_OFFSETS[5], (0x10, 4));
        assert_eq!(HEADER_OFFSETS[6], (0x14, 4));
        assert_eq!(HEADER_OFFSETS[7], (0x18, 4));
        assert_eq!(HEADER_OFFSETS[8], (0x1C, 4));
        assert_eq!(HEADER_OFFSETS[9], (0x20, 8));
        assert_eq!(HEADER_OFFSETS[10], (0x40, 8));
        assert_eq!(HEADER_OFFSETS[11], (0x48, 8));
        assert_eq!(HEADER_OFFSETS[12], (0x80, 8));
        assert_eq!(HEADER_OFFSETS[13], (0x28, 24));
        assert_eq!(HEADER_OFFSETS[14], (0x50, 48));
        assert_eq!(HEADER_OFFSETS[15], (0x88, 56));

        assert_eq!(RING_MAGIC, 0x5254_5352);
        assert_eq!(RING_VERSION, 1);
        assert_eq!(RingKind::Pcm.as_u16(), 1);
        assert_eq!(RingKind::Vis.as_u16(), 2);
        assert_eq!(RingKind::from_u16(3), None);
    }

    fn assert_slot_table() {
        assert_eq!(VIS_SLOT_SIZE, 9232);
        assert_eq!(size_of::<VisSlot>(), 9232);
        assert_eq!(offset_of!(VisSlot, seq), 0x00);
        assert_eq!(offset_of!(VisSlot, timestamp_us), 0x08);
        assert_eq!(offset_of!(VisSlot, waveform), 0x10);
        assert_eq!(offset_of!(VisSlot, spectrum), 0x1210);
    }

    #[test]
    fn layout_is_arch_independent() {
        assert_header_table();
        assert_slot_table();
    }

    #[cfg(target_arch = "x86")]
    #[test]
    fn x86_ring_layout() {
        assert_header_table();
        assert_slot_table();
        assert_eq!(size_of::<usize>(), 4);
        assert_eq!(align_of::<AtomicU64>(), 8);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn x86_64_ring_layout() {
        assert_header_table();
        assert_slot_table();
        assert_eq!(size_of::<usize>(), 8);
    }

    #[test]
    fn vis_slot_frame_roundtrip() {
        let mut frame = VisFrame {
            timestamp_us: 4242,
            ..VisFrame::default()
        };
        frame.waveform[1][575] = -0.5;
        frame.spectrum[0][3] = 0.25;
        let slot = VisSlot::from_frame(&frame);
        assert_eq!(slot.seq, 0);
        assert_eq!(slot.timestamp_us, 4242);
        assert_eq!(slot.to_frame(), frame);
        assert_eq!(slot.clone().to_frame(), frame);
    }
}
