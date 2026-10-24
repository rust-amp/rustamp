//! Acceptance test for the control-channel load path
//! (`docs/tasks/P3-dll-acceptance.md` §5.3, card `docs/tasks/W23-p3-acceptance.md`).
//!
//! Gated on the Hyper-V test VM ([`rustamp_plughost::require_test_vm`]) and on
//! an i686 process, like the other Group D tests.
//!
//! It drives the **production host binary** `rustamp-plughost32.exe` over the
//! real framed stdio control channel (RFC 0006 §8) — not an in-process call —
//! and proves, in that separate i686 process:
//!
//! 1. the host greets with a compatible `Hello` on the framed channel;
//! 2. a `LoadPlugin` asking for the wrong kind is refused with a `PluginError`
//!    that says it was refused **before any mapping**, so no `DllMain` ran;
//! 3. a `LoadPlugin` for bytes the approval store does not approve (one flipped
//!    byte of the approved image) is refused with a `PluginError`, before the
//!    OS loader sees the file;
//! 4. a `LoadPlugin` for the approved `in_openmpt.dll` comes back as
//!    `PluginLoaded` with kind `Input`, a non-empty plugin-supplied name, and
//!    `plugin_id` 1 — the refusals above consumed no id;
//! 5. every frame on the host's stdout decodes, so nothing printed into the
//!    channel; and the host exits 0 on `Shutdown`.
//!
//! What this is **not**: acceptance of any plugin kind other than input, of a
//! window, a dialog, a subclassed procedure or a DLL-origin message. P3 §7
//! lists `PluginLoaded` among the false passes for *those* claims; here it is
//! the claim under test and nothing more.
//!
//! The second test in this file adds the **general-purpose** kind at the
//! process level: the production host maps our own clean-room `gen_`
//! fixture, resolves `winampGetGeneralPurposePlugin`, runs its `init` — which
//! subclasses the host's window and creates a window of its own — and quits
//! it on shutdown. The evidence is the plugin's own trace file, written from
//! inside the host process and named after the pid the `Hello` frame reports.
//! The in-process half of that milestone (the plugin's own IPC queries, its
//! configuration window and the subclass restore) is
//! `tests/gen_acceptance.rs`.
//!
//! Must refuse loudly (not skip silently) when the VM, the host binary or the
//! plugin binary is missing.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

#[path = "common/gen_fixture.rs"]
pub mod gen_fixture;

#[path = "common/gen_protocol.rs"]
pub mod gen_protocol;

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use rustamp_ipc::{Message, PROTOCOL_VERSION, StreamTransport, Transport};
use rustamp_plughost::approval::{ApprovalStore, Sha256};
use rustamp_plughost::host_window::require_test_vm;
use rustamp_plugin_abi::PluginKind;

/// How long any single reply may take. A deadline on a condition, never a sleep
/// used to order events: the test blocks on the channel and gives up loudly.
const REPLY_DEADLINE: Duration = Duration::from_secs(20);

/// Unique scratch directory for the approval store and the tampered copy.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rustamp-control-load-{}-{}",
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

/// Find `in_openmpt.dll` across the runner's staged locations, or refuse loudly.
/// Same search order as `tests/dll_acceptance.rs`.
fn find_in_openmpt_dll() -> PathBuf {
    let mut checked = Vec::new();
    if let Some(value) = std::env::var_os("RUSTAMP_TESTPLUGINS") {
        let base = PathBuf::from(value);
        if base.is_file() {
            return base;
        }
        for candidate in [
            base.join("in_openmpt.dll"),
            base.join("libopenmpt")
                .join("Winamp")
                .join("in_openmpt.dll"),
        ] {
            if candidate.is_file() {
                return candidate;
            }
            checked.push(candidate);
        }
    }
    for candidate in [
        PathBuf::from(r"C:\rustamp\testplugins\libopenmpt\Winamp\in_openmpt.dll"),
        PathBuf::from(r"C:\rustamp\testplugins\in_openmpt.dll"),
        PathBuf::from(r"D:\code\rustamp-testplugins\libopenmpt\Winamp\in_openmpt.dll"),
    ] {
        if candidate.is_file() {
            return candidate;
        }
        checked.push(candidate);
    }
    panic!(
        "REFUSING LOUDLY: missing plugin binary in_openmpt.dll. Checked RUSTAMP_TESTPLUGINS \
         and {checked:?}. DLL acceptance tests must refuse loudly by design when reference \
         binaries are missing."
    );
}

/// The host's replies, as they arrive, decoded off its real stdout.
struct Host {
    child: Child,
    writer: StreamTransport<io::Empty, std::process::ChildStdin>,
    replies: mpsc::Receiver<Message>,
}

impl Host {
    fn spawn(exe: &Path, approvals: &Path) -> Self {
        let mut child = Command::new(exe)
            .arg("--approvals")
            .arg(approvals)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Inherited, so the host's own diagnostics land in the run log.
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| panic!("spawning {} failed: {e}", exe.display()));

        let stdout = child.stdout.take().expect("piped stdout");
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

        Self {
            child,
            writer: StreamTransport::new(io::empty(), stdin),
            replies,
        }
    }

    fn send(&mut self, message: &Message) {
        self.writer
            .send(message)
            .unwrap_or_else(|e| panic!("sending {message:?} to the host failed: {e}"));
    }

    fn expect_reply(&mut self, what: &str) -> Message {
        match self.replies.recv_timeout(REPLY_DEADLINE) {
            Ok(message) => message,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!(
                "the host sent no {what} within {:?}; it is hung or silent, which is a failure, \
                 not a pass",
                REPLY_DEADLINE
            ),
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!(
                "the host's stdout ended before any {what} arrived: it died or wrote a frame that \
                 does not decode"
            ),
        }
    }

    fn load(&mut self, path: &Path, kind: PluginKind, what: &str) -> Message {
        self.send(&Message::LoadPlugin {
            path: path.to_string_lossy().into_owned(),
            kind,
        });
        self.expect_reply(what)
    }
}

fn plugin_error(reply: &Message, what: &str) -> String {
    match reply {
        Message::PluginError { message, .. } => message.clone(),
        other => panic!("{what}: expected a PluginError, got {other:?}"),
    }
}

#[test]
#[ignore = "VM-only: spawns the real i686 host process and loads a real 32-bit DLL; run in test VM"]
fn the_real_host_process_loads_an_approved_dll_through_the_control_channel() {
    // 1. Gate before anything is spawned, mapped or created.
    require_test_vm();
    if !cfg!(target_arch = "x86") {
        panic!(
            "REFUSING LOUDLY: the host must be i686 to map a 32-bit plugin DLL; \
             this test binary is not x86, so the child it spawns is not either"
        );
    }

    let exe = find_host_exe();
    let plugin = find_in_openmpt_dll();
    let approved = std::fs::read(&plugin).expect("the staged fixture must be readable");
    // Not a pin (the input fixture's hash is still unpinned, P3 §2), but the
    // identity this run actually used, recorded for the run bundle.
    println!(
        "FIXTURE {} size={} sha256={}",
        plugin.display(),
        approved.len(),
        Sha256::digest_hex(&approved)
    );
    println!("HOST {}", exe.display());

    // 2. The operator's allowlist: exactly these bytes, nothing else.
    let scratch = Scratch::new();
    let approvals = scratch.join("plugins-approved.toml");
    let mut store = ApprovalStore::new();
    store
        .approve_bytes("in_openmpt.dll", &approved)
        .expect("the fixture must pass passive preflight");
    store
        .save_to_file(&approvals)
        .expect("the approval store must be writable");

    // A copy of the approved image with one byte flipped: structurally the same
    // input plugin, a different image, and therefore not approved.
    let tampered_path = scratch.join("in_tampered.dll");
    let mut tampered = approved.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xFF;
    std::fs::write(&tampered_path, &tampered).expect("the tampered copy must be writable");

    let mut host = Host::spawn(&exe, &approvals);

    // 3. The framed handshake, on the real channel.
    match host.expect_reply("Hello") {
        Message::Hello {
            protocol_version,
            pid,
        } => {
            assert_eq!(
                protocol_version, PROTOCOL_VERSION,
                "the host greeted with an incompatible protocol version"
            );
            assert_ne!(pid, 0, "the host must report its real pid");
            println!("HOST_PID {pid}");
        }
        other => panic!("the host's first frame must be Hello, got {other:?}"),
    }

    // 4. Negative control: the wrong kind is refused before anything is mapped.
    let reply = host.load(&plugin, PluginKind::General, "wrong-kind refusal");
    let message = plugin_error(&reply, "wrong-kind request");
    assert!(
        message.contains("refused before any mapping"),
        "a wrong-kind request must be refused before the OS loader runs any plugin code: {message}"
    );

    // 5. Negative control: unapproved bytes are refused before the OS loader.
    let reply = host.load(&tampered_path, PluginKind::Input, "unapproved refusal");
    let message = plugin_error(&reply, "tampered request");
    assert!(
        message.contains("approved image"),
        "one flipped byte must be refused by the approval gate: {message}"
    );

    // 6. The load under test: approved bytes, right kind, real DLL, real host.
    let reply = host.load(&plugin, PluginKind::Input, "PluginLoaded");
    match &reply {
        Message::PluginLoaded {
            plugin_id,
            name,
            kind,
        } => {
            assert_eq!(
                *plugin_id, 1,
                "the first successful load is id 1; the refusals above consumed no id"
            );
            assert_eq!(*kind, PluginKind::Input);
            assert!(
                !name.is_empty(),
                "the name must come from the plugin's own description"
            );
            let lowered = name.to_ascii_lowercase();
            assert!(
                lowered.contains("openmpt") || lowered.contains("module"),
                "unexpected plugin description: {name:?}"
            );
            println!("PLUGIN_LOADED id={plugin_id} kind={kind:?} name={name:?}");
        }
        Message::PluginError { message, .. } => panic!(
            "the approved fixture must load in the real i686 host, but it was refused: {message}"
        ),
        other => panic!("expected PluginLoaded, got {other:?}"),
    }

    // 7. Clean shutdown: the host quits its plugins and exits 0.
    host.send(&Message::Shutdown);
    let status = host
        .child
        .wait()
        .expect("waiting for the host process must succeed");
    assert_eq!(
        status.code(),
        Some(0),
        "the host must exit 0 after Shutdown, got {status:?}"
    );
}

/// The plugin's own trace file, once it holds `marker`.
///
/// The file is written from **inside** the plugin, in the host process, and
/// named after that process id — so it is evidence the host's own plugin ran,
/// not something the runner could have written.
fn wait_for_trace(pid: u32, marker: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "{}{pid}.{}",
        gen_protocol::TRACE_FILE_PREFIX,
        gen_protocol::TRACE_FILE_EXTENSION
    ));
    let deadline = std::time::Instant::now() + REPLY_DEADLINE;
    loop {
        if let Ok(text) = std::fs::read_to_string(&path)
            && text.contains(marker)
        {
            return text;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "the plugin wrote no trace containing '{marker}' at {} within {REPLY_DEADLINE:?}",
                path.display()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The production host process must load a real 32-bit **general-purpose**
/// plugin DLL and quit it cleanly.
///
/// This is the process-level half of P3 §2 ("the production i686 host"): the
/// host maps the DLL, resolves `winampGetGeneralPurposePlugin`, fills the
/// host-owned members, calls `init` — which subclasses the host's window and
/// creates a window of its own — and quits the plugin on shutdown, restoring
/// the host's original window procedure.
///
/// The plugin's own trace file proves that all of that happened **inside the
/// host process**: it is written by the plugin and named after the host's
/// pid, which the `Hello` frame reports.
#[test]
#[ignore = "VM-only: spawns the real i686 host process and loads a real 32-bit general-purpose DLL; run in test VM"]
fn the_real_host_process_loads_a_general_purpose_dll_and_quits_it_cleanly() {
    require_test_vm();
    if !cfg!(target_arch = "x86") {
        panic!(
            "REFUSING LOUDLY: the host must be i686 to map a 32-bit plugin DLL; \
             this test binary is not x86, so the child it spawns is not either"
        );
    }

    let exe = find_host_exe();
    let fixture = gen_fixture::find_fixture_dll();
    let approved = std::fs::read(&fixture).expect("the staged fixture must be readable");
    println!(
        "FIXTURE {} size={} sha256={}",
        fixture.display(),
        approved.len(),
        Sha256::digest_hex(&approved)
    );
    println!("HOST {}", exe.display());

    // The operator's allowlist: exactly these bytes, nothing else.
    let scratch = Scratch::new();
    let approvals = scratch.join("plugins-approved.toml");
    let mut store = ApprovalStore::new();
    store
        .approve_bytes(gen_fixture::FIXTURE_FILE_NAME, &approved)
        .expect("the fixture must pass passive preflight as a general plugin");
    store
        .save_to_file(&approvals)
        .expect("the approval store must be writable");

    let mut host = Host::spawn(&exe, &approvals);

    // The handshake, and the host's real pid.
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
            println!("HOST_PID {pid}");
            pid
        }
        other => panic!("the host's first frame must be Hello, got {other:?}"),
    };

    // The load under test: a real general-purpose plugin in the production host.
    let reply = host.load(&fixture, PluginKind::General, "PluginLoaded");
    match &reply {
        Message::PluginLoaded {
            plugin_id,
            name,
            kind,
        } => {
            assert_eq!(*kind, PluginKind::General);
            assert_eq!(*plugin_id, 1, "the first successful load is id 1");
            assert!(
                !name.is_empty(),
                "the name must come from the plugin's own description"
            );
            println!("PLUGIN_LOADED id={plugin_id} kind={kind:?} name={name:?}");
        }
        Message::PluginError { message, .. } => panic!(
            "the approved fixture must load in the real i686 host, but it was refused: {message}"
        ),
        other => panic!("expected PluginLoaded, got {other:?}"),
    }

    // The plugin's own trace: it ran inside the host process, subclassed the
    // host's window and created a window of its own.
    let trace = wait_for_trace(pid, "window created");
    println!("PLUGIN_TRACE\n{trace}");
    assert!(
        trace.contains("init parent="),
        "the plugin's init must have run inside the host process:\n{trace}"
    );
    assert!(
        trace.contains("subclass installed"),
        "the plugin must have subclassed the host's window:\n{trace}"
    );
    assert!(
        trace.contains("parent class ok=1"),
        "the plugin must have seen the host's Winamp v1.x window:\n{trace}"
    );

    // Shutdown: the host quits the plugin and exits 0.
    host.send(&Message::Shutdown);
    let status = host
        .child
        .wait()
        .expect("waiting for the host process must succeed");
    assert_eq!(
        status.code(),
        Some(0),
        "the host must exit 0 after Shutdown, got {status:?}"
    );

    // The plugin's quit ran in the host process too: the host's original
    // window procedure is back and the plugin's window is gone.
    let trace = wait_for_trace(pid, "quit subclass_restored=true");
    assert!(
        trace.contains("window_destroyed=true"),
        "the plugin must have destroyed its own window:\n{trace}"
    );
}
