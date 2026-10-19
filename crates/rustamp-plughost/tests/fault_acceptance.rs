//! Acceptance test for the plugin-fault path (PLAN M3: "Crash isolation:
//! restart the host on plugin crash").
//!
//! Gated on the Hyper-V test VM ([`rustamp_plughost::require_test_vm`]) and on
//! an i686 process, like the other Group D tests.
//!
//! It drives the **production host binary** `rustamp-plughost32.exe` over the
//! real framed stdio control channel and loads the **crashing fixture**
//! (`examples/crash_rustamp_fixture.rs`), a real 32-bit DLL whose `init`
//! writes through a null pointer. What it proves, in that separate i686
//! process:
//!
//! 1. the host greets with a compatible `Hello` on the framed channel;
//! 2. the load request reaches the plugin: the plugin's own trace file says
//!    that its `init` ran (plugin-side evidence, written from inside the
//!    plugin);
//! 3. the host does **not** die silently: it writes one diagnostic line to
//!    stderr naming the plugin, the call and the Windows exception;
//! 4. the host ends with the **fault exit code** the supervisor recognises
//!    (`rustamp_plughost::PLUGIN_FAULT_EXIT_CODE`), so the supervisor can tell
//!    "the host caught a plugin fault" from "the host died from an exception";
//! 5. the control channel ends with the host — no frame after the fault, and
//!    no reply to the load request.
//!
//! What this is **not**: proof that the *supervisor* restarts the host. That
//! is the app side: the restart/backoff decision and the report text are unit
//! tested headlessly in `rustamp-plughost::restart` and
//! `rustamp-app/src/supervise.rs`.
//!
//! Must refuse loudly (not skip silently) when the VM, the host binary or the
//! fixture is missing.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

#[path = "common/crash_protocol.rs"]
pub mod crash_protocol;

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crash_protocol::FIXTURE_FILE_NAME;
use rustamp_ipc::{Message, PROTOCOL_VERSION, StreamTransport, Transport};
use rustamp_plughost::PLUGIN_FAULT_EXIT_CODE;
use rustamp_plughost::approval::ApprovalStore;
use rustamp_plughost::host_window::require_test_vm;
use rustamp_plugin_abi::PluginKind;

/// How long the host may take to die after the fault.
const DEATH_DEADLINE: Duration = Duration::from_secs(20);

/// Unique scratch directory for the approval store.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rustamp-fault-acceptance-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("scratch directory must be creatable");
        Self(dir)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Candidate locations of the production host binary.
///
/// `current_exe` first: the VM runner stages the test binaries into
/// `target\i686-pc-windows-msvc\debug\deps` and the host beside them in
/// `...\debug` (`docs/notes/vm-run-tests.ps1`), while `CARGO_BIN_EXE_*` is
/// baked in at compile time and points at the *build* machine's tree.
fn host_exe_candidates() -> Vec<PathBuf> {
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
}

fn find_host_exe() -> PathBuf {
    let candidates = host_exe_candidates();
    for candidate in &candidates {
        if candidate.is_file() {
            return candidate.clone();
        }
    }
    let checked = candidates
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    panic!(
        "REFUSING LOUDLY: no rustamp-plughost32.exe found. Checked [{checked}]. \
         Stage it with `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` \
         beside the test binaries. This test refuses rather than passing vacuously."
    );
}

/// Find the crashing fixture, or refuse loudly.
fn find_fixture_dll() -> PathBuf {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join(FIXTURE_FILE_NAME));
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join(FIXTURE_FILE_NAME));
            if let Some(root) = parent.parent() {
                candidates.push(
                    root.join("i686-pc-windows-msvc")
                        .join("debug")
                        .join("examples")
                        .join(FIXTURE_FILE_NAME),
                );
            }
        }
    }
    for candidate in &candidates {
        if candidate.is_file() {
            return candidate.clone();
        }
    }
    let checked = candidates
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    panic!(
        "REFUSING LOUDLY: missing the crashing fixture {FIXTURE_FILE_NAME}. Checked [{checked}]. \
         Build it with `cargo build -p rustamp-plughost --example crash_rustamp_fixture --target \
         i686-pc-windows-msvc` and stage it beside the test binaries. This test refuses rather \
         than passing vacuously."
    );
}

/// The host process, with its stderr captured.
struct Host {
    child: Child,
    writer: StreamTransport<io::Empty, std::process::ChildStdin>,
    replies: mpsc::Receiver<Message>,
    stderr: mpsc::Receiver<String>,
}

impl Host {
    fn spawn(exe: &Path, approvals: &Path) -> Self {
        let mut child = Command::new(exe)
            .arg("--approvals")
            .arg(approvals)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Piped, not inherited: the fault report is the evidence here.
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("spawning {} failed: {e}", exe.display()));

        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let stdin = child.stdin.take().expect("piped stdin");
        let (tx, replies) = mpsc::channel();
        std::thread::Builder::new()
            .name("host-reader".to_string())
            .spawn(move || {
                let mut reader = StreamTransport::new(stdout, io::sink());
                // Stops on EOF or on the first frame that does not decode: a
                // corrupted stream must not look like silence.
                while let Ok(message) = reader.recv() {
                    if tx.send(message).is_err() {
                        break;
                    }
                }
            })
            .expect("reader thread");

        let (stderr_tx, stderr_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("host-stderr".to_string())
            .spawn(move || {
                use std::io::Read;
                let mut text = String::new();
                let mut handle = stderr;
                let _ = handle.read_to_string(&mut text);
                let _ = stderr_tx.send(text);
            })
            .expect("stderr thread");

        Self {
            child,
            writer: StreamTransport::new(io::empty(), stdin),
            replies,
            stderr: stderr_rx,
        }
    }

    fn send(&mut self, message: &Message) {
        self.writer
            .send(message)
            .unwrap_or_else(|e| panic!("sending {message:?} to the host failed: {e}"));
    }

    fn expect_reply(&mut self, what: &str) -> Message {
        match self.replies.recv_timeout(DEATH_DEADLINE) {
            Ok(message) => message,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!(
                "the host sent no {what} within {DEATH_DEADLINE:?}; it is hung or silent, \
                 which is a failure, not a pass"
            ),
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!(
                "the host's stdout ended before any {what} arrived: it died or wrote a frame \
                 that does not decode"
            ),
        }
    }

    /// Wait for the host to end, or fail loudly.
    fn wait_for_death(&mut self) -> i32 {
        let deadline = Instant::now() + DEATH_DEADLINE;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    return status.code().unwrap_or_else(|| {
                        panic!("the host ended without an exit code: {status:?}")
                    });
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        panic!(
                            "the host did not end within {DEATH_DEADLINE:?} after the fault; \
                             a host that survives a plugin fault is a failure, not a pass"
                        );
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => panic!("waiting for the host process failed: {error}"),
            }
        }
    }

    /// Everything the host wrote to stderr.
    fn stderr_text(&self) -> String {
        self.stderr.recv_timeout(DEATH_DEADLINE).unwrap_or_default()
    }
}

/// The plugin's own trace file, once it holds `marker`.
///
/// The file is written from **inside** the plugin, in the host process, and
/// named after that process id — so it is evidence the plugin's own `init`
/// ran, not something the runner could have written.
fn wait_for_trace(pid: u32, marker: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "{}{pid}.{}",
        crash_protocol::TRACE_FILE_PREFIX,
        crash_protocol::TRACE_FILE_EXTENSION
    ));
    let deadline = Instant::now() + DEATH_DEADLINE;
    loop {
        if let Ok(text) = std::fs::read_to_string(&path)
            && text.contains(marker)
        {
            return text;
        }
        if Instant::now() >= deadline {
            panic!(
                "the plugin wrote no trace containing '{marker}' at {} within {DEATH_DEADLINE:?}",
                path.display()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "VM-only: spawns the real i686 host process and loads a plugin that faults on purpose; run in test VM"]
fn a_faulting_plugin_ends_the_host_with_the_fault_code_and_a_report() {
    // 1. Gate before anything is spawned, mapped or created.
    require_test_vm();
    if !cfg!(target_arch = "x86") {
        panic!(
            "REFUSING LOUDLY: the host must be i686 to map a 32-bit plugin DLL; \
             this test binary is not x86, so the child it spawns is not either"
        );
    }

    let exe = find_host_exe();
    let fixture = find_fixture_dll();
    let approved = std::fs::read(&fixture).expect("the staged fixture must be readable");

    // 2. The operator's allowlist: exactly these bytes, nothing else.
    let scratch = Scratch::new();
    let approvals = scratch.join("plugins-approved.toml");
    let mut store = ApprovalStore::new();
    store
        .approve_bytes(FIXTURE_FILE_NAME, &approved)
        .expect("the fixture must pass passive preflight");
    store
        .save_to_file(&approvals)
        .expect("the approval store must be writable");

    let mut host = Host::spawn(&exe, &approvals);

    // 3. The framed handshake, on the real channel.
    let pid = match host.expect_reply("Hello") {
        Message::Hello {
            protocol_version,
            pid,
        } => {
            assert_eq!(
                protocol_version, PROTOCOL_VERSION,
                "the host greeted with an incompatible protocol version"
            );
            assert_ne!(pid, 0, "the host must report its real pid");
            pid
        }
        other => panic!("the host's first frame must be Hello, got {other:?}"),
    };

    // 4. The load under test: a real plugin that faults in its `init`.
    host.send(&Message::LoadPlugin {
        path: fixture.to_string_lossy().into_owned(),
        kind: PluginKind::General,
    });

    // 5. The host must end, and end with the fault exit code.
    let code = host.wait_for_death();
    assert_eq!(
        code, PLUGIN_FAULT_EXIT_CODE as i32,
        "the host must end with the fault exit code the supervisor recognises, got {code}"
    );

    // 6. The report: one line on stderr naming the plugin, the call and the
    // exception. The host must not die silently.
    let stderr = host.stderr_text();
    assert!(
        stderr.contains("plugin fault in"),
        "the host must report the fault on stderr, got:\n{stderr}"
    );
    assert!(
        stderr.contains(FIXTURE_FILE_NAME),
        "the report must name the plugin, got:\n{stderr}"
    );
    assert!(
        stderr.contains("call init"),
        "the report must name the call, got:\n{stderr}"
    );
    assert!(
        stderr.contains("Windows exception 0x"),
        "the report must name the exception, got:\n{stderr}"
    );

    // 7. The plugin's own evidence: its `init` really ran, inside the host.
    let trace = wait_for_trace(pid, "init entered");
    assert!(
        trace.contains("init entered"),
        "the plugin's init must have run inside the host process:\n{trace}"
    );

    // 8. No reply to the load request: the host died inside the call.
    //    `recv_timeout` reports both "nothing came" and "the channel is
    //    gone" as an error, so the check does not race the reader thread.
    assert!(
        host.replies.recv_timeout(Duration::from_secs(2)).is_err(),
        "the host must not answer a load request it died in"
    );
}
