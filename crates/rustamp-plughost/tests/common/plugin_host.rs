//! What the plug-in host tests that run real plug-ins have in common: where the
//! plug-ins and the host binary are, how a host process is spawned with its
//! trace and stderr captured, and two small helpers for the guest's desktop.
//!
//! VM-only code (the plug-ins run only in the Hyper-V guest).
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

#![allow(dead_code)]

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use rustamp_ipc::{Message, StreamTransport, Transport};

pub const REPLY_DEADLINE: Duration = Duration::from_secs(12);

pub fn survey_dir() -> PathBuf {
    let mut candidates = Vec::new();
    if let Some(base) = std::env::var_os("RUSTAMP_TESTPLUGINS") {
        candidates.push(PathBuf::from(&base).join("w566"));
        candidates.push(PathBuf::from(base));
    }
    candidates.push(PathBuf::from(r"C:\rustamp\testplugins\w566"));
    candidates.push(PathBuf::from(r"D:\code\rustamp-testplugins\w566"));
    candidates
        .iter()
        .find(|dir| dir.join("PROVENANCE.txt").is_file())
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "REFUSING LOUDLY: no survey directory (with PROVENANCE.txt) in {candidates:?}; \
                 the runner stages the test plug-in directory into the guest"
            )
        })
}

pub fn host_exe() -> PathBuf {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(deps) = exe.parent()
    {
        candidates.push(deps.join("rustamp-plughost32.exe"));
        if let Some(debug) = deps.parent() {
            candidates.push(debug.join("rustamp-plughost32.exe"));
        }
    }
    candidates.push(PathBuf::from(env!("CARGO_BIN_EXE_rustamp-plughost32")));
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .expect("REFUSING LOUDLY: no rustamp-plughost32.exe beside the test binaries")
}

pub struct Host {
    pub child: Child,
    pub writer: StreamTransport<io::Empty, std::process::ChildStdin>,
    pub replies: mpsc::Receiver<Message>,
}

impl Host {
    pub fn spawn(exe: &Path, approvals: &Path, scratch: &Path) -> Self {
        let stderr = std::fs::File::create(scratch.join("stderr.txt")).expect("stderr file");
        let mut child = Command::new(exe)
            .arg("--approvals")
            .arg(approvals)
            .env("RUSTAMP_HOST_TRACE", scratch.join("trace.txt"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .unwrap_or_else(|e| panic!("spawning {} failed: {e}", exe.display()));
        let stdout = child.stdout.take().expect("piped stdout");
        let stdin = child.stdin.take().expect("piped stdin");
        let (tx, replies) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = StreamTransport::new(stdout, io::sink());
            while let Ok(message) = reader.recv() {
                if tx.send(message).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            writer: StreamTransport::new(io::empty(), stdin),
            replies,
        }
    }
}

/// The window title `tasklist` reports for a process (`N/A` when it has none).
pub fn window_title_of(pid: u32) -> String {
    let out = Command::new("tasklist")
        .args(["/v", "/fo", "csv", "/nh", "/fi", &format!("PID eq {pid}")])
        .output();
    match out {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            // The title is the last CSV column.
            text.trim()
                .rsplit("\",\"")
                .next()
                .unwrap_or("")
                .trim_matches('"')
                .to_string()
        }
        Err(error) => format!("tasklist failed: {error}"),
    }
}

pub fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

/// Runs a PowerShell one-liner without a window and returns what it printed.
///
/// With a **deadline**: a script that asks a window of a stuck process for its
/// title (`MainWindowTitle` sends the window a message) waits as long as that
/// process's UI thread does, so after `seconds` the script is killed and the
/// answer says so.
pub fn powershell(script: &str, seconds: u64) -> String {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    let spawned = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(0x0800_0000)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => return format!("powershell failed: {error}"),
    };
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return format!("(no answer within {seconds} s: the script was stopped)");
            }
        }
    }
    reader.join().unwrap_or_default().trim().to_string()
}
