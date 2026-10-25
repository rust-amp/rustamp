//! MilkDrop (a real Winamp 5.x visualization plug-in) in the production host,
//! fed by the player's own messages. VM-only. See the test's own docs.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::path::PathBuf;
use std::time::Duration;

use rustamp_ipc::{Message, Transport};
use rustamp_plughost::approval::ApprovalStore;
use rustamp_plughost::host_window::require_test_vm;
use rustamp_plugin_abi::PluginKind;

#[path = "common/plugin_host.rs"]
mod plugin_host;
use plugin_host::*;

/// Appends a line to the report and writes the whole report out at once, so a
/// run that is killed (the runner's per-binary timeout) still leaves what it
/// had learnt.
fn note(report: &mut Vec<String>, out: &std::path::Path, line: String) {
    report.push(line);
    let _ = std::fs::write(out.join("milkdrop-result.txt"), report.join("\r\n"));
}

/// A frame of synthetic audio data: a sine in the waveform and bars that move
/// with `step` in the spectrum, so successive frames differ.
fn synthetic_frame(step: u64) -> rustamp_proto::vis::VisFrame {
    use rustamp_proto::vis::{VIS_CHANNELS, VIS_SAMPLES, VisFrame};
    let mut frame = VisFrame {
        timestamp_us: step * 33_000,
        ..VisFrame::default()
    };
    for channel in 0..VIS_CHANNELS {
        for index in 0..VIS_SAMPLES {
            let phase = (index as f32 * 0.07) + (step as f32 * 0.21) + channel as f32;
            frame.waveform[channel][index] = 0.7 * phase.sin();
            let band = (index as f32 / 24.0 + step as f32 * 0.1).sin().abs();
            frame.spectrum[channel][index] = 0.9 * band / (1.0 + index as f32 / 40.0);
        }
    }
    frame
}

/// MilkDrop in the production host, fed by the player's own messages: the
/// plug-in is loaded, the toggle starts it, frames and a "playing" state keep
/// it drawing, and two screenshots of the screen tell whether anything changes.
/// The result is written where the runner copies the guest's results back from.
///
/// Informational, like the survey: what it asserts is that the host stays alive
/// and the toggle reaches a visualization; what is on the screen is for a
/// person (the PNGs) and for the pixel comparison written into the result.
#[test]
#[ignore = "VM-only: runs a real visualization plug-in (DirectX 9) in the real i686 host; run in the test VM"]
fn milkdrop_runs_in_the_host_while_the_player_feeds_it() {
    use rustamp_proto::command::Command as PlayerCommand;
    use rustamp_proto::state::{PlayerState, TransportState};

    require_test_vm();
    let dir = survey_dir();
    let exe = host_exe();
    let dll = dir.join("vis_milk2.dll");
    assert!(dll.is_file(), "{} is missing", dll.display());
    let out = PathBuf::from(r"C:\rustamp\out\gui-session");
    let _ = std::fs::create_dir_all(&out);
    let mut report = vec![format!("host={} plug-in={}", exe.display(), dll.display())];

    let scratch = std::env::temp_dir().join(format!("rustamp-milkdrop-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let approvals = scratch.join("approved.toml");
    let mut store = ApprovalStore::new();
    store
        .approve_bytes("vis_milk2.dll", &std::fs::read(&dll).unwrap())
        .expect("approvable");
    store.save_to_file(&approvals).unwrap();

    let mut host = Host::spawn(&exe, &approvals, &scratch);
    match host.replies.recv_timeout(REPLY_DEADLINE) {
        Ok(Message::Hello { .. }) => {}
        other => panic!("the host's first frame must be Hello, got {other:?}"),
    }
    host.writer
        .send(&Message::LoadPlugin {
            path: dll.to_string_lossy().into_owned(),
            kind: PluginKind::Visualization,
        })
        .unwrap();
    let loaded = host.replies.recv_timeout(REPLY_DEADLINE);
    note(&mut report, &out, format!("load: {loaded:?}"));
    assert!(
        matches!(loaded, Ok(Message::PluginLoaded { .. })),
        "{loaded:?}"
    );

    let playing = PlayerState {
        transport: TransportState::Playing,
        ..PlayerState::default()
    };
    host.writer
        .send(&Message::PlayerStateUpdate(Box::new(playing)))
        .unwrap();
    host.writer
        .send(&Message::CommandToHost(PlayerCommand::ToggleVisPlugin))
        .unwrap();

    // The frames go out from a thread of their own: a host whose UI thread is
    // stuck stops reading (its inbound queue is bounded and the pipe fills), and
    // a feeder that blocks there must not take the observer down with it. The
    // observer below keeps writing what it sees once a second.
    let sent = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // The host is taken apart: the writer goes to the feeder, the reply channel
    // stays alive here (its reader thread stops when nobody listens), and the
    // child handle stays with the observer.
    let Host {
        mut child,
        mut writer,
        replies: _replies,
    } = host;
    let feeder = {
        let (sent, stop) = (sent.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut step = 0u64;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let frame = synthetic_frame(step);
                step += 1;
                let wire = rustamp_ipc::message::VisFrameWire::from(&frame);
                if writer.send(&Message::Vis(Box::new(wire))).is_err() {
                    break;
                }
                sent.store(step, std::sync::atomic::Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(33));
            }
        })
    };

    let started = std::time::Instant::now();
    let mut shots = 0;
    let mut shot_at = [3u64, 8u64].into_iter();
    let mut next_shot = shot_at.next();
    let mut alive = true;
    let mut last_second = 0u64;
    while started.elapsed() < Duration::from_secs(10) {
        let step = sent.load(std::sync::atomic::Ordering::Relaxed);
        if feeder.is_finished() {
            alive = false;
            break;
        }
        if let Some(at) = next_shot
            && started.elapsed() >= Duration::from_secs(at)
        {
            shots += 1;
            let png = out.join(format!("milkdrop-{shots}.png"));
            let title = powershell(
                &format!("(Get-Process -Id {}).MainWindowTitle", child.id()),
                8,
            );
            note(
                &mut report,
                &out,
                format!("t={at}s host window title: {title:?}"),
            );
            powershell(
                &format!(
                    "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; \
                 $b = New-Object System.Drawing.Bitmap 1024,768; \
                 $g = [System.Drawing.Graphics]::FromImage($b); \
                 $g.CopyFromScreen(0,0,0,0,$b.Size); \
                 $b.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png)",
                    png.display()
                ),
                15,
            );
            next_shot = shot_at.next();
        }
        // Once a second: what the host said and the last calls it made into the
        // plug-in, so a run that dies or stalls still shows where.
        if started.elapsed().as_secs() > last_second {
            last_second = started.elapsed().as_secs();
            let tail = |name: &str, lines: usize| {
                let text = std::fs::read_to_string(scratch.join(name)).unwrap_or_default();
                let all: Vec<&str> = text.lines().collect();
                all[all.len().saturating_sub(lines)..].join(" | ")
            };
            note(
                &mut report,
                &out,
                format!(
                    "t={last_second}s frames={step} stderr: {} || trace: {}",
                    tail("stderr.txt", 3),
                    tail("trace.txt", 4)
                ),
            );
        }
        std::thread::sleep(Duration::from_millis(33));
    }
    let exited = child.try_wait().ok().flatten();
    note(
        &mut report,
        &out,
        format!(
            "frames sent: {}; host exited early: {exited:?}",
            sent.load(std::sync::atomic::Ordering::Relaxed)
        ),
    );
    // The host is killed rather than asked to quit: a host stuck in the plug-in
    // would not hear a stop request, and killing it unblocks the feeder too.
    let _ = child.kill();
    let _ = child.wait();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = feeder.join();
    let stderr = std::fs::read_to_string(scratch.join("stderr.txt")).unwrap_or_default();
    let trace = std::fs::read_to_string(scratch.join("trace.txt")).unwrap_or_default();
    note(
        &mut report,
        &out,
        format!("--- host stderr ---\r\n{stderr}"),
    );
    let tail: Vec<&str> = trace.lines().rev().take(25).collect();
    note(
        &mut report,
        &out,
        format!(
            "--- last trace lines ---\r\n{}",
            tail.into_iter().rev().collect::<Vec<_>>().join("\r\n")
        ),
    );
    let _ = std::fs::write(out.join("milkdrop-result.txt"), report.join("\r\n"));
    let _ = std::fs::remove_dir_all(&scratch);

    assert!(
        alive && exited.is_none(),
        "the host died while MilkDrop ran: {exited:?}"
    );
    assert!(
        stderr.contains("started"),
        "the toggle never started a visualization: {stderr}"
    );
}
