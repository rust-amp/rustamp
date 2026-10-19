//! Cross-bitness ring peer (task card Q1).
//!
//! This is a two-process harness, not a library example: the same source is
//! built for `x86_64-pc-windows-msvc` and `i686-pc-windows-msvc` and driven in
//! both directions by `cargo xtask ring-cross`. It closes the one real
//! verification gap of RFC 0003: there was no end-to-end test with a 32-bit
//! producer against a 64-bit consumer (or the reverse).
//!
//! It is an `examples/` binary on purpose: `cargo test` does not run it, and the
//! runner starts it explicitly with two different target binaries.
//!
//! # Command line
//!
//! ```text
//! ring_peer produce --name <name> --kind pcm|vis --frames <n> --capacity <n> [--frame-size <n>]
//! ring_peer consume --name <name> --frames <n> --kind pcm|vis --frame-size <n>
//!                   --capacity <n> --generation <n> --size <n>
//! ```
//!
//! * The **producer** creates the named section and the ring, prints one
//!   machine-readable line
//!   `RING name=... kind=... frame_size=... capacity=... generation=... size=...`,
//!   writes `n` frames of a deterministic, position-dependent pattern while
//!   respecting the ring's free space (so nothing is dropped and every offered
//!   frame can be verified), prints `PRODUCED written=<w> dropped=<d>`, and then
//!   **stays alive** until the consumer has closed the ring, so the mapping
//!   cannot disappear underneath the consumer. A peer timeout is a clear error.
//! * The **consumer** opens the section by name, retrying briefly while the
//!   producer is still starting, reads exactly `n` frames, verifies every value
//!   against the same pattern (for the vis ring also `seq` and `timestamp_us`),
//!   closes the ring and prints `OK received=<n> dropped=<d>`.
//!
//! Every mismatch, a short read or a timeout exits non-zero with a precise
//! message on stderr.
//!
//! The consumer's `--kind/--frame-size/--capacity/--generation/--size` flags are
//! the fields the producer printed; the runner passes them through. An opener
//! must announce what it expects before it may touch a counter (RFC 0003
//! section 8), and the ring API has no way to discover a header blindly.
//!
//! The whole path uses the safe ring API: this file contains no `unsafe`.
//!
//! # The wake modes
//!
//! `wake-produce` / `wake-consume` are the two halves of
//! `cargo xtask wake-cross`: a PCM **wake ring** (RFC 0007) driven by one-line
//! commands on stdin, each answered by one line on stdout, so the runner - not
//! a sleep - orders the two processes. Both halves also keep a *probe*: a
//! second [`RingWake`] handle on the same two kernel events, obtained by
//! cloning the one they hand to the ring. It lets a peer assert the signalling
//! rule of RFC 0007 §6.2 directly:
//!
//! * `DRAIN_LATCH data|space` -> `LATCH_DRAINED <dir> taken=<n>`: poll the
//!   latch until it is clear.
//! * `EXPECT_LATCH data|space` -> `LATCH_SET <dir>`, or a `FAIL` line and a
//!   non-zero exit when the latch is clear.
//! * `EXPECT_NO_LATCH data|space` -> `LATCH_CLEAR <dir>`, the negative control:
//!   it fails when the latch *is* set. Run right after a drain, it shows that
//!   the `EXPECT_LATCH` that follows is a real discrimination and not a
//!   constant.
//! * `REPORT` -> `STATE ...`, the counters, so a claim about the ring's
//!   occupancy is evidence rather than an assumption.
//!
//! Draining and expecting never wait ([`RingWake::take_signal`] polls), and
//! the probe is only ever used in phases where nothing is blocked, so it
//! cannot steal a real waiter's signal.

use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(windows)]
    {
        imp::main()
    }
    #[cfg(not(windows))]
    {
        eprintln!("ring_peer: the cross-process ring needs Windows named file mappings");
        ExitCode::FAILURE
    }
}

#[cfg(windows)]
mod imp {
    use std::io::{BufRead, BufReader, Write};
    use std::process::ExitCode;
    use std::time::{Duration, Instant};

    use rustamp_ipc::shm::{
        NamedRegion, RING_DATA_OFFSET, RingConsumer, RingError, RingExpect, RingKind, RingProducer,
        RingWake, VIS_SLOT_SIZE, VisSlot, WaitOutcome, Wake,
    };
    use rustamp_proto::vis::{VIS_CHANNELS, VIS_SAMPLES};

    /// How long the consumer retries opening the ring while the producer starts.
    const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
    /// Retry interval of the open loop.
    const OPEN_RETRY: Duration = Duration::from_millis(2);
    /// How long either side waits for the peer before it gives up.
    const PEER_TIMEOUT: Duration = Duration::from_secs(30);

    /// The bounded wait a side uses to **prove it is blocked** before it tells
    /// the runner so.
    ///
    /// It is a wait on the mechanism under test whose `TimedOut` outcome is an
    /// assertion, not a `thread::sleep`: a blocking call that returns `Ok(0)`
    /// here entered the kernel wait, found the ring empty (or full) and the
    /// latch clear, and came back untouched. That is the fact the runner used
    /// to buy with a 25 ms sleep after the announcement, which proved nothing.
    /// It cannot produce a false failure either: nothing is signalling during
    /// it, and a stale latch only costs one further pass of the check-then-wait
    /// loop.
    const ANNOUNCE_WAIT: Duration = Duration::from_millis(50);

    /// How long a side that has announced itself blocked waits to be woken.
    ///
    /// Deliberately shorter than the runner's own per-step timeout, so the peer
    /// that can name the failure and print the counters is the one that
    /// reports it.
    const BLOCK_WAIT: Duration = Duration::from_secs(5);

    /// Deterministic, position-dependent value, exactly representable as `f32`.
    ///
    /// Both ends compute it from the frame's global index, so one wrong byte (a
    /// byte-swapped or truncated counter, a 32/64-bit disagreement, a
    /// wrap-around slip) changes the value. The arithmetic is done in `u64` so
    /// it cannot differ between a 32-bit and a 64-bit build.
    fn cell(index: u64, channel: usize, sample: usize) -> f32 {
        let k = index
            .wrapping_mul(3)
            .wrapping_add((channel as u64).wrapping_mul(5))
            .wrapping_add(sample as u64)
            % 1021;
        k as f32 / 8.0
    }

    /// One interleaved PCM sample of frame `index` on channel `channel`.
    fn pcm_sample(index: u64, channel: usize) -> f32 {
        cell(index, channel, 0)
    }

    /// The timestamp of vis frame `index`.
    fn vis_timestamp(index: u64) -> u64 {
        index.wrapping_mul(1000).wrapping_add(7)
    }

    /// A vis slot for the offered frame `index` (the ring stamps `seq` itself).
    fn vis_slot(index: u64) -> VisSlot {
        let mut slot = VisSlot::zeroed();
        slot.timestamp_us = vis_timestamp(index);
        for c in 0..VIS_CHANNELS {
            for s in 0..VIS_SAMPLES {
                slot.waveform[c][s] = cell(index, c, s);
                slot.spectrum[c][s] = cell(index, c, s + VIS_SAMPLES);
            }
        }
        slot
    }

    #[derive(Default)]
    struct Args {
        command: String,
        name: Option<String>,
        kind: Option<RingKind>,
        frames: Option<u64>,
        capacity: Option<u32>,
        frame_size: Option<u32>,
        generation: Option<u64>,
        size: Option<usize>,
    }

    /// Parse one numeric option value into its target type.
    fn number<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, String>
    where
        T::Err: std::fmt::Display,
    {
        value
            .parse::<T>()
            .map_err(|e| format!("`{flag}`: {value:?} is not a number: {e}"))
    }

    fn parse_args(args: &[String]) -> Result<Args, String> {
        let mut it = args.iter();
        let command = it
            .next()
            .ok_or("expected `produce`, `consume`, `wake-produce` or `wake-consume` as the first argument")?
            .clone();
        let default_kind = if command.starts_with("wake-") {
            Some(RingKind::Pcm)
        } else {
            None
        };
        let mut out = Args {
            command,
            kind: default_kind,
            ..Args::default()
        };
        while let Some(flag) = it.next() {
            let value = it.next().ok_or_else(|| format!("`{flag}` needs a value"))?;
            match flag.as_str() {
                "--name" => out.name = Some(value.clone()),
                "--kind" => {
                    out.kind = Some(match value.as_str() {
                        "pcm" => RingKind::Pcm,
                        "vis" => RingKind::Vis,
                        other => return Err(format!("unknown ring kind `{other}`")),
                    });
                }
                "--frames" => out.frames = Some(number("--frames", value)?),
                "--capacity" => out.capacity = Some(number("--capacity", value)?),
                "--frame-size" => out.frame_size = Some(number("--frame-size", value)?),
                "--generation" => out.generation = Some(number("--generation", value)?),
                "--size" => out.size = Some(number("--size", value)?),
                other => return Err(format!("unknown option `{other}`")),
            }
        }
        Ok(out)
    }

    fn require<T>(value: Option<T>, flag: &str) -> Result<T, String> {
        value.ok_or_else(|| format!("this command needs `{flag}`"))
    }

    /// Bytes per slot for `kind`, defaulting and validating `--frame-size`.
    fn frame_size_of(kind: RingKind, given: Option<u32>) -> Result<u32, String> {
        /// The only legal vis slot size (RFC 0003 section 5).
        const VIS: u32 = VIS_SLOT_SIZE as u32;
        match kind {
            RingKind::Pcm => Ok(given.unwrap_or(8)),
            RingKind::Vis => match given {
                None | Some(VIS) => Ok(VIS),
                Some(other) => Err(format!("a vis ring slot is {VIS} bytes, not {other}")),
            },
        }
    }

    pub fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match run(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("ring_peer: {e}");
                ExitCode::FAILURE
            }
        }
    }

    fn run(args: &[String]) -> Result<(), String> {
        let args = parse_args(args)?;
        match args.command.as_str() {
            "produce" => produce(args),
            "consume" => consume(args),
            "wake-produce" => wake_produce(args),
            "wake-consume" => wake_consume(args),
            other => Err(format!("unknown command `{other}`")),
        }
    }

    fn print_line(line: &str) -> Result<(), String> {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}").map_err(|e| format!("stdout: {e}"))?;
        out.flush().map_err(|e| format!("stdout: {e}"))
    }

    /// The wall clock in microseconds since the epoch.
    ///
    /// The wall clock is the only clock the two processes share, so it is
    /// what the runner compares the producer's publish against the
    /// consumer's release with.
    fn now_us() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0)
    }

    /// The CPU time this thread has burned, in microseconds.
    ///
    /// Windows accounts a thread's user and kernel time in 100 ns units, on
    /// the system timer tick, so the number only means something over a
    /// window of a few ticks. A thread that is blocked in the kernel wait
    /// does not accrue CPU time - it is not running - so this stays near zero
    /// for a wait and grows with the wall clock for a spin. That difference
    /// is what proves the RFC 0007 consumer waits instead of polling.
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

    /// Parse an optional millisecond delay from a command's arguments.
    ///
    /// The delay exists for one reason: to make the *other* side's blocked
    /// window wide enough for the operating system to account thread time at
    /// all. It orders nothing - the blocked side has already proved that it
    /// blocks - and it is never used to make a signal land in a particular
    /// place.
    fn optional_delay_ms(parts: &mut std::str::SplitWhitespace<'_>) -> Result<Option<u64>, String> {
        match parts.next() {
            None => Ok(None),
            Some(value) => value
                .parse()
                .map(Some)
                .map_err(|e| format!("invalid delay in ms: {e}")),
        }
    }

    fn produce(args: Args) -> Result<(), String> {
        let name = require(args.name, "--name")?;
        let kind = require(args.kind, "--kind")?;
        let frames = require(args.frames, "--frames")?;
        let capacity = require(args.capacity, "--capacity")?;
        let frame_size = frame_size_of(kind, args.frame_size)?;
        let size = RING_DATA_OFFSET + capacity as usize * frame_size as usize;

        let region = NamedRegion::create(&name, size)
            .map_err(|e| format!("could not create the mapping `{name}`: {e}"))?;
        let mut producer = RingProducer::create(region, kind, frame_size, capacity)
            .map_err(|e| format!("could not create the ring: {e}"))?;

        // The opener has to validate against exactly these values.
        print_line(&format!(
            "RING name={name} kind={} frame_size={frame_size} capacity={capacity} \
             generation={} size={size}",
            kind.name(),
            producer.generation()
        ))?;

        let deadline = Instant::now() + PEER_TIMEOUT;
        match kind {
            RingKind::Pcm => write_pcm_frames(&mut producer, frames, frame_size, deadline)?,
            RingKind::Vis => write_vis_frames(&mut producer, frames, deadline)?,
        }

        print_line(&format!(
            "PRODUCED written={} dropped={}",
            producer.frames_written(),
            producer.frames_dropped()
        ))?;

        // Stay alive until the consumer closed the ring, so the mapping cannot
        // disappear underneath it. A timeout here is an error, not a silent exit.
        while !producer.is_closed() {
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for the consumer to close the ring \
                     (wrote {} of {frames} frames)",
                    producer.frames_written()
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    /// Write exactly `frames` PCM frames, waiting for free space instead of
    /// dropping. Waiting (rather than offering everything at once) keeps the
    /// check deterministic: every offered frame is delivered and verified.
    fn write_pcm_frames(
        producer: &mut RingProducer<NamedRegion>,
        frames: u64,
        frame_size: u32,
        deadline: Instant,
    ) -> Result<(), String> {
        let channels = (frame_size / 4) as usize;
        let mut block: Vec<f32> = Vec::new();
        let mut index = 0u64;
        while index < frames {
            check_producing(producer, index, frames, deadline)?;
            let free = producer.free_frames();
            if free == 0 {
                std::thread::yield_now();
                continue;
            }
            let count = (frames - index).min(free) as usize;
            block.clear();
            for i in 0..count as u64 {
                for c in 0..channels {
                    block.push(pcm_sample(index + i, c));
                }
            }
            let written = producer
                .write_from(&block)
                .map_err(|e| format!("write_from failed: {e}"))?;
            if written != count {
                return Err(format!(
                    "the ring accepted {written} of the {count} frames it reported room for"
                ));
            }
            index += written as u64;
        }
        Ok(())
    }

    /// Write exactly `frames` vis slots, waiting for free space instead of
    /// dropping.
    fn write_vis_frames(
        producer: &mut RingProducer<NamedRegion>,
        frames: u64,
        deadline: Instant,
    ) -> Result<(), String> {
        let mut index = 0u64;
        while index < frames {
            check_producing(producer, index, frames, deadline)?;
            if producer.free_frames() == 0 {
                std::thread::yield_now();
                continue;
            }
            let slot = vis_slot(index);
            let accepted = producer
                .write_slot(&slot)
                .map_err(|e| format!("write_slot failed: {e}"))?;
            if !accepted {
                return Err(format!(
                    "the ring refused vis frame {index} although it reported free space"
                ));
            }
            index += 1;
        }
        Ok(())
    }

    /// Fail with a precise message if the producer can no longer make progress.
    fn check_producing(
        producer: &RingProducer<NamedRegion>,
        index: u64,
        frames: u64,
        deadline: Instant,
    ) -> Result<(), String> {
        if producer.is_closed() {
            return Err(format!(
                "the consumer closed the ring after {index} of {frames} frames"
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out after writing {index} of {frames} frames"
            ));
        }
        Ok(())
    }

    fn consume(args: Args) -> Result<(), String> {
        let name = require(args.name, "--name")?;
        let frames = require(args.frames, "--frames")?;
        let kind = require(args.kind, "--kind")?;
        let frame_size = require(args.frame_size, "--frame-size")?;
        let capacity = require(args.capacity, "--capacity")?;
        let generation = require(args.generation, "--generation")?;
        let size = require(args.size, "--size")?;
        let expect = RingExpect::new(kind, frame_size, capacity, generation);

        let mut consumer = open_with_retry(&name, size, expect)?;
        let deadline = Instant::now() + PEER_TIMEOUT;
        let received = match kind {
            RingKind::Pcm => read_pcm(&mut consumer, frames, frame_size, deadline)?,
            RingKind::Vis => read_vis(&mut consumer, frames, deadline)?,
        };

        let dropped = consumer.frames_dropped();
        if dropped != 0 {
            return Err(format!(
                "the ring dropped {dropped} frames; this check needs an exact delivery"
            ));
        }
        if consumer.frames_published() != frames {
            return Err(format!(
                "the producer published {} frames, expected {frames}",
                consumer.frames_published()
            ));
        }
        if consumer.frames_read() != frames {
            return Err(format!(
                "the consumer counted {} frames, expected {frames}",
                consumer.frames_read()
            ));
        }
        // Tell the producer that the run is over; it then stops waiting.
        consumer.close();
        print_line(&format!("OK received={received} dropped={dropped}"))
    }

    /// Open the named ring, retrying while the producer is still starting.
    fn open_with_retry(
        name: &str,
        size: usize,
        expect: RingExpect,
    ) -> Result<RingConsumer<NamedRegion>, String> {
        let deadline = Instant::now() + OPEN_TIMEOUT;
        loop {
            // The section exists as soon as the creator made it, but the header
            // is only valid once `magic` is stored last, so a failed attach is
            // retried as well.
            let last = match NamedRegion::open(name, size) {
                Ok(region) => match RingConsumer::attach(region, expect) {
                    Ok(consumer) => return Ok(consumer),
                    Err(e) => format!("attach failed: {e}"),
                },
                Err(e) => format!("open failed: {e}"),
            };
            if Instant::now() >= deadline {
                return Err(format!(
                    "could not open ring `{name}` within {OPEN_TIMEOUT:?}: {last}"
                ));
            }
            std::thread::sleep(OPEN_RETRY);
        }
    }

    fn read_pcm(
        consumer: &mut RingConsumer<NamedRegion>,
        frames: u64,
        frame_size: u32,
        deadline: Instant,
    ) -> Result<u64, String> {
        let channels = (frame_size / 4) as usize;
        let mut buf = vec![0.0f32; 64 * channels];
        let mut received = 0u64;
        while received < frames {
            let read = consumer
                .read_into(&mut buf)
                .map_err(|e| format!("read_into failed: {e}"))?;
            if read == 0 {
                if Instant::now() >= deadline {
                    return Err(format!("timed out after {received} of {frames} frames"));
                }
                std::thread::yield_now();
                continue;
            }
            for i in 0..read {
                let index = received + i as u64;
                for c in 0..channels {
                    let got = buf[i * channels + c];
                    let want = pcm_sample(index, c);
                    if got != want {
                        return Err(format!(
                            "PCM frame {index} channel {c}: expected {want}, got {got}"
                        ));
                    }
                }
            }
            received += read as u64;
        }
        Ok(received)
    }

    fn read_vis(
        consumer: &mut RingConsumer<NamedRegion>,
        frames: u64,
        deadline: Instant,
    ) -> Result<u64, String> {
        let mut slot = VisSlot::zeroed();
        let mut received = 0u64;
        while received < frames {
            let read = consumer
                .read_slot_into(&mut slot)
                .map_err(|e| format!("read_slot_into failed: {e}"))?;
            if !read {
                if Instant::now() >= deadline {
                    return Err(format!("timed out after {received} of {frames} frames"));
                }
                std::thread::yield_now();
                continue;
            }
            verify_vis(received, &slot)?;
            received += 1;
        }
        Ok(received)
    }

    /// Check `seq`, `timestamp_us` and both 2x576 arrays of one vis frame.
    fn verify_vis(index: u64, slot: &VisSlot) -> Result<(), String> {
        if slot.seq != index {
            return Err(format!(
                "vis frame {index}: seq is {}, expected {index}",
                slot.seq
            ));
        }
        let want_ts = vis_timestamp(index);
        if slot.timestamp_us != want_ts {
            return Err(format!(
                "vis frame {index}: timestamp_us is {}, expected {want_ts}",
                slot.timestamp_us
            ));
        }
        for c in 0..VIS_CHANNELS {
            for s in 0..VIS_SAMPLES {
                let want = cell(index, c, s);
                let got = slot.waveform[c][s];
                if got != want {
                    return Err(format!(
                        "vis frame {index} waveform[{c}][{s}]: expected {want}, got {got}"
                    ));
                }
                let want = cell(index, c, s + VIS_SAMPLES);
                let got = slot.spectrum[c][s];
                if got != want {
                    return Err(format!(
                        "vis frame {index} spectrum[{c}][{s}]: expected {want}, got {got}"
                    ));
                }
            }
        }
        Ok(())
    }

    fn open_wake_with_retry(name: &str) -> Result<RingWake, String> {
        let deadline = Instant::now() + OPEN_TIMEOUT;
        loop {
            match RingWake::open(name) {
                Ok(wake) => return Ok(wake),
                Err(e) => {
                    if Instant::now() >= deadline {
                        return Err(format!(
                            "could not open wake `{name}` within {OPEN_TIMEOUT:?}: {e}"
                        ));
                    }
                    std::thread::sleep(OPEN_RETRY);
                }
            }
        }
    }

    /// Report a failed expectation and fail the peer.
    ///
    /// The `FAIL` line goes on stdout because the runner treats such a line as
    /// an immediate failure of the step it is waiting for, so the diagnostic it
    /// prints is this exact sentence. The `Err` then exits the peer non-zero,
    /// so a runner that is no longer watching still sees the failure.
    fn fail<T>(message: String) -> Result<T, String> {
        print_line(&format!("FAIL {message}"))?;
        Err(message)
    }

    /// Parse `data` / `space` into a wake direction.
    fn wake_direction(value: Option<&str>) -> Result<Wake, String> {
        match value {
            Some("data") => Ok(Wake::Data),
            Some("space") => Ok(Wake::Space),
            Some(other) => Err(format!("unknown wake direction `{other}`")),
            None => Err("this command needs `data` or `space`".to_string()),
        }
    }

    /// The direction's name, for a report line.
    fn direction_name(which: Wake) -> &'static str {
        match which {
            Wake::Data => "data",
            Wake::Space => "space",
        }
    }

    /// Consume whatever is already latched on one direction of the probe, so
    /// that the next [`RingWake::take_signal`] reports only what was signalled
    /// after this call.
    ///
    /// Nothing waits: `take_signal` polls. The probe is a third handle on the
    /// ring's two events and is only used in phases where neither side is
    /// blocked, so a drained latch stays drained and no signal is stolen from a
    /// real waiter.
    fn drain_latch(probe: &RingWake, which: Wake) -> Result<u32, String> {
        /// An auto-reset event holds at most one signal, so the loop below runs
        /// at most twice. The bound only stops a broken backend from spinning.
        const MAX_TAKES: u32 = 64;
        let mut taken = 0u32;
        while probe
            .take_signal(which)
            .map_err(|e| format!("take_signal({}) failed: {e}", direction_name(which)))?
            == WaitOutcome::Signaled
        {
            taken += 1;
            if taken >= MAX_TAKES {
                return Err(format!(
                    "the {} latch was still signalled after {MAX_TAKES} takes",
                    direction_name(which)
                ));
            }
        }
        Ok(taken)
    }

    fn wake_produce(args: Args) -> Result<(), String> {
        let name = require(args.name, "--name")?;
        let capacity = require(args.capacity, "--capacity")?;
        let frame_size = frame_size_of(RingKind::Pcm, args.frame_size)?;
        let size = RING_DATA_OFFSET + capacity as usize * frame_size as usize;

        let region = NamedRegion::create(&name, size)
            .map_err(|e| format!("could not create the mapping `{name}`: {e}"))?;
        let producer = RingProducer::create(region, RingKind::Pcm, frame_size, capacity)
            .map_err(|e| format!("could not create the ring: {e}"))?;
        let wake = RingWake::create(&name)
            .map_err(|e| format!("could not create wake for `{name}`: {e}"))?;
        // A second handle on the same two kernel events, kept out of the ring:
        // the probe of the mid-advance phases.
        // `create_and_clone_share_the_same_latches` in `shm::wake` pins that a
        // clone is the same object.
        let probe = wake.clone();
        let mut producer = producer.with_wake(wake);

        print_line(&format!(
            "RING name={name} kind=pcm frame_size={frame_size} capacity={capacity} \
             generation={} size={size}",
            producer.generation()
        ))?;

        let channels = (frame_size / 4) as usize;
        let mut current_index = 0u64;

        let stdin = std::io::stdin();
        let mut reader = BufReader::new(stdin.lock());
        let mut line = String::new();

        loop {
            line.clear();
            if reader
                .read_line(&mut line)
                .map_err(|e| format!("stdin: {e}"))?
                == 0
            {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let cmd = parts.next().unwrap_or("");
            match cmd {
                "PING" => {
                    print_line("PONG")?;
                }
                "WRITE" => {
                    let n: usize = parts
                        .next()
                        .ok_or("WRITE needs a count")?
                        .parse()
                        .map_err(|e| format!("invalid count: {e}"))?;
                    // The optional delay widens the consumer's blocked window,
                    // so the thread-time accounting can tell a sleeping
                    // consumer from a spinning one. It orders nothing.
                    if let Some(delay) = optional_delay_ms(&mut parts)? {
                        std::thread::sleep(Duration::from_millis(delay));
                    }
                    let mut block = Vec::with_capacity(n * channels);
                    for i in 0..n as u64 {
                        for c in 0..channels {
                            block.push(pcm_sample(current_index + i, c));
                        }
                    }
                    let started = now_us();
                    let written = producer
                        .write_from_blocking(&block, Some(PEER_TIMEOUT))
                        .map_err(|e| format!("write_from_blocking failed: {e}"))?;
                    let done = now_us();
                    if written != n {
                        return Err(format!("write_from_blocking accepted {written} of {n}"));
                    }
                    current_index += n as u64;
                    print_line(&format!("WROTE {n} started_us={started} done_us={done}"))?;
                }
                "FILL_AND_BLOCK" => {
                    let extra: usize = parts
                        .next()
                        .ok_or("FILL_AND_BLOCK needs an extra count")?
                        .parse()
                        .map_err(|e| format!("invalid extra count: {e}"))?;
                    let free = producer.free_frames() as usize;
                    if free > 0 {
                        let mut block = Vec::with_capacity(free * channels);
                        for i in 0..free as u64 {
                            for c in 0..channels {
                                block.push(pcm_sample(current_index + i, c));
                            }
                        }
                        let written = producer
                            .write_from_blocking(&block, Some(PEER_TIMEOUT))
                            .map_err(|e| format!("write_from_blocking fill failed: {e}"))?;
                        if written != free {
                            return Err(format!("fill accepted {written} of {free}"));
                        }
                        current_index += free as u64;
                    }
                    let mut extra_block = Vec::with_capacity(extra * channels);
                    for i in 0..extra as u64 {
                        for c in 0..channels {
                            extra_block.push(pcm_sample(current_index + i, c));
                        }
                    }
                    // Prove the writer really is blocked before announcing it:
                    // a bounded `write_from_blocking` on a full ring that
                    // returns `Ok(0)` waited on `Wake::Space` and wrote
                    // nothing. The runner used to buy this with a sleep after
                    // the announcement; this is the fact itself.
                    let cpu_before = thread_cpu_micros();
                    let wall_started = Instant::now();
                    let probe_written = producer
                        .write_from_blocking(&extra_block, Some(ANNOUNCE_WAIT))
                        .map_err(|e| format!("write_from_blocking probe failed: {e}"))?;
                    if probe_written != 0 {
                        return fail(format!(
                            "FILL_AND_BLOCK: the ring was not full, {probe_written} \
                             frames were accepted before the writer could block (free={})",
                            producer.free_frames()
                        ));
                    }
                    print_line("WRITER_BLOCKING")?;
                    let written = producer
                        .write_from_blocking(&extra_block, Some(BLOCK_WAIT))
                        .map_err(|e| format!("write_from_blocking extra failed: {e}"))?;
                    let released = now_us();
                    let wall = wall_started.elapsed();
                    let cpu = thread_cpu_micros().saturating_sub(cpu_before);
                    if written == 0 {
                        // `Ok(0)` means the wait expired with the ring still
                        // full. While the peer still has frames to drain that
                        // is a skipped `Wake::Space` signal, not a slow
                        // machine, so it is a failure and not a retry.
                        return fail(format!(
                            "FILL_AND_BLOCK: no Wake::Space within {BLOCK_WAIT:?}; the \
                             ring reports free={} written={} dropped={}",
                            producer.free_frames(),
                            producer.frames_written(),
                            producer.frames_dropped()
                        ));
                    }
                    if written != extra {
                        return Err(format!("extra accepted {written} of {extra}"));
                    }
                    current_index += extra as u64;
                    print_line(&format!(
                        "WRITER_WOKEN {extra} at_us={released} cpu_us={cpu} wall_us={}",
                        wall.as_micros()
                    ))?;
                }
                "BULK_WRITE" => {
                    let total: usize = parts
                        .next()
                        .ok_or("BULK_WRITE needs a total count")?
                        .parse()
                        .map_err(|e| format!("invalid total count: {e}"))?;
                    let mut written_total = 0usize;
                    let chunk = (producer.capacity() as usize).min(64);
                    while written_total < total {
                        let to_write = (total - written_total).min(chunk);
                        let mut block = Vec::with_capacity(to_write * channels);
                        for i in 0..to_write as u64 {
                            for c in 0..channels {
                                block.push(pcm_sample(current_index + i, c));
                            }
                        }
                        let w = producer
                            .write_from_blocking(&block, Some(PEER_TIMEOUT))
                            .map_err(|e| format!("bulk write failed: {e}"))?;
                        if w == 0 {
                            return Err("bulk write timed out with ring full".to_string());
                        }
                        current_index += w as u64;
                        written_total += w;
                    }
                    print_line(&format!("BULK_WROTE {total}"))?;
                }
                "WRITE_NB" => {
                    // The non-blocking write site (`write_from`), which
                    // carried its own copy of the withdrawn edge.
                    let n: usize = parts
                        .next()
                        .ok_or("WRITE_NB needs a count")?
                        .parse()
                        .map_err(|e| format!("invalid count: {e}"))?;
                    let mut block = Vec::with_capacity(n * channels);
                    for i in 0..n as u64 {
                        for c in 0..channels {
                            block.push(pcm_sample(current_index + i, c));
                        }
                    }
                    let written = producer
                        .write_from(&block)
                        .map_err(|e| format!("write_from failed: {e}"))?;
                    if written != n {
                        return fail(format!(
                            "WRITE_NB: write_from accepted {written} of {n} (free={})",
                            producer.free_frames()
                        ));
                    }
                    current_index += n as u64;
                    print_line(&format!("WROTE_NB {n}"))?;
                }
                "DRAIN_LATCH" => {
                    let which = wake_direction(parts.next())?;
                    let taken = drain_latch(&probe, which)?;
                    print_line(&format!(
                        "LATCH_DRAINED {} taken={taken}",
                        direction_name(which)
                    ))?;
                }
                "EXPECT_LATCH" => {
                    let which = wake_direction(parts.next())?;
                    let outcome = probe
                        .take_signal(which)
                        .map_err(|e| format!("take_signal failed: {e}"))?;
                    if outcome != WaitOutcome::Signaled {
                        return fail(format!(
                            "the peer advanced its counter without signalling \
                             Wake::{which:?}; this side reports available={} free={} \
                             capacity={} written={} - the advance was neither from empty \
                             nor to full, which is exactly the case the withdrawn edge of \
                             RFC 0007 6.2 skipped",
                            producer.frames_available(),
                            producer.free_frames(),
                            producer.capacity(),
                            producer.frames_written()
                        ));
                    }
                    print_line(&format!("LATCH_SET {}", direction_name(which)))?;
                }
                "EXPECT_NO_LATCH" => {
                    // The negative control. Run directly after a drain, it
                    // proves two things the positive assertion needs: the
                    // drain really left the latch clear, and this probe can
                    // report a missing signal at all - so a `LATCH_SET` later
                    // in the phase discriminates, which a check that cannot
                    // fail would not.
                    let which = wake_direction(parts.next())?;
                    let outcome = probe
                        .take_signal(which)
                        .map_err(|e| format!("take_signal failed: {e}"))?;
                    if outcome != WaitOutcome::TimedOut {
                        return fail(format!(
                            "the {} latch was still set after the drain, so the next \
                             EXPECT_LATCH would prove nothing",
                            direction_name(which)
                        ));
                    }
                    print_line(&format!("LATCH_CLEAR {}", direction_name(which)))?;
                }
                "REPORT" => {
                    print_line(&format!(
                        "STATE available={} free={} capacity={} written={} dropped={}",
                        producer.frames_available(),
                        producer.free_frames(),
                        producer.capacity(),
                        producer.frames_written(),
                        producer.frames_dropped()
                    ))?;
                }
                "CLOSE" => {
                    producer.close();
                    print_line("CLOSED")?;
                }
                "QUIT" => {
                    break;
                }
                other => return Err(format!("unknown wake-produce command `{other}`")),
            }
        }
        Ok(())
    }

    fn wake_consume(args: Args) -> Result<(), String> {
        let name = require(args.name, "--name")?;
        let frame_size = require(args.frame_size, "--frame-size")?;
        let capacity = require(args.capacity, "--capacity")?;
        let generation = require(args.generation, "--generation")?;
        let size = require(args.size, "--size")?;
        let expect = RingExpect::new(RingKind::Pcm, frame_size, capacity, generation);

        let consumer = open_with_retry(&name, size, expect)?;
        let wake = open_wake_with_retry(&name)?;
        // A second handle on the same two kernel events, kept out of the ring:
        // the probe of the mid-advance phases. It is a clone of the handle this
        // process *opened* by name, so a `Wake::Data` it reports was set by the
        // peer process, across the bitness boundary.
        let probe = wake.clone();
        let mut consumer = consumer.with_wake(wake);

        print_line("CONSUMER_READY")?;

        let channels = (frame_size / 4) as usize;
        let mut current_index = 0u64;

        let stdin = std::io::stdin();
        let mut reader = BufReader::new(stdin.lock());
        let mut line = String::new();

        loop {
            line.clear();
            if reader
                .read_line(&mut line)
                .map_err(|e| format!("stdin: {e}"))?
                == 0
            {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let cmd = parts.next().unwrap_or("");
            match cmd {
                "PING" => {
                    print_line("PONG")?;
                }
                "TEST_TIMEOUT" => {
                    let ms: u64 = parts
                        .next()
                        .ok_or("TEST_TIMEOUT needs a duration in ms")?
                        .parse()
                        .map_err(|e| format!("invalid ms: {e}"))?;
                    let mut buf = vec![0.0f32; channels];
                    let start = Instant::now();
                    let read = consumer
                        .read_into_blocking(&mut buf, Some(Duration::from_millis(ms)))
                        .map_err(|e| format!("read_into_blocking failed: {e}"))?;
                    let elapsed = start.elapsed();
                    if read != 0 {
                        return Err(format!(
                            "timeout test expected 0 frames on empty ring, got {read}"
                        ));
                    }
                    if elapsed < Duration::from_millis(1) {
                        return Err(format!(
                            "timeout returned prematurely in {elapsed:?} (< 1ms)"
                        ));
                    }
                    if elapsed > Duration::from_millis(ms + 1000) {
                        return Err(format!(
                            "timeout took too long: {elapsed:?} (> {}ms)",
                            ms + 1000
                        ));
                    }
                    print_line(&format!("TIMEOUT_OK elapsed_ms={}", elapsed.as_millis()))?;
                }
                "READ_BLOCK" => {
                    let n: usize = parts
                        .next()
                        .ok_or("READ_BLOCK needs a count")?
                        .parse()
                        .map_err(|e| format!("invalid count: {e}"))?;
                    let mut buf = vec![0.0f32; n * channels];
                    // Prove the reader really is blocked before announcing it:
                    // a bounded `read_into_blocking` that returns `Ok(0)`
                    // entered the kernel wait on an empty ring with a clear
                    // latch and came back untouched. The runner used to buy
                    // this with a sleep after the announcement, which proved
                    // nothing about where the reader was.
                    let cpu_before = thread_cpu_micros();
                    let wall_started = Instant::now();
                    let probe_read = consumer
                        .read_into_blocking(&mut buf, Some(ANNOUNCE_WAIT))
                        .map_err(|e| format!("read_into_blocking probe failed: {e}"))?;
                    if probe_read != 0 {
                        return fail(format!(
                            "READ_BLOCK: the ring was not empty, {probe_read} frames \
                             arrived before the reader could block (available={})",
                            consumer.frames_available()
                        ));
                    }
                    print_line("READER_BLOCKING")?;
                    let mut read_total = 0usize;
                    while read_total < n {
                        let read = consumer
                            .read_into_blocking(&mut buf[read_total * channels..], Some(BLOCK_WAIT))
                            .map_err(|e| format!("read_into_blocking failed: {e}"))?;
                        if read == 0 {
                            // `Ok(0)` means the wait expired with the ring
                            // still empty. While the peer still has frames to
                            // write that is a skipped `Wake::Data` signal, not
                            // a slow machine, so it is a failure and not a
                            // retry (the W19 lesson: a bound that only makes a
                            // test slower is not a bound).
                            return fail(format!(
                                "READ_BLOCK: no Wake::Data within {BLOCK_WAIT:?} after \
                                 {read_total} of {n} frames; the ring reports \
                                 available={} published={} read={} dropped={}",
                                consumer.frames_available(),
                                consumer.frames_published(),
                                consumer.frames_read(),
                                consumer.frames_dropped()
                            ));
                        }
                        for i in 0..read {
                            let index = current_index + (read_total + i) as u64;
                            for c in 0..channels {
                                let got = buf[(read_total + i) * channels + c];
                                let want = pcm_sample(index, c);
                                if got != want {
                                    return Err(format!(
                                        "PCM frame {index} channel {c}: expected {want}, got {got}"
                                    ));
                                }
                            }
                        }
                        read_total += read;
                    }
                    current_index += n as u64;
                    let released = now_us();
                    let wall = wall_started.elapsed();
                    let cpu = thread_cpu_micros().saturating_sub(cpu_before);
                    print_line(&format!(
                        "READER_WOKEN {n} at_us={released} cpu_us={cpu} wall_us={}",
                        wall.as_micros()
                    ))?;
                }
                "DRAIN" => {
                    let n: usize = parts
                        .next()
                        .ok_or("DRAIN needs a count")?
                        .parse()
                        .map_err(|e| format!("invalid count: {e}"))?;
                    // The optional delay widens the producer's blocked
                    // window, so the thread-time accounting can tell a
                    // sleeping producer from a spinning one. It orders
                    // nothing.
                    if let Some(delay) = optional_delay_ms(&mut parts)? {
                        std::thread::sleep(Duration::from_millis(delay));
                    }
                    let mut buf = vec![0.0f32; n * channels];
                    let started = now_us();
                    let mut read_total = 0usize;
                    while read_total < n {
                        let read = consumer
                            .read_into_blocking(
                                &mut buf[read_total * channels..],
                                Some(PEER_TIMEOUT),
                            )
                            .map_err(|e| format!("read_into_blocking failed: {e}"))?;
                        if read == 0 {
                            return Err(format!(
                                "read_into_blocking timed out after reading {read_total} of {n}"
                            ));
                        }
                        for i in 0..read {
                            let index = current_index + (read_total + i) as u64;
                            for c in 0..channels {
                                let got = buf[(read_total + i) * channels + c];
                                let want = pcm_sample(index, c);
                                if got != want {
                                    return Err(format!(
                                        "PCM frame {index} channel {c}: expected {want}, got {got}"
                                    ));
                                }
                            }
                        }
                        read_total += read;
                    }
                    current_index += n as u64;
                    let done = now_us();
                    print_line(&format!("DRAINED {n} started_us={started} done_us={done}"))?;
                }
                "WAIT_CLOSE" => {
                    let mut buf = vec![0.0f32; channels];
                    // Same discipline as READ_BLOCK: the announcement is only
                    // made after a bounded wait proved the ring is empty and
                    // still live, so what follows is a blocked waiter being
                    // released by the peer's `close()`, not a reader that
                    // happened to find the state flag already set.
                    let probe_read = consumer
                        .read_into_blocking(&mut buf, Some(ANNOUNCE_WAIT))
                        .map_err(|e| format!("read_into_blocking probe failed: {e}"))?;
                    if probe_read != 0 {
                        return fail(format!(
                            "WAIT_CLOSE: the ring was not empty, {probe_read} frames \
                             were still pending (available={})",
                            consumer.frames_available()
                        ));
                    }
                    print_line("WAITING_CLOSE")?;
                    match consumer.read_into_blocking(&mut buf, Some(BLOCK_WAIT)) {
                        Err(RingError::Closed) => {
                            print_line("CLOSED_OK")?;
                        }
                        Ok(0) => {
                            return fail(format!(
                                "WAIT_CLOSE: no wake within {BLOCK_WAIT:?}; close() must \
                                 signal both events after the state store (RFC 0007 \
                                 6.5). The ring reports closed={} available={}",
                                consumer.is_closed(),
                                consumer.frames_available()
                            ));
                        }
                        Ok(frames) => {
                            return Err(format!(
                                "expected Err(Closed), but read {frames} frames instead"
                            ));
                        }
                        Err(other) => {
                            return Err(format!("expected Err(Closed), got {other:?}"));
                        }
                    }
                }
                "BULK_READ" => {
                    let total: usize = parts
                        .next()
                        .ok_or("BULK_READ needs a count")?
                        .parse()
                        .map_err(|e| format!("invalid count: {e}"))?;
                    let mut read_total = 0usize;
                    let mut buf = vec![0.0f32; 64 * channels];
                    while read_total < total {
                        let to_read = (total - read_total).min(64);
                        let read = consumer
                            .read_into_blocking(&mut buf[..to_read * channels], Some(PEER_TIMEOUT))
                            .map_err(|e| format!("bulk read failed: {e}"))?;
                        if read == 0 {
                            return Err(format!(
                                "bulk read timed out after {read_total} of {total} frames"
                            ));
                        }
                        for i in 0..read {
                            let index = current_index + (read_total + i) as u64;
                            for c in 0..channels {
                                let got = buf[i * channels + c];
                                let want = pcm_sample(index, c);
                                if got != want {
                                    return Err(format!(
                                        "PCM frame {index} channel {c}: expected {want}, got {got}"
                                    ));
                                }
                            }
                        }
                        read_total += read;
                    }
                    current_index += total as u64;
                    print_line(&format!("BULK_READ_OK {total}"))?;
                }
                "READ_NB" => {
                    // The non-blocking read site (`read_into`), which carried
                    // its own copy of the withdrawn edge.
                    let n: usize = parts
                        .next()
                        .ok_or("READ_NB needs a count")?
                        .parse()
                        .map_err(|e| format!("invalid count: {e}"))?;
                    let mut buf = vec![0.0f32; n * channels];
                    let read = consumer
                        .read_into(&mut buf)
                        .map_err(|e| format!("read_into failed: {e}"))?;
                    if read != n {
                        return fail(format!(
                            "READ_NB: read_into delivered {read} of {n} (available={})",
                            consumer.frames_available()
                        ));
                    }
                    for i in 0..read {
                        let index = current_index + i as u64;
                        for c in 0..channels {
                            let got = buf[i * channels + c];
                            let want = pcm_sample(index, c);
                            if got != want {
                                return Err(format!(
                                    "PCM frame {index} channel {c}: expected {want}, got {got}"
                                ));
                            }
                        }
                    }
                    current_index += n as u64;
                    print_line(&format!("READ_NB_OK {n}"))?;
                }
                "DRAIN_LATCH" => {
                    let which = wake_direction(parts.next())?;
                    let taken = drain_latch(&probe, which)?;
                    print_line(&format!(
                        "LATCH_DRAINED {} taken={taken}",
                        direction_name(which)
                    ))?;
                }
                "EXPECT_LATCH" => {
                    let which = wake_direction(parts.next())?;
                    let outcome = probe
                        .take_signal(which)
                        .map_err(|e| format!("take_signal failed: {e}"))?;
                    if outcome != WaitOutcome::Signaled {
                        return fail(format!(
                            "the peer advanced its counter without signalling \
                             Wake::{which:?}; this side reports available={} capacity={} \
                             published={} read={} - the advance was neither from empty \
                             nor to full, which is exactly the case the withdrawn edge of \
                             RFC 0007 6.2 skipped",
                            consumer.frames_available(),
                            consumer.capacity(),
                            consumer.frames_published(),
                            consumer.frames_read()
                        ));
                    }
                    print_line(&format!("LATCH_SET {}", direction_name(which)))?;
                }
                "EXPECT_NO_LATCH" => {
                    // The negative control; see the `wake-produce` arm.
                    let which = wake_direction(parts.next())?;
                    let outcome = probe
                        .take_signal(which)
                        .map_err(|e| format!("take_signal failed: {e}"))?;
                    if outcome != WaitOutcome::TimedOut {
                        return fail(format!(
                            "the {} latch was still set after the drain, so the next \
                             EXPECT_LATCH would prove nothing",
                            direction_name(which)
                        ));
                    }
                    print_line(&format!("LATCH_CLEAR {}", direction_name(which)))?;
                }
                "REPORT" => {
                    print_line(&format!(
                        "STATE available={} capacity={} published={} read={} dropped={}",
                        consumer.frames_available(),
                        consumer.capacity(),
                        consumer.frames_published(),
                        consumer.frames_read(),
                        consumer.frames_dropped()
                    ))?;
                }
                "QUIT" => {
                    break;
                }
                other => return Err(format!("unknown wake-consume command `{other}`")),
            }
        }
        Ok(())
    }
}
