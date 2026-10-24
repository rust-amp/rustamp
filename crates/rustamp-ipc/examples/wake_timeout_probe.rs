//! Headless probe for the blocking-wait timeout behaviour (RFC 0007 §6.4,
//! task card G5).
//!
//! Task card G3 claimed a VM measurement — a 10 ms blocking wait returning
//! `Ok(0)` in ~7.8 ms — that left no artefact in the tree, and W10 finding F7
//! showed the card's tick mechanism was self-inconsistent. This probe is the
//! reproducible replacement: it drives the real mechanism —
//! `RingConsumer::read_into_blocking` on an empty wake ring, the exact path
//! the claim is about — for several durations and many runs, and reports the
//! elapsed time of each distribution (min / median / max) together with the
//! system timer resolution the wait is quantized against.
//!
//! Run it on any Windows machine (host or test VM):
//!
//! ```text
//! cargo run -p rustamp-ipc --example wake_timeout_probe
//! ```
//!
//! It opens no window, plays no audio, sends no input and takes no
//! arguments. It is a measurement, not a gate: every run must return `Ok(0)`
//! on the empty ring, and any other outcome exits non-zero with the failing
//! run named. The numbers it prints are the OS behaviour, not a pass/fail
//! bound; interpret them against the reported timer resolution.

use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(windows)]
    {
        imp::main()
    }
    #[cfg(not(windows))]
    {
        eprintln!("wake_timeout_probe: the blocking wake needs Windows named events");
        ExitCode::FAILURE
    }
}

#[cfg(windows)]
mod imp {
    use std::process::ExitCode;
    use std::time::{Duration, Instant};

    use rustamp_ipc::shm::{
        AnonymousRegion, RING_DATA_OFFSET, RingConsumer, RingKind, RingProducer, RingWake,
        ring_name,
    };

    /// Durations probed: the G3 set (10 / 30 / 60 ms) plus 1 ms, the smallest
    /// timeout the mechanism can request, and a comfortably multi-tick 100 ms.
    const DURATIONS_MS: [u64; 5] = [1, 10, 30, 60, 100];
    /// Runs per duration.
    const RUNS: usize = 25;

    pub fn main() -> ExitCode {
        match run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("wake_timeout_probe: {e}");
                ExitCode::FAILURE
            }
        }
    }

    fn run() -> Result<(), String> {
        // A stereo PCM wake ring. The producer exists but never writes, so
        // every read blocks for the full timeout and returns Ok(0).
        const CAPACITY: u32 = 8;
        const FRAME_SIZE: u32 = 8; // two interleaved f32 channels
        let region =
            AnonymousRegion::new(RING_DATA_OFFSET + CAPACITY as usize * FRAME_SIZE as usize)
                .map_err(|e| format!("could not allocate the ring region: {e}"))?;
        let producer = RingProducer::create(region.clone(), RingKind::Pcm, FRAME_SIZE, CAPACITY)
            .map_err(|e| format!("could not create the ring: {e}"))?;
        let wake = RingWake::create(&ring_name(RingKind::Pcm, producer.generation()))
            .map_err(|e| format!("could not create the wake events: {e}"))?;
        let producer = producer.with_wake(wake.clone());
        let mut consumer = RingConsumer::attach(region, producer.expect())
            .map_err(|e| format!("could not attach the consumer: {e}"))?
            .with_wake(wake);

        println!(
            "wake_timeout_probe: rustamp-ipc blocking-wait timeout probe \
             (RFC 0007 §6.4, task G5)"
        );
        println!(
            "os={} arch={}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        print_timer_resolution("before");
        println!(
            "ring: pcm capacity={CAPACITY} frame_size={FRAME_SIZE} wake=attached \
             empty=yes producer_writes=no"
        );
        println!("path: RingConsumer::read_into_blocking on an empty wake ring");
        println!("duration_ms runs min_ms median_ms max_ms early_runs");

        let mut out = vec![0.0f32; CAPACITY as usize * (FRAME_SIZE as usize / 4)];
        let mut total_waits = 0usize;
        let mut total_early = 0usize;
        for &ms in &DURATIONS_MS {
            let requested = Duration::from_millis(ms);
            let mut samples: Vec<Duration> = Vec::with_capacity(RUNS);
            let mut early = 0usize;
            for run in 0..RUNS {
                let start = Instant::now();
                let read = consumer
                    .read_into_blocking(&mut out, Some(requested))
                    .map_err(|e| format!("run {run} of {ms} ms: read_into_blocking: {e}"))?;
                let elapsed = start.elapsed();
                if read != 0 {
                    return Err(format!(
                        "run {run} of {ms} ms: expected Ok(0) on an empty ring, got {read} frames"
                    ));
                }
                if elapsed < requested {
                    early += 1;
                }
                samples.push(elapsed);
            }
            samples.sort_unstable();
            let median = samples[RUNS / 2];
            let ms_of = |d: Duration| format!("{:.3}", d.as_secs_f64() * 1000.0);
            println!(
                "{ms} {RUNS} {} {} {} {early}",
                ms_of(samples[0]),
                ms_of(median),
                ms_of(samples[RUNS - 1])
            );
            total_waits += RUNS;
            total_early += early;
        }
        print_timer_resolution("after");
        println!(
            "summary: {total_waits} waits, {total_early} expired before the requested duration"
        );
        println!("probe: done");
        Ok(())
    }

    /// Print the system timer resolution, in 100 ns units and in ms, in the
    /// parameter order of the API (maximum, minimum, current).
    fn print_timer_resolution(when: &str) {
        match timer_resolution() {
            Ok((maximum, minimum, current)) => println!(
                "timer_resolution_ms ({when}): maximum={:.3} minimum={:.3} current={:.3} \
                 (NtQueryTimerResolution, 100 ns units: {maximum} {minimum} {current})",
                maximum as f64 / 10_000.0,
                minimum as f64 / 10_000.0,
                current as f64 / 10_000.0
            ),
            Err(e) => println!("timer_resolution_ms ({when}): unavailable ({e})"),
        }
    }

    /// The system timer resolution in 100 ns units, in the API's parameter
    /// order (maximum, minimum, current).
    fn timer_resolution() -> Result<(u32, u32, u32), String> {
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn NtQueryTimerResolution(
                maximum: *mut u32,
                minimum: *mut u32,
                current: *mut u32,
            ) -> i32;
        }

        let (mut maximum, mut minimum, mut current) = (0u32, 0u32, 0u32);
        // SAFETY: the three out-parameters are valid, aligned `u32` slots that
        // outlive the call, and the status is checked before they are read.
        let status = unsafe { NtQueryTimerResolution(&mut maximum, &mut minimum, &mut current) };
        if status != 0 {
            return Err(format!(
                "NtQueryTimerResolution failed with status {status:#010x}"
            ));
        }
        Ok((maximum, minimum, current))
    }
}
