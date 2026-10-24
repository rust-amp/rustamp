//! C4-2 (W14 wave review): a failure in one media-host session leaves the
//! other one in a defined state.
//!
//! The media host runs a **decoder** session and a **DSP** session side by
//! side: the decoder session is the source of the PCM, the DSP session is the
//! chain the frames run through. Both are independent state machines — each
//! owns its own control peer, its own rings and its own terminal state — so a
//! failure or a kill of one must not corrupt, stall or silently end the
//! other.
//!
//! The claims this file pins:
//!
//! 1. **A failed decoder leaves the DSP fully usable.** The DSP session still
//!    takes writes and delivers frames, its ring counters follow the loop's
//!    own traffic and nothing else (the decoder's failure contributes
//!    nothing to them), and it drops no frame.
//! 2. **A failed DSP leaves the decoder fully usable.** The mirror image:
//!    the decoder session still delivers frames, its counters move with its
//!    own traffic, and it drops no frame.
//! 3. **The host's control loop neither spins nor exits.** The loop that
//!    drives both sessions reports a failed session exactly once, asks it no
//!    further, and keeps working on the session that is still alive.
//! 4. **The failed session stays terminal.** The failure is sticky: every
//!    later call names the same cause, never a fresh success and never a
//!    false end of stream.
//!
//! Every fixture is in-process: a fake resident behind the production
//! `serve_decoder`/`serve_dsp` workers, or a fake helper on the raw wire
//! that dies the way a killed helper dies (control EOF, rings open). No
//! process is spawned, no window is opened and no audio device is touched.

#![cfg(windows)]

#[path = "support/frame_pattern.rs"]
mod frame_pattern;
#[path = "support/streams.rs"]
mod streams;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rustamp_ipc::Message;
use rustamp_ipc::shm::{RingDirection, RingKind};
use rustamp_ipc::{StreamTransport, Transport};
use rustamp_mediahost::control::{ControlPeer, Liveness};
use rustamp_mediahost::decoder::{
    DecoderInfo, DecoderResident, DecoderSession, ResidentDecoder, serve_decoder,
};
use rustamp_mediahost::dsp::{DspResident, DspSession, ResidentChain, serve_dsp};
use rustamp_mediahost::pcm::{NamedPcmRings, PcmRings};
use rustamp_mediahost::{MediaHostError, Result};
use rustamp_proto::host::{DspStageDesc, MediaLocationWire, PcmSpecWire};

/// The stereo spec both sessions use: one frame is two interleaved samples.
const SPEC: PcmSpecWire = PcmSpecWire {
    sample_rate_hz: 44_100,
    channels: 2,
};

/// The liveness policy of the in-process peers. A healthy peer answers
/// within milliseconds, so only a dead one reaches the bound — and a dead
/// peer is caught by the control EOF long before that.
const LIVENESS: Liveness = Liveness {
    interval: Duration::from_millis(100),
    timeout: Duration::from_secs(30),
};

/// The bound for a wait that must not block for ever. A healthy host answers
/// within milliseconds, so reaching the bound is a stall and fails loudly
/// with the step named — never a pass.
const STALL_WAIT: Duration = Duration::from_secs(30);

/// The bound of the control loop's own waits. The loop blocks in a live
/// session's wait instead of spinning, so its turn rate is bounded by this
/// slice.
const POLL: Duration = Duration::from_millis(20);

/// The decoder ring's capacity: a few frames, so the ring really turns over.
const DECODER_RING_FRAMES: u32 = 8;

/// The DSP in-ring's capacity.
const DSP_RING_FRAMES: u32 = 8;

/// The fake DSP helper's back-ring: it holds every frame the helper
/// publishes, so the helper never blocks on a full ring.
const HELPER_BACK_RING_FRAMES: u32 = 64;

/// The frames the control loop reads from the decoder in one turn.
const LOOP_BLOCK_FRAMES: usize = 64;

/// The turns a wait may take. A healthy session answers within milliseconds,
/// so reaching the bound is a stall and fails loudly.
const MAX_TURNS: usize = 500;

/// The window the "does not spin" measurement looks at.
const SPIN_WINDOW: Duration = Duration::from_millis(250);

/// The turns the control loop may do in that window. A loop that blocks in a
/// session's own wait turns a few times per poll slice; a loop that spins
/// turns thousands of times, so the bound catches it with a wide margin.
const MAX_SPIN_TURNS: usize = 100;

// ---------------------------------------------------------------------------
// The fixtures
// ---------------------------------------------------------------------------

/// A decoder resident that publishes the shared frame pattern and fails as
/// soon as the test releases it.
///
/// The failure is the resident's own error, so the production worker turns it
/// into the terminal `DecoderError` diagnostic the player side reads.
struct PatternResident {
    frames: usize,
    fail: Arc<AtomicBool>,
}

struct PatternDecoder {
    frames: usize,
    fail: Arc<AtomicBool>,
    next: usize,
}

impl DecoderResident for PatternResident {
    fn open(&self, _location: MediaLocationWire) -> Result<Box<dyn ResidentDecoder>> {
        Ok(Box::new(PatternDecoder {
            frames: self.frames,
            fail: self.fail.clone(),
            next: 0,
        }))
    }
}

impl ResidentDecoder for PatternDecoder {
    fn info(&self) -> DecoderInfo {
        DecoderInfo {
            spec: SPEC,
            duration_ms: None,
            seekable: false,
            metadata: Default::default(),
        }
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(MediaHostError::OperationFailed(
                "the fixture decoder failed".into(),
            ));
        }
        let channels = usize::from(SPEC.channels);
        let count = (self.frames - self.next).min(out.len() / channels);
        for (index, frame) in out[..count * channels]
            .chunks_exact_mut(channels)
            .enumerate()
        {
            frame.copy_from_slice(&frame_pattern::frame_samples(self.next + index));
        }
        self.next += count;
        Ok(count)
    }

    fn seek(&mut self, _to_ms: u64) -> Result<u64> {
        Err(MediaHostError::OperationFailed(
            "the fixture decoder is not seekable".into(),
        ))
    }
}

/// A DSP chain that passes every block through unchanged and fails as soon
/// as the test releases it.
///
/// The failure is the chain's own error, so the production worker turns it
/// into the terminal `DspError` diagnostic the player side reads.
struct PassthroughResident {
    fail: Arc<AtomicBool>,
}

struct PassthroughChain {
    fail: Arc<AtomicBool>,
}

impl DspResident for PassthroughResident {
    fn open(&self, _spec: PcmSpecWire, _stages: &[DspStageDesc]) -> Result<Box<dyn ResidentChain>> {
        Ok(Box::new(PassthroughChain {
            fail: self.fail.clone(),
        }))
    }
}

impl ResidentChain for PassthroughChain {
    fn latency_frames(&self) -> u32 {
        0
    }

    fn process(&mut self, _buf: &mut [f32]) -> Result<()> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(MediaHostError::OperationFailed(
                "the fixture chain failed".into(),
            ));
        }
        Ok(())
    }

    fn reset(&mut self) {}
}

// ---------------------------------------------------------------------------
// The sessions
// ---------------------------------------------------------------------------

/// The decoder session with an in-process host that serves `resident`.
///
/// The worker handle is the host's outcome: `is_finished` tells "the host is
/// still serving" from "the host is gone", without waiting for it.
fn start_decoder(
    resident: impl DecoderResident + Send + 'static,
) -> (DecoderSession<NamedPcmRings>, thread::JoinHandle<()>) {
    let ((ar, aw), (br, bw)) = streams::streams();
    let player = ControlPeer::start(ar, aw, LIVENESS, false).expect("the player peer starts");
    let host = ControlPeer::start(br, bw, LIVENESS, true).expect("the host peer starts");
    let worker = thread::spawn(move || {
        let _ = serve_decoder(&host, &NamedPcmRings, &resident);
    });
    let session = DecoderSession::open(
        player,
        NamedPcmRings,
        MediaLocationWire::Url("fixture:pattern".into()),
        DECODER_RING_FRAMES,
    )
    .expect("the decoder handshake completes");
    (session, worker)
}

/// The DSP session with an in-process host that serves `resident`.
fn start_dsp(
    resident: impl DspResident + Send + 'static,
) -> (DspSession<NamedPcmRings>, thread::JoinHandle<()>) {
    let ((ar, aw), (br, bw)) = streams::streams();
    let player = ControlPeer::start(ar, aw, LIVENESS, false).expect("the player peer starts");
    let host = ControlPeer::start(br, bw, LIVENESS, true).expect("the host peer starts");
    let worker = thread::spawn(move || {
        let _ = serve_dsp(&host, &NamedPcmRings, &resident);
    });
    let session = DspSession::open(player, NamedPcmRings, SPEC, &[], DSP_RING_FRAMES)
        .expect("the handshake completes");
    (session, worker)
}

// ---------------------------------------------------------------------------
// The killed helpers
//
// A killed helper leaves the control channel ended and its rings open: the
// crash detection is the control channel's job (RFC 0006 §8). The helpers
// below speak the raw wire on purpose — a `ControlPeer` would close the
// rings when it is dropped, which is not what a kill leaves behind.
// ---------------------------------------------------------------------------

/// A fake decoder helper that publishes `frames` frames and then dies
/// without a terminal message.
fn dying_decoder_helper(
    reader: streams::ByteReader,
    writer: streams::ByteWriter,
    frames: usize,
    killed: mpsc::Receiver<()>,
) {
    let mut transport = StreamTransport::new(reader, writer);
    transport
        .send(&Message::hello())
        .expect("the helper says hello");
    let prefetch = loop {
        match transport.recv().expect("the player speaks") {
            Message::Hello { .. } => {}
            Message::Ping(token) => transport
                .send(&Message::Pong(token))
                .expect("the helper answers the ping"),
            Message::DecoderOpen {
                prefetch_frames, ..
            } => break prefetch_frames,
            message => panic!("unexpected message: {message:?}"),
        }
    };
    transport
        .send(&Message::DecoderOpened {
            spec: SPEC,
            duration_ms: None,
            seekable: false,
            metadata: Default::default(),
        })
        .expect("the helper opens the stream");
    let mut ring = NamedPcmRings
        .create(SPEC, prefetch)
        .expect("the helper creates its ring");
    let generation = ring.producer.generation();
    transport
        .send(&ring.message)
        .expect("the helper offers its ring");
    loop {
        match transport.recv().expect("the player speaks") {
            Message::Ping(token) => transport
                .send(&Message::Pong(token))
                .expect("the helper answers the ping"),
            Message::RingReady {
                kind: RingKind::Pcm,
                generation: ready,
            } if ready == generation => break,
            message => panic!("unexpected message: {message:?}"),
        }
    }
    for index in 0..frames {
        ring.producer
            .write_from_blocking(&frame_pattern::frame_samples(index), Some(STALL_WAIT))
            .expect("the helper publishes its frames");
    }
    killed
        .recv_timeout(STALL_WAIT)
        .expect("stall: the test never killed the helper");
    // The helper dies here: the control channel ends, the ring stays open.
    drop(transport);
    drop(ring);
}

/// The decoder session whose helper dies on demand.
fn start_dying_decoder(
    frames: usize,
) -> (
    DecoderSession<NamedPcmRings>,
    mpsc::Sender<()>,
    thread::JoinHandle<()>,
) {
    let ((ar, aw), (br, bw)) = streams::streams();
    let player = ControlPeer::start(ar, aw, LIVENESS, false).expect("the player peer starts");
    let (kill, killed) = mpsc::channel();
    let worker = thread::spawn(move || dying_decoder_helper(br, bw, frames, killed));
    let session = DecoderSession::open(
        player,
        NamedPcmRings,
        MediaLocationWire::Url("fixture:dying".into()),
        DECODER_RING_FRAMES,
    )
    .expect("the decoder handshake completes");
    (session, kill, worker)
}

/// A fake DSP helper that completes the two-ring handshake, echoes every
/// frame it takes and then dies without a terminal message.
fn dying_dsp_helper(
    reader: streams::ByteReader,
    writer: streams::ByteWriter,
    killed: mpsc::Receiver<()>,
) {
    let mut transport = StreamTransport::new(reader, writer);
    transport
        .send(&Message::hello())
        .expect("the helper says hello");
    let spec = loop {
        match transport.recv().expect("the player speaks") {
            Message::Hello { .. } => {}
            Message::Ping(token) => transport
                .send(&Message::Pong(token))
                .expect("the helper answers the ping"),
            Message::DspConfigure { spec, .. } => break spec,
            message => panic!("unexpected message: {message:?}"),
        }
    };
    let offer = loop {
        match transport.recv().expect("the player speaks") {
            Message::Ping(token) => transport
                .send(&Message::Pong(token))
                .expect("the helper answers the ping"),
            offer @ Message::RingOffer { .. } => break offer,
            message => panic!("unexpected message: {message:?}"),
        }
    };
    let mut input = NamedPcmRings
        .open_directed(&offer, spec, RingDirection::MainToHost)
        .expect("the helper opens the in-ring");
    transport
        .send(&Message::RingReady {
            kind: RingKind::Pcm,
            generation: input.consumer.generation(),
        })
        .expect("the helper readies the in-ring");
    let mut back = NamedPcmRings
        .create_directed(spec, HELPER_BACK_RING_FRAMES, RingDirection::HostToMain)
        .expect("the helper creates the back-ring");
    let back_generation = back.producer.generation();
    transport
        .send(&back.message)
        .expect("the helper offers the back-ring");
    loop {
        match transport.recv().expect("the player speaks") {
            Message::Ping(token) => transport
                .send(&Message::Pong(token))
                .expect("the helper answers the ping"),
            Message::RingReady {
                kind: RingKind::Pcm,
                generation,
            } if generation == back_generation => break,
            message => panic!("unexpected message: {message:?}"),
        }
    }
    transport
        .send(&Message::DspReady { latency_frames: 0 })
        .expect("the helper is ready");
    // The transfer: every frame the player writes comes back unchanged.
    let mut buf = [0.0f32; 2];
    loop {
        match killed.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        match input.consumer.read_into_blocking(&mut buf, Some(POLL)) {
            Ok(0) => continue,
            Ok(count) => {
                back.producer
                    .write_from_blocking(&buf[..count * 2], Some(STALL_WAIT))
                    .expect("the helper publishes its frames");
            }
            Err(error) => panic!("the helper's in-ring failed: {error}"),
        }
    }
    // The helper dies here: the control channel ends, the rings stay open.
    drop(transport);
    drop(input);
    drop(back);
}

/// The DSP session whose helper dies on demand.
fn start_dying_dsp() -> (
    DspSession<NamedPcmRings>,
    mpsc::Sender<()>,
    thread::JoinHandle<()>,
) {
    let ((ar, aw), (br, bw)) = streams::streams();
    let player = ControlPeer::start(ar, aw, LIVENESS, false).expect("the player peer starts");
    let (kill, killed) = mpsc::channel();
    let worker = thread::spawn(move || dying_dsp_helper(br, bw, killed));
    let session = DspSession::open(player, NamedPcmRings, SPEC, &[], DSP_RING_FRAMES)
        .expect("the handshake completes");
    (session, kill, worker)
}

// ---------------------------------------------------------------------------
// The host's control loop
// ---------------------------------------------------------------------------

/// The failure the control loop reported.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Report {
    /// The decoder session failed; the reason is its terminal diagnosis.
    Decoder(String),
    /// The DSP session failed; the reason is its terminal diagnosis.
    Dsp(String),
}

/// The host's control loop: the player side that drives the decoder and the
/// DSP session side by side, the way the engine's decode worker drives them.
///
/// A turn pulls a block from the decoder, pushes it through the chain and
/// reads the result back. A session that failed is asked no more: the loop
/// reports the failure once, leaves the session alone and keeps working on
/// the session that is still alive. Every turn ends in a bounded wait on a
/// live session, so the loop cannot spin.
struct ControlLoop {
    decoder: DecoderSession<NamedPcmRings>,
    dsp: DspSession<NamedPcmRings>,
    /// The failures the loop reported, in order.
    reports: Vec<Report>,
    /// The decoder's terminal state, once it failed.
    decoder_failure: Option<String>,
    /// Whether the decoder reached the end of its stream.
    decoder_done: bool,
    /// The DSP's terminal state, once it failed.
    dsp_failure: Option<String>,
    /// How often the loop asked the decoder.
    decoder_asks: usize,
    /// The asks the decoder had seen when it failed.
    decoder_asks_at_failure: usize,
    /// How often the loop asked the DSP.
    dsp_asks: usize,
    /// The asks the DSP had seen when it failed.
    dsp_asks_at_failure: usize,
    /// The frames the loop pulled from the decoder.
    decoded: u64,
    /// The frames the loop pushed into the chain.
    pushed: u64,
    /// The frames the loop read back from the chain.
    read_back: u64,
    /// The frames the loop pulled but has not pushed yet.
    pending: Vec<f32>,
    /// The decoder's block buffer.
    block: Vec<f32>,
}

impl ControlLoop {
    /// The loop over the two sessions.
    fn new(decoder: DecoderSession<NamedPcmRings>, dsp: DspSession<NamedPcmRings>) -> Self {
        let channels = usize::from(SPEC.channels);
        Self {
            decoder,
            dsp,
            reports: Vec::new(),
            decoder_failure: None,
            decoder_done: false,
            dsp_failure: None,
            decoder_asks: 0,
            decoder_asks_at_failure: 0,
            dsp_asks: 0,
            dsp_asks_at_failure: 0,
            decoded: 0,
            pushed: 0,
            read_back: 0,
            pending: Vec::new(),
            block: vec![0.0; LOOP_BLOCK_FRAMES * channels],
        }
    }

    /// The decoder failed: the loop reports it once and leaves the session
    /// alone.
    fn fail_decoder(&mut self, error: MediaHostError) {
        let reason = error.to_string();
        self.decoder_asks_at_failure = self.decoder_asks;
        self.decoder_failure = Some(reason.clone());
        self.reports.push(Report::Decoder(reason));
    }

    /// The DSP failed: the loop reports it once and leaves the session alone.
    fn fail_dsp(&mut self, error: MediaHostError) {
        let reason = error.to_string();
        self.dsp_asks_at_failure = self.dsp_asks;
        self.dsp_failure = Some(reason.clone());
        self.reports.push(Report::Dsp(reason));
    }

    /// One turn of the loop.
    ///
    /// The turn pulls a block from the decoder, pushes it through the chain
    /// and reads the result back. Returns `false` when no session can make
    /// progress any more.
    fn turn(&mut self) -> bool {
        let channels = usize::from(SPEC.channels);
        // 1. The decoder: the source of the frames.
        if self.decoder_failure.is_none() {
            self.decoder_asks += 1;
            match self.decoder.read(&mut self.block, Some(POLL)) {
                Ok(0) => self.decoder_done = true,
                Ok(frames) => {
                    self.pending.clear();
                    self.pending
                        .extend_from_slice(&self.block[..frames * channels]);
                    self.decoded += frames as u64;
                }
                Err(MediaHostError::TimedOut) => {}
                Err(error) => self.fail_decoder(error),
            }
        }
        // 2. The chain: the frames the loop pulled go through it.
        if self.dsp_failure.is_none() {
            if !self.pending.is_empty() {
                self.dsp_asks += 1;
                match self.dsp.write_timeout(&self.pending, Some(STALL_WAIT)) {
                    Ok(()) => self.pushed += (self.pending.len() / channels) as u64,
                    Err(error) => self.fail_dsp(error),
                }
                self.pending.clear();
            }
            if self.dsp_failure.is_none() {
                // 3. The result: the chain returns the frames it was given,
                //    in order. The read is bounded, so a turn that waits for
                //    nothing is paced instead of spinning.
                let outstanding = (self.pushed - self.read_back) as usize;
                let mut out = vec![0.0f32; outstanding.max(1) * channels];
                let bound = if outstanding > 0 { STALL_WAIT } else { POLL };
                self.dsp_asks += 1;
                match self.dsp.read(&mut out, Some(bound)) {
                    Ok(0) => {}
                    Ok(frames) => {
                        for (index, frame) in
                            out[..frames * channels].chunks_exact(channels).enumerate()
                        {
                            assert_eq!(
                                frame,
                                frame_pattern::frame_samples(self.read_back as usize + index),
                                "the chain must return the frames it was given, in order"
                            );
                        }
                        self.read_back += frames as u64;
                    }
                    Err(MediaHostError::TimedOut) => {}
                    Err(error) => self.fail_dsp(error),
                }
            }
        } else {
            // The chain is gone: the frames have nowhere to go.
            self.pending.clear();
        }
        !self.done()
    }

    /// The loop is done when no session can make progress any more: the
    /// decoder failed or reached the end of its stream, and the chain
    /// failed.
    fn done(&self) -> bool {
        let decoder_live = self.decoder_failure.is_none() && !self.decoder_done;
        let chain_live = self.dsp_failure.is_none();
        !decoder_live && !chain_live
    }

    /// Turn the loop until `done` holds.
    fn run_until(&mut self, done: impl Fn(&Self) -> bool, step: &str) {
        for _ in 0..MAX_TURNS {
            if done(self) {
                return;
            }
            if !self.turn() {
                panic!("stall: {step} did not happen; the loop has nothing left to do");
            }
        }
        panic!("stall: {step} did not happen");
    }

    /// Turn the loop until the chain returned every frame the loop pushed.
    ///
    /// The chain's counters are settled then, which is what a baseline needs:
    /// a counter that is still moving would hide a change.
    fn settle(&mut self) {
        self.run_until(
            |host| host.pushed > 0 && host.read_back == host.pushed,
            "the chain's first round trip",
        );
    }

    /// The chain's counters must follow the loop's own traffic and nothing
    /// else.
    ///
    /// The player wrote what it pushed, the chain took every frame (the
    /// delivery fence), the chain published what the player read back, and
    /// neither ring is closed. A failure of the *other* session contributes
    /// nothing to any of the four numbers — that is what "the failure leaves
    /// the other session alone" means in numbers.
    fn assert_chain_counters_follow_traffic(&self, step: &str) {
        let in_counters = self.dsp.in_ring_counters();
        let back_counters = self.dsp.back_ring_counters();
        assert_eq!(
            in_counters.published, self.pushed,
            "{step}: the in-ring counts the frames the loop wrote"
        );
        assert_eq!(
            in_counters.read, self.pushed,
            "{step}: the chain took every frame the loop wrote"
        );
        assert!(
            back_counters.published >= self.read_back,
            "{step}: the chain published at least the frames the loop read"
        );
        assert_eq!(
            back_counters.read, self.read_back,
            "{step}: the loop read exactly the frames it read"
        );
        assert!(
            !in_counters.closed,
            "{step}: the chain's in-ring stays open"
        );
        assert!(
            !back_counters.closed,
            "{step}: the chain's back-ring stays open"
        );
    }

    /// The decoder's counters must follow the loop's own traffic and nothing
    /// else.
    fn assert_decoder_counters_follow_traffic(&self, step: &str) {
        let counters = self.decoder.ring_counters();
        assert!(
            counters.published >= self.decoded,
            "{step}: the decoder published at least the frames the loop read"
        );
        assert_eq!(
            counters.read, self.decoded,
            "{step}: the loop read exactly the frames it read"
        );
        assert!(!counters.closed, "{step}: the decoder's ring stays open");
    }

    /// Turn the loop for `window` and count the turns.
    fn turns_in(&mut self, window: Duration) -> usize {
        let start = Instant::now();
        let mut turns = 0usize;
        while start.elapsed() < window {
            if !self.turn() {
                break;
            }
            turns += 1;
        }
        turns
    }
}

// ---------------------------------------------------------------------------
// The claims
// ---------------------------------------------------------------------------

/// A failed decoder leaves the DSP session fully usable.
///
/// The DSP session still takes writes and delivers frames, its ring counters
/// follow the loop's own traffic and nothing else — the decoder's failure
/// contributes nothing to them — and it drops no frame. The failed decoder
/// session stays terminal: every later call names the same cause.
#[test]
fn a_failed_decoder_session_leaves_the_dsp_session_fully_usable() {
    let fail = Arc::new(AtomicBool::new(false));
    let (decoder, decoder_worker) = start_decoder(PatternResident {
        frames: 4096,
        fail: fail.clone(),
    });
    let (dsp, dsp_worker) = start_dsp(PassthroughResident {
        fail: Arc::new(AtomicBool::new(false)),
    });
    let mut host = ControlLoop::new(decoder, dsp);

    // The loop works and the chain returns what it was given.
    host.settle();
    assert!(
        host.decoder_failure.is_none(),
        "the decoder must not fail yet"
    );
    assert!(host.dsp_failure.is_none(), "the chain must not fail");
    assert!(
        host.pushed > 0,
        "the loop must push frames through the chain"
    );
    assert_eq!(
        host.read_back, host.pushed,
        "the chain must return the frames it was given"
    );

    // The decoder fails now.
    fail.store(true, Ordering::SeqCst);
    host.run_until(|host| host.decoder_failure.is_some(), "the decoder failure");

    // The loop reports the failure exactly once and asks the failed session
    // no more.
    assert_eq!(
        host.reports.len(),
        1,
        "the loop must report the failed session once"
    );
    match &host.reports[0] {
        Report::Decoder(reason) => assert!(
            reason.contains("resident decoder failed"),
            "the report must name the decoder's own diagnosis: {reason}"
        ),
        Report::Dsp(_) => panic!("the loop must not report the chain"),
    }
    assert_eq!(
        host.decoder_asks, host.decoder_asks_at_failure,
        "the loop must not ask the failed session again"
    );
    assert!(
        host.reports
            .iter()
            .all(|report| !matches!(report, Report::Dsp(_))),
        "the chain must not be reported as failed"
    );

    // The failed session stays terminal: the same cause, never a fresh
    // success and never an end of stream.
    let error = host
        .decoder
        .read(&mut [0.0; 2], Some(POLL))
        .expect_err("the failed decoder must stay failed");
    assert!(
        error.to_string().contains("resident decoder failed"),
        "the failure must name the decoder's own diagnosis: {error}"
    );
    let again = host
        .decoder
        .read(&mut [0.0; 2], Some(POLL))
        .expect_err("the failed decoder must stay failed");
    assert_eq!(
        again.to_string(),
        error.to_string(),
        "the failure must stick"
    );

    // The chain's counters follow the loop's own traffic and nothing else:
    // the decoder's failure reached nothing of the DSP session.
    host.assert_chain_counters_follow_traffic("the decoder's failure");
    assert_eq!(
        host.dsp.frames_dropped(),
        0,
        "the decoder's failure must not drop a frame of the chain"
    );

    // The chain is fully usable: the write is delivered and comes back.
    let in_counters = host.dsp.in_ring_counters();
    let back_counters = host.dsp.back_ring_counters();
    let frame = frame_pattern::frame_samples(4095);
    host.dsp
        .write_timeout(&frame, Some(STALL_WAIT))
        .expect("the chain must still take writes");
    let mut out = [0.0f32; 2];
    assert_eq!(
        host.dsp
            .read(&mut out, Some(STALL_WAIT))
            .expect("the chain must still deliver frames"),
        1
    );
    assert_eq!(out, frame, "the frame must survive the round trip");
    assert_eq!(host.dsp.frames_dropped(), 0, "no frame may be dropped");
    assert_eq!(
        host.dsp.in_ring_counters().published,
        in_counters.published + 1,
        "the chain's in-ring counts the frame the loop wrote"
    );
    assert_eq!(
        host.dsp.back_ring_counters().published,
        back_counters.published + 1,
        "the chain's back-ring counts the frame the chain returned"
    );
    assert!(!host.dsp.in_ring_counters().closed);
    assert!(!host.dsp.back_ring_counters().closed);

    // The chain's host is still serving: it did not end with the decoder.
    assert!(
        !dsp_worker.is_finished(),
        "the chain's host must still be serving"
    );
    // The decoder's host ended with its failure: it does not spin either.
    decoder_worker
        .join()
        .expect("the decoder's host must not panic");

    drop(host);
    dsp_worker.join().expect("the chain's host must not panic");
}

/// A killed decoder leaves the DSP session fully usable.
///
/// The kill is the crash surrogate: the helper's control channel ends
/// without a terminal message and its ring stays open. The DSP session is
/// untouched by it and stays fully usable.
#[test]
fn a_killed_decoder_session_leaves_the_dsp_session_fully_usable() {
    let (decoder, kill, decoder_worker) = start_dying_decoder(4);
    let (dsp, dsp_worker) = start_dsp(PassthroughResident {
        fail: Arc::new(AtomicBool::new(false)),
    });
    let mut host = ControlLoop::new(decoder, dsp);

    // The loop works and the chain returns what it was given.
    host.settle();
    assert!(
        host.decoder_failure.is_none(),
        "the decoder must be alive yet"
    );
    assert!(host.dsp_failure.is_none(), "the chain must not fail");
    assert!(
        host.pushed > 0,
        "the loop must push frames through the chain"
    );
    assert_eq!(
        host.read_back, host.pushed,
        "the chain must return the frames it was given"
    );

    // The helper dies: the control channel ends, the ring stays open.
    kill.send(()).expect("the helper must take the kill");
    host.run_until(|host| host.decoder_failure.is_some(), "the decoder's death");

    // The loop reports the death exactly once and asks the dead session no
    // more.
    assert_eq!(
        host.reports.len(),
        1,
        "the loop must report the failed session once"
    );
    match &host.reports[0] {
        Report::Decoder(reason) => assert!(
            reason.contains("control stream ended"),
            "the report must name the control channel: {reason}"
        ),
        Report::Dsp(_) => panic!("the loop must not report the chain"),
    }
    assert_eq!(
        host.decoder_asks, host.decoder_asks_at_failure,
        "the loop must not ask the failed session again"
    );

    // The dead session stays terminal.
    let error = host
        .decoder
        .read(&mut [0.0; 2], Some(POLL))
        .expect_err("the dead decoder must stay failed");
    assert!(
        error.to_string().contains("control stream ended"),
        "the death must be seen on the control channel: {error}"
    );

    // The chain's counters follow the loop's own traffic and nothing else:
    // the decoder's death reached nothing of the DSP session.
    host.assert_chain_counters_follow_traffic("the decoder's death");
    assert_eq!(
        host.dsp.frames_dropped(),
        0,
        "the decoder's death must not drop a frame of the chain"
    );

    // The chain is fully usable: the write is delivered and comes back.
    let frame = frame_pattern::frame_samples(4095);
    host.dsp
        .write_timeout(&frame, Some(STALL_WAIT))
        .expect("the chain must still take writes");
    let mut out = [0.0f32; 2];
    assert_eq!(
        host.dsp
            .read(&mut out, Some(STALL_WAIT))
            .expect("the chain must still deliver frames"),
        1
    );
    assert_eq!(out, frame, "the frame must survive the round trip");
    assert_eq!(host.dsp.frames_dropped(), 0, "no frame may be dropped");
    assert!(!host.dsp.in_ring_counters().closed);
    assert!(!host.dsp.back_ring_counters().closed);

    // The chain's host is still serving.
    assert!(
        !dsp_worker.is_finished(),
        "the chain's host must still be serving"
    );
    decoder_worker
        .join()
        .expect("the killed helper must not panic");

    drop(host);
    dsp_worker.join().expect("the chain's host must not panic");
}

/// A failed DSP session leaves the decoder session fully usable.
///
/// The mirror image of the decoder case: the chain's failure is terminal and
/// sticky, and the decoder session keeps delivering frames with counters
/// that move with its own traffic only.
#[test]
fn a_failed_dsp_session_leaves_the_decoder_session_fully_usable() {
    let fail = Arc::new(AtomicBool::new(false));
    let (decoder, decoder_worker) = start_decoder(PatternResident {
        frames: 4096,
        fail: Arc::new(AtomicBool::new(false)),
    });
    let (dsp, dsp_worker) = start_dsp(PassthroughResident { fail: fail.clone() });
    let mut host = ControlLoop::new(decoder, dsp);

    // The loop works and the chain returns what it was given.
    host.settle();
    assert!(host.dsp_failure.is_none(), "the chain must not fail yet");
    assert!(host.decoder_failure.is_none(), "the decoder must not fail");
    assert!(
        host.pushed > 0,
        "the loop must push frames through the chain"
    );
    assert_eq!(
        host.read_back, host.pushed,
        "the chain must return the frames it was given"
    );

    // The chain fails now.
    fail.store(true, Ordering::SeqCst);
    host.run_until(|host| host.dsp_failure.is_some(), "the chain failure");

    // The loop reports the failure exactly once and asks the failed session
    // no more.
    assert_eq!(
        host.reports.len(),
        1,
        "the loop must report the failed session once"
    );
    match &host.reports[0] {
        Report::Dsp(reason) => assert!(
            reason.contains("resident DSP failed"),
            "the report must name the chain's own diagnosis: {reason}"
        ),
        Report::Decoder(_) => panic!("the loop must not report the decoder"),
    }
    assert_eq!(
        host.dsp_asks, host.dsp_asks_at_failure,
        "the loop must not ask the failed session again"
    );

    // The failed session stays terminal.
    let mut out = [0.0f32; 2];
    let error = host
        .dsp
        .read(&mut out, Some(POLL))
        .expect_err("the failed chain must stay failed");
    assert!(
        error.to_string().contains("resident DSP failed"),
        "the failure must name the chain's own diagnosis: {error}"
    );
    let again = host
        .dsp
        .read(&mut out, Some(POLL))
        .expect_err("the failed chain must stay failed");
    assert_eq!(
        again.to_string(),
        error.to_string(),
        "the failure must stick"
    );

    // The decoder is untouched by the chain's failure: its counters move
    // with its own traffic only, and it drops no frame.
    assert_eq!(
        host.decoder.frames_dropped(),
        0,
        "the chain's failure must not drop a frame of the decoder"
    );

    // The decoder is fully usable: it still delivers its frames.
    let mut read = 0usize;
    let mut block = [0.0f32; 8];
    while read < 4 {
        let frames = host
            .decoder
            .read(&mut block, Some(STALL_WAIT))
            .expect("the decoder must still deliver frames");
        assert!(frames > 0, "the decoder must deliver frames");
        for (index, frame) in block[..frames * 2].as_chunks::<2>().0.iter().enumerate() {
            let expected = frame_pattern::frame_samples(host.decoded as usize + index);
            assert_eq!(
                frame.as_slice(),
                expected.as_slice(),
                "the frame must be whole"
            );
        }
        host.decoded += frames as u64;
        read += frames;
    }
    host.assert_decoder_counters_follow_traffic("the chain's failure");

    // The decoder's host is still serving: it did not end with the chain.
    assert!(
        !decoder_worker.is_finished(),
        "the decoder's host must still be serving"
    );
    dsp_worker.join().expect("the chain's host must not panic");

    drop(host);
    decoder_worker
        .join()
        .expect("the decoder's host must not panic");
}

/// A killed DSP session leaves the decoder session fully usable.
///
/// The kill is the crash surrogate: the helper's control channel ends
/// without a terminal message and its rings stay open. The decoder session
/// is untouched by it and stays fully usable.
#[test]
fn a_killed_dsp_session_leaves_the_decoder_session_fully_usable() {
    let (decoder, decoder_worker) = start_decoder(PatternResident {
        frames: 4096,
        fail: Arc::new(AtomicBool::new(false)),
    });
    let (dsp, kill, dsp_worker) = start_dying_dsp();
    let mut host = ControlLoop::new(decoder, dsp);

    // The loop works and the chain returns what it was given.
    host.settle();
    assert!(host.dsp_failure.is_none(), "the chain must be alive yet");
    assert!(host.decoder_failure.is_none(), "the decoder must not fail");
    assert!(
        host.pushed > 0,
        "the loop must push frames through the chain"
    );
    assert_eq!(
        host.read_back, host.pushed,
        "the chain must return the frames it was given"
    );

    // The helper dies: the control channel ends, the rings stay open.
    kill.send(()).expect("the helper must take the kill");
    host.run_until(|host| host.dsp_failure.is_some(), "the chain's death");

    // The loop reports the death exactly once and asks the dead session no
    // more.
    assert_eq!(
        host.reports.len(),
        1,
        "the loop must report the failed session once"
    );
    match &host.reports[0] {
        Report::Dsp(reason) => assert!(
            reason.contains("control stream ended"),
            "the report must name the control channel: {reason}"
        ),
        Report::Decoder(_) => panic!("the loop must not report the decoder"),
    }
    assert_eq!(
        host.dsp_asks, host.dsp_asks_at_failure,
        "the loop must not ask the failed session again"
    );

    // The dead session stays terminal.
    let mut out = [0.0f32; 2];
    let error = host
        .dsp
        .read(&mut out, Some(POLL))
        .expect_err("the dead chain must stay failed");
    assert!(
        error.to_string().contains("control stream ended"),
        "the death must be seen on the control channel: {error}"
    );

    // The decoder is untouched by the kill and still usable: it delivers its
    // frames and drops none.
    assert_eq!(
        host.decoder.frames_dropped(),
        0,
        "the chain's death must not drop a frame of the decoder"
    );
    let mut read = 0usize;
    let mut block = [0.0f32; 8];
    while read < 4 {
        let frames = host
            .decoder
            .read(&mut block, Some(STALL_WAIT))
            .expect("the decoder must still deliver frames");
        assert!(frames > 0, "the decoder must deliver frames");
        for (index, frame) in block[..frames * 2].as_chunks::<2>().0.iter().enumerate() {
            let expected = frame_pattern::frame_samples(host.decoded as usize + index);
            assert_eq!(
                frame.as_slice(),
                expected.as_slice(),
                "the frame must be whole"
            );
        }
        host.decoded += frames as u64;
        read += frames;
    }
    host.assert_decoder_counters_follow_traffic("the chain's death");

    // The decoder's host is still serving.
    assert!(
        !decoder_worker.is_finished(),
        "the decoder's host must still be serving"
    );
    dsp_worker.join().expect("the killed helper must not panic");

    drop(host);
    decoder_worker
        .join()
        .expect("the decoder's host must not panic");
}

/// The host's control loop neither spins nor exits when a session fails.
///
/// The loop reports the failed session once, asks it no more, keeps working
/// on the session that is still alive and stays within a sane turn rate: it
/// blocks in the live session's own bounded wait instead of spinning on the
/// dead one.
#[test]
fn the_control_loop_keeps_running_and_does_not_spin_after_a_failure() {
    let fail = Arc::new(AtomicBool::new(false));
    let (decoder, decoder_worker) = start_decoder(PatternResident {
        frames: 4096,
        fail: fail.clone(),
    });
    let (dsp, dsp_worker) = start_dsp(PassthroughResident {
        fail: Arc::new(AtomicBool::new(false)),
    });
    let mut host = ControlLoop::new(decoder, dsp);

    // The loop works before the failure.
    host.settle();
    assert!(
        host.pushed > 0,
        "the loop must push frames through the chain"
    );

    // The decoder fails.
    fail.store(true, Ordering::SeqCst);
    host.run_until(|host| host.decoder_failure.is_some(), "the decoder failure");
    assert_eq!(host.reports.len(), 1, "the loop reports the failure once");

    // The loop does not exit: it keeps turning, and the chain it still owns
    // keeps working.
    let before = host.dsp.back_ring_counters().published;
    host.turn();
    assert!(
        host.turns_in(SPIN_WINDOW) > 0,
        "the loop must keep running after the failure"
    );
    assert_eq!(
        host.decoder_asks, host.decoder_asks_at_failure,
        "the loop must not ask the failed session again"
    );
    assert_eq!(host.reports.len(), 1, "the loop reports the failure once");

    // The loop does not spin: it blocks in the live session's own wait, so
    // its turn rate stays far below a spin.
    let turns = host.turns_in(SPIN_WINDOW);
    assert!(
        turns <= MAX_SPIN_TURNS,
        "the loop must not spin: {turns} turns in {SPIN_WINDOW:?}"
    );

    // The chain is still alive and still serving: the failure did not take
    // it with it.
    assert!(
        !dsp_worker.is_finished(),
        "the chain's host must still be serving"
    );
    let after = host.dsp.back_ring_counters().published;
    assert!(
        after >= before,
        "the chain's counters must not go backwards"
    );
    assert_eq!(host.dsp.frames_dropped(), 0, "no frame may be dropped");
    // The chain still takes a write and returns the frame while the loop
    // keeps running.
    let frame = frame_pattern::frame_samples(1);
    host.dsp
        .write_timeout(&frame, Some(STALL_WAIT))
        .expect("the chain must still take writes");
    let mut out = [0.0f32; 2];
    assert_eq!(
        host.dsp
            .read(&mut out, Some(STALL_WAIT))
            .expect("the chain must still deliver frames"),
        1
    );
    assert_eq!(out, frame, "the frame must survive the round trip");

    decoder_worker
        .join()
        .expect("the decoder's host must not panic");
    drop(host);
    dsp_worker.join().expect("the chain's host must not panic");
}
