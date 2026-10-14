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
    use std::io::Write;
    use std::process::ExitCode;
    use std::time::{Duration, Instant};

    use rustamp_ipc::shm::{
        NamedRegion, RING_DATA_OFFSET, RingConsumer, RingExpect, RingKind, RingProducer,
        VIS_SLOT_SIZE, VisSlot,
    };
    use rustamp_proto::vis::{VIS_CHANNELS, VIS_SAMPLES};

    /// How long the consumer retries opening the ring while the producer starts.
    const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
    /// Retry interval of the open loop.
    const OPEN_RETRY: Duration = Duration::from_millis(2);
    /// How long either side waits for the peer before it gives up.
    const PEER_TIMEOUT: Duration = Duration::from_secs(30);

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
        let mut out = Args {
            command: it
                .next()
                .ok_or("expected `produce` or `consume` as the first argument")?
                .clone(),
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
            other => Err(format!("unknown command `{other}`")),
        }
    }

    fn print_line(line: &str) -> Result<(), String> {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}").map_err(|e| format!("stdout: {e}"))?;
        out.flush().map_err(|e| format!("stdout: {e}"))
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
}
