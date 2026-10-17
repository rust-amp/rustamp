//! RFC 0006 DSP chain worker, resident interface and player-side client.
//!
//! Two PCM rings, strict FIFO, the same `frame_size` on both. The player
//! (main) creates and offers the in-ring (`MainToHost`); the host creates and
//! offers the back-ring (`HostToMain`). The host processes strictly in order,
//! so block *n* out is block *n* back; no sequence numbers cross the
//! boundary. All counts are **frames**, never interleaved samples.
//!
//! End of stream has no marker message (unlike `DecoderEos`): the player
//! closes its in-ring after the final block, the host publishes everything it
//! read and closes the back-ring, and the player exposes `Ok(0)` only after
//! that closed back-ring is drained, then sticks there. A dead host is never
//! silent EOS: the supervisor fails sticky and every wait turns loud.

use std::time::{Duration, Instant};

use rustamp_ipc::Message;
use rustamp_ipc::shm::{
    MappedRegion, RingConsumer, RingDirection, RingError, RingKind, RingProducer, RingRefuseReason,
};
use rustamp_proto::host::{DspStageDesc, PcmSpecWire};

use crate::control::{ControlPeer, RingAbort};
use crate::pcm::{ClosePair, PcmOffer, PcmRings, validate_spec};
use crate::{MediaHostError, Result};

const CONTROL_WAIT: Duration = Duration::from_secs(5);
const BLOCK_FRAMES: usize = 1024;
/// Host back-ring capacity in frames. `DspConfigure` carries no capacity hint
/// (RFC 0006 §6), so this is an own choice: room for a few blocks while the
/// player drains, small enough to stay far under the mapping limit.
const BACK_RING_FRAMES: u32 = 4096;

/// A live DSP chain supplied by a separately licensed resident crate.
pub trait ResidentChain: Send {
    /// Total chain latency in frames at the prepared rate.
    fn latency_frames(&self) -> u32;
    /// Process interleaved frames in place; the frame count never changes.
    fn process(&mut self, buf: &mut [f32]);
    /// Clear internal state (filter history, look-ahead); rings untouched.
    fn reset(&mut self);
}

/// Opens one DSP chain in the host process for a validated spec and stage list.
pub trait DspResident {
    /// Unknown stage ids or undecodable params fail here, before any ring
    /// exists, and become `DspError`, never silence.
    fn open(&self, spec: PcmSpecWire, stages: &[DspStageDesc]) -> Result<Box<dyn ResidentChain>>;
}

/// Explicit unavailable resident used by the framework's placeholder binary.
pub struct UnavailableDspResident;

impl DspResident for UnavailableDspResident {
    fn open(&self, _spec: PcmSpecWire, _stages: &[DspStageDesc]) -> Result<Box<dyn ResidentChain>> {
        Err(MediaHostError::NotImplemented(
            "no DSP resident linked; select a resident helper binary".into(),
        ))
    }
}

struct ActiveRing<R: MappedRegion>(RingProducer<R>);
impl<R: MappedRegion> Drop for ActiveRing<R> {
    fn drop(&mut self) {
        self.0.close();
    }
}

struct ActiveConsumer<R: MappedRegion>(RingConsumer<R>);
impl<R: MappedRegion> Drop for ActiveConsumer<R> {
    fn drop(&mut self) {
        self.0.close();
    }
}

enum Transition {
    Reconfigure {
        spec: PcmSpecWire,
        stages: Vec<DspStageDesc>,
    },
    Stop,
}

enum PumpOut {
    Reconfigure {
        spec: PcmSpecWire,
        stages: Vec<DspStageDesc>,
    },
    Stop,
    InputClosed,
}

impl Transition {
    fn into_pump(self) -> PumpOut {
        match self {
            Transition::Reconfigure { spec, stages } => PumpOut::Reconfigure { spec, stages },
            Transition::Stop => PumpOut::Stop,
        }
    }
}

/// A reconfigure or shutdown command; `DspReset` is a no-op here (it clears
/// stage state but never touches rings, so there is nothing to transition).
fn command(message: Message) -> Result<Option<Transition>> {
    match message {
        Message::DspConfigure { spec, stages } => {
            Ok(Some(Transition::Reconfigure { spec, stages }))
        }
        Message::Shutdown => Ok(Some(Transition::Stop)),
        Message::DspReset => Ok(None),
        message => Err(MediaHostError::InvalidMessage(format!(
            "unexpected DSP command: {message:?}"
        ))),
    }
}

fn wait_command(peer: &ControlPeer) -> Result<Message> {
    loop {
        match peer.recv(CONTROL_WAIT) {
            Err(MediaHostError::TimedOut) => continue, // healthy idle peer
            result => return result,
        }
    }
}

fn refuse(peer: &ControlPeer, offer: &Message, error: &MediaHostError) {
    if let Some(expect) = offer.ring_expect() {
        let reason = match error {
            MediaHostError::Ring(error) => {
                error.reason().unwrap_or(RingRefuseReason::InvalidLayout)
            }
            _ => RingRefuseReason::InvalidLayout,
        };
        let _ = peer.send(Message::ring_refused(
            expect.kind,
            expect.generation,
            reason,
        ));
        let _ = peer.flush(CONTROL_WAIT);
    }
}

fn open_chain(
    resident: &dyn DspResident,
    spec: PcmSpecWire,
    stages: &[DspStageDesc],
) -> Result<Box<dyn ResidentChain>> {
    validate_spec(spec)?;
    resident.open(spec, stages)
}

#[allow(clippy::too_many_lines)]
fn pump<R: MappedRegion>(
    peer: &ControlPeer,
    chain: &mut dyn ResidentChain,
    input: &mut RingConsumer<R>,
    output: &mut RingProducer<R>,
    spec: PcmSpecWire,
) -> Result<PumpOut> {
    let channels = usize::from(spec.channels);
    let mut block = vec![0.0; BLOCK_FRAMES * channels];
    loop {
        if let Some(message) = peer.try_recv()? {
            match message {
                Message::DspReset => chain.reset(),
                message => {
                    if let Some(transition) = command(message)? {
                        return Ok(transition.into_pump());
                    }
                }
            }
        }
        // Infinite, lossless wait. The independent control monitor owns a
        // close-only view of both rings and wakes this worker on death,
        // shutdown or reconfigure.
        let frames = match input.read_into_blocking(&mut block, None) {
            Ok(frames) => frames,
            Err(RingError::Closed) => {
                peer.check()?;
                return Ok(PumpOut::InputClosed);
            }
            Err(error) => return Err(error.into()),
        };
        chain.process(&mut block[..frames * channels]);
        let mut offset = 0;
        while offset < frames {
            if let Some(message) = peer.try_recv()? {
                match message {
                    Message::DspReset => chain.reset(),
                    message => {
                        if let Some(transition) = command(message)? {
                            return Ok(transition.into_pump());
                        }
                    }
                }
            }
            match output.write_from_blocking(&block[offset * channels..frames * channels], None) {
                Ok(written) => offset += written,
                Err(RingError::Closed) => {
                    peer.check()?;
                    if let Some(transition) = command(wait_command(peer)?)? {
                        return Ok(transition.into_pump());
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

/// Serve one DSP chain until shutdown. Native DSP panics/crashes remain in
/// this helper process; errors become DspError, never player-process exit.
pub fn serve_dsp<F: PcmRings>(
    peer: &ControlPeer,
    rings: &F,
    resident: &dyn DspResident,
) -> Result<()> {
    let result = serve_inner(peer, rings, resident);
    if let Err(error) = &result {
        let _ = peer.send(Message::DspError {
            message: error.to_string(),
        });
        // Give the writer a bounded chance to publish the terminal diagnostic.
        let _ = peer.flush(CONTROL_WAIT);
    }
    result
}

#[allow(clippy::too_many_lines)]
fn serve_inner<F: PcmRings>(
    peer: &ControlPeer,
    rings: &F,
    resident: &dyn DspResident,
) -> Result<()> {
    let (mut spec, stages) = match wait_command(peer)? {
        Message::DspConfigure { spec, stages } => (spec, stages),
        Message::Shutdown => return Ok(()),
        message => {
            return Err(MediaHostError::InvalidMessage(format!(
                "expected DspConfigure: {message:?}"
            )));
        }
    };
    let mut chain = open_chain(resident, spec, &stages)?;
    let mut in_generation: Option<u64> = None;
    let mut back_generation: Option<u64> = None;
    'generation: loop {
        // 1. The player offers its in-ring; the host opens it and answers.
        let in_input = loop {
            match wait_command(peer)? {
                offer @ Message::RingOffer { .. } => {
                    match rings.open_directed(&offer, spec, RingDirection::MainToHost) {
                        Ok(input) => break input,
                        Err(error) => {
                            refuse(peer, &offer, &error);
                            return Err(error);
                        }
                    }
                }
                message => match command(message)? {
                    Some(Transition::Reconfigure { spec: next, stages }) => {
                        chain = open_chain(resident, next, &stages)?;
                        spec = next;
                    }
                    Some(Transition::Stop) => {
                        peer.flush(CONTROL_WAIT)?;
                        return Ok(());
                    }
                    None => {}
                },
            }
        };
        let generation = in_input.consumer.generation();
        if in_generation == Some(generation) {
            return Err(MediaHostError::OperationFailed(
                "ring factory reused a generation".into(),
            ));
        }
        in_generation = Some(generation);
        // 2. The host offers its back-ring; both aborts register before either
        // side is told its ring is live.
        let back_offer =
            rings.create_directed(spec, BACK_RING_FRAMES, RingDirection::HostToMain)?;
        let back_gen = back_offer.producer.generation();
        if back_generation == Some(back_gen) {
            return Err(MediaHostError::OperationFailed(
                "ring factory reused a generation".into(),
            ));
        }
        back_generation = Some(back_gen);
        let crate::pcm::PcmInput {
            consumer: in_consumer,
            abort: in_abort,
        } = in_input;
        let PcmOffer {
            producer: back_producer,
            message: back_message,
            abort: back_abort,
        } = back_offer;
        let mut input = ActiveConsumer(in_consumer);
        let mut back = ActiveRing(back_producer);
        peer.set_abort(Box::new(ClosePair(in_abort, back_abort)) as Box<dyn RingAbort>)?;
        peer.send(Message::RingReady {
            kind: RingKind::Pcm,
            generation,
        })?;
        peer.send(back_message)?;
        // 3. Fresh chain, fresh rings: report the latency before the first
        // block, exactly as `DspStage::latency_frames` is compensated.
        loop {
            match wait_command(peer)? {
                Message::RingReady {
                    kind: RingKind::Pcm,
                    generation: ready,
                } if ready == back_gen => break,
                Message::RingRefused { reason, .. } => {
                    return Err(MediaHostError::OperationFailed(format!(
                        "player refused back-ring: {reason:?}"
                    )));
                }
                message => match command(message)? {
                    Some(Transition::Reconfigure { spec: next, stages }) => {
                        chain = open_chain(resident, next, &stages)?;
                        spec = next;
                        continue 'generation;
                    }
                    Some(Transition::Stop) => {
                        peer.flush(CONTROL_WAIT)?;
                        return Ok(());
                    }
                    None => {}
                },
            }
        }
        peer.send(Message::DspReady {
            latency_frames: chain.latency_frames(),
        })?;
        // 4. Run until the input closes, a reconfigure lands, or we stop.
        // Every ring closes before replacement, including early errors via Drop.
        match pump(peer, chain.as_mut(), &mut input.0, &mut back.0, spec)? {
            PumpOut::Reconfigure { spec: next, stages } => {
                input.0.close();
                back.0.close();
                chain = open_chain(resident, next, &stages)?;
                spec = next;
            }
            PumpOut::Stop => {
                input.0.close();
                back.0.close();
                peer.flush(CONTROL_WAIT)?;
                return Ok(());
            }
            PumpOut::InputClosed => {
                // Graceful end-of-writes: every read frame was processed and
                // published. Close the back-ring so the player drains to a
                // sticky EOS, then wait for Shutdown or a fresh configure.
                back.0.close();
                input.0.close();
                match command(wait_command(peer)?)? {
                    Some(Transition::Reconfigure { spec: next, stages }) => {
                        chain = open_chain(resident, next, &stages)?;
                        spec = next;
                    }
                    Some(Transition::Stop) => {
                        peer.flush(CONTROL_WAIT)?;
                        return Ok(());
                    }
                    None => {}
                }
            }
        }
    }
}

/// Player-side DSP client. Owns the in-ring producer and the back-ring
/// consumer; the host owns the chain. Never use from a device callback.
pub struct DspSession<F: PcmRings> {
    peer: ControlPeer,
    rings: F,
    to_host: RingProducer<F::Region>,
    from_host: RingConsumer<F::Region>,
    spec: PcmSpecWire,
    latency_frames: u32,
    eos: bool,
    input_closed: bool,
    in_generation: u64,
    back_generation: u64,
}

impl<F: PcmRings> DspSession<F> {
    /// Configure a chain and complete the two-ring handshake. `capacity` sizes
    /// the player-created in-ring; the host back-ring uses a fixed capacity.
    pub fn open(
        peer: ControlPeer,
        rings: F,
        spec: PcmSpecWire,
        stages: &[DspStageDesc],
        capacity: u32,
    ) -> Result<Self> {
        validate_spec(spec)?;
        if capacity == 0 {
            return Err(MediaHostError::InvalidMessage(
                "zero DSP ring capacity".into(),
            ));
        }
        peer.send(Message::DspConfigure {
            spec,
            stages: stages.to_vec(),
        })?;
        let offer = rings.create_directed(spec, capacity, RingDirection::MainToHost)?;
        let (to_host, from_host, in_generation, back_generation, latency_frames) =
            handshake(&peer, &rings, offer, spec)?;
        Ok(Self {
            peer,
            rings,
            to_host,
            from_host,
            spec,
            latency_frames,
            eos: false,
            input_closed: false,
            in_generation,
            back_generation,
        })
    }

    /// Current stream specification.
    pub fn spec(&self) -> PcmSpecWire {
        self.spec
    }

    /// Chain latency in frames, as reported by `DspReady`.
    pub fn latency_frames(&self) -> u32 {
        self.latency_frames
    }

    /// Ring generations `(in-ring, back-ring)`; a reconfigure bumps both.
    pub fn generations(&self) -> (u64, u64) {
        (self.in_generation, self.back_generation)
    }

    /// Transport drop counter of the back-ring: must stay zero; the DSP path
    /// runs only under the lossless guarantee, never a drop policy.
    pub fn frames_dropped(&self) -> u64 {
        self.from_host.frames_dropped()
    }

    /// Push whole frames toward the host. Infinite lossless wait; a closed
    /// ring with a healthy control channel is a loud disconnect, never data
    /// loss. Misaligned or empty buffers are caller errors.
    pub fn write(&mut self, buf: &[f32]) -> Result<()> {
        self.peer.check()?;
        if self.input_closed {
            return Err(MediaHostError::InvalidMessage(
                "DSP input is finished; reconfigure for a new stream".into(),
            ));
        }
        if buf.is_empty() || !buf.len().is_multiple_of(usize::from(self.spec.channels)) {
            return Err(MediaHostError::InvalidMessage(
                "empty or non-frame-aligned DSP write".into(),
            ));
        }
        let channels = usize::from(self.spec.channels);
        let frames = buf.len() / channels;
        let mut offset = 0;
        while offset < frames {
            if let Some(message) = self.peer.try_recv()? {
                return Err(MediaHostError::InvalidMessage(format!(
                    "unexpected DSP message while writing: {message:?}"
                )));
            }
            match self
                .to_host
                .write_from_blocking(&buf[offset * channels..frames * channels], None)
            {
                Ok(written) => offset += written,
                Err(RingError::Closed) => {
                    self.peer.check()?;
                    return Err(MediaHostError::Disconnected(
                        "DSP host closed its input ring".into(),
                    ));
                }
                Err(error) => {
                    self.peer.fail(&error.to_string());
                    return Err(error.into());
                }
            }
        }
        Ok(())
    }

    /// Pull processed frames. `None` waits until data, a drained close, or a
    /// loud control failure; an explicit timeout returns `TimedOut`, never
    /// false EOS. EOS requires a drained closed back-ring and then sticks.
    pub fn read(&mut self, out: &mut [f32], timeout: Option<Duration>) -> Result<usize> {
        self.peer.check()?;
        if out.is_empty() || !out.len().is_multiple_of(usize::from(self.spec.channels)) {
            return Err(MediaHostError::InvalidMessage(
                "empty or non-frame-aligned DSP read".into(),
            ));
        }
        if self.eos {
            return Ok(0);
        }
        if let Some(message) = self.peer.try_recv()? {
            return Err(MediaHostError::InvalidMessage(format!(
                "unexpected DSP message while reading: {message:?}"
            )));
        }
        let deadline = timeout.map(|duration| Instant::now() + duration);
        match self.from_host.read_into_blocking(out, remaining(deadline)?) {
            Ok(0) => Err(MediaHostError::TimedOut),
            Ok(frames) => {
                self.peer.check()?;
                Ok(frames)
            }
            Err(RingError::Closed) => {
                // Drained close: a healthy control channel means the host
                // published everything and closed gracefully (sticky EOS); a
                // failed one means the host died (loud, never EOS).
                self.peer.check()?;
                self.eos = true;
                Ok(0)
            }
            Err(error) => {
                self.peer.fail(&error.to_string());
                Err(error.into())
            }
        }
    }

    /// Signal end-of-writes after the final block. Reading continues until the
    /// back-ring drains to a sticky EOS; the host stays alive for Shutdown or
    /// a reconfigure.
    pub fn finish_writes(&mut self) -> Result<()> {
        self.peer.check()?;
        self.to_host.close();
        self.input_closed = true;
        Ok(())
    }

    /// Clear stage state without touching the rings (seek / track change).
    pub fn reset(&mut self) -> Result<()> {
        self.peer.check()?;
        self.peer.send(Message::DspReset)
    }

    /// Replace the chain: close both rings, run a fresh two-ring handshake,
    /// and report the new latency. Old unread PCM is dead with the old
    /// generation, exactly like a seek discarding prefetch.
    pub fn reconfigure(
        &mut self,
        spec: PcmSpecWire,
        stages: &[DspStageDesc],
        capacity: u32,
    ) -> Result<()> {
        let result = self.reconfigure_inner(spec, stages, capacity);
        if let Err(error) = &result {
            self.peer.fail(&format!("DSP reconfigure failed: {error}"));
        }
        result
    }

    fn reconfigure_inner(
        &mut self,
        spec: PcmSpecWire,
        stages: &[DspStageDesc],
        capacity: u32,
    ) -> Result<()> {
        validate_spec(spec)?;
        if capacity == 0 {
            return Err(MediaHostError::InvalidMessage(
                "zero DSP ring capacity".into(),
            ));
        }
        self.peer.send(Message::DspConfigure {
            spec,
            stages: stages.to_vec(),
        })?;
        self.to_host.close();
        self.from_host.close();
        let offer = self
            .rings
            .create_directed(spec, capacity, RingDirection::MainToHost)?;
        if offer.producer.generation() == self.in_generation {
            return Err(MediaHostError::InvalidMessage(
                "replacement reused the old ring generation".into(),
            ));
        }
        let (to_host, from_host, in_generation, back_generation, latency_frames) =
            handshake(&self.peer, &self.rings, offer, spec)?;
        if back_generation == self.back_generation {
            return Err(MediaHostError::InvalidMessage(
                "replacement reused the old ring generation".into(),
            ));
        }
        self.to_host = to_host;
        self.from_host = from_host;
        self.spec = spec;
        self.latency_frames = latency_frames;
        self.in_generation = in_generation;
        self.back_generation = back_generation;
        self.eos = false;
        self.input_closed = false;
        Ok(())
    }
}

impl<F: PcmRings> Drop for DspSession<F> {
    fn drop(&mut self) {
        self.to_host.close();
        self.from_host.close();
        let _ = self.peer.send(Message::Shutdown);
    }
}

/// Rings and handshake results of one DSP generation: the in-ring producer,
/// the back-ring consumer, both generations, and the reported latency.
type LiveRings<F> = (RingProducer<F>, RingConsumer<F>, u64, u64, u32);
/// Offer the in-ring, open the back-ring, exchange Ready on both, and wait for
/// `DspReady`. Any refusal or unexpected message fails loudly; a resident
/// refusal arrives as a sticky control failure carrying the `DspError` text.
fn handshake<F: PcmRings>(
    peer: &ControlPeer,
    rings: &F,
    offer: PcmOffer<F::Region>,
    spec: PcmSpecWire,
) -> Result<LiveRings<F::Region>> {
    let in_generation = offer.producer.generation();
    let PcmOffer {
        producer: to_host,
        message: offer_message,
        abort: in_abort,
    } = offer;
    peer.send(offer_message)?;
    match peer.recv(CONTROL_WAIT)? {
        Message::RingReady {
            kind: RingKind::Pcm,
            generation,
        } if generation == in_generation => {}
        Message::RingRefused { reason, .. } => {
            return Err(MediaHostError::OperationFailed(format!(
                "DSP host refused the in-ring: {reason:?}"
            )));
        }
        message => {
            return Err(MediaHostError::InvalidMessage(format!(
                "expected in-ring Ready: {message:?}"
            )));
        }
    }
    let back = match peer.recv(CONTROL_WAIT)? {
        offer @ Message::RingOffer { .. } => attach_back(peer, rings, &offer, spec)?,
        message => {
            return Err(MediaHostError::InvalidMessage(format!(
                "expected back-ring offer: {message:?}"
            )));
        }
    };
    let back_generation = back.consumer.generation();
    // Register supervision for both rings before telling the host its ring is
    // live; from here a dead host wakes every blocked wait.
    peer.set_abort(Box::new(ClosePair(in_abort, back.abort)) as Box<dyn RingAbort>)?;
    peer.send(Message::RingReady {
        kind: RingKind::Pcm,
        generation: back_generation,
    })?;
    match peer.recv(CONTROL_WAIT)? {
        Message::DspReady { latency_frames } => Ok((
            to_host,
            back.consumer,
            in_generation,
            back_generation,
            latency_frames,
        )),
        message => Err(MediaHostError::InvalidMessage(format!(
            "expected DspReady: {message:?}"
        ))),
    }
}

fn attach_back<F: PcmRings>(
    peer: &ControlPeer,
    rings: &F,
    offer: &Message,
    spec: PcmSpecWire,
) -> Result<crate::pcm::PcmInput<F::Region>> {
    match rings.open_directed(offer, spec, RingDirection::HostToMain) {
        Ok(input) => Ok(input),
        Err(error) => {
            refuse(peer, offer, &error);
            Err(error)
        }
    }
}

fn remaining(deadline: Option<Instant>) -> Result<Option<Duration>> {
    deadline
        .map(|deadline| {
            deadline
                .checked_duration_since(Instant::now())
                .ok_or(MediaHostError::TimedOut)
        })
        .transpose()
}

/// Owned child plus worker session. Drop always kills and reaps the helper.
#[cfg(windows)]
pub struct DspProcess {
    session: Option<DspSession<crate::pcm::NamedPcmRings>>,
    child: std::process::Child,
}

#[cfg(windows)]
impl DspProcess {
    /// Spawn a separately licensed resident binary without a console/window.
    /// A missing binary is an ordinary unavailable-feature error.
    pub fn spawn(
        executable: &std::path::Path,
        spec: PcmSpecWire,
        stages: Vec<DspStageDesc>,
        capacity: u32,
        liveness: crate::control::Liveness,
    ) -> Result<Self> {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        let mut child = Command::new(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|error| {
                MediaHostError::OperationFailed(format!("DSP helper unavailable: {error}"))
            })?;
        let result = (|| {
            let peer = ControlPeer::start(
                child.stdout.take().unwrap(),
                child.stdin.take().unwrap(),
                liveness,
                false,
            )?;
            DspSession::open(peer, crate::pcm::NamedPcmRings, spec, &stages, capacity)
        })();
        match result {
            Ok(session) => Ok(Self {
                session: Some(session),
                child,
            }),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    /// DSP worker session; never use from the device callback.
    pub fn session(&mut self) -> &mut DspSession<crate::pcm::NamedPcmRings> {
        self.session.as_mut().unwrap()
    }

    /// OS child id (for diagnostics and VM crash-isolation tests).
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Terminate and reap a stalled/crashed helper; the next read fails loudly.
    pub fn terminate(&mut self) -> Result<()> {
        self.child
            .kill()
            .map_err(|error| MediaHostError::OperationFailed(error.to_string()))?;
        self.child
            .wait()
            .map_err(|error| MediaHostError::OperationFailed(error.to_string()))?;
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for DspProcess {
    fn drop(&mut self) {
        self.session.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
