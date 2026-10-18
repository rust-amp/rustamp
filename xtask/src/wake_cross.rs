//! `cargo xtask wake-cross` - cross-bitness blocking wake check (RFC 0007).
//!
//! Drives a real two-process check between `x86_64-pc-windows-msvc` and
//! `i686-pc-windows-msvc` (in both directions) exercising the RFC 0007 blocking
//! wake mechanism:
//!
//! 1. **Timeout precision**: verifies a wait on an empty ring returns `TimedOut`
//!    honouring the requested duration within OS timer resolution bounds.
//! 2. **Blocked reader woken by writer**: reader blocks in `WaitForSingleObject`
//!    on `Wake::Data`; writer writes samples and signals `Wake::Data`; reader is
//!    woken across the process boundary and verifies the samples.
//! 3. **Blocked writer woken by reader**: writer fills the ring and blocks in
//!    `WaitForSingleObject` on `Wake::Space`; reader drains frames and signals
//!    `Wake::Space`; writer is woken and finishes publishing.
//! 4. **Bulk lossless delivery**: multi-frame transfer using `write_from_blocking`
//!    and `read_into_blocking` with zero drops and mathematical sample verification.
//! 5. **Mid-advance signalling, non-blocking sites**: the signalling rule of
//!    RFC 0007 §6.2 as corrected on 2026-10-18 - *every* advance signals, not
//!    only one that crossed the empty or the full boundary - proved across the
//!    process boundary for `write_from` and `read_into`.
//! 6. **Mid-advance signalling, blocking sites**: the same for
//!    `write_from_blocking` and `read_into_blocking`, which carried their own
//!    copy of the withdrawn edge.
//! 7. **Shutdown wake**: reader blocks on empty ring; peer calls `close()`; reader
//!    is woken immediately and returns `Err(RingError::Closed)`.
//!
//! # Why phases 5 and 6 exist
//!
//! Phases 1 to 4 and 7 all wake a waiter across an **edge**: the reader waits
//! only on an empty ring, so the write that wakes it starts from empty, and the
//! writer waits only on a full one, so the read that wakes it starts from full.
//! Every one of them therefore passes under the edge-triggered rule that RFC
//! 0007 §6.2 recommended until it was withdrawn - which is what this check did
//! when it was written (2026-10-17), one day before the edge was removed. A
//! proof of the ring as it actually is has to assert the part the edge skipped,
//! and no waiter can observe that part: the drain needs the condition unmet
//! (empty or full) while the advance under test needs it partly met.
//!
//! Phases 5 and 6 therefore ask the events directly, in the shape the in-process
//! test `every_advance_signals_not_only_the_edge` uses: each peer keeps a
//! **probe**, a second handle on the ring's two latches, drains one direction,
//! performs one advance in a ring that is neither empty nor full, and requires
//! that the latch is set again. The runner asserts the occupancy from the
//! peers' own `REPORT` lines first, so "neither empty nor full" is evidence.
//! Nothing blocks in these phases, so the probe cannot steal a waiter's signal,
//! and nothing sleeps: the signal is a `SetEvent` the ring made before the
//! command was acknowledged, so the probe's poll is already decided when it
//! runs.
//!
//! # No sleep orders anything here
//!
//! Every step is ordered by a line the peer printed after the operation it
//! describes returned. The two "I am blocked" announcements are ordered by a
//! *bounded wait that timed out* inside the peer (`ANNOUNCE_WAIT` in
//! `ring_peer`), which is positive evidence that the waiter sat in
//! `WaitForSingleObject` with the latch clear - not by a `thread::sleep` in the
//! runner, which is what this file used until this card and which proved
//! nothing. The only `sleep` left is the 2 ms backoff of `wait_timeout`, which
//! polls a child **process** for exit and orders nothing about the ring.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::{cargo, no_window_command};

const X64: &str = "x86_64-pc-windows-msvc";
const X86: &str = "i686-pc-windows-msvc";

/// The two cross-bitness directions tested.
const RUNS: [(&str, &str); 2] = [(X86, X64), (X64, X86)];

const START_TIMEOUT: Duration = Duration::from_secs(30);
const STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// Capacity of the wake ring every direction runs on, and the frame size.
///
/// The capacity is what makes the mid-advance phases meaningful: an advance has
/// to be able to happen with the ring neither empty nor full.
const CAPACITY: u32 = 16;
const FRAME_SIZE: u32 = 8;

/// How many phase rows one direction contributes.
const PHASES_PER_DIRECTION: usize = 7;

/// The direction of a wake, as the peer protocol spells it.
#[derive(Clone, Copy)]
enum Wake {
    Data,
    Space,
}

impl Wake {
    fn as_str(self) -> &'static str {
        match self {
            Wake::Data => "data",
            Wake::Space => "space",
        }
    }
}

struct Row {
    direction: String,
    phase: String,
    result: String,
}

struct Peers {
    x64: PathBuf,
    x86: PathBuf,
}

impl Peers {
    fn get(&self, triple: &str) -> &Path {
        if triple == X64 { &self.x64 } else { &self.x86 }
    }
}

/// `cargo xtask wake-cross`
pub fn run(args: &[String]) -> Result<(), String> {
    if let Some(extra) = args.first() {
        return Err(format!("unknown wake-cross option `{extra}`"));
    }
    if !cfg!(windows) {
        println!("wake-cross: SKIPPED - the cross-process wake needs Windows named events");
        return Ok(());
    }

    let root = rustamp_testkit::workspace_root();
    let target_dir = target_dir(&root);
    println!("wake-cross: building the peer for {X64} and {X86}");
    for triple in [X64, X86] {
        build_peer(&root, triple)?;
    }
    let peers = Peers {
        x64: peer_path(&target_dir, X64),
        x86: peer_path(&target_dir, X86),
    };
    for (triple, path) in [(X64, &peers.x64), (X86, &peers.x86)] {
        if !path.is_file() {
            return Err(format!(
                "the {triple} peer binary was not built: {}",
                path.display()
            ));
        }
    }

    let mut rows: Vec<Row> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for (producer_arch, consumer_arch) in RUNS {
        let direction = format!("{} -> {}", short(producer_arch), short(consumer_arch));
        let outcome = run_direction(
            peers.get(producer_arch),
            peers.get(consumer_arch),
            &direction,
            &mut rows,
        );
        if let Err(e) = outcome {
            failures.push(format!("{direction}: {e}"));
        }
    }

    print_table(&rows);

    let expected_steps = RUNS.len() * PHASES_PER_DIRECTION;
    if !failures.is_empty() {
        return Err(format!(
            "{} of {} directions failed:\n  - {}",
            failures.len(),
            RUNS.len(),
            failures.join("\n  - ")
        ));
    }
    if rows.len() != expected_steps {
        return Err(format!(
            "expected {expected_steps} phase results, but got {}",
            rows.len()
        ));
    }

    println!(
        "wake-cross: all {} checks passed ({} directions x {PHASES_PER_DIRECTION} wake phases)",
        rows.len(),
        RUNS.len()
    );
    Ok(())
}

fn print_table(rows: &[Row]) {
    println!();
    println!("{:<16} {:<16} result", "direction", "phase");
    for row in rows {
        println!("{:<16} {:<16} {}", row.direction, row.phase, row.result);
    }
    println!();
}

fn run_direction(
    producer_exe: &Path,
    consumer_exe: &Path,
    direction: &str,
    rows: &mut Vec<Row>,
) -> Result<(), String> {
    let name = unique_name();
    let mut producer = no_window_command(producer_exe)
        .args([
            "wake-produce",
            "--name",
            &name,
            "--capacity",
            &CAPACITY.to_string(),
            "--frame-size",
            &FRAME_SIZE.to_string(),
            "--frames",
            "512",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start producer {}: {e}", producer_exe.display()))?;

    let mut producer_in = producer.stdin.take().expect("stdin is piped");
    let producer_out = collect(producer.stdout.take().expect("stdout is piped"));
    let producer_err = collect(producer.stderr.take().expect("stderr is piped"));

    let ring_line = wait_for_line(&producer_out, "RING ", START_TIMEOUT)
        .map_err(|e| format!("producer failed to offer ring: {e}"))?;
    let meta = RingMeta::parse(&ring_line)?;

    let mut consumer = no_window_command(consumer_exe)
        .args([
            "wake-consume",
            "--name",
            &meta.name,
            "--capacity",
            &meta.capacity.to_string(),
            "--frame-size",
            &meta.frame_size.to_string(),
            "--generation",
            &meta.generation.to_string(),
            "--size",
            &meta.size.to_string(),
            "--frames",
            "512",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start consumer {}: {e}", consumer_exe.display()))?;

    let mut consumer_in = consumer.stdin.take().expect("stdin is piped");
    let consumer_out = collect(consumer.stdout.take().expect("stdout is piped"));
    let consumer_err = collect(consumer.stderr.take().expect("stderr is piped"));

    wait_for_line(&consumer_out, "CONSUMER_READY", START_TIMEOUT)
        .map_err(|e| format!("consumer failed to report ready: {e}"))?;

    let outcome = (|| -> Result<(), String> {
        // Phase 1: Timeout precision on empty ring
        writeln!(consumer_in, "TEST_TIMEOUT 50").map_err(|e| format!("stdin: {e}"))?;
        consumer_in.flush().map_err(|e| format!("flush: {e}"))?;
        let timeout_line = wait_for_line(&consumer_out, "TIMEOUT_OK", STEP_TIMEOUT)
            .map_err(|e| format!("STEP timeout FAILED: {e}"))?;
        let ms = field(&timeout_line, "elapsed_ms").unwrap_or(0);
        rows.push(Row {
            direction: direction.to_owned(),
            phase: "timeout".to_owned(),
            result: format!("OK (elapsed {ms}ms >= 1ms)"),
        });

        // Phase 2: Blocked reader woken by writer
        writeln!(consumer_in, "READ_BLOCK 16").map_err(|e| format!("stdin: {e}"))?;
        consumer_in.flush().map_err(|e| format!("flush: {e}"))?;
        // `READER_BLOCKING` is printed only after a bounded `read_into_blocking`
        // returned `Ok(0)`, i.e. the reader sat in the kernel wait on an empty
        // ring and came back untouched. That is the ordering fact; the runner
        // needs no sleep for it.
        wait_for_line(&consumer_out, "READER_BLOCKING", STEP_TIMEOUT)
            .map_err(|e| format!("STEP reader-wake FAILED (reader did not block): {e}"))?;
        writeln!(producer_in, "WRITE 16").map_err(|e| format!("stdin: {e}"))?;
        producer_in.flush().map_err(|e| format!("flush: {e}"))?;
        wait_for_line(&producer_out, "WROTE 16", STEP_TIMEOUT)
            .map_err(|e| format!("STEP reader-wake FAILED (producer write failed): {e}"))?;
        wait_for_line(&consumer_out, "READER_WOKEN 16", STEP_TIMEOUT).map_err(|e| {
            format!("STEP reader-wake FAILED (reader was not woken or data mismatch): {e}")
        })?;
        rows.push(Row {
            direction: direction.to_owned(),
            phase: "reader-wake".to_owned(),
            result: "OK (reader woken by writer, 16 frames verified)".to_owned(),
        });

        // Phase 3: Blocked writer woken by reader (Space wake)
        writeln!(producer_in, "FILL_AND_BLOCK 8").map_err(|e| format!("stdin: {e}"))?;
        producer_in.flush().map_err(|e| format!("flush: {e}"))?;
        // Same discipline: `WRITER_BLOCKING` follows a bounded
        // `write_from_blocking` that returned `Ok(0)` on a full ring.
        wait_for_line(&producer_out, "WRITER_BLOCKING", STEP_TIMEOUT).map_err(|e| {
            format!("STEP writer-wake FAILED (producer did not block on full ring): {e}")
        })?;
        writeln!(consumer_in, "DRAIN 16").map_err(|e| format!("stdin: {e}"))?;
        consumer_in.flush().map_err(|e| format!("flush: {e}"))?;
        wait_for_line(&consumer_out, "DRAINED 16", STEP_TIMEOUT)
            .map_err(|e| format!("STEP writer-wake FAILED (consumer failed to drain): {e}"))?;
        wait_for_line(&producer_out, "WRITER_WOKEN 8", STEP_TIMEOUT)
            .map_err(|e| format!("STEP writer-wake FAILED (writer was not woken on space): {e}"))?;
        writeln!(consumer_in, "DRAIN 8").map_err(|e| format!("stdin: {e}"))?;
        consumer_in.flush().map_err(|e| format!("flush: {e}"))?;
        wait_for_line(&consumer_out, "DRAINED 8", STEP_TIMEOUT)
            .map_err(|e| format!("STEP writer-wake FAILED (consumer failed to drain tail): {e}"))?;
        rows.push(Row {
            direction: direction.to_owned(),
            phase: "writer-wake".to_owned(),
            result: "OK (writer woken by reader, space wake verified)".to_owned(),
        });

        // Phase 4: Bulk lossless transfer
        writeln!(producer_in, "BULK_WRITE 256").map_err(|e| format!("stdin: {e}"))?;
        producer_in.flush().map_err(|e| format!("flush: {e}"))?;
        writeln!(consumer_in, "BULK_READ 256").map_err(|e| format!("stdin: {e}"))?;
        consumer_in.flush().map_err(|e| format!("flush: {e}"))?;
        wait_for_line(&producer_out, "BULK_WROTE 256", STEP_TIMEOUT)
            .map_err(|e| format!("STEP bulk-lossless FAILED (bulk write): {e}"))?;
        wait_for_line(&consumer_out, "BULK_READ_OK 256", STEP_TIMEOUT)
            .map_err(|e| format!("STEP bulk-lossless FAILED (bulk read): {e}"))?;
        rows.push(Row {
            direction: direction.to_owned(),
            phase: "bulk-lossless".to_owned(),
            result: "OK (256 frames delivered, 0 dropped)".to_owned(),
        });

        // Phase 5: every advance signals, non-blocking sites (`write_from`,
        // `read_into`). The bulk phase left the ring empty and both sides in
        // step, so the occupancies the peers report below are exact - and they
        // are asserted, not assumed.
        //
        // One frame first, so the ring is neither empty nor full for the
        // advance that is actually under test.
        step(&mut producer_in, &producer_out, "WRITE_NB 1", "WROTE_NB 1")
            .map_err(|e| format!("STEP mid-advance FAILED (priming write): {e}"))?;
        prove_mid_advance(
            Side {
                stdin: &mut consumer_in,
                out: &consumer_out,
            },
            "available",
            Side {
                stdin: &mut producer_in,
                out: &producer_out,
            },
            Advance {
                command: "WRITE_NB 1",
                ack: "WROTE_NB 1",
            },
            Wake::Data,
            "mid-advance",
        )?;
        prove_mid_advance(
            Side {
                stdin: &mut producer_in,
                out: &producer_out,
            },
            "free",
            Side {
                stdin: &mut consumer_in,
                out: &consumer_out,
            },
            Advance {
                command: "READ_NB 1",
                ack: "READ_NB_OK 1",
            },
            Wake::Space,
            "mid-advance",
        )?;
        rows.push(Row {
            direction: direction.to_owned(),
            phase: "mid-advance".to_owned(),
            result: "OK (write_from signals Data, read_into signals Space, off the edge)"
                .to_owned(),
        });

        // Phase 6: the same two assertions through the blocking methods, which
        // carried their own copy of the withdrawn edge. Neither call waits
        // here - each finds its condition already met - so what is under test
        // is their store-and-signal step, not their wait.
        prove_mid_advance(
            Side {
                stdin: &mut consumer_in,
                out: &consumer_out,
            },
            "available",
            Side {
                stdin: &mut producer_in,
                out: &producer_out,
            },
            Advance {
                command: "WRITE 1",
                ack: "WROTE 1",
            },
            Wake::Data,
            "mid-advance-blk",
        )?;
        prove_mid_advance(
            Side {
                stdin: &mut producer_in,
                out: &producer_out,
            },
            "free",
            Side {
                stdin: &mut consumer_in,
                out: &consumer_out,
            },
            Advance {
                command: "DRAIN 1",
                ack: "DRAINED 1",
            },
            Wake::Space,
            "mid-advance-blk",
        )?;
        // Leave the ring empty, so the close phase below is a waiter on an
        // empty ring and not the drain-then-EOS case of RFC 0007 6.5. The
        // peer's own counters say whether it worked.
        step(&mut consumer_in, &consumer_out, "DRAIN 1", "DRAINED 1")
            .map_err(|e| format!("STEP mid-advance-blk FAILED (tail drain): {e}"))?;
        let state = step(&mut consumer_in, &consumer_out, "REPORT", "STATE ")
            .map_err(|e| format!("STEP mid-advance-blk FAILED (final report): {e}"))?;
        let available = field(&state, "available")?;
        if available != 0 {
            return Err(format!(
                "STEP mid-advance-blk FAILED: the ring must be empty before the close \
                 phase, but `{state}` reports available={available}"
            ));
        }
        rows.push(Row {
            direction: direction.to_owned(),
            phase: "mid-advance-blk".to_owned(),
            result: "OK (the blocking pair signals off the edge too)".to_owned(),
        });

        // Phase 7: Close wakes blocked waiter with Closed
        writeln!(consumer_in, "WAIT_CLOSE").map_err(|e| format!("stdin: {e}"))?;
        consumer_in.flush().map_err(|e| format!("flush: {e}"))?;
        wait_for_line(&consumer_out, "WAITING_CLOSE", STEP_TIMEOUT)
            .map_err(|e| format!("STEP close-wake FAILED (consumer did not block): {e}"))?;
        writeln!(producer_in, "CLOSE").map_err(|e| format!("stdin: {e}"))?;
        producer_in.flush().map_err(|e| format!("flush: {e}"))?;
        wait_for_line(&producer_out, "CLOSED", STEP_TIMEOUT)
            .map_err(|e| format!("STEP close-wake FAILED (producer close): {e}"))?;
        wait_for_line(&consumer_out, "CLOSED_OK", STEP_TIMEOUT).map_err(|e| {
            format!("STEP close-wake FAILED (close did not wake consumer with Closed): {e}")
        })?;
        rows.push(Row {
            direction: direction.to_owned(),
            phase: "close-wake".to_owned(),
            result: "OK (close woke blocked consumer with Closed)".to_owned(),
        });

        let _ = writeln!(producer_in, "QUIT");
        let _ = producer_in.flush();
        let _ = writeln!(consumer_in, "QUIT");
        let _ = consumer_in.flush();

        let prod_status = wait_timeout(&mut producer, Duration::from_secs(5))?;
        let cons_status = wait_timeout(&mut consumer, Duration::from_secs(5))?;
        if !prod_status.success() {
            return Err(format!(
                "producer exited with error ({prod_status}): {}",
                join(&drain(&producer_err))
            ));
        }
        if !cons_status.success() {
            return Err(format!(
                "consumer exited with error ({cons_status}): {}",
                join(&drain(&consumer_err))
            ));
        }

        Ok(())
    })();

    if producer.try_wait().ok().flatten().is_none() {
        let _ = producer.kill();
        let _ = producer.wait();
    }
    if consumer.try_wait().ok().flatten().is_none() {
        let _ = consumer.kill();
        let _ = consumer.wait();
    }

    outcome
}

#[derive(Default)]
struct RingMeta {
    name: String,
    frame_size: u32,
    capacity: u32,
    generation: u64,
    size: usize,
}

impl RingMeta {
    fn parse(line: &str) -> Result<RingMeta, String> {
        let mut meta = RingMeta::default();
        for field in line.split_whitespace().skip(1) {
            let Some((key, value)) = field.split_once('=') else {
                return Err(format!("`{line}`: field `{field}` is not key=value"));
            };
            match key {
                "name" => meta.name = value.to_owned(),
                "frame_size" => meta.frame_size = number(line, key, value)?,
                "capacity" => meta.capacity = number(line, key, value)?,
                "generation" => meta.generation = number(line, key, value)?,
                "size" => meta.size = number(line, key, value)?,
                _ => {}
            }
        }
        if meta.name.is_empty() {
            return Err(format!("`{line}`: no ring name"));
        }
        Ok(meta)
    }
}

/// Send one command to a peer and wait for the line that acknowledges it.
///
/// The acknowledgement is printed by the peer *after* the operation returned,
/// which is what orders the two processes against each other. A `FAIL` line
/// from the peer short-circuits with the peer's own diagnostic
/// (see [`wait_for_line`]).
fn step(
    stdin: &mut impl Write,
    out: &Receiver<String>,
    command: &str,
    expect: &str,
) -> Result<String, String> {
    writeln!(stdin, "{command}").map_err(|e| format!("`{command}`: stdin: {e}"))?;
    stdin
        .flush()
        .map_err(|e| format!("`{command}`: flush: {e}"))?;
    wait_for_line(out, expect, STEP_TIMEOUT).map_err(|e| format!("`{command}`: {e}"))
}

/// One side of the ring, as the runner talks to it.
struct Side<'a> {
    stdin: &'a mut std::process::ChildStdin,
    out: &'a Receiver<String>,
}

/// The advance under test: the command that performs it and the line the peer
/// prints once it has returned.
struct Advance<'a> {
    command: &'a str,
    ack: &'a str,
}

/// Prove one direction of RFC 0007 §6.2 across the process boundary: an advance
/// that starts away from both the empty and the full boundary still signals.
///
/// `probe` is the side that waits on `which` and holds the third handle on the
/// latches; `actor` is the side in the *other* process that performs the
/// advance. `occupancy_key` is the counter on `probe`'s `STATE` line
/// (`available` for the consumer, `free` for the producer) that has to be
/// strictly between 0 and `capacity`.
///
/// The five steps, and what each is for:
///
/// 1. **drain** - the probe polls its latch clear, so what follows is fresh.
/// 2. **the negative control** - require that the latch now reads *clear*.
///    Without it step 5 could be a check that cannot fail: a drain that did
///    nothing, or a probe that answered `Signaled` unconditionally, would
///    satisfy it. This exercises the other branch of the same assertion over
///    the same handle.
/// 3. **the premise** - the probe's own counters must show a ring that is
///    neither empty nor full, because an advance at either boundary would be
///    signalled by the withdrawn edge too and would prove nothing.
/// 4. **the advance**, in the other process, acknowledged only after the ring
///    call returned - so the `SetEvent` it owes has already happened or never
///    will. Nothing sleeps to wait for it.
/// 5. **the assertion** - the latch must be set again.
fn prove_mid_advance(
    probe: Side<'_>,
    occupancy_key: &str,
    actor: Side<'_>,
    advance: Advance<'_>,
    which: Wake,
    phase: &str,
) -> Result<(), String> {
    let dir = which.as_str();
    let Side {
        stdin: probe_in,
        out: probe_out,
    } = probe;
    let Side {
        stdin: actor_in,
        out: actor_out,
    } = actor;

    step(
        probe_in,
        probe_out,
        &format!("DRAIN_LATCH {dir}"),
        &format!("LATCH_DRAINED {dir}"),
    )
    .map_err(|e| format!("STEP {phase} FAILED (drain {dir}): {e}"))?;
    step(
        probe_in,
        probe_out,
        &format!("EXPECT_NO_LATCH {dir}"),
        &format!("LATCH_CLEAR {dir}"),
    )
    .map_err(|e| format!("STEP {phase} FAILED ({dir} not clear after the drain): {e}"))?;
    let state = step(probe_in, probe_out, "REPORT", "STATE ")
        .map_err(|e| format!("STEP {phase} FAILED (report before the {dir} advance): {e}"))?;
    assert_strictly_between(&state, occupancy_key, "capacity", phase)?;
    step(actor_in, actor_out, advance.command, advance.ack)
        .map_err(|e| format!("STEP {phase} FAILED (`{}`): {e}", advance.command))?;
    step(
        probe_in,
        probe_out,
        &format!("EXPECT_LATCH {dir}"),
        &format!("LATCH_SET {dir}"),
    )
    .map_err(|e| {
        format!(
            "STEP {phase} FAILED: `{}` away from the empty/full boundary did not \
             signal Wake::{dir} across the process boundary: {e}",
            advance.command
        )
    })?;
    Ok(())
}

/// Require `0 < line[key] < line[bound]` on a peer's `STATE` line.
///
/// This is what turns "the ring was neither empty nor full" from an assumption
/// into evidence: the mid-advance phases only mean something if the advance
/// under test really did happen away from both boundaries, and the peer's own
/// counters are the only place that fact lives.
fn assert_strictly_between(line: &str, key: &str, bound: &str, what: &str) -> Result<(), String> {
    let value = field(line, key)?;
    let limit = field(line, bound)?;
    if value == 0 || value >= limit {
        return Err(format!(
            "{what} needs 0 < {key} < {bound}, but the peer reports `{line}`: a ring at \
             a boundary would be signalled by the withdrawn edge too, so the phase would \
             prove nothing"
        ));
    }
    Ok(())
}

fn field(line: &str, key: &str) -> Result<u64, String> {
    let prefix = format!("{key}=");
    let value = line
        .split_whitespace()
        .find_map(|f| f.strip_prefix(prefix.as_str()))
        .ok_or_else(|| format!("`{line}` has no `{key}=` field"))?;
    number(line, key, value)
}

fn number<T: std::str::FromStr>(line: &str, key: &str, value: &str) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    value
        .parse::<T>()
        .map_err(|e| format!("`{line}`: {key} {value:?} is not a number: {e}"))
}

fn collect(pipe: impl Read + Send + 'static) -> Receiver<String> {
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

fn wait_for_line(rx: &Receiver<String>, prefix: &str, timeout: Duration) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(line) if line.starts_with(prefix) => return Ok(line),
            Ok(line) if line.starts_with("FAIL") => return Err(line),
            Ok(_) => continue,
            Err(RecvTimeoutError::Timeout) => {
                return Err(format!("no `{prefix}` line within {timeout:?}"));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(format!("the peer exited without a `{prefix}` line"));
            }
        }
    }
}

fn drain(rx: &Receiver<String>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Ok(line) = rx.recv_timeout(Duration::from_millis(500)) {
        lines.push(line);
    }
    lines
}

fn wait_timeout(child: &mut Child, timeout: Duration) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child
            .try_wait()
            .map_err(|e| format!("waiting for peer failed: {e}"))?
        {
            Some(status) => return Ok(status),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("peer did not exit within {timeout:?}"));
            }
            None => std::thread::sleep(Duration::from_millis(2)),
        }
    }
}

fn join(lines: &[String]) -> String {
    if lines.is_empty() {
        "(no output)".to_owned()
    } else {
        lines.join(" | ")
    }
}

fn build_peer(root: &Path, triple: &str) -> Result<(), String> {
    let mut cmd = cargo();
    cmd.current_dir(root).args([
        "build",
        "-p",
        "rustamp-ipc",
        "--example",
        "ring_peer",
        "--target",
        triple,
    ]);
    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("failed to build peer for {triple}: {status}"))
    }
}

fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
}

fn peer_path(target_dir: &Path, triple: &str) -> PathBuf {
    target_dir
        .join(triple)
        .join("debug")
        .join("examples")
        .join("ring_peer.exe")
}

fn short(triple: &str) -> &str {
    triple.split('-').next().unwrap_or(triple)
}

fn unique_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("rustamp-pcm-wake-xcheck-{}-{nanos:x}", std::process::id())
}
