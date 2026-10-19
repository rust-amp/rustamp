//! PCM ring allocation/opening, separate from residents and control messages.
//!
//! Every PCM wait in this crate goes through [`read_frames`]/[`write_frames`],
//! which slice the wait so that delivery depends on the ring counters alone
//! and never on a wake signal arriving (see [`RING_POLL`]).

use std::time::{Duration, Instant};

use rustamp_ipc::Message;
use rustamp_ipc::shm::{
    MappedRegion, RingConsumer, RingDirection, RingError, RingKind, RingProducer,
};
use rustamp_proto::host::PcmSpecWire;

use crate::control::RingAbort;
use crate::{MediaHostError, Result};

/// Longest single wait on one ring wake event.
///
/// RFC 0007 §6.1 and §6.3 make the ring counters the source of truth and the
/// wake event only a hint: "a lost, duplicated or spurious signal can never
/// turn into a wrong frame", because the waiter re-derives its condition from
/// the counters after every wake. A waiter that spends its whole deadline in
/// one `WaitForSingleObject` turns that hint into a liveness dependency: if
/// the signal is skipped, the wait runs to the caller's bound even though the
/// counters already say the condition is met. Slicing the wait restores the
/// contract — a skipped signal then costs one slice, not the whole bound.
///
/// The ring now signals after **every** counter advance. RFC 0007 §6.2's
/// edge-triggered refinement was withdrawn, not repaired; see
/// `docs/tasks/W19-ring-wake-root.md`. Slicing remains a defensive fallback
/// for a missing/mispaired wake (injected by `tests/deaf_wake.rs`), not a
/// prerequisite for progress with the current, correctly paired ring. A
/// crashed peer's Drop does not signal or close: the independent control
/// monitor must detect that failure and close its supervised ring view.
///
/// 20 ms is the DSP worker's existing `CONTROL_POLL`, which already bounds
/// both of its ring waits for the same kind of reason: invisible next to
/// every caller bound on this path (5 s in the engine adapter, 30 s in the
/// suites) and 50 event waits per second for a ring that is idle anyway.
pub(crate) const RING_POLL: Duration = Duration::from_millis(20);

/// Ring counters at one instant. Diagnostics only — the three values are
/// read one after the other, so a live peer may advance between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingCounters {
    /// Frames the producer has published.
    pub published: u64,
    /// Frames the consumer has taken.
    pub read: u64,
    /// Whether either end has closed the ring.
    pub closed: bool,
}

impl RingCounters {
    /// Frames published and not yet taken.
    ///
    /// This is the value that tells a lost wake from a starved schedule: a
    /// read that timed out while this is nonzero waited on data the ring
    /// already held.
    pub fn available(&self) -> u64 {
        self.published.wrapping_sub(self.read)
    }
}

/// The next slice to wait, or `None` once the caller's deadline has passed.
fn next_slice(deadline: Option<Instant>) -> Option<Duration> {
    match deadline {
        None => Some(RING_POLL),
        Some(deadline) => {
            let left = deadline.saturating_duration_since(Instant::now());
            (!left.is_zero()).then(|| left.min(RING_POLL))
        }
    }
}

/// Read whole frames, with the wait sliced to [`RING_POLL`] (see there).
///
/// `deadline` keeps the meanings [`RingConsumer::read_into_blocking`] gives a
/// timeout, so no caller contract changes: `None` waits until data, a closed
/// ring or a failure; `Some` returns `Ok(0)` once it has passed. A slice that
/// expires is not an outcome — it only sends the loop back to the counters.
pub(crate) fn read_frames<R: MappedRegion>(
    consumer: &mut RingConsumer<R>,
    out: &mut [f32],
    deadline: Option<Instant>,
) -> std::result::Result<usize, RingError> {
    loop {
        let Some(slice) = next_slice(deadline) else {
            return Ok(0);
        };
        match consumer.read_into_blocking(out, Some(slice))? {
            0 => {}
            frames => return Ok(frames),
        }
    }
}

/// Write whole frames, with the wait sliced to [`RING_POLL`] (see there).
///
/// Same deadline meanings as [`read_frames`]. `Ok(0)` means the deadline
/// passed with the ring still full; the blocking write never drops, so no
/// frame is lost by a slice or by the timeout.
pub(crate) fn write_frames<R: MappedRegion>(
    producer: &mut RingProducer<R>,
    samples: &[f32],
    deadline: Option<Instant>,
) -> std::result::Result<usize, RingError> {
    loop {
        let Some(slice) = next_slice(deadline) else {
            return Ok(0);
        };
        match producer.write_from_blocking(samples, Some(slice))? {
            0 => {}
            written => return Ok(written),
        }
    }
}

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
///
/// Every implementation must apply the offer rules of [`check_offer`]: a ring
/// is offered in one direction only and carries the spec's frame size, and an
/// opener refuses an offer whose direction or frame size is not the one it
/// expects, before any mapping opens.
pub trait PcmRings: Send + 'static {
    /// Mapping backing each endpoint.
    type Region: MappedRegion + 'static;
    /// Allocate a fresh ring and its wake before announcing it.
    ///
    /// The decoder path: the ring runs host -> main, so the offer announces
    /// [`RingDirection::HostToMain`].
    fn create(&self, spec: PcmSpecWire, capacity: u32) -> Result<PcmOffer<Self::Region>>;
    /// Validate/open the ring and wake before returning RingReady.
    ///
    /// The decoder path: an offer in another direction or with another frame
    /// size is refused ([`check_offer`]).
    fn open(&self, offer: &Message, spec: PcmSpecWire) -> Result<PcmInput<Self::Region>>;

    /// Direction-aware allocation. The default covers the decoder path
    /// (`HostToMain`) through [`PcmRings::create`]; factories with a real
    /// `MainToHost` mapping override it (the DSP in-ring, RFC 0006 §6).
    fn create_directed(
        &self,
        spec: PcmSpecWire,
        capacity: u32,
        direction: RingDirection,
    ) -> Result<PcmOffer<Self::Region>> {
        if direction == RingDirection::HostToMain {
            self.create(spec, capacity)
        } else {
            Err(MediaHostError::NotImplemented(
                "MainToHost rings need a directed factory".into(),
            ))
        }
    }

    /// Direction-aware open. The default covers the decoder path through
    /// [`PcmRings::open`]; see [`PcmRings::create_directed`].
    ///
    /// An offer whose direction is not `direction`, or whose frame size is
    /// not the spec's, is refused ([`check_offer`]).
    fn open_directed(
        &self,
        offer: &Message,
        spec: PcmSpecWire,
        direction: RingDirection,
    ) -> Result<PcmInput<Self::Region>> {
        if direction == RingDirection::HostToMain {
            self.open(offer, spec)
        } else {
            Err(MediaHostError::NotImplemented(
                "MainToHost rings need a directed factory".into(),
            ))
        }
    }
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

/// Refuse an offer that is not a PCM ring in the expected direction with the
/// expected frame size.
///
/// The direction tells an opener which end it is (RFC 0003 §9): the decoder's
/// PCM ring runs host -> main (`HostToMain`), the DSP in-ring main -> host
/// (`MainToHost`). An opener that expects one direction refuses an offer for
/// the other one **before** it opens a mapping, so a ring offered backwards
/// never carries data.
///
/// The frame size is the other half of the same contract: a ring whose slots
/// are not the spec's frame size cannot be read as PCM at all. A mismatch is
/// reported as [`RingError::FrameSizeMismatch`] — the error production
/// returns for it — and never as a panic, so an opener and its test double
/// answer a foreign offer the same way.
///
/// This is the single copy of both rules: production ([`NamedPcmRings`]) and
/// the in-process test doubles call it, so a test double cannot drift from
/// the rules production enforces.
pub fn check_offer(offer: &Message, expected: RingDirection, spec: PcmSpecWire) -> Result<()> {
    let Message::RingOffer {
        kind,
        direction,
        frame_size,
        ..
    } = offer
    else {
        return Err(MediaHostError::InvalidMessage(
            "expected PCM RingOffer".into(),
        ));
    };
    if *kind != RingKind::Pcm || *direction != expected {
        return Err(MediaHostError::InvalidMessage(
            "wrong ring kind/direction".into(),
        ));
    }
    if *frame_size != spec.frame_size() {
        return Err(RingError::FrameSizeMismatch {
            expected: spec.frame_size(),
            found: *frame_size,
        }
        .into());
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

/// Close-only view over two rings. The DSP conversation holds an in-ring and
/// a back-ring per generation, but the supervisor registers a single abort,
/// so both directions wake on control loss, shutdown or reconfigure.
pub struct ClosePair(pub Box<dyn RingAbort>, pub Box<dyn RingAbort>);

impl RingAbort for ClosePair {
    fn abort(&mut self) {
        self.0.abort();
        self.1.abort();
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
        Self::create_impl(spec, capacity, RingDirection::HostToMain)
    }

    fn open(&self, offer: &Message, spec: PcmSpecWire) -> Result<PcmInput<Self::Region>> {
        Self::open_impl(offer, spec, RingDirection::HostToMain)
    }

    fn create_directed(
        &self,
        spec: PcmSpecWire,
        capacity: u32,
        direction: RingDirection,
    ) -> Result<PcmOffer<Self::Region>> {
        Self::create_impl(spec, capacity, direction)
    }

    fn open_directed(
        &self,
        offer: &Message,
        spec: PcmSpecWire,
        direction: RingDirection,
    ) -> Result<PcmInput<Self::Region>> {
        Self::open_impl(offer, spec, direction)
    }
}

#[cfg(windows)]
impl NamedPcmRings {
    fn create_impl(
        spec: PcmSpecWire,
        capacity: u32,
        direction: RingDirection,
    ) -> Result<PcmOffer<rustamp_ipc::shm::NamedRegion>> {
        use rustamp_ipc::shm::{
            MAX_RING_BYTES, NamedRegion, RING_DATA_OFFSET, RingWake, ring_name,
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
            direction,
        };
        Ok(PcmOffer {
            producer,
            message,
            abort: Box::new(CloseRing(abort)),
        })
    }

    fn open_impl(
        offer: &Message,
        spec: PcmSpecWire,
        expected: RingDirection,
    ) -> Result<PcmInput<rustamp_ipc::shm::NamedRegion>> {
        use rustamp_ipc::shm::{
            MAX_RING_BYTES, NamedRegion, RING_DATA_OFFSET, RingError, RingWake,
        };
        validate_spec(spec)?;
        // The kind, the direction and the frame size are checked before any
        // mapping opens, so a ring offered backwards or with foreign slots
        // never carries data.
        check_offer(offer, expected, spec)?;
        let Message::RingOffer {
            name,
            frame_size,
            capacity,
            size,
            generation,
            ..
        } = offer
        else {
            // `check_offer` already refused every other message.
            return Err(MediaHostError::InvalidMessage(
                "expected PCM RingOffer".into(),
            ));
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A PCM ring offer in `direction`.
    fn pcm_offer(direction: RingDirection) -> Message {
        Message::RingOffer {
            name: "mediahost-direction-rule".into(),
            kind: RingKind::Pcm,
            frame_size: 8,
            capacity: 1,
            size: 64,
            generation: 1,
            direction,
        }
    }

    /// The spec behind [`pcm_offer`]: stereo, so a frame is 8 bytes.
    const SPEC: PcmSpecWire = PcmSpecWire {
        sample_rate_hz: 44100,
        channels: 2,
    };

    /// The spec with one channel: a frame is 4 bytes, not 8.
    const MONO: PcmSpecWire = PcmSpecWire {
        sample_rate_hz: 44100,
        channels: 1,
    };

    #[test]
    fn an_offer_in_the_expected_direction_is_accepted() {
        assert!(
            check_offer(
                &pcm_offer(RingDirection::HostToMain),
                RingDirection::HostToMain,
                SPEC
            )
            .is_ok()
        );
        assert!(
            check_offer(
                &pcm_offer(RingDirection::MainToHost),
                RingDirection::MainToHost,
                SPEC
            )
            .is_ok()
        );
    }

    #[test]
    fn an_offer_in_the_other_direction_is_refused() {
        for (offered, expected) in [
            (RingDirection::MainToHost, RingDirection::HostToMain),
            (RingDirection::HostToMain, RingDirection::MainToHost),
        ] {
            assert!(
                matches!(
                    check_offer(&pcm_offer(offered), expected, SPEC),
                    Err(MediaHostError::InvalidMessage(_))
                ),
                "a ring offered {offered:?} must not open as {expected:?}"
            );
        }
    }

    #[test]
    fn a_message_that_is_no_pcm_ring_offer_is_refused() {
        assert!(matches!(
            check_offer(&Message::hello(), RingDirection::HostToMain, SPEC),
            Err(MediaHostError::InvalidMessage(_))
        ));
        let mut vis = pcm_offer(RingDirection::HostToMain);
        if let Message::RingOffer { kind, .. } = &mut vis {
            *kind = RingKind::Vis;
        }
        assert!(matches!(
            check_offer(&vis, RingDirection::HostToMain, SPEC),
            Err(MediaHostError::InvalidMessage(_))
        ));
    }

    /// The frame size rule is the second half of the offer contract, and it
    /// is reported with the typed error production returns — never as a
    /// panic and never as a generic message.
    #[test]
    fn an_offer_with_a_foreign_frame_size_is_refused_with_the_frame_size_mismatch() {
        // The spec expects 4 bytes per frame, the offer carries 8.
        let error = check_offer(
            &pcm_offer(RingDirection::HostToMain),
            RingDirection::HostToMain,
            MONO,
        )
        .unwrap_err();
        assert!(
            matches!(
                &error,
                MediaHostError::Ring(RingError::FrameSizeMismatch {
                    expected: 4,
                    found: 8
                })
            ),
            "a foreign frame size must be reported as the frame size mismatch: {error}"
        );
        assert_eq!(
            error.to_string(),
            "frame size mismatch: expected 4, found 8"
        );

        // The other way round: the offer claims 4 bytes, the spec wants 8.
        let mut lying = pcm_offer(RingDirection::HostToMain);
        if let Message::RingOffer { frame_size, .. } = &mut lying {
            *frame_size = 4;
        }
        let error = check_offer(&lying, RingDirection::HostToMain, SPEC).unwrap_err();
        assert!(
            matches!(
                &error,
                MediaHostError::Ring(RingError::FrameSizeMismatch {
                    expected: 8,
                    found: 4
                })
            ),
            "a lying offer must be reported as the frame size mismatch: {error}"
        );
        assert_eq!(
            error.to_string(),
            "frame size mismatch: expected 8, found 4"
        );
    }

    /// The direction is checked before the frame size, so an offer that is
    /// wrong in both ways reports the direction — the order production
    /// refuses it in.
    #[test]
    fn the_direction_is_refused_before_the_frame_size() {
        let mut offer = pcm_offer(RingDirection::MainToHost);
        if let Message::RingOffer { frame_size, .. } = &mut offer {
            *frame_size = 4;
        }
        assert!(matches!(
            check_offer(&offer, RingDirection::HostToMain, SPEC),
            Err(MediaHostError::InvalidMessage(_))
        ));
    }
}
