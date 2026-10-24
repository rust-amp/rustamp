//! The RFC 0007 blocking wake, proved across a process boundary.
//!
//! The mechanism this file exercises is the one RFC 0007 is about: a consumer
//! that **blocks** in an empty-ring read until a producer in **another
//! process** publishes a frame and signals `Wake::Data`. Two claims are made
//! about that wait, and both are measured here rather than argued:
//!
//! 1. **The wake latency is bounded.** The producer's publish and the
//!    consumer's release are timestamped in the two processes and compared
//!    across the process boundary. A wake that never arrives fails the test
//!    through the bounded wait; a wake that arrives late fails the bound.
//! 2. **No busy-spin.** The consumer's thread burns no CPU while it waits.
//!    A polling consumer would burn the whole wall clock, so the two cases
//!    are an order of magnitude apart, and the thread's own CPU accounting
//!    (user + kernel, from `GetThreadTimes`) tells them apart.
//!
//! The same two claims are proved for the other direction of the mechanism:
//! a producer that waits for space on a full ring and is released by a
//! consumer in the other process (`Wake::Space`).
//!
//! # Why this is a separate process and not a second thread
//!
//! A thread shares the address space; a process does not. The wake is two
//! **named** kernel events and the ring is a **named** section, so the
//! mechanism is only exercised across a process boundary if the two ends
//! really are in different address spaces. `cargo xtask wake-cross` proves
//! the same thing across the 32-bit/64-bit boundary; this file proves it
//! where the ordinary test suite can see it.
//!
//! # The peer
//!
//! The peer is **this same test binary**, started again with a role in its
//! environment. That keeps the check self-contained: no second program to
//! build, stage or find, and the guest's test runner stages the test binary
//! itself, so the peer is always next to it. `RUSTAMP_WAKE_PEER_BIN` points
//! the check at a different executable instead - the i686 build of this very
//! test binary, for instance - and the proof then runs across the bitness
//! boundary as well.
//!
//! # VM only
//!
//! The check spawns a process, so it is `#[ignore]`d and refuses to run
//! outside the disposable test VM (the same rule the app's window tests
//! follow, see `docs/HANDOFF.md`).
//!
//! # What the peer protocol looks like
//!
//! The peer reads one command per line on stdin and answers with one line on
//! stdout:
//!
//! | direction | line |
//! |---|---|
//! | parent -> peer | `PUBLISH <frames>` / `DRAIN <frames>` / `QUIT` |
//! | peer -> parent | `PEER_RING name=… generation=… size=…` |
//! | peer -> parent | `PEER_READY` |
//! | peer -> parent | `PEER_PUBLISHED frames=… started_us=… done_us=…` |
//! | peer -> parent | `PEER_READ frames=… started_us=… done_us=…` |
//!
//! The timestamps are epoch microseconds, so the two processes can compare
//! them: the wall clock is the only clock both of them have.

#![cfg(windows)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustamp_ipc::shm::{
    NamedRegion, RING_DATA_OFFSET, RingConsumer, RingExpect, RingKind, RingProducer, RingWake,
    ring_name,
};

/// Slots in the ring. Four is enough to fill it and to leave room for the
/// frame the blocked producer writes once the consumer has drained it.
const CAPACITY: u32 = 4;
/// Bytes per slot: two interleaved `f32` channels.
const FRAME_SIZE: u32 = 8;
/// Interleaved `f32` channels per frame.
const CHANNELS: usize = (FRAME_SIZE / 4) as usize;

/// The timeout of a blocking call in the check.
///
/// Every wait is bounded: a lost wake must fail the check loudly instead of
/// hanging the suite (RFC 0007 §6.1, and the lesson of
/// `docs/tasks/W19-twoproc-stall.md`).
const WAIT: Option<Duration> = Some(Duration::from_secs(10));

/// The bounded wait that **proves** the consumer is blocked.
///
/// A blocking read on an empty ring that returns `Ok(0)` after this long
/// entered the kernel wait, found the ring empty with the latch clear and
/// came back untouched. That is the fact the parent needs before it lets the
/// peer publish; a sleep would only buy a guess.
const ANNOUNCE_WAIT: Duration = Duration::from_millis(100);

/// How long the parent waits before it lets the peer publish.
///
/// This is **not** an ordering mechanism: the consumer has already proved
/// that it blocks, and the publish follows the command, not the delay. The
/// delay exists so the consumer's blocked window is wide enough for the
/// operating system to account thread time at all - Windows accounts it on
/// the system timer tick, so a window of a few milliseconds could not tell a
/// sleeping thread from a spinning one.
const PUBLISH_DELAY: Duration = Duration::from_millis(300);

/// The upper bound for the wake latency: from the producer's decision to
/// publish until the consumer is released again.
///
/// Generous enough for a loaded machine, tight enough that a wake that only
/// arrives when the wait expires cannot pass.
const WAKE_LATENCY_BOUND: Duration = Duration::from_secs(2);

/// The CPU time a blocked thread may burn, in microseconds.
///
/// A wait in the kernel costs next to nothing; a poll costs the whole wall
/// clock. The bound sits between the two by three orders of magnitude.
const CPU_BOUND_MICROS: u64 = 50_000;

/// How long the parent waits for the peer's first line.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// `CreateProcess` flag: run the child with a console but **no console
/// window**. A console-subsystem child spawned without it gets a window of
/// its own, and a window on the owner's desktop is a rule violation
/// (`docs/HANDOFF.md`, the GUI rule).
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The environment variable that tells a peer which role it plays.
const ROLE_ENV: &str = "RUSTAMP_WAKE_PEER_ROLE";
/// The environment variable that carries the ring's name to the peer.
const NAME_ENV: &str = "RUSTAMP_WAKE_PEER_NAME";
/// The environment variable that carries the ring's capacity to the peer.
const CAPACITY_ENV: &str = "RUSTAMP_WAKE_PEER_CAPACITY";
/// The environment variable that carries the ring's frame size to the peer.
const FRAME_SIZE_ENV: &str = "RUSTAMP_WAKE_PEER_FRAME_SIZE";
/// The environment variable that carries the ring's generation to the peer.
const GENERATION_ENV: &str = "RUSTAMP_WAKE_PEER_GENERATION";
/// The environment variable that carries the mapping's size to the peer.
const SIZE_ENV: &str = "RUSTAMP_WAKE_PEER_SIZE";
/// The environment variable that points the check at a peer executable other
/// than this test binary (the i686 build of it, for the cross-bitness run).
const PEER_BIN_ENV: &str = "RUSTAMP_WAKE_PEER_BIN";

/// The role of a peer that produces.
const ROLE_PRODUCER: &str = "producer";
/// The role of a peer that consumes.
const ROLE_CONSUMER: &str = "consumer";

/// The name of the test that runs the consumer side of the first check.
///
/// The peer is this same binary, so it is started with `--exact <name>`; the
/// name has to match the function that spawns it.
const CONSUMER_TEST_NAME: &str = "cross_process_consumer_is_woken_by_the_remote_producer";
/// The name of the test that runs the producer side of the second check.
const PRODUCER_TEST_NAME: &str = "cross_process_producer_is_woken_by_the_remote_consumer";

/// The computer name of the disposable test VM.
const TEST_VM_COMPUTER: &str = "DESKTOP-UL3J9H6";
/// The path the test runner stages the repository tree to inside the guest.
/// The host never has it, so its presence proves the run is in the VM.
const TEST_VM_MARKER: &str = r"C:\rustamp\code\Rustamp";

/// Refuse to run outside the disposable test VM.
///
/// The check spawns a process, which the repository's rules treat like a
/// window: it belongs in the VM, not on the owner's desktop. The gate has no
/// override, exactly like the app's own (`crates/rustamp-app/src/window.rs`).
fn require_test_vm() {
    let by_name = std::env::var("COMPUTERNAME")
        .map(|name| name.eq_ignore_ascii_case(TEST_VM_COMPUTER))
        .unwrap_or(false);
    assert!(
        by_name || Path::new(TEST_VM_MARKER).exists(),
        "refusing to run: this check spawns a process and is VM-only, but this machine is \
         neither the headless test VM ({TEST_VM_COMPUTER}) nor carries the runner's staged \
         tree ({TEST_VM_MARKER})"
    );
}

/// The value of one sample of frame `index` on `channel`.
///
/// Both processes compute it from the frame's global index, so a wrong byte,
/// a byte-swapped counter or a disagreement between a 32-bit and a 64-bit
/// build changes the value.
fn pcm_sample(index: u64, channel: usize) -> f32 {
    let k = index
        .wrapping_mul(3)
        .wrapping_add((channel as u64).wrapping_mul(5))
        % 1021;
    k as f32 / 8.0
}

/// The current wall clock in microseconds since the epoch.
///
/// The wall clock is the only clock two processes share, so it is what the
/// cross-process latency is measured with.
fn epoch_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// The CPU time the calling thread has burned, in microseconds.
///
/// The same helper as in `shm::ring`'s tests; the crate has no public API
/// for it, and it must not grow one for a test's sake.
#[cfg(windows)]
fn thread_cpu_micros() -> u64 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: all four out-parameters are live locals, and `GetCurrentThread`
    // returns the pseudo-handle of the calling thread, which is always valid
    // for `GetThreadTimes`.
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

/// The ring's metadata, as the two processes agree on it.
#[derive(Clone)]
struct Offer {
    name: String,
    generation: u64,
    size: usize,
}

/// A name no other ring in this session uses.
fn unique_name(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!(
        "{}-{tag}-{nanos:x}",
        ring_name(RingKind::Pcm, nanos ^ u64::from(std::process::id()))
    )
}

/// Read one `key=value` field out of a peer line.
fn field(line: &str, key: &str) -> u64 {
    let prefix = format!("{key}=");
    line.split_whitespace()
        .find_map(|f| f.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("`{line}` has no `{key}=` field"))
        .parse()
        .unwrap_or_else(|e| panic!("`{line}`: {key} is not a number: {e}"))
}

/// Collect a pipe's lines into a channel, so the parent never blocks on a
/// full pipe while the peer is writing.
fn collect(pipe: impl std::io::Read + Send + 'static) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// The peer process: the other end of the ring, in another process.
struct Peer {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    stderr: Receiver<String>,
}

impl Peer {
    /// Start the peer.
    ///
    /// `test_name` is the test the peer runs: this binary again, with the
    /// role in its environment, so the peer is always the same program as the
    /// parent and needs no separate build or staging.
    fn spawn(role: &str, test_name: &str, offer: &Offer) -> Result<Peer, String> {
        let executable = match std::env::var_os(PEER_BIN_ENV) {
            Some(path) => PathBuf::from(path),
            None => std::env::current_exe()
                .map_err(|e| format!("the test binary's own path is unknown: {e}"))?,
        };
        let mut command = Command::new(&executable);
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
            .args([
                "--ignored",
                "--exact",
                test_name,
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ROLE_ENV, role)
            .env(NAME_ENV, &offer.name)
            .env(CAPACITY_ENV, CAPACITY.to_string())
            .env(FRAME_SIZE_ENV, FRAME_SIZE.to_string())
            .env(GENERATION_ENV, offer.generation.to_string())
            .env(SIZE_ENV, offer.size.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|e| format!("could not start the peer {}: {e}", executable.display()))?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let lines = collect(child.stdout.take().expect("stdout is piped"));
        let stderr = collect(child.stderr.take().expect("stderr is piped"));
        Ok(Peer {
            child,
            stdin,
            lines,
            stderr,
        })
    }

    /// Send one command to the peer.
    fn send(&mut self, command: &str) -> Result<(), String> {
        writeln!(self.stdin, "{command}").map_err(|e| format!("`{command}`: stdin: {e}"))?;
        self.stdin
            .flush()
            .map_err(|e| format!("`{command}`: flush: {e}"))
    }

    /// Wait for the peer's next line that carries `marker`, and return the
    /// part of the line from the marker on.
    ///
    /// The marker is looked for **anywhere** in the line, not at its start:
    /// the peer is a test binary, and the test harness prints
    /// `test <name> ... ` without a newline before the test's own output
    /// runs, so the peer's first line is glued to that prefix.
    fn wait_for(&self, marker: &str, timeout: Duration) -> Result<String, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) if line.contains(marker) => {
                    let start = line.find(marker).expect("the marker is in the line");
                    return Ok(line[start..].to_owned());
                }
                Ok(line) if line.contains("FAILED") || line.contains("panicked") => {
                    return Err(format!("the peer's test failed: {line}"));
                }
                Ok(_) => continue,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!(
                        "the peer printed no `{marker}` line within {timeout:?}: {}",
                        self.stderr_text()
                    ));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!(
                        "the peer exited without a `{marker}` line: {}",
                        self.stderr_text()
                    ));
                }
            }
        }
    }

    /// Whatever the peer wrote to stderr so far.
    fn stderr_text(&self) -> String {
        let mut lines = Vec::new();
        while let Ok(line) = self.stderr.try_recv() {
            lines.push(line);
        }
        if lines.is_empty() {
            "(no output)".to_owned()
        } else {
            lines.join(" | ")
        }
    }

    /// Let the peer go and wait for its exit.
    fn finish(&mut self) -> Result<(), String> {
        let _ = self.send("QUIT");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(format!(
                            "the peer exited with {status}: {}",
                            self.stderr_text()
                        ))
                    };
                }
                Ok(None) if Instant::now() >= deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return Err("the peer did not exit within 10s".to_string());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(2)),
                Err(e) => return Err(format!("waiting for the peer failed: {e}")),
            }
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        // A peer that is still alive would keep the ring's section and events
        // open; it must not outlive the check.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Open a named region, retrying briefly while the peer is still starting.
fn open_region(name: &str, size: usize) -> NamedRegion {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match NamedRegion::open(name, size) {
            Ok(region) => return region,
            Err(e) => {
                if Instant::now() >= deadline {
                    panic!("could not open the section `{name}` within {START_TIMEOUT:?}: {e}");
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

/// Open a ring's wake events, retrying briefly while the peer is still
/// starting.
fn open_wake(name: &str) -> RingWake {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match RingWake::open(name) {
            Ok(wake) => return wake,
            Err(e) => {
                if Instant::now() >= deadline {
                    panic!("could not open the wake of `{name}` within {START_TIMEOUT:?}: {e}");
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

/// The peer's producer role: create the ring, offer it, publish on command.
fn peer_producer(offer: &Offer) {
    let region = NamedRegion::create(&offer.name, offer.size)
        .unwrap_or_else(|e| panic!("the peer could not create the section: {e}"));
    let producer = RingProducer::create(region, RingKind::Pcm, FRAME_SIZE, CAPACITY)
        .unwrap_or_else(|e| panic!("the peer could not create the ring: {e}"));
    let wake = RingWake::create(&offer.name)
        .unwrap_or_else(|e| panic!("the peer could not create the wake: {e}"));
    let mut producer = producer.with_wake(wake);
    let generation = producer.generation();

    print_line(&format!(
        "PEER_RING name={} generation={generation} size={}",
        offer.name, offer.size
    ));

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.expect("the peer's stdin");
        let mut parts = line.split_whitespace();
        match parts.next().unwrap_or("") {
            "PUBLISH" => {
                let frames: usize = parts
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("PUBLISH needs a frame count");
                let mut block = Vec::with_capacity(frames * CHANNELS);
                for index in 0..frames as u64 {
                    for channel in 0..CHANNELS {
                        block.push(pcm_sample(index, channel));
                    }
                }
                let started = epoch_micros();
                let written = producer
                    .write_from_blocking(&block, WAIT)
                    .unwrap_or_else(|e| panic!("the peer could not publish: {e}"));
                let done = epoch_micros();
                assert_eq!(
                    written, frames,
                    "the peer published {written} of {frames} frames"
                );
                print_line(&format!(
                    "PEER_PUBLISHED frames={frames} started_us={started} done_us={done}"
                ));
            }
            "QUIT" => break,
            other => panic!("unknown peer command `{other}`"),
        }
    }
}

/// The peer's consumer role: open the ring, drain on command, verify every
/// sample it read.
fn peer_consumer(offer: &Offer) {
    let region = open_region(&offer.name, offer.size);
    let expect = RingExpect::new(RingKind::Pcm, FRAME_SIZE, CAPACITY, offer.generation);
    let consumer = RingConsumer::attach(region, expect)
        .unwrap_or_else(|e| panic!("the peer could not attach to the ring: {e}"));
    let wake = open_wake(&offer.name);
    let mut consumer = consumer.with_wake(wake);

    print_line("PEER_READY");

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.expect("the peer's stdin");
        let mut parts = line.split_whitespace();
        match parts.next().unwrap_or("") {
            "DRAIN" => {
                let frames: usize = parts
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("DRAIN needs a frame count");
                let mut buffer = vec![0.0f32; frames * CHANNELS];
                let started = epoch_micros();
                let read = consumer
                    .read_into_blocking(&mut buffer, WAIT)
                    .unwrap_or_else(|e| panic!("the peer could not read: {e}"));
                let done = epoch_micros();
                assert_eq!(read, frames, "the peer read {read} of {frames} frames");
                for index in 0..frames {
                    for channel in 0..CHANNELS {
                        let got = buffer[index * CHANNELS + channel];
                        let want = pcm_sample(index as u64, channel);
                        assert_eq!(
                            got, want,
                            "the peer read frame {index} channel {channel}: expected {want}, got {got}"
                        );
                    }
                }
                print_line(&format!(
                    "PEER_READ frames={frames} started_us={started} done_us={done}"
                ));
            }
            "QUIT" => break,
            other => panic!("unknown peer command `{other}`"),
        }
    }
}

/// Print one line for the parent to read.
fn print_line(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// Run the peer role this process was started for, if it was started as one.
///
/// The peer is this same test binary, started again with a role in its
/// environment. A process without the role is the parent and runs the check.
fn peer_role() -> Option<String> {
    std::env::var(ROLE_ENV).ok()
}

/// The offer the parent hands to a peer that opens the ring.
fn offer_from_env() -> Offer {
    Offer {
        name: std::env::var(NAME_ENV).expect("the peer needs the ring's name"),
        generation: std::env::var(GENERATION_ENV)
            .expect("the peer needs the ring's generation")
            .parse()
            .expect("the ring's generation is a number"),
        size: std::env::var(SIZE_ENV)
            .expect("the peer needs the mapping's size")
            .parse()
            .expect("the mapping's size is a number"),
    }
}

// ---------------------------------------------------------------------------
// The checks
// ---------------------------------------------------------------------------

/// A consumer blocked in an empty-ring read is woken by a producer in the
/// other process, and the wake is bounded and free of busy-spinning.
#[test]
#[ignore = "spawns a process; Hyper-V VM only"]
fn cross_process_consumer_is_woken_by_the_remote_producer() {
    if let Some(role) = peer_role() {
        assert_eq!(role, ROLE_PRODUCER, "this check's peer produces");
        peer_producer(&offer_from_env());
        return;
    }
    require_test_vm();

    let name = unique_name("consumer-woken");
    // The peer creates the ring and the wake (RFC 0007 §5.2: the creator
    // creates the events, the opener opens them), so the offer starts out
    // empty and the peer's own line carries the real generation.
    let mut peer = Peer::spawn(
        ROLE_PRODUCER,
        CONSUMER_TEST_NAME,
        &Offer {
            name: name.clone(),
            generation: 0,
            size: RING_DATA_OFFSET + CAPACITY as usize * FRAME_SIZE as usize,
        },
    )
    .expect("the peer must start");

    let ring_line = peer
        .wait_for("PEER_RING ", START_TIMEOUT)
        .expect("the peer must offer the ring");
    let generation = field(&ring_line, "generation");
    let size = field(&ring_line, "size") as usize;

    let region = open_region(&name, size);
    let expect = RingExpect::new(RingKind::Pcm, FRAME_SIZE, CAPACITY, generation);
    let consumer = RingConsumer::attach(region, expect).expect("the parent must attach");
    let wake = open_wake(&name);
    let mut consumer = consumer.with_wake(wake);

    // The consumer blocks on an empty ring. The announce wait is the proof
    // that it really waits: a bounded read on the empty ring that returns
    // `Ok(0)` entered the kernel wait with the latch clear.
    let (blocked_tx, blocked_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut out = [0.0f32; CHANNELS];
        let cpu_before = thread_cpu_micros();
        let entered = epoch_micros();
        let started = Instant::now();
        let probe = consumer
            .read_into_blocking(&mut out, Some(ANNOUNCE_WAIT))
            .expect("the announce read must not fail");
        blocked_tx
            .send(probe)
            .expect("the parent must still be listening");
        let read = consumer
            .read_into_blocking(&mut out, WAIT)
            .expect("the blocking read must not fail");
        let released = epoch_micros();
        let wall = started.elapsed();
        let cpu = thread_cpu_micros().saturating_sub(cpu_before);
        (read, out, entered, released, wall, cpu)
    });

    let probe = blocked_rx.recv_timeout(START_TIMEOUT).unwrap_or_else(|e| {
        panic!(
            "the consumer never reached its wait ({e:?}); if the channel closed, the \
             consumer thread panicked and its message is in the test output above"
        )
    });
    assert_eq!(
        probe, 0,
        "the ring was not empty: the consumer found data before it could block"
    );

    // The publish follows the command, not the delay; the delay only widens
    // the window the thread's CPU time is measured over.
    std::thread::sleep(PUBLISH_DELAY);
    peer.send("PUBLISH 1")
        .expect("the peer must be told to publish");
    let published = peer
        .wait_for("PEER_PUBLISHED ", WAIT.unwrap())
        .expect("the peer must publish");

    let (read, out, entered, released, wall, cpu) =
        worker.join().expect("the consumer thread must not panic");

    assert_eq!(
        read, 1,
        "the consumer must be woken with exactly one frame, got {read}"
    );
    for (channel, got) in out.iter().enumerate().take(CHANNELS) {
        assert_eq!(
            *got,
            pcm_sample(0, channel),
            "the frame the consumer read does not match the one the producer published"
        );
    }

    // The two processes' clocks agree, and the publish happened inside the
    // window the consumer was blocked in.
    let started = field(&published, "started_us");
    assert!(
        started >= entered,
        "the publish at {started} us precedes the consumer's entry at {entered} us: the \
         processes do not share a clock"
    );
    assert!(
        released >= started,
        "the consumer was released at {released} us, before the publish at {started} us"
    );

    // The wake latency is bounded.
    let latency = released - started;
    assert!(
        latency <= WAKE_LATENCY_BOUND.as_micros() as u64,
        "the consumer was released {latency} us after the producer published, which is \
         beyond the {WAKE_LATENCY_BOUND:?} bound"
    );

    // The consumer slept: it burned no CPU while it waited.
    assert!(
        cpu * 4 < wall.as_micros() as u64,
        "the blocked consumer burned {cpu} us of CPU over {wall:?} of wall clock: it \
         polled instead of sleeping"
    );
    assert!(
        cpu < CPU_BOUND_MICROS,
        "the blocked consumer burned {cpu} us of CPU; a wait in the kernel costs next to \
         nothing"
    );

    println!(
        "cross-process wake: {latency} us of latency, {cpu} us of CPU over {wall:?} of \
         wall clock"
    );

    peer.finish().expect("the peer must end cleanly");
}

/// A producer blocked on a full ring is woken by a consumer in the other
/// process, and the wake is bounded and free of busy-spinning.
#[test]
#[ignore = "spawns a process; Hyper-V VM only"]
fn cross_process_producer_is_woken_by_the_remote_consumer() {
    if let Some(role) = peer_role() {
        assert_eq!(role, ROLE_CONSUMER, "this check's peer consumes");
        peer_consumer(&offer_from_env());
        return;
    }
    require_test_vm();

    let name = unique_name("producer-woken");
    let size = RING_DATA_OFFSET + CAPACITY as usize * FRAME_SIZE as usize;
    // The parent creates the ring here, so it creates the wake as well
    // (RFC 0007 §5.2) and the peer opens both by name.
    let region = NamedRegion::create(&name, size).expect("the parent must create the section");
    let producer =
        RingProducer::create(region, RingKind::Pcm, FRAME_SIZE, CAPACITY).expect("the ring");
    let wake = RingWake::create(&name).expect("the parent must create the wake");
    let mut producer = producer.with_wake(wake);
    let generation = producer.generation();

    let mut peer = Peer::spawn(
        ROLE_CONSUMER,
        PRODUCER_TEST_NAME,
        &Offer {
            name: name.clone(),
            generation,
            size,
        },
    )
    .expect("the peer must start");
    peer.wait_for("PEER_READY", START_TIMEOUT)
        .expect("the peer must attach to the ring");

    // Fill the ring, so the producer has to wait for space.
    let mut block = Vec::with_capacity(CAPACITY as usize * CHANNELS);
    for index in 0..u64::from(CAPACITY) {
        for channel in 0..CHANNELS {
            block.push(pcm_sample(index, channel));
        }
    }
    assert_eq!(
        producer
            .write_from(&block)
            .expect("the parent must fill the ring"),
        CAPACITY as usize
    );
    assert_eq!(producer.free_frames(), 0, "the ring is full");

    let (blocked_tx, blocked_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let extra = [
            pcm_sample(u64::from(CAPACITY), 0),
            pcm_sample(u64::from(CAPACITY), 1),
        ];
        let cpu_before = thread_cpu_micros();
        let entered = epoch_micros();
        let started = Instant::now();
        // The announce is the proof that the producer really waits: a bounded
        // write on the full ring that returns `Ok(0)` waited on `Wake::Space`
        // and wrote nothing.
        let probe = producer
            .write_from_blocking(&extra, Some(ANNOUNCE_WAIT))
            .expect("the announce write must not fail");
        blocked_tx
            .send(probe)
            .expect("the parent must still be listening");
        let written = producer
            .write_from_blocking(&extra, WAIT)
            .expect("the blocking write must not fail");
        let released = epoch_micros();
        let wall = started.elapsed();
        let cpu = thread_cpu_micros().saturating_sub(cpu_before);
        let dropped = producer.frames_dropped();
        (written, extra, entered, released, wall, cpu, dropped)
    });

    let probe = blocked_rx.recv_timeout(START_TIMEOUT).unwrap_or_else(|e| {
        panic!(
            "the producer never reached its wait ({e:?}); if the channel closed, the \
             producer thread panicked and its message is in the test output above"
        )
    });
    assert_eq!(
        probe, 0,
        "the ring was not full: the producer found space before it could block"
    );

    std::thread::sleep(PUBLISH_DELAY);
    peer.send(&format!("DRAIN {CAPACITY}"))
        .expect("the peer must be told to drain");
    let drained = peer
        .wait_for("PEER_READ ", WAIT.unwrap())
        .expect("the peer must drain the ring");

    let (written, extra, entered, released, wall, cpu, dropped) =
        worker.join().expect("the producer thread must not panic");

    assert_eq!(
        written, 1,
        "the producer must be woken and write exactly one frame, got {written}"
    );
    assert_eq!(dropped, 0, "a blocking write never drops");
    assert_eq!(field(&drained, "frames"), u64::from(CAPACITY));

    let started = field(&drained, "started_us");
    assert!(
        started >= entered,
        "the drain at {started} us precedes the producer's entry at {entered} us: the \
         processes do not share a clock"
    );
    assert!(
        released >= started,
        "the producer was released at {released} us, before the drain at {started} us"
    );

    let latency = released - started;
    assert!(
        latency <= WAKE_LATENCY_BOUND.as_micros() as u64,
        "the producer was released {latency} us after the consumer drained, which is \
         beyond the {WAKE_LATENCY_BOUND:?} bound"
    );

    assert!(
        cpu * 4 < wall.as_micros() as u64,
        "the blocked producer burned {cpu} us of CPU over {wall:?} of wall clock: it \
         polled instead of sleeping"
    );
    assert!(
        cpu < CPU_BOUND_MICROS,
        "the blocked producer burned {cpu} us of CPU; a wait in the kernel costs next to \
         nothing"
    );

    println!(
        "cross-process space wake: {latency} us of latency, {cpu} us of CPU over {wall:?} \
         of wall clock; the producer wrote {extra:?}"
    );

    peer.finish().expect("the peer must end cleanly");
}
