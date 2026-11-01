// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The player's own MilkDrop window, end to end, in the test VM.
//!
//! The real `rustamp.exe` runs (windowed, without a plug-in and without
//! `--null`) with a configuration of its own, and the test opens the window
//! through the local control channel: `Command::ToggleWindow(WindowId::Vis)`,
//! the same command the `Alt+D` shortcut and the Visualization menu send. It
//! then reads what the player says on stderr (the `MilkDrop:` notes), waits for
//! the OS window, and counts the non-black pixels of its body in a screenshot:
//! a preset picture, or the message that says why there is none.
//!
//! The second case configures an empty preset folder: the window must stay up
//! and show the message instead of a picture, and the player must say why.
//!
//! VM-only (it opens a window), `#[ignore]`d by default. Nothing of this test
//! touches the host.

use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};
use std::time::{Duration, Instant};

use rustamp_core::command::Command;
use rustamp_core::state::WindowId;
use rustamp_testkit::control::ControlClient;

#[cfg(windows)]
#[path = "child_windows/mod.rs"]
#[allow(dead_code)] // this test uses only the window geometry helpers it names
mod child_windows;

/// `CreateProcess` flag: no console window of the child's own.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The title of the player's own MilkDrop window (`window/milkdrop.rs`).
const WINDOW_TITLE: &str = "MilkDrop";

#[link(name = "user32")]
unsafe extern "system" {
    fn IsWindowVisible(hwnd: *mut std::ffi::c_void) -> i32;
}

/// A channel name no other test in this binary (or player) uses.
fn unique_name(label: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_millis())
        .unwrap_or(0);
    format!("test-{}-{n}-{label}-{millis}", std::process::id())
}

/// Appends a line to the report and writes the whole report at once, so a run
/// that is killed still leaves what it had learnt.
fn note(report: &mut Vec<String>, out: &Path, line: String) {
    report.push(line);
    let _ = std::fs::write(out.join("milkdrop-window.txt"), report.join("\r\n"));
}

/// A PowerShell one-liner without a window, stopped after `seconds`.
fn powershell(script: &str, seconds: u64) -> String {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    let Ok(mut child) = Process::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return "powershell did not start".to_string();
    };
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return format!("(no answer within {seconds} s)");
            }
        }
    }
    reader.join().unwrap_or_default().trim().to_string()
}

fn wait_for(deadline: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    condition()
}

fn tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join(" | ")
}

/// Kills the player at the end of a case, whatever the case did.
struct Player {
    child: Child,
    stderr: PathBuf,
}

impl Player {
    fn start(dir: &Path, config: &Path, name: &str) -> Self {
        let stderr = dir.join("player-stderr.txt");
        let mut command = Process::new(env!("CARGO_BIN_EXE_rustamp"));
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let child = command
            .arg("--no-audio")
            .arg("--seconds")
            .arg("90")
            .arg(format!("--control={name}"))
            .env("RUSTAMP_CONFIG", config)
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&stderr).unwrap())
            .stdin(Stdio::null())
            .spawn()
            .expect("spawn rustamp");
        Player { child, stderr }
    }

    fn said(&self) -> String {
        std::fs::read_to_string(&self.stderr).unwrap_or_default()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = Process::new("taskkill")
            .args(["/PID", &self.child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = self.child.wait();
    }
}

/// A `[milkdrop]` configuration. `preset_folder` is written when it is given.
fn write_config(dir: &Path, preset_folder: Option<&Path>) -> PathBuf {
    let config = dir.join("rustamp.toml");
    let mut text = String::from("[milkdrop]\npreset_seconds = 0\n");
    if let Some(folder) = preset_folder {
        text.push_str(&format!(
            "preset_folder = '{}'\n",
            folder.display().to_string().replace('\\', "/")
        ));
    }
    std::fs::write(&config, text).unwrap();
    config
}

/// Opens the window through the control channel and counts the lit pixels of
/// its body in a screenshot.
///
/// `empty_presets` configures a preset folder that holds no preset;
/// `expected_note` is the part of the `MilkDrop:` note the case must find on
/// stderr; `shot` names the screenshot kept as evidence.
/// The MilkDrop window is a real window on the test VM's single desktop and the
/// picture is read back from a copy of that screen area. Two cases at once would
/// photograph each other's windows, so the cases run one after the other.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run_case(label: &str, empty_presets: bool, expected_note: &str, shot: &str) {
    let _serial = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    rustamp_app::window::require_test_vm();
    let out = PathBuf::from(r"C:\rustamp\out\gui-session");
    let _ = std::fs::create_dir_all(&out);
    let mut report = Vec::new();

    let work = tempfile::tempdir().expect("tempdir");
    let folder = work.path().join("presets");
    if empty_presets {
        std::fs::create_dir_all(&folder).unwrap();
    }
    let config = write_config(work.path(), empty_presets.then_some(folder.as_path()));
    let name = unique_name(label);
    let mut player = Player::start(work.path(), &config, &name);
    let pipe = rustamp_app::control::pipe_name(&name);
    let mut client = ControlClient::connect(&pipe)
        .unwrap_or_else(|error| panic!("{pipe} (player said: {}): {error}", player.said()));

    // The operator's act: the Visualization menu's MilkDrop row, `Alt+D`.
    let response = client
        .send(Command::ToggleWindow(WindowId::Vis))
        .expect("the player answers ToggleWindow");
    assert!(
        response.ok,
        "ToggleWindow was refused: {}",
        response.message
    );
    note(&mut report, &out, format!("{label}: ToggleWindow sent"));

    // The note says what the session does; a preset note means a picture.
    let heard = wait_for(Duration::from_secs(60), || {
        player.said().contains("MilkDrop:") && player.said().contains(expected_note)
    });
    note(
        &mut report,
        &out,
        format!(
            "{label}: the player said {expected_note:?}: {heard}; stderr tail: {}",
            tail(&player.stderr, 4)
        ),
    );
    assert!(
        heard,
        "no MilkDrop note {expected_note:?}: {}",
        tail(&player.stderr, 8)
    );

    let window =
        child_windows::find_window(player.child.id(), WINDOW_TITLE, Duration::from_secs(30));
    let (left, top, right, bottom) = child_windows::outer_rect(window);
    let (width, height) = (right - left, bottom - top);
    note(
        &mut report,
        &out,
        format!("{label}: window at {left},{top} size {width}x{height}"),
    );
    assert!(
        width >= 200 && height >= 200,
        "the window never got its size: {width}x{height}"
    );

    // A few frames of silence: the session renders anyway.
    std::thread::sleep(Duration::from_secs(3));
    let png = out.join(shot);
    let body_top = rustamp_ui::chrome::TITLE_BAR_HEIGHT;
    let bright = powershell(
        &format!(
            "Add-Type -AssemblyName System.Drawing; \
             $b = New-Object System.Drawing.Bitmap {width},{height}; \
             $g = [System.Drawing.Graphics]::FromImage($b); \
             $g.CopyFromScreen({left},{top},0,0,$b.Size); \
             $b.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png); \
             $n = 0; \
             for ($y = {body_top}; $y -lt {height}; $y += 2) {{ \
                 for ($x = 0; $x -lt {width}; $x += 2) {{ \
                     if ($b.GetPixel($x,$y).GetBrightness() -gt 0.1) {{ $n++ }} \
                 }} \
             }}; $n",
            png.display()
        ),
        120,
    );
    note(
        &mut report,
        &out,
        format!("{label}: lit pixels (sampled) {bright}"),
    );
    let bright: u32 = bright
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("no pixel count: {bright}"));
    assert!(
        bright > 20,
        "{label}: the body is black, {bright} lit pixels; see {}",
        png.display()
    );

    // Closing the window through the same command leaves the player running.
    let response = client
        .send(Command::ToggleWindow(WindowId::Vis))
        .expect("the player answers the second ToggleWindow");
    assert!(response.ok, "{}", response.message);
    let closed = wait_for(Duration::from_secs(20), || {
        // SAFETY: `window` came from the OS enumeration; a destroyed handle is
        // reported as not visible.
        unsafe { IsWindowVisible(window) == 0 }
    });
    note(
        &mut report,
        &out,
        format!("{label}: the window closed: {closed}"),
    );
    assert!(
        closed,
        "the MilkDrop window stayed up: {}",
        tail(&player.stderr, 6)
    );
    assert!(
        player.child.try_wait().ok().flatten().is_none(),
        "closing MilkDrop quit the player"
    );

    note(
        &mut report,
        &out,
        format!("--- {label} player stderr ---\r\n{}", player.said()),
    );
}

#[test]
#[ignore = "VM-only: opens the player's MilkDrop window; run in the headless test VM"]
fn the_player_opens_the_milkdrop_window_and_renders_a_preset() {
    run_case("builtin", false, "preset:", "milkdrop-window.png");
}

#[test]
#[ignore = "VM-only: opens the player's MilkDrop window; run in the headless test VM"]
fn an_empty_preset_folder_keeps_the_window_up_with_its_message() {
    run_case(
        "empty",
        true,
        "holds no MilkDrop presets",
        "milkdrop-window-empty.png",
    );
}
