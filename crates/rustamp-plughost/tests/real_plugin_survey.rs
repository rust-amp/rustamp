//! A survey of real classic plug-ins through the production host.
//!
//! For every `*.dll` in the survey directory (`<testplugins>\w566`, extracted
//! from the Winamp 5.66 installer for local testing; nothing in it ships) this
//! test starts the **real i686 host**, approves exactly those bytes, asks for
//! the plug-in by the kind its file-name prefix declares, and records what the
//! host says: loaded (with the plug-in's own name), refused (with the reason),
//! silent (hung) or dead (the plug-in took the host down). Each plug-in gets
//! its own host, so one that crashes cannot colour the next.
//!
//! The table is the product: many 5.x plug-ins need services only a full
//! Winamp 5 provides, so a refusal, a hang or a crash is **information, not a
//! failure**. The test fails only on a survey that found no plug-in at all
//! (a staging fault).
//!
//! Gated on the Hyper-V test VM like the other Group D tests.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use rustamp_ipc::{Message, StreamTransport, Transport};
use rustamp_plughost::approval::ApprovalStore;
use rustamp_plughost::host_window::require_test_vm;
use rustamp_plugin_abi::PluginKind;

const REPLY_DEADLINE: Duration = Duration::from_secs(10);

fn survey_dir() -> PathBuf {
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

fn host_exe() -> PathBuf {
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

fn kind_of(file: &str) -> Option<PluginKind> {
    let lower = file.to_ascii_lowercase();
    [
        ("in_", PluginKind::Input),
        ("out_", PluginKind::Output),
        ("vis_", PluginKind::Visualization),
        ("dsp_", PluginKind::Dsp),
        ("gen_", PluginKind::General),
    ]
    .into_iter()
    .find(|(prefix, _)| lower.starts_with(prefix))
    .map(|(_, kind)| kind)
}

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
            .stderr(Stdio::null())
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

#[derive(Debug)]
enum Outcome {
    Loaded(String),
    Refused(String),
    Hung,
    Died,
}

impl Outcome {
    fn describe(&self) -> String {
        match self {
            Outcome::Loaded(name) => format!("LOADED as {name:?}"),
            Outcome::Refused(why) => format!("REFUSED: {why}"),
            Outcome::Hung => "HUNG (no reply)".to_string(),
            Outcome::Died => "DIED (the host's channel closed)".to_string(),
        }
    }
}

fn survey_one(exe: &Path, dll: &Path, kind: PluginKind) -> Outcome {
    let file = dll.file_name().unwrap().to_string_lossy().into_owned();
    let scratch =
        std::env::temp_dir().join(format!("rustamp-survey-{}-{file}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let approvals = scratch.join("approved.toml");
    let bytes = std::fs::read(dll).unwrap();
    let mut store = ApprovalStore::new();
    if let Err(error) = store.approve_bytes(&file, &bytes) {
        let _ = std::fs::remove_dir_all(&scratch);
        return Outcome::Refused(format!("not approvable: {error}"));
    }
    store.save_to_file(&approvals).unwrap();

    let mut host = Host::spawn(exe, &approvals);
    let outcome = (|| {
        match host.replies.recv_timeout(REPLY_DEADLINE) {
            Ok(Message::Hello { .. }) => {}
            Ok(other) => return Outcome::Refused(format!("unexpected first frame {other:?}")),
            Err(mpsc::RecvTimeoutError::Timeout) => return Outcome::Hung,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Outcome::Died,
        }
        host.writer
            .send(&Message::LoadPlugin {
                path: dll.to_string_lossy().into_owned(),
                kind,
            })
            .expect("sending the load request");
        match host.replies.recv_timeout(REPLY_DEADLINE) {
            Ok(Message::PluginLoaded { name, .. }) => Outcome::Loaded(name),
            Ok(Message::PluginError { message, .. }) => Outcome::Refused(message),
            Ok(other) => Outcome::Refused(format!("unexpected reply {other:?}")),
            Err(mpsc::RecvTimeoutError::Timeout) => Outcome::Hung,
            Err(mpsc::RecvTimeoutError::Disconnected) => Outcome::Died,
        }
    })();
    let _ = host.writer.send(&Message::Shutdown);
    std::thread::sleep(Duration::from_millis(500));
    let _ = host.child.kill();
    let _ = host.child.wait();
    let _ = std::fs::remove_dir_all(&scratch);
    outcome
}

#[test]
#[ignore = "VM-only: spawns the real i686 host and maps real 32-bit plug-in DLLs; run in the test VM"]
fn survey_of_real_plugins_through_the_production_host() {
    require_test_vm();
    let dir = survey_dir();
    let exe = host_exe();
    let mut dlls: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
        })
        .collect();
    dlls.sort();
    assert!(!dlls.is_empty(), "no DLL in {}", dir.display());

    let mut loaded = 0;
    let mut hung = Vec::new();
    // libtest hides stdout of a passing test, so the table is also written
    // where the runner copies the guest's results back from.
    let mut table = vec![format!(
        "SURVEY host={} dir={}",
        exe.display(),
        dir.display()
    )];
    for dll in &dlls {
        let file = dll.file_name().unwrap().to_string_lossy().into_owned();
        let Some(kind) = kind_of(&file) else {
            table.push(format!("SURVEY {file}: skipped (no kind prefix)"));
            continue;
        };
        let outcome = survey_one(&exe, dll, kind);
        table.push(format!("SURVEY {file} [{kind:?}]: {}", outcome.describe()));
        match outcome {
            Outcome::Loaded(_) => loaded += 1,
            Outcome::Hung => hung.push(file),
            Outcome::Refused(_) | Outcome::Died => {}
        }
    }
    table.push(format!("SURVEY loaded {loaded} of {}", dlls.len()));
    for line in &table {
        println!("{line}");
    }
    let _ = std::fs::write(
        r"C:\rustamp\out\gui-session\plugin-survey.txt",
        table.join("\r\n"),
    );
    // A hang is a finding, not a verdict of this survey: on 2026-10-24 nine of
    // the Winamp 5.66 plug-ins (with their runtime DLLs staged) made the host
    // answer nothing within the deadline. It is recorded in the table and in
    // docs/notes/real-plugin-survey.md; fixing it is its own task.
    if !hung.is_empty() {
        println!("SURVEY the host gave no reply for {hung:?}");
    }
}
