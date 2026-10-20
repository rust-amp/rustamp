//! Property tests for the media host's session and control state machine.
//!
//! The player-side sessions ([`DspSession`], [`DecoderSession`]) and the
//! control peer under them are one state machine: a live session takes
//! writes and delivers frames, a finished transfer ends in a sticky `Ok(0)`
//! and a failed one ends in a sticky loud error. The last waves changed
//! exactly this layer — the delivery fence (wave204), the kill handling
//! (wave196) and the truncation evidence (wave200) — so its invariants are
//! pinned here as **properties over random operation sequences** instead of
//! as single hand-picked scenarios.
//!
//! The invariants:
//!
//! 1. **A terminal state is sticky.** A session that failed keeps failing:
//!    every later call returns the same error, never a fresh success and
//!    never a false end of stream. A session whose transfer ended keeps
//!    returning `Ok(0)`. A *caller* error (a buffer that is not frame
//!    aligned) is not a session failure and never makes the session
//!    terminal.
//! 2. **A write that returned `Ok` was taken by the peer.** The delivery
//!    fence (RFC 0006 §6): when the write returns, the frames it published
//!    are already gone from the in-ring, so the counter identity
//!    (`published == read`) holds after every successful write, whatever
//!    the interleaving of reads and writes is.
//! 3. **No frame is torn, duplicated or reordered.** The frames that come
//!    back are exactly the frames that went in, in order, whatever the chunk
//!    sizes are.
//! 4. **Close is idempotent.** Closing a ring twice, or ending the writes
//!    twice, changes nothing: the counters stay and the session stays in
//!    the same state.
//!
//! Every sequence is driven by a printed seed, so a failing run is
//! replayable. The peer is a scripted in-process host: it performs the
//! production handshake, echoes the frames it takes and can be killed at
//! any point, which is what the failure paths need.

#![cfg(windows)]

#[path = "support/frame_pattern.rs"]
mod frame_pattern;
#[path = "support/rng.rs"]
mod rng;
#[path = "support/streams.rs"]
mod streams;

use std::collections::VecDeque;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rustamp_ipc::Message;
use rustamp_ipc::shm::{RingDirection, RingError, RingKind};
use rustamp_mediahost::control::{ControlPeer, Liveness};
use rustamp_mediahost::decoder::{
    DecoderInfo, DecoderResident, DecoderSession, ResidentDecoder, serve_decoder,
};
use rustamp_mediahost::dsp::DspSession;
use rustamp_mediahost::pcm::{NamedPcmRings, PcmOffer, PcmRings};
use rustamp_mediahost::{MediaHostError, Result};
use rustamp_proto::host::{MediaLocationWire, PcmSpecWire};

use rng::Rng;

/// The stereo spec every fixture uses: one frame is two interleaved samples.
const SPEC: PcmSpecWire = PcmSpecWire {
    sample_rate_hz: 44_100,
    channels: 2,
};

/// The liveness policy of the in-process peers. A healthy peer answers
/// within milliseconds, so only a dead one reaches the bound — and a dead
/// peer is caught by the control EOF long before that.
const LIVENESS: Liveness = Liveness {
    interval: Duration::from_millis(50),
    timeout: Duration::from_secs(10),
};

/// The bound for a wait that must not block for ever. A healthy host answers
/// within milliseconds, so reaching the bound is a stall and fails loudly
/// with the step named — never a pass.
const WAIT: Duration = Duration::from_secs(10);

/// How long the scripted host waits for a frame before it services the
/// control channel again. The reset fence must not wait a whole poll slice.
const HOST_POLL: Duration = Duration::from_millis(2);

/// The in-ring capacity: small enough that the ring really wraps, large
/// enough that a write of a few frames fits.
const IN_RING_FRAMES: u32 = 8;

/// The back-ring capacity. It has to hold every frame a sequence publishes,
/// so the host never blocks on a full back-ring while the player writes.
const BACK_RING_FRAMES: u32 = 512;

/// The seeds of the property runs. A failing run prints its seed, so the
/// sequence is replayable.
const SEEDS: [u64; 12] = [
    0x0000_0001,
    0x0000_00a3,
    0x0000_07ff,
    0x0000_ffff,
    0x0001_2345,
    0x00c0_ffee,
    0x0bad_c0de,
    0x5eed_0001,
    0x5eed_0002,
    0xdead_beef,
    0xface_b00c,
    0xffff_ffff,
];

/// The number of operations in one random sequence.
const STEPS: usize = 24;

/// The frames one operation may write or read.
const MAX_CHUNK: usize = 4;

/// What the scripted host does with the frames it takes from the in-ring.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HostMode {
    /// Publish every frame it takes, unchanged.
    Echo,
    /// Publish at most this many frames and then stop: a truncated transfer.
    Truncate(usize),
    /// Publish nothing and close the back-ring right after the handshake: a
    /// host that ends the transfer while the input is still open.
    CloseEarly,
}

/// The scripted host: the peer on the other end of the player's control
/// stream.
///
/// It performs the production handshake in the production order, echoes the
/// frames it takes from the in-ring to the back-ring and answers `DspReset`
/// with `DspResetAck` — the three things the player's session expects. The
/// test drives it: the `kill` channel stops it without a terminal message
/// (a killed helper leaves the rings open and says nothing) and the mode
/// decides what it does with the frames.
fn run_scripted_host(peer: ControlPeer, mode: HostMode, kill: mpsc::Receiver<()>) {
    // 1. The handshake: the configure, the player's in-ring offer, the
    //    in-ring Ready, the back-ring offer, the back-ring Ready and
    //    `DspReady` (the order `DspSession::open` expects).
    let spec = match peer.recv(WAIT) {
        Ok(Message::DspConfigure { spec, .. }) => spec,
        other => panic!("expected DspConfigure, got {other:?}"),
    };
    let offer = match peer.recv(WAIT) {
        Ok(offer @ Message::RingOffer { .. }) => offer,
        other => panic!("expected the in-ring offer, got {other:?}"),
    };
    let mut input = NamedPcmRings
        .open_directed(&offer, spec, RingDirection::MainToHost)
        .expect("the scripted host opens the in-ring");
    peer.send(Message::RingReady {
        kind: RingKind::Pcm,
        generation: input.consumer.generation(),
    })
    .expect("the scripted host acknowledges the in-ring");
    let PcmOffer {
        producer: mut back,
        message: back_offer,
        abort: _,
    } = NamedPcmRings
        .create_directed(spec, BACK_RING_FRAMES, RingDirection::HostToMain)
        .expect("the scripted host creates the back-ring");
    let back_generation = back.generation();
    peer.send(back_offer)
        .expect("the scripted host offers the back-ring");
    match peer.recv(WAIT) {
        Ok(Message::RingReady {
            kind: RingKind::Pcm,
            generation,
        }) => assert_eq!(generation, back_generation, "the back-ring generation"),
        other => panic!("expected the back-ring Ready, got {other:?}"),
    }
    peer.send(Message::DspReady { latency_frames: 0 })
        .expect("the scripted host is ready");

    if mode == HostMode::CloseEarly {
        // The transfer ends while the input is still open: the back-ring
        // closes, the in-ring stays open.
        back.close();
    }
    let limit = match mode {
        HostMode::Truncate(limit) => limit,
        _ => usize::MAX,
    };

    // 2. The transfer: take a block, publish what the mode allows.
    let channels = usize::from(spec.channels);
    let mut buf = vec![0.0f32; 64 * channels];
    let mut published = 0usize;
    let mut transfer = true;
    loop {
        // The kill wins over everything: a killed helper leaves the rings
        // open and sends no terminal message.
        match kill.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => return,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        // Service the control channel: the reset fence needs its answer.
        loop {
            match peer.try_recv() {
                Ok(Some(Message::DspReset)) => peer
                    .send(Message::DspResetAck)
                    .expect("the scripted host answers the reset"),
                Ok(Some(Message::Shutdown)) => return,
                Ok(Some(other)) => panic!("unexpected message from the player: {other:?}"),
                Ok(None) => break,
                // The player is gone: the production worker ends the same
                // way. The player side notices a premature end (the frames
                // never arrive), so the test still fails loudly.
                Err(_) => return,
            }
        }
        if !transfer {
            // The transfer is over. The host stays alive for a reset or a
            // shutdown, exactly like the production worker, so the player's
            // end of the stream and its reset fence keep working.
            thread::sleep(HOST_POLL);
            continue;
        }
        match input.consumer.read_into_blocking(&mut buf, Some(HOST_POLL)) {
            Ok(0) => continue,
            Ok(frames) => {
                let room = limit.saturating_sub(published).min(frames);
                if room > 0 {
                    back.write_from_blocking(&buf[..room * channels], Some(WAIT))
                        .expect("the scripted host publishes its frames");
                    published += room;
                }
            }
            Err(RingError::Closed) => {
                // The player ended its writes: the transfer ends here and the
                // back-ring is closed for the drain.
                back.close();
                transfer = false;
            }
            Err(error) => panic!("the in-ring failed: {error}"),
        }
    }
}

/// A player session and the handle of its scripted host.
struct Fixture {
    session: DspSession<NamedPcmRings>,
    kill: mpsc::Sender<()>,
    dead: mpsc::Receiver<()>,
    host: thread::JoinHandle<()>,
}

impl Fixture {
    /// Kill the host: it stops without a terminal message, exactly like a
    /// helper that is killed from the outside. The rings stay open; only the
    /// control channel ends.
    fn kill_host(&mut self) {
        let _ = self.kill.send(());
        self.dead
            .recv_timeout(WAIT)
            .expect("stall: the scripted host must stop when it is killed");
    }

    /// End the session and join the host, so no thread outlives the test.
    fn finish(self) {
        drop(self.session);
        self.host.join().expect("the scripted host must not panic");
    }
}

/// A player session with a scripted host in `mode`.
fn start(mode: HostMode) -> Fixture {
    let ((ar, aw), (br, bw)) = streams::streams();
    let player = ControlPeer::start(ar, aw, LIVENESS, false).expect("the player peer starts");
    let host = ControlPeer::start(br, bw, LIVENESS, false).expect("the host peer starts");
    let (kill, killed) = mpsc::channel();
    let (dead_tx, dead) = mpsc::channel();
    let handle = thread::spawn(move || {
        run_scripted_host(host, mode, killed);
        // The peer is dropped when the host returns, so the control channel
        // has ended when the test is told about it.
        let _ = dead_tx.send(());
    });
    let session = DspSession::open(player, NamedPcmRings, SPEC, &[], IN_RING_FRAMES)
        .expect("the handshake completes");
    Fixture {
        session,
        kill,
        dead,
        host: handle,
    }
}

/// The frames of the shared pattern, `count` of them, starting at `index`.
fn frames(index: usize, count: usize) -> Vec<[f32; 2]> {
    (index..index + count)
        .map(frame_pattern::frame_samples)
        .collect()
}

/// The interleaved samples of `frames`.
fn samples(frames: &[[f32; 2]]) -> Vec<f32> {
    frames
        .iter()
        .flat_map(|frame| frame.iter().copied())
        .collect()
}

/// Compare the frames that came back with the frames that went in.
///
/// A torn, duplicated, missing or reordered frame breaks the shared pattern,
/// so the comparison is exact: the samples must be bit-identical.
fn check_frames(received: &[f32], expected: &mut VecDeque<[f32; 2]>, step: &str) {
    let channels = usize::from(SPEC.channels);
    for (index, frame) in received.chunks_exact(channels).enumerate() {
        let want = expected.pop_front().unwrap_or_else(|| {
            panic!("{step}: more frames came back than went in (frame {index})")
        });
        assert_eq!(
            frame,
            want.as_slice(),
            "{step}: frame {index} was torn, duplicated or reordered"
        );
    }
}

/// The state the model predicts for the session under test.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    /// The session takes writes and delivers frames.
    Live,
    /// The player ended its writes; the transfer is still running.
    InputClosed,
    /// The transfer ended cleanly: every read returns `Ok(0)`.
    Eos,
    /// The session failed; every later call fails with the same reason.
    Failed(String),
}

/// One operation of a random sequence.
#[derive(Clone, Copy, Debug)]
enum Op {
    /// Write a few frames.
    Write(usize),
    /// Read a few frames.
    Read(usize),
    /// Read into a buffer that is not frame-aligned: a caller error.
    ReadMisaligned,
    /// Write a buffer that is not frame-aligned: a caller error.
    WriteMisaligned,
    /// Reset the chain (the reset fence).
    Reset,
    /// End the writes.
    FinishWrites,
}

/// The next operation of a random sequence.
///
/// The choice respects the model: in the live phase a read waits for frames,
/// and frames only exist if the player wrote some, so a sequence never
/// blocks on a frame nobody will ever publish.
fn choose_op(rng: &mut Rng, phase: &Phase, outstanding: usize, allow_finish: bool) -> Op {
    if let Phase::Failed(_) = phase {
        // Every operation has to fail now; the model only checks that.
        return match rng.below(4) {
            0 => Op::Write(chunk(rng)),
            1 => Op::Read(chunk(rng)),
            2 => Op::Reset,
            _ => Op::FinishWrites,
        };
    }
    if phase == &Phase::Eos {
        // The transfer is over: the interesting operations are the reads
        // that must keep reporting the end of the stream.
        return match rng.below(4) {
            0 => Op::Read(chunk(rng)),
            1 => Op::Write(chunk(rng)),
            2 => Op::Reset,
            _ => Op::ReadMisaligned,
        };
    }
    if phase == &Phase::InputClosed {
        return match rng.below(6) {
            0 => Op::Read(chunk(rng)),
            1 => Op::Reset,
            2 => Op::FinishWrites,
            3 => Op::ReadMisaligned,
            4 => Op::WriteMisaligned,
            _ => Op::Write(chunk(rng)),
        };
    }
    // The live phase: a read waits for a frame, and a frame only exists if
    // the player wrote some, so a read is only chosen while a frame is on
    // its way back. Otherwise the read would wait for a frame nobody will
    // ever publish.
    if outstanding == 0 {
        return match rng.below(4) {
            0 => Op::Reset,
            1 => Op::ReadMisaligned,
            2 => Op::WriteMisaligned,
            _ => Op::Write(chunk(rng)),
        };
    }
    match rng.below(12) {
        0..=3 => Op::Write(chunk(rng)),
        4..=6 => Op::Read(chunk(rng)),
        7 => Op::Reset,
        8 => Op::ReadMisaligned,
        9 => Op::WriteMisaligned,
        _ => {
            if allow_finish {
                Op::FinishWrites
            } else {
                Op::Write(chunk(rng))
            }
        }
    }
}

/// A random chunk size: at least one frame, at most [`MAX_CHUNK`].
fn chunk(rng: &mut Rng) -> usize {
    1 + rng.below(MAX_CHUNK as u64) as usize
}

/// Assert that a call on a failed session fails with the sticky reason.
fn assert_sticky<T>(result: &Result<T>, reason: &str, step: &str) {
    match result {
        Ok(_) => panic!("{step}: a failed session must not succeed again"),
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains(reason),
                "{step}: the failure must stick: {text} does not name {reason}"
            );
        }
    }
}

/// Drain a session whose host died: the frames that made it through are
/// still delivered, then the failure comes. The returned text is the sticky
/// reason every later call has to name.
fn drain_to_failure(
    session: &mut DspSession<NamedPcmRings>,
    expected: &mut VecDeque<[f32; 2]>,
) -> String {
    let mut out = [0.0f32; 64];
    loop {
        match session.read(&mut out, Some(WAIT)) {
            Ok(0) => panic!("a dead host must not look like an end of stream"),
            Ok(frames) => check_frames(
                &out[..frames * usize::from(SPEC.channels)],
                expected,
                "the frames that made it through the death",
            ),
            Err(MediaHostError::TimedOut) => panic!(
                "stall: the dead host was not noticed within {WAIT:?} \
                 (the read waited for a frame nobody will ever publish)"
            ),
            Err(error) => return error.to_string(),
        }
    }
}

/// The phases a sequence visited.
///
/// The tests assert that the seeds together reach every phase: a property
/// that stops reaching a phase would be a silent false green.
#[derive(Clone, Copy, Debug, Default)]
struct Coverage {
    /// The session took writes and delivered frames.
    live: bool,
    /// The player ended its writes.
    input_closed: bool,
    /// The transfer ended with the end of the stream.
    eos: bool,
    /// The session failed.
    failed: bool,
}

impl Coverage {
    /// Add the phases of another sequence.
    fn merge(&mut self, other: Coverage) {
        self.live |= other.live;
        self.input_closed |= other.input_closed;
        self.eos |= other.eos;
        self.failed |= other.failed;
    }
}

/// Run one random operation sequence and check it against the model.
///
/// `kill_at` is the step at which the scripted host dies; `None` runs the
/// sequence to its end. The kill is only injected in the live phase: the
/// model reaches the other phases through the end of the writes. The
/// returned coverage names the phases the sequence visited.
fn run_sequence(seed: u64, kill_at: Option<usize>) -> Coverage {
    let mut rng = Rng::new(seed);
    let mut fixture = start(HostMode::Echo);
    let mut phase = Phase::Live;
    let mut expected: VecDeque<[f32; 2]> = VecDeque::new();
    let mut written = 0usize;
    let mut coverage = Coverage {
        live: true,
        ..Coverage::default()
    };

    for step in 1..=STEPS {
        if let Some(kill) = kill_at
            && step == kill
            && phase == Phase::Live
        {
            // The host dies: the session is doomed from here on.
            fixture.kill_host();
            let reason = drain_to_failure(&mut fixture.session, &mut expected);
            phase = Phase::Failed(reason);
            coverage.failed = true;
            continue;
        }
        let allow_finish = kill_at.is_none_or(|kill| step > kill);
        let op = choose_op(&mut rng, &phase, expected.len(), allow_finish);
        let label = format!("seed {seed}, step {step}, {op:?}");
        match op {
            Op::Write(count) => {
                let block = frames(written, count);
                written += count;
                let result = fixture.session.write(&samples(&block));
                match phase.clone() {
                    Phase::Live => {
                        result.unwrap_or_else(|error| {
                            panic!("{label}: a live session accepts the write: {error}")
                        });
                        // The delivery fence: the frames are already gone
                        // from the in-ring when the write returns.
                        let counters = fixture.session.in_ring_counters();
                        assert_eq!(
                            counters.read, counters.published,
                            "{label}: the host must have taken every published frame"
                        );
                        expected.extend(block);
                    }
                    Phase::InputClosed | Phase::Eos => {
                        let error = result
                            .expect_err(&format!("{label}: a finished input refuses the write"));
                        assert!(
                            error.to_string().contains("DSP input is finished"),
                            "{label}: {error}"
                        );
                    }
                    Phase::Failed(reason) => assert_sticky(&result, &reason, &label),
                }
            }
            Op::Read(count) => {
                let mut out = vec![0.0f32; count * usize::from(SPEC.channels)];
                let result = fixture.session.read(&mut out, Some(WAIT));
                match phase.clone() {
                    Phase::Live | Phase::InputClosed => match result {
                        Ok(0) => {
                            // The transfer ended: the end of the stream is
                            // only possible after the writes ended.
                            assert_eq!(
                                phase,
                                Phase::InputClosed,
                                "{label}: an end of the stream before the writes ended"
                            );
                            phase = Phase::Eos;
                            coverage.eos = true;
                        }
                        Ok(frames) => {
                            assert!(frames > 0, "{label}: a read delivers frames");
                            check_frames(
                                &out[..frames * usize::from(SPEC.channels)],
                                &mut expected,
                                &label,
                            );
                        }
                        Err(MediaHostError::TimedOut) => {
                            panic!("stall: {label} received no frame within {WAIT:?}")
                        }
                        Err(error) => panic!("{label}: unexpected error: {error}"),
                    },
                    Phase::Eos => {
                        let frames = result.unwrap_or_else(|error| {
                            panic!("{label}: the end of the stream must be Ok(0): {error}")
                        });
                        assert_eq!(frames, 0, "{label}: the end of the stream sticks");
                    }
                    Phase::Failed(reason) => assert_sticky(&result, &reason, &label),
                }
            }
            Op::ReadMisaligned => {
                // One sample too many: the buffer is not frame-aligned.
                let mut out = vec![0.0f32; 3];
                let result = fixture.session.read(&mut out, Some(WAIT));
                match phase.clone() {
                    Phase::Failed(reason) => assert_sticky(&result, &reason, &label),
                    _ => {
                        let error = result
                            .expect_err(&format!("{label}: a misaligned read is a caller error"));
                        assert!(
                            matches!(error, MediaHostError::InvalidMessage(_)),
                            "{label}: {error}"
                        );
                    }
                }
            }
            Op::WriteMisaligned => {
                let result = fixture.session.write(&[0.0f32; 3]);
                match phase.clone() {
                    Phase::Failed(reason) => assert_sticky(&result, &reason, &label),
                    Phase::InputClosed | Phase::Eos => {
                        let error = result
                            .expect_err(&format!("{label}: a finished input refuses the write"));
                        assert!(
                            error.to_string().contains("DSP input is finished"),
                            "{label}: {error}"
                        );
                    }
                    Phase::Live => {
                        let error = result
                            .expect_err(&format!("{label}: a misaligned write is a caller error"));
                        assert!(
                            matches!(error, MediaHostError::InvalidMessage(_)),
                            "{label}: {error}"
                        );
                    }
                }
            }
            Op::Reset => {
                let result = fixture.session.reset();
                match phase.clone() {
                    Phase::Failed(reason) => assert_sticky(&result, &reason, &label),
                    _ => result.unwrap_or_else(|error| {
                        panic!("{label}: the reset fence must land: {error}")
                    }),
                }
            }
            Op::FinishWrites => {
                let result = fixture.session.finish_writes();
                match phase.clone() {
                    Phase::Failed(reason) => assert_sticky(&result, &reason, &label),
                    Phase::Live => {
                        result.unwrap_or_else(|error| {
                            panic!("{label}: ending the writes must work: {error}")
                        });
                        phase = Phase::InputClosed;
                        coverage.input_closed = true;
                    }
                    Phase::InputClosed | Phase::Eos => {
                        // Ending the writes twice is the same ending.
                        result.unwrap_or_else(|error| {
                            panic!("{label}: ending the writes twice must work: {error}")
                        });
                    }
                }
            }
        }
    }
    fixture.finish();
    coverage
}

/// The state machine of a session under a random sequence of operations:
/// the live phase, the finished transfer and the sticky end of the stream.
#[test]
fn a_random_operation_sequence_keeps_the_session_state_machine_consistent() {
    let mut coverage = Coverage::default();
    for seed in SEEDS {
        coverage.merge(run_sequence(seed, None));
    }
    assert!(
        coverage.live && coverage.input_closed && coverage.eos,
        "the seeds must reach every phase of the state machine: {coverage:?}"
    );
}

/// The kill handling: a host that dies in the live phase leaves a session
/// that fails loudly and keeps failing.
#[test]
fn a_kill_in_the_live_phase_fails_the_session_loudly_and_sticks() {
    let mut coverage = Coverage::default();
    for seed in SEEDS {
        let kill = 1 + Rng::new(seed).below(STEPS as u64) as usize;
        coverage.merge(run_sequence(seed, Some(kill)));
    }
    assert!(
        coverage.failed,
        "every seed must kill the host in the live phase: {coverage:?}"
    );
}

/// The delivery fence (RFC 0006 §6): a write that returned `Ok` was taken by
/// the peer. The counter identity is checked after every successful write,
/// whatever the interleaving of reads and writes is.
#[test]
fn a_write_that_returned_ok_was_taken_by_the_peer() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let mut fixture = start(HostMode::Echo);
        let mut written = 0u64;
        let mut read = 0u64;
        let mut expected: VecDeque<[f32; 2]> = VecDeque::new();
        for _ in 0..STEPS {
            let count = 1 + rng.below(MAX_CHUNK as u64) as usize;
            let block = frames(written as usize, count);
            written += count as u64;
            fixture
                .session
                .write(&samples(&block))
                .unwrap_or_else(|error| {
                    panic!("seed {seed}: the live host takes the frames: {error}")
                });
            // The frames that must come back.
            expected.extend(block);
            // The delivery fence (RFC 0006 §6): the frames the write
            // published are already gone from the in-ring when it returns.
            let counters = fixture.session.in_ring_counters();
            assert_eq!(
                counters.published, written,
                "seed {seed}: the in-ring counts the published frames"
            );
            assert_eq!(
                counters.read, written,
                "seed {seed}: the host must have taken every published frame"
            );
            assert_eq!(
                counters.available(),
                0,
                "seed {seed}: the in-ring must be empty when the write returns"
            );
            // The frames come back in random chunk sizes, and they are the
            // frames that went in.
            while read < written {
                let chunk = 1 + rng.below(MAX_CHUNK as u64) as usize;
                let mut out = vec![0.0f32; chunk * usize::from(SPEC.channels)];
                let frames_read = fixture
                    .session
                    .read(&mut out, Some(WAIT))
                    .unwrap_or_else(|error| panic!("seed {seed}: the read failed: {error}"));
                assert!(frames_read > 0, "seed {seed}: a read delivers frames");
                read += frames_read as u64;
                check_frames(
                    &out[..frames_read * usize::from(SPEC.channels)],
                    &mut expected,
                    "the frames that came back",
                );
            }
        }
        assert_eq!(read, written, "seed {seed}: every frame comes back once");
        fixture.finish();
    }
}

/// The truncation evidence (RFC 0006 §6): a host that returns fewer frames
/// than it was given must not look like a finished transfer, and the failure
/// must name the exact counts and stick.
#[test]
fn a_truncated_transfer_is_a_loud_failure_with_the_exact_counts() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let total = 2 + rng.below(12) as usize;
        let returned = rng.below(total as u64) as usize;
        let mut fixture = start(HostMode::Truncate(returned));
        let block = frames(0, total);
        fixture
            .session
            .write(&samples(&block))
            .unwrap_or_else(|error| panic!("seed {seed}: the write failed: {error}"));
        fixture
            .session
            .finish_writes()
            .unwrap_or_else(|error| panic!("seed {seed}: ending the writes failed: {error}"));

        let mut out = [0.0f32; 64];
        let mut received = 0usize;
        let error = loop {
            match fixture.session.read(&mut out, Some(WAIT)) {
                Ok(0) => panic!("seed {seed}: a truncated transfer must not be an end of stream"),
                Ok(frames) => received += frames,
                Err(MediaHostError::TimedOut) => {
                    panic!("seed {seed}: the truncated transfer was not noticed within {WAIT:?}")
                }
                Err(error) => break error,
            }
        };
        assert_eq!(
            received, returned,
            "seed {seed}: the frames the host returned must arrive"
        );
        let message = error.to_string();
        assert!(
            message.contains("truncated"),
            "seed {seed}: the truncation must be reported: {message}"
        );
        assert!(
            message.contains(&format!("{total} frames written")),
            "seed {seed}: the truncation must name the written frames: {message}"
        );
        assert!(
            message.contains(&format!("{returned} returned")),
            "seed {seed}: the truncation must name the returned frames: {message}"
        );
        // The failure sticks: a later read fails loud too, never an EOS.
        let again = fixture
            .session
            .read(&mut out, Some(WAIT))
            .expect_err("seed {seed}: the truncation must stick");
        assert!(
            again.to_string().contains("truncated"),
            "seed {seed}: the truncation must stick: {again}"
        );
        fixture.finish();
    }
}

/// A back-ring that closes while the input is still open ends the transfer
/// loudly: the session must not read it as an end of the stream, and the
/// failure must stick.
#[test]
fn a_back_ring_that_closes_before_the_input_finished_is_a_sticky_failure() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let mut fixture = start(HostMode::CloseEarly);
        let mut out = [0.0f32; 64];
        // The host closed the back-ring while the input is still open: the
        // transfer ends loudly, never as an end of the stream.
        let first = fixture
            .session
            .read(&mut out, Some(WAIT))
            .expect_err("seed {seed}: a closed back-ring must not be an end of the stream");
        assert!(
            matches!(first, MediaHostError::Disconnected(_)),
            "seed {seed}: {first}"
        );
        assert!(
            first.to_string().contains("before input finished"),
            "seed {seed}: {first}"
        );
        // The failure is the session's terminal state: every later call
        // names the same cause, whatever it is.
        let count = 1 + rng.below(MAX_CHUNK as u64) as usize;
        let block = frames(0, count);
        let again = fixture
            .session
            .write(&samples(&block))
            .expect_err("seed {seed}: the failure must stick");
        assert!(
            again.to_string().contains(&first.to_string()),
            "seed {seed}: the failure must stick: {again}"
        );
        let third = fixture
            .session
            .read(&mut out, Some(WAIT))
            .expect_err("seed {seed}: the failure must stick");
        assert!(
            third.to_string().contains(&first.to_string()),
            "seed {seed}: the failure must stick: {third}"
        );
        fixture.finish();
    }
}

/// Close is idempotent: closing a ring twice changes nothing. The counters
/// stay, the state stays and a blocked reader still wakes with the close.
#[test]
fn closing_a_ring_twice_changes_nothing() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let capacity = 1 + rng.below(8) as u32;
        let PcmOffer {
            mut producer,
            message,
            mut abort,
        } = NamedPcmRings
            .create(SPEC, capacity)
            .expect("the ring is created");
        let mut reader = NamedPcmRings
            .open(&message, SPEC)
            .expect("the ring is opened");
        let watcher = NamedPcmRings
            .open(&message, SPEC)
            .expect("the ring is opened again");

        // A blocked reader waits for a frame that never comes.
        let (woke, waiter) = mpsc::channel();
        let handle = thread::spawn(move || {
            let _ = woke.send(reader.consumer.read_into_blocking(&mut [0.0; 2], None));
        });

        // Close the ring, twice, from both ends.
        producer.close();
        producer.close();
        assert!(producer.is_closed(), "seed {seed}: the ring stays closed");
        assert_eq!(
            producer.frames_written(),
            0,
            "seed {seed}: no frame was written"
        );
        assert_eq!(
            producer.frames_dropped(),
            0,
            "seed {seed}: no frame was dropped"
        );
        abort.abort();
        abort.abort();
        assert!(
            watcher.consumer.is_closed(),
            "seed {seed}: the ring stays closed"
        );
        assert_eq!(
            watcher.consumer.frames_read(),
            0,
            "seed {seed}: no frame was read"
        );

        let outcome = waiter
            .recv_timeout(WAIT)
            .expect("stall: the close must wake the blocked reader");
        assert!(
            matches!(outcome, Err(RingError::Closed)),
            "seed {seed}: the blocked reader must see the close: {outcome:?}"
        );
        handle.join().expect("the reader must not panic");
    }
}

/// Ending the writes twice is the same ending: the session stays in the
/// state the first ending put it in.
#[test]
fn ending_the_writes_twice_is_the_same_ending() {
    for seed in SEEDS {
        let mut fixture = start(HostMode::Echo);
        fixture
            .session
            .finish_writes()
            .unwrap_or_else(|error| panic!("seed {seed}: ending the writes failed: {error}"));
        fixture
            .session
            .finish_writes()
            .unwrap_or_else(|error| panic!("seed {seed}: ending the writes twice failed: {error}"));
        // The state is the one the first ending put the session in: the
        // input is finished, so a write is refused, and the transfer ends.
        let error = fixture
            .session
            .write(&[0.0f32; 2])
            .expect_err("seed {seed}: a finished input refuses the write");
        assert!(
            error.to_string().contains("DSP input is finished"),
            "seed {seed}: {error}"
        );
        let mut out = [0.0f32; 2];
        assert_eq!(
            fixture
                .session
                .read(&mut out, Some(WAIT))
                .unwrap_or_else(|error| panic!("seed {seed}: the drain failed: {error}")),
            0,
            "seed {seed}: the transfer ends"
        );
        assert_eq!(
            fixture
                .session
                .read(&mut out, Some(WAIT))
                .unwrap_or_else(|error| panic!("seed {seed}: the drain failed: {error}")),
            0,
            "seed {seed}: the end of the stream sticks"
        );
        fixture.finish();
    }
}

/// A decoder resident that publishes the shared frame pattern, `block`
/// frames at a time, and that fails when it reaches `fail_at`.
struct PatternResident {
    frames: usize,
    block: usize,
    fail_at: Option<usize>,
}

struct PatternDecoder {
    frames: usize,
    block: usize,
    fail_at: Option<usize>,
    next: usize,
}

impl DecoderResident for PatternResident {
    fn open(&self, _location: MediaLocationWire) -> Result<Box<dyn ResidentDecoder>> {
        Ok(Box::new(PatternDecoder {
            frames: self.frames,
            block: self.block,
            fail_at: self.fail_at,
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
        if self.fail_at == Some(self.next) {
            return Err(MediaHostError::OperationFailed(
                "the fixture decoder failed".into(),
            ));
        }
        let channels = usize::from(SPEC.channels);
        let count = (self.frames - self.next)
            .min(self.block)
            .min(out.len() / channels);
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

/// A decoder session with an in-process host that runs `resident`.
fn start_decoder(
    resident: PatternResident,
    capacity: u32,
) -> (DecoderSession<NamedPcmRings>, thread::JoinHandle<()>) {
    let ((ar, aw), (br, bw)) = streams::streams();
    let player = ControlPeer::start(ar, aw, LIVENESS, false).expect("the player peer starts");
    let host = ControlPeer::start(br, bw, LIVENESS, true).expect("the host peer starts");
    let handle = thread::spawn(move || {
        let _ = serve_decoder(&host, &NamedPcmRings, &resident);
    });
    let session = DecoderSession::open(
        player,
        NamedPcmRings,
        MediaLocationWire::Url("fixture:pattern".into()),
        capacity,
    )
    .expect("the handshake completes");
    (session, handle)
}

/// The decoder path: random read chunk sizes must deliver every frame
/// exactly once and in order, and then a sticky end of the stream.
#[test]
fn a_random_read_sequence_receives_every_frame_once_and_then_a_sticky_eos() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let total = 1 + rng.below(200) as usize;
        let block = 1 + rng.below(16) as usize;
        let capacity = 1 + rng.below(16) as u32;
        let (mut session, host) = start_decoder(
            PatternResident {
                frames: total,
                block,
                fail_at: None,
            },
            capacity,
        );
        let mut received: Vec<[f32; 2]> = Vec::new();
        loop {
            let want = 1 + rng.below(8) as usize;
            let mut out = vec![0.0f32; want * usize::from(SPEC.channels)];
            match session.read(&mut out, Some(WAIT)) {
                Ok(0) => break,
                Ok(frames) => {
                    for frame in out[..frames * usize::from(SPEC.channels)]
                        .as_chunks::<2>()
                        .0
                    {
                        received.push(*frame);
                    }
                }
                Err(MediaHostError::TimedOut) => {
                    panic!("stall: seed {seed} received no PCM within {WAIT:?}")
                }
                Err(error) => panic!("seed {seed}: unexpected error: {error}"),
            }
        }
        assert_eq!(
            received.len(),
            total,
            "seed {seed}: every frame must arrive"
        );
        for (index, frame) in received.iter().enumerate() {
            assert_eq!(
                *frame,
                frame_pattern::frame_samples(index),
                "seed {seed}: frame {index} was torn, duplicated or reordered"
            );
        }
        // The end of the stream sticks.
        assert_eq!(
            session
                .read(&mut [0.0f32; 2], Some(WAIT))
                .unwrap_or_else(|error| panic!("seed {seed}: the drain failed: {error}")),
            0,
            "seed {seed}: the transfer ended"
        );
        assert_eq!(
            session
                .read(&mut [0.0f32; 2], Some(WAIT))
                .unwrap_or_else(|error| panic!("seed {seed}: the drain failed: {error}")),
            0,
            "seed {seed}: the end of the stream sticks"
        );
        drop(session);
        let _ = host.join();
    }
}

/// The decoder path: a host that fails ends the session loudly, never as an
/// end of the stream, and the failure sticks.
#[test]
fn a_decoder_failure_is_loud_and_sticks() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let total = 1 + rng.below(50) as usize;
        let (mut session, host) = start_decoder(
            PatternResident {
                frames: total,
                block: 1 + rng.below(8) as usize,
                fail_at: Some(total),
            },
            1 + rng.below(16) as u32,
        );
        let mut out = [0.0f32; 64];
        let mut received = 0usize;
        let error = loop {
            match session.read(&mut out, Some(WAIT)) {
                Ok(0) => panic!("seed {seed}: a failed host must not be an end of the stream"),
                Ok(frames) => received += frames,
                Err(MediaHostError::TimedOut) => {
                    panic!("stall: seed {seed} did not notice the failure within {WAIT:?}")
                }
                Err(error) => break error,
            }
        };
        assert!(
            matches!(error, MediaHostError::Disconnected(_)),
            "seed {seed}: {error}"
        );
        assert!(
            error.to_string().contains("resident decoder failed"),
            "seed {seed}: {error}"
        );
        assert!(
            received <= total,
            "seed {seed}: no frame is delivered twice"
        );
        // The failure sticks: a later read fails loud too, never an EOS.
        let again = session
            .read(&mut out, Some(WAIT))
            .expect_err("seed {seed}: the failure must stick");
        assert!(
            matches!(again, MediaHostError::Disconnected(_)),
            "seed {seed}: {again}"
        );
        drop(session);
        let _ = host.join();
    }
}
