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

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use rustamp_ipc::{Message, Transport};
use rustamp_plughost::approval::ApprovalStore;
use rustamp_plughost::host_window::require_test_vm;
use rustamp_plugin_abi::PluginKind;

#[path = "common/plugin_host.rs"]
mod plugin_host;
use plugin_host::*;

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

#[derive(Debug)]
enum Outcome {
    Loaded(String),
    Refused(String),
    Hung,
    Died,
    HungAt(String),
    DiedWith(String),
}

impl Outcome {
    fn describe(&self) -> String {
        match self {
            Outcome::Loaded(name) => format!("LOADED as {name:?}"),
            Outcome::Refused(why) => format!("REFUSED: {why}"),
            Outcome::Hung => "HUNG (no reply)".to_string(),
            Outcome::Died => "DIED (the host's channel closed)".to_string(),
            Outcome::HungAt(trace) => format!("HUNG, last calls: {trace}"),
            Outcome::DiedWith(what) => format!("DIED, {what}"),
        }
    }
}

/// The `Microsoft.VC*.CRT` assembly (and version) named in the DLL's embedded
/// manifest, if there is one: a plain byte search, the manifest is XML text.
fn wanted_runtime(dll: &Path) -> Option<String> {
    let bytes = std::fs::read(dll).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let at = text.find("Microsoft.VC")?;
    let window = &text[at..(at + 160).min(text.len())];
    let version = window
        .split("version=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or("?");
    let name: String = window
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '"')
        .collect();
    Some(format!("{name} {version}"))
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

    let mut host = Host::spawn(exe, &approvals, &scratch);
    let mut sent_at_ms = 0u128;
    let outcome = (|| {
        match host.replies.recv_timeout(REPLY_DEADLINE) {
            Ok(Message::Hello { .. }) => {}
            Ok(other) => return Outcome::Refused(format!("unexpected first frame {other:?}")),
            Err(mpsc::RecvTimeoutError::Timeout) => return Outcome::Hung,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Outcome::Died,
        }
        sent_at_ms = now_ms();
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
    // A host that is stuck may be stuck behind a dialog the plug-in or its
    // runtime raised (a missing runtime shows one and waits for a click): the
    // window title of the process says so before the host is told to quit.
    let stuck_window = if matches!(outcome, Outcome::Hung) {
        window_title_of(host.child.id())
    } else {
        String::new()
    };
    let _ = host.writer.send(&Message::Shutdown);
    std::thread::sleep(Duration::from_millis(500));
    let _ = host.child.kill();
    let _ = host.child.wait();
    // What the host last did, and what it said, is the whole point of a hang.
    let tail = |name: &str, lines: usize| -> String {
        let text = std::fs::read_to_string(scratch.join(name)).unwrap_or_default();
        let all: Vec<&str> = text.lines().collect();
        all[all.len().saturating_sub(lines)..].join(" | ")
    };
    // Trace lines are `<unix ms> <event>`: shown relative to the load request, so
    // a slow return reads as a number, not as a guess.
    let trace = tail("trace.txt", 40)
        .split(" | ")
        .map(|line| match line.split_once(' ') {
            Some((ms, event)) => match ms.parse::<u128>() {
                Ok(ms) => format!("+{}ms {event}", ms as i128 - sent_at_ms as i128),
                Err(_) => line.to_string(),
            },
            None => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" | ");
    let said = tail("stderr.txt", 4);
    let _ = std::fs::remove_dir_all(&scratch);
    match outcome {
        Outcome::Hung => Outcome::HungAt(format!("{trace}; window of the host: {stuck_window:?}")),
        Outcome::Died => Outcome::DiedWith(format!("trace: {trace}; stderr: {said}")),
        Outcome::Refused(why) => Outcome::Refused(format!("{why}; trace: {trace}")),
        other => other,
    }
}

#[test]
#[ignore = "VM-only: spawns the real i686 host and maps real 32-bit plug-in DLLs; run in the test VM"]
fn survey_of_real_plugins_through_the_production_host() {
    require_test_vm();
    let dir = survey_dir();
    let exe = host_exe();
    // The VC++ 2008 runtime is a side-by-side assembly: a plug-in's manifest is
    // resolved against the folder of the **process** (the host), not against the
    // plug-in's own folder, which is where Winamp keeps it. Staging it beside
    // the host mirrors a Winamp install.
    let crt = dir.join("Microsoft.VC90.CRT");
    if crt.is_dir()
        && let Some(host_dir) = exe.parent()
    {
        let target = host_dir.join("Microsoft.VC90.CRT");
        let _ = std::fs::create_dir_all(&target);
        for entry in std::fs::read_dir(&crt).unwrap().filter_map(Result::ok) {
            let _ = std::fs::copy(entry.path(), target.join(entry.file_name()));
        }
    }
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
        // The runtime assembly the plug-in's embedded manifest asks for.
        if let Some(wanted) = wanted_runtime(dll) {
            table.push(format!("SURVEY   {file} wants {wanted}"));
        }
        table.push(format!("SURVEY {file} [{kind:?}]: {}", outcome.describe()));
        match outcome {
            Outcome::Loaded(_) => loaded += 1,
            Outcome::Hung | Outcome::HungAt(_) => hung.push(file),
            Outcome::Refused(_) | Outcome::Died | Outcome::DiedWith(_) => {}
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
