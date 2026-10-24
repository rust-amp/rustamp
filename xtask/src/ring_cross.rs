//! `cargo xtask ring-cross` - the cross-bitness ring check (task card Q1).
//!
//! Builds the `ring_peer` example of `rustamp-ipc` for both
//! `x86_64-pc-windows-msvc` and `i686-pc-windows-msvc` and runs the four
//! producer/consumer combinations (2 directions x 2 ring kinds) against a real
//! Windows named file mapping. The 32-bit side and the 64-bit side have to agree
//! on the header layout, the counters and the slot bytes; the two peer processes
//! do the actual verification, this runner only starts them and compares their
//! reports.
//!
//! The check is deterministic: a fixed frame count, a fixed capacity, a bounded
//! timeout for every wait and no timing-dependent assertion. A run fails if a
//! peer exits non-zero, does not report the expected counters, or does not
//! finish in time.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::{cargo, no_window_command, run_status};

/// The two targets the check compares.
const X64: &str = "x86_64-pc-windows-msvc";
const X86: &str = "i686-pc-windows-msvc";

/// Fixed layout of every run: 512 frames through a 250-slot ring, i.e. the ring
/// wraps twice and the capacity is deliberately not a power of two. Every frame
/// is offered only when the ring has room, so nothing is dropped and every value
/// can be checked exactly.
const FRAMES: u64 = 512;
const CAPACITY: u32 = 250;

/// The four combinations this check must run: both directions for both ring
/// kinds. They are data, not a loop nest, so the expected set is reviewable in
/// one place and the table always has exactly this many rows.
const RUNS: [(&str, &str, &str); 4] = [
    (X86, X64, "pcm"),
    (X86, X64, "vis"),
    (X64, X86, "pcm"),
    (X64, X86, "vis"),
];

/// Upper bound for one peer phase (start, produce, consume).
const PEER_TIMEOUT: Duration = Duration::from_secs(60);
/// Upper bound for the first `RING` line of a producer.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// One row of the printed table.
struct Row {
    direction: String,
    kind: String,
    frames: u64,
    dropped: u64,
    result: String,
}

/// What the two peers reported for one run.
struct RunStats {
    received: u64,
    dropped: u64,
}

/// The built peer binaries.
struct Peers {
    x64: PathBuf,
    x86: PathBuf,
}

impl Peers {
    fn get(&self, triple: &str) -> &Path {
        if triple == X64 { &self.x64 } else { &self.x86 }
    }
}

/// `cargo xtask ring-cross`
pub fn run(args: &[String]) -> Result<(), String> {
    if let Some(extra) = args.first() {
        return Err(format!("unknown ring-cross option `{extra}`"));
    }
    if !cfg!(windows) {
        println!("ring-cross: SKIPPED - the cross-process ring needs Windows named file mappings");
        return Ok(());
    }

    let root = rustamp_testkit::workspace_root();
    let target_dir = target_dir(&root);
    println!("ring-cross: building the peer for {X64} and {X86}");
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
    for (producer, consumer, kind) in RUNS {
        let direction = format!("{} -> {}", short(producer), short(consumer));
        let name = unique_name(kind);
        match run_once(peers.get(producer), peers.get(consumer), kind, &name) {
            Ok(stats) => rows.push(Row {
                direction,
                kind: kind.to_owned(),
                frames: FRAMES,
                dropped: stats.dropped,
                result: format!("OK received={} dropped={}", stats.received, stats.dropped),
            }),
            Err(e) => {
                rows.push(Row {
                    direction: direction.clone(),
                    kind: kind.to_owned(),
                    frames: FRAMES,
                    dropped: 0,
                    result: format!("FAILED: {e}"),
                });
                failures.push(format!("{direction} {kind}: {e}"));
            }
        }
    }

    print_table(&rows);
    if rows.len() != RUNS.len() {
        return Err(format!(
            "the table has {} rows but {} combinations are defined",
            rows.len(),
            RUNS.len()
        ));
    }
    if failures.is_empty() {
        println!(
            "ring-cross: {} of {} combinations passed (2 directions x 2 ring kinds)",
            rows.len(),
            RUNS.len()
        );
        Ok(())
    } else {
        Err(format!(
            "{} of {} runs failed:\n  - {}",
            failures.len(),
            RUNS.len(),
            failures.join("\n  - ")
        ))
    }
}

fn print_table(rows: &[Row]) {
    println!();
    println!(
        "{:<16} {:<5} {:>7} {:>8}  result",
        "direction", "kind", "frames", "dropped"
    );
    for row in rows {
        println!(
            "{:<16} {:<5} {:>7} {:>8}  {}",
            row.direction, row.kind, row.frames, row.dropped, row.result
        );
    }
    println!();
}

/// Run one producer/consumer pair and cross-check their reports.
fn run_once(
    producer_exe: &Path,
    consumer_exe: &Path,
    kind: &str,
    name: &str,
) -> Result<RunStats, String> {
    let mut producer = no_window_command(producer_exe)
        .args([
            "produce",
            "--name",
            name,
            "--kind",
            kind,
            "--frames",
            &FRAMES.to_string(),
            "--capacity",
            &CAPACITY.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "failed to start the producer {}: {e}",
                producer_exe.display()
            )
        })?;
    let producer_out = collect(producer.stdout.take().expect("stdout is piped"));
    let producer_err = collect(producer.stderr.take().expect("stderr is piped"));

    let outcome = drive(
        &mut producer,
        &producer_out,
        &producer_err,
        consumer_exe,
        kind,
    );

    // Whatever happened, the producer must not outlive this run: it is still
    // waiting for a consumer that may never have started.
    if producer.try_wait().ok().flatten().is_none() {
        let _ = producer.kill();
        let _ = producer.wait();
    }
    outcome
}

fn drive(
    producer: &mut Child,
    producer_out: &Receiver<String>,
    producer_err: &Receiver<String>,
    consumer_exe: &Path,
    kind: &str,
) -> Result<RunStats, String> {
    let ring_line = wait_for_line(producer_out, "RING ", START_TIMEOUT)
        .map_err(|e| format!("the producer printed no ring offer: {e}"))?;
    let meta = RingMeta::parse(&ring_line)?;
    if meta.kind != kind {
        return Err(format!(
            "the producer offered kind `{}`, expected `{kind}`",
            meta.kind
        ));
    }
    if meta.capacity != CAPACITY {
        return Err(format!(
            "the producer offered a capacity of {}, expected {CAPACITY}",
            meta.capacity
        ));
    }

    let mut consumer = no_window_command(consumer_exe)
        .args([
            "consume",
            "--name",
            &meta.name,
            "--frames",
            &FRAMES.to_string(),
            "--kind",
            &meta.kind,
            "--frame-size",
            &meta.frame_size.to_string(),
            "--capacity",
            &meta.capacity.to_string(),
            "--generation",
            &meta.generation.to_string(),
            "--size",
            &meta.size.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "failed to start the consumer {}: {e}",
                consumer_exe.display()
            )
        })?;
    let consumer_out = collect(consumer.stdout.take().expect("stdout is piped"));
    let consumer_err = collect(consumer.stderr.take().expect("stderr is piped"));

    let status = wait_timeout(&mut consumer, PEER_TIMEOUT)?;
    let consumer_lines = drain(&consumer_out);
    let consumer_errs = drain(&consumer_err);
    if !status.success() {
        return Err(format!(
            "the consumer failed ({status}): {}",
            join(&consumer_errs)
        ));
    }
    let ok_line = consumer_lines
        .iter()
        .find(|line| line.starts_with("OK "))
        .cloned()
        .ok_or_else(|| {
            format!(
                "the consumer printed no `OK` line (stdout: {})",
                join(&consumer_lines)
            )
        })?;
    let received = field(&ok_line, "received")?;
    let dropped = field(&ok_line, "dropped")?;

    // The consumer closed the ring, so the producer stops waiting now.
    let status = wait_timeout(producer, PEER_TIMEOUT)?;
    let producer_lines = drain(producer_out);
    let producer_errs = drain(producer_err);
    if !status.success() {
        return Err(format!(
            "the producer failed ({status}): {}",
            join(&producer_errs)
        ));
    }
    let produced = producer_lines
        .iter()
        .find(|line| line.starts_with("PRODUCED "))
        .cloned()
        .ok_or_else(|| {
            format!(
                "the producer printed no `PRODUCED` line (stdout: {})",
                join(&producer_lines)
            )
        })?;
    let written = field(&produced, "written")?;
    let producer_dropped = field(&produced, "dropped")?;

    // Cross-check both sides: everything written was published, everything
    // published arrived exactly once, and nothing was dropped.
    if written != FRAMES {
        return Err(format!(
            "the producer wrote {written} frames, expected {FRAMES}"
        ));
    }
    if received != FRAMES {
        return Err(format!(
            "the consumer received {received} frames, expected {FRAMES}"
        ));
    }
    if producer_dropped != 0 || dropped != 0 {
        return Err(format!(
            "the ring dropped frames (producer {producer_dropped}, consumer {dropped})"
        ));
    }
    Ok(RunStats { received, dropped })
}

/// The producer's offer line, parsed.
#[derive(Default)]
struct RingMeta {
    name: String,
    kind: String,
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
                "kind" => meta.kind = value.to_owned(),
                "frame_size" => meta.frame_size = number(line, key, value)?,
                "capacity" => meta.capacity = number(line, key, value)?,
                "generation" => meta.generation = number(line, key, value)?,
                "size" => meta.size = number(line, key, value)?,
                other => return Err(format!("`{line}`: unexpected field `{other}`")),
            }
        }
        if meta.name.is_empty() {
            return Err(format!("`{line}`: no ring name"));
        }
        Ok(meta)
    }
}

/// Read one `key=value` field out of a peer report line.
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

/// Forward a child's pipe to a channel of lines, so a wait can be bounded.
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

/// Wait for a line starting with `prefix`, bounded by `timeout`.
fn wait_for_line(rx: &Receiver<String>, prefix: &str, timeout: Duration) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(line) if line.starts_with(prefix) => return Ok(line),
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

/// Collect whatever a finished peer had buffered on that pipe.
fn drain(rx: &Receiver<String>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Ok(line) = rx.recv_timeout(Duration::from_millis(500)) {
        lines.push(line);
    }
    lines
}

/// Wait for a child to exit, killing it after `timeout`.
fn wait_timeout(child: &mut Child, timeout: Duration) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child
            .try_wait()
            .map_err(|e| format!("waiting for a peer failed: {e}"))?
        {
            Some(status) => return Ok(status),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("a peer did not exit within {timeout:?}"));
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
    run_status(&mut cmd)
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

/// `x86_64` / `i686`, for the table.
fn short(triple: &str) -> &str {
    triple.split('-').next().unwrap_or(triple)
}

/// A section name that no other run of this check can collide with.
fn unique_name(kind: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("rustamp-{kind}-xcheck-{}-{nanos:x}", std::process::id())
}
