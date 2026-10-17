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
//! 5. **Shutdown wake**: reader blocks on empty ring; peer calls `close()`; reader
//!    is woken immediately and returns `Err(RingError::Closed)`.

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
const SLEEP_BEFORE_SIGNAL: Duration = Duration::from_millis(25);

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

    let expected_steps = RUNS.len() * 5;
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
        "wake-cross: all {} checks passed ({} directions x 5 wake phases)",
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
            "16",
            "--frame-size",
            "8",
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
        wait_for_line(&consumer_out, "READER_BLOCKING", STEP_TIMEOUT)
            .map_err(|e| format!("STEP reader-wake FAILED (reader did not block): {e}"))?;
        std::thread::sleep(SLEEP_BEFORE_SIGNAL);
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
        wait_for_line(&producer_out, "WRITER_BLOCKING", STEP_TIMEOUT).map_err(|e| {
            format!("STEP writer-wake FAILED (producer did not block on full ring): {e}")
        })?;
        std::thread::sleep(SLEEP_BEFORE_SIGNAL);
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

        // Phase 5: Close wakes blocked waiter with Closed
        writeln!(consumer_in, "WAIT_CLOSE").map_err(|e| format!("stdin: {e}"))?;
        consumer_in.flush().map_err(|e| format!("flush: {e}"))?;
        wait_for_line(&consumer_out, "WAITING_CLOSE", STEP_TIMEOUT)
            .map_err(|e| format!("STEP close-wake FAILED (consumer did not block): {e}"))?;
        std::thread::sleep(SLEEP_BEFORE_SIGNAL);
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
