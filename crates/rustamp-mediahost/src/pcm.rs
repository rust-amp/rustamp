//! PCM ring allocation/opening, separate from residents and control messages.

use rustamp_ipc::Message;
use rustamp_ipc::shm::{MappedRegion, RingConsumer, RingProducer};
use rustamp_proto::host::PcmSpecWire;

use crate::control::RingAbort;
use crate::{MediaHostError, Result};

/// A newly allocated host-to-player ring. No PCM may flow until RingReady.
pub struct PcmOffer<R: MappedRegion> {
    /// The host's sole PCM writer (always configured with wake events).
    pub producer: RingProducer<R>,
    /// The unmodified RFC 0003 offer.
    pub message: Message,
    /// A separate view used only to close, never to consume PCM.
    pub abort: Box<dyn RingAbort>,
}

/// An opened ring and its independent supervision close handle.
pub struct PcmInput<R: MappedRegion> {
    /// The player's sole PCM reader, with the matching wake events.
    pub consumer: RingConsumer<R>,
    /// A separate view used only to close.
    pub abort: Box<dyn RingAbort>,
}

/// Mapping policy. The real implementation is [`NamedPcmRings`] on Windows;
/// tests substitute shared anonymous regions, not a second transfer algorithm.
pub trait PcmRings: Send + 'static {
    /// Mapping backing each endpoint.
    type Region: MappedRegion + 'static;
    /// Allocate a fresh ring and its wake before announcing it.
    fn create(&self, spec: PcmSpecWire, capacity: u32) -> Result<PcmOffer<Self::Region>>;
    /// Validate/open the ring and wake before returning RingReady.
    fn open(&self, offer: &Message, spec: PcmSpecWire) -> Result<PcmInput<Self::Region>>;
}

/// Validate a wire format before doing channel arithmetic or allocating.
pub fn validate_spec(spec: PcmSpecWire) -> Result<()> {
    if spec.channels == 0 || spec.sample_rate_hz == 0 {
        return Err(MediaHostError::InvalidMessage(
            "zero PCM channels/sample rate".into(),
        ));
    }
    Ok(())
}

/// Close-only view: deliberately exposes no reads, writes or counter updates.
pub struct CloseRing<R: MappedRegion>(pub RingConsumer<R>);

impl<R: MappedRegion> RingAbort for CloseRing<R> {
    fn abort(&mut self) {
        self.0.close();
    }
}

/// Windows shared-memory sections with two named auto-reset events per ring.
#[cfg(windows)]
#[derive(Clone, Copy, Default)]
pub struct NamedPcmRings;

#[cfg(windows)]
impl PcmRings for NamedPcmRings {
    type Region = rustamp_ipc::shm::NamedRegion;

    fn create(&self, spec: PcmSpecWire, capacity: u32) -> Result<PcmOffer<Self::Region>> {
        use rustamp_ipc::shm::{
            MAX_RING_BYTES, NamedRegion, RING_DATA_OFFSET, RingDirection, RingKind, RingWake,
            ring_name,
        };
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        static SERIAL: AtomicU64 = AtomicU64::new(1);
        validate_spec(spec)?;
        let size = RING_DATA_OFFSET as u64 + u64::from(capacity) * u64::from(spec.frame_size());
        if capacity == 0 || size > MAX_RING_BYTES {
            return Err(rustamp_ipc::shm::RingError::InvalidLayout.into());
        }
        // Mapping names are opaque on the wire. Time + process + monotonic
        // serial distinguish simultaneous decoders and successive generations.
        let name = format!(
            "{}-{:x}",
            ring_name(RingKind::Pcm, SERIAL.fetch_add(1, Ordering::Relaxed)),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let producer = RingProducer::create(
            NamedRegion::create(&name, size as usize)?,
            RingKind::Pcm,
            spec.frame_size(),
            capacity,
        )?;
        let producer = producer.with_wake(RingWake::create(&name)?);
        let abort =
            RingConsumer::attach(NamedRegion::open(&name, size as usize)?, producer.expect())?
                .with_wake(RingWake::open(&name)?);
        let message = Message::RingOffer {
            name,
            kind: RingKind::Pcm,
            frame_size: spec.frame_size(),
            capacity,
            size,
            generation: producer.generation(),
            direction: RingDirection::HostToMain,
        };
        Ok(PcmOffer {
            producer,
            message,
            abort: Box::new(CloseRing(abort)),
        })
    }

    fn open(&self, offer: &Message, spec: PcmSpecWire) -> Result<PcmInput<Self::Region>> {
        use rustamp_ipc::shm::{
            MAX_RING_BYTES, NamedRegion, RING_DATA_OFFSET, RingDirection, RingError, RingKind,
            RingWake,
        };
        validate_spec(spec)?;
        let Message::RingOffer {
            name,
            kind,
            frame_size,
            capacity,
            size,
            generation,
            direction,
        } = offer
        else {
            return Err(MediaHostError::InvalidMessage(
                "expected PCM RingOffer".into(),
            ));
        };
        if *kind != RingKind::Pcm || *direction != RingDirection::HostToMain {
            return Err(MediaHostError::InvalidMessage(
                "wrong decoder ring kind/direction".into(),
            ));
        }
        if *frame_size != spec.frame_size() {
            return Err(RingError::FrameSizeMismatch {
                expected: spec.frame_size(),
                found: *frame_size,
            }
            .into());
        }
        if *capacity == 0
            || *generation == 0
            || *size > MAX_RING_BYTES
            || *size != RING_DATA_OFFSET as u64 + u64::from(*capacity) * u64::from(*frame_size)
            || name.is_empty()
            || name.contains('\0')
        {
            return Err(RingError::InvalidLayout.into());
        }
        let expect = offer.ring_expect().unwrap();
        let consumer = RingConsumer::attach(NamedRegion::open(name, *size as usize)?, expect)?
            .with_wake(RingWake::open(name)?);
        let abort = RingConsumer::attach(NamedRegion::open(name, *size as usize)?, expect)?
            .with_wake(RingWake::open(name)?);
        Ok(PcmInput {
            consumer,
            abort: Box::new(CloseRing(abort)),
        })
    }
}
