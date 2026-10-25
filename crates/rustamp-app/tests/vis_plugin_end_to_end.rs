//! The player starts MilkDrop, end to end, in the test VM.
//!
//! The real `rustamp.exe` runs (silent windowed mode, `--no-audio`) with a
//! plug-in folder that holds `vis_milk2.dll` laid out the way Winamp lays it out
//! (`Plugins\vis_milk2.dll`, `Plugins\Milkdrop2\data`, the C runtime's assembly
//! folder one level above), an approval for that exact image made with
//! `rustamp --approve-plugin`, and a long synthetic tone to play. The test then
//! sends the player the command id of "Start/Stop plug-in" (`WM_COMMAND`
//! 40192, the id the Visualization menu entry and `Ctrl+Shift+K` carry) and
//! watches what the plug-in host does: it loads the plug-in on start-up, the
//! command reaches it, MilkDrop calls `render`, and a screenshot of the screen
//! is kept.
//!
//! VM-only (a plug-in DLL runs only there), `#[ignore]`d by default. The plug-in
//! files are staged by the VM runner under `%RUSTAMP_TESTPLUGINS%` (`w566` and
//! `milkdrop2-data`); nothing of them is in the repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(windows)]
#[path = "child_windows/mod.rs"]
#[allow(dead_code)] // this test uses only `find_window`
mod child_windows;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const WM_COMMAND: u32 = 0x0111;

#[link(name = "user32")]
unsafe extern "system" {
    fn PostMessageW(hwnd: *mut std::ffi::c_void, msg: u32, wparam: usize, lparam: isize) -> i32;
}

fn testplugins() -> PathBuf {
    std::env::var_os("RUSTAMP_TESTPLUGINS")
        .map(PathBuf::from)
        .into_iter()
        .chain([
            PathBuf::from(r"C:\rustamp\testplugins"),
            PathBuf::from(r"D:\code\rustamp-testplugins"),
        ])
        .find(|dir| dir.join("w566").join("vis_milk2.dll").is_file())
        .expect("REFUSING LOUDLY: the staged plug-ins (w566\\vis_milk2.dll) were not found")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// A mono 16-bit 22.05 kHz WAV of `seconds`, a slowly rising tone with a beat, so
/// there is something to see in the analysis the whole time.
fn tone_wav(seconds: u32) -> Vec<u8> {
    let rate = 22_050u32;
    let samples = rate * seconds;
    let mut data = Vec::with_capacity(samples as usize * 2);
    let mut phase = 0.0f32;
    for n in 0..samples {
        let t = n as f32 / rate as f32;
        let frequency = 220.0 + 110.0 * (t * 0.7).sin();
        phase += std::f32::consts::TAU * frequency / rate as f32;
        let beat = 0.55 + 0.45 * (t * 4.0).sin().abs();
        let value = (phase.sin() * beat * 0.6 * f32::from(i16::MAX)) as i16;
        data.extend_from_slice(&value.to_le_bytes());
    }
    let mut wav = Vec::with_capacity(44 + data.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    wav
}

/// Appends a line to the report and writes the whole report at once, so a run
/// that is killed still leaves what it had learnt.
fn note(report: &mut Vec<String>, out: &Path, line: String) {
    report.push(line);
    let _ = std::fs::write(out.join("vis-end-to-end.txt"), report.join("\r\n"));
}

/// A PowerShell one-liner without a window, stopped after `seconds`.
fn powershell(script: &str, seconds: u64) -> String {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    let Ok(mut child) = Command::new("powershell")
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

#[test]
#[ignore = "VM-only: runs the real player, the real plug-in host and MilkDrop; run in the test VM"]
fn the_player_starts_milkdrop_and_feeds_it_what_it_plays() {
    rustamp_app::window::require_test_vm();
    let out = PathBuf::from(r"C:\rustamp\out\gui-session");
    let _ = std::fs::create_dir_all(&out);
    let mut report = Vec::new();

    // ---- the plug-in folder, laid out the way Winamp lays it out ----
    let staged = testplugins();
    let work = tempfile::tempdir().expect("tempdir");
    let root = work.path();
    let plugins = root.join("Plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    for file in ["vis_milk2.dll", "d3dx9_31.dll"] {
        std::fs::copy(staged.join("w566").join(file), plugins.join(file)).unwrap();
    }
    copy_tree(
        &staged.join("w566").join("Microsoft.VC90.CRT"),
        &root.join("Microsoft.VC90.CRT"),
    );
    copy_tree(
        &staged
            .join("milkdrop2-data")
            .join("Plugins")
            .join("Milkdrop2"),
        &plugins.join("Milkdrop2"),
    );
    let wav = root.join("tone.wav");
    std::fs::write(&wav, tone_wav(120)).unwrap();
    let config = root.join("rustamp.toml");
    std::fs::write(
        &config,
        format!(
            "[plugins]\nplugin_dir = '{}'\n",
            plugins.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();

    // ---- the operator's act: approve this exact image ----
    let approve = Command::new(env!("CARGO_BIN_EXE_rustamp"))
        .arg("--approve-plugin")
        .arg(plugins.join("vis_milk2.dll"))
        .env("RUSTAMP_CONFIG", &config)
        .output()
        .expect("run --approve-plugin");
    note(
        &mut report,
        &out,
        format!(
            "approve: exit {:?}; {}",
            approve.status.code(),
            String::from_utf8_lossy(&approve.stdout).replace('\n', " | ")
        ),
    );
    assert!(approve.status.success(), "{approve:?}");
    assert!(root.join(rustamp_prefs::APPROVALS_FILE).is_file());

    // ---- the player ----
    let trace = root.join("host-trace.txt");
    let stderr_path = root.join("player-stderr.txt");
    let mut player = {
        use std::os::windows::process::CommandExt;
        Command::new(env!("CARGO_BIN_EXE_rustamp"))
            .arg("--no-audio")
            .arg(&wav)
            .env("RUSTAMP_CONFIG", &config)
            .env("RUSTAMP_HOST_TRACE", &trace)
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&stderr_path).unwrap())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("spawn rustamp")
    };
    let pid = player.id();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let window = child_windows::find_window(pid, "Rustamp", Duration::from_secs(30));
        note(&mut report, &out, "the player's window is up".to_string());

        // The start-up load: the player asks the host for the enabled plug-in.
        let loaded = wait_for(Duration::from_secs(40), || {
            std::fs::read_to_string(&trace)
                .unwrap_or_default()
                .contains("<- LoadLibraryExW")
        });
        note(
            &mut report,
            &out,
            format!("the host loaded the plug-in at start-up: {loaded}"),
        );
        assert!(loaded, "the plug-in was never loaded: {}", tail(&trace, 6));
        std::thread::sleep(Duration::from_secs(3));

        // "Start/Stop plug-in".
        // SAFETY: posting a command to the player's own window.
        unsafe {
            PostMessageW(window, WM_COMMAND, 40192, 0);
        }
        note(&mut report, &out, "WM_COMMAND 40192 posted".to_string());

        let rendering = wait_for(Duration::from_secs(40), || {
            std::fs::read_to_string(&trace)
                .unwrap_or_default()
                .contains("-> render")
        });
        note(
            &mut report,
            &out,
            format!(
                "MilkDrop's render was called: {rendering}; trace: {}",
                tail(&trace, 4)
            ),
        );
        std::thread::sleep(Duration::from_secs(5));
        let title = powershell(
            "(Get-Process -Name rustamp-plughost32 -ErrorAction SilentlyContinue | \
             ForEach-Object { $_.MainWindowTitle }) -join ';'",
            10,
        );
        note(&mut report, &out, format!("host window title: {title:?}"));
        let png = out.join("vis-end-to-end.png");
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
        note(&mut report, &out, "screenshot taken".to_string());
        assert!(
            rendering,
            "the toggle never reached a render: {}",
            tail(&trace, 8)
        );
        assert!(
            title.contains("MilkDrop"),
            "no MilkDrop window: {title:?}; player said: {}",
            tail(&stderr_path, 6)
        );
    }));

    let said = std::fs::read_to_string(&stderr_path).unwrap_or_default();
    note(
        &mut report,
        &out,
        format!("--- player stderr ---\r\n{said}"),
    );
    note(
        &mut report,
        &out,
        format!(
            "--- host trace tail ---\r\n{}",
            tail(&trace, 25).replace(" | ", "\r\n")
        ),
    );
    // The whole tree goes: the player and the host it started.
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = player.wait();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
