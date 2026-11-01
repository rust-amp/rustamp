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
const WM_KEYDOWN: u32 = 0x0100;
const WM_COMMAND: u32 = 0x0111;

#[link(name = "user32")]
unsafe extern "system" {
    fn FindWindowW(class: *const u16, title: *const u16) -> *mut std::ffi::c_void;
    fn PostMessageW(hwnd: *mut std::ffi::c_void, msg: u32, wparam: usize, lparam: isize) -> i32;
}

/// A plug-in's window is a real window on the test VM's single desktop, and the
/// picture is read back from a screenshot of that desktop. Two cases at once
/// would photograph each other's windows and time each other out, so the cases
/// of this binary run one after the other.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    the_player_runs(
        "w566",
        "vis_milk2.dll",
        &["d3dx9_31.dll"],
        "MilkDrop",
        "vis-end-to-end.png",
        None,
    );
}

#[test]
#[ignore = "VM-only: runs the real player, the real plug-in host and AVS; run in the test VM"]
fn the_player_starts_avs_and_feeds_it_what_it_plays() {
    the_player_runs(
        "w566",
        "vis_avs.dll",
        &[],
        "AVS",
        "vis-avs.png",
        Some("t=t-0.05"),
    );
}

/// Cockos Happy AVS, the 2.91 fork with the newer EEL. Its preset is written in
/// that syntax, which the older AVS cannot read: a picture proves the new EEL.
#[test]
#[ignore = "VM-only: runs the real player, the real plug-in host and CHAVS; run in the test VM"]
fn the_player_starts_chavs_and_feeds_it_what_it_plays() {
    the_player_runs(
        "chavs",
        "vis_chavs.dll",
        &[],
        "AVS",
        "vis-chavs.png",
        Some("t > -100 ? t = t - 0.05 : t = 0;"),
    );
}

/// The same fork with the older syntax: its difference from the test above is
/// the new EEL.
#[test]
#[ignore = "VM-only: runs the real player, the real plug-in host and CHAVS; run in the test VM"]
fn the_player_starts_chavs_with_a_classic_preset() {
    the_player_runs(
        "chavs",
        "vis_chavs.dll",
        &[],
        "AVS",
        "vis-chavs-classic.png",
        Some("t=t-0.05"),
    );
}

/// A preset as AVS keeps it in `vis_avs.dat`: the list clears every frame and
/// holds one SuperScope, whose per-frame code is `frame`. (The format is in the
/// BSD-licensed AVS source, `r_list.cpp` and `r_sscope.cpp`.)
fn superscope_preset(frame: &str) -> Vec<u8> {
    const SUPERSCOPE: u32 = 36;
    let mut scope = vec![1u8];
    for code in [
        "d=i+v*0.2; r=t+i*$PI*4; x=cos(r)*d; y=sin(r)*d",
        frame,
        "",
        "n=100",
    ] {
        if code.is_empty() {
            scope.extend_from_slice(&0u32.to_le_bytes());
        } else {
            scope.extend_from_slice(&(code.len() as u32 + 1).to_le_bytes());
            scope.extend_from_slice(code.as_bytes());
            scope.push(0);
        }
    }
    // Both channels, one white colour, lines.
    for value in [2u32, 1, 0x00FF_FFFF, 1] {
        scope.extend_from_slice(&value.to_le_bytes());
    }
    let mut preset = b"Nullsoft AVS Preset 0.2\x1a".to_vec();
    preset.push(1);
    preset.extend_from_slice(&SUPERSCOPE.to_le_bytes());
    preset.extend_from_slice(&(scope.len() as u32).to_le_bytes());
    preset.extend_from_slice(&scope);
    preset
}

/// Starts `plugin` in the real player and waits for its `render`, then for a
/// host window whose title holds `title_part`. `plugin` and its `companions`,
/// the DLLs it needs beside it, come from the staged `folder`, and `shot` names
/// the screenshot. `preset_frame` asks for a SuperScope preset with that
/// per-frame code, and for its picture on the screen.
fn the_player_runs(
    folder: &str,
    plugin: &str,
    companions: &[&str],
    title_part: &str,
    shot: &str,
    preset_frame: Option<&str>,
) {
    let _serial = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
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
    for file in std::iter::once(&plugin).chain(companions) {
        std::fs::copy(staged.join(folder).join(file), plugins.join(file)).unwrap();
    }
    if let Some(frame) = preset_frame {
        std::fs::write(plugins.join("vis_avs.dat"), superscope_preset(frame)).unwrap();
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
            "version = 2\n[plugins]\nplugin_dir = '{}'\nenabled = ['{plugin}']\n",
            plugins.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();

    // ---- the operator's act: approve this exact image ----
    let approve = Command::new(env!("CARGO_BIN_EXE_rustamp"))
        .arg("--approve-plugin")
        .arg(plugins.join(plugin))
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
                "{plugin}'s render was called: {rendering}; trace: {}",
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
        let png = out.join(shot);
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
        // The track's title reached the host's window, and the host's window
        // passes a plug-in's key on to the player.
        let told = wait_for(Duration::from_secs(10), || {
            std::fs::read_to_string(&trace)
                .unwrap_or_default()
                .contains("now playing: 1. ")
        });
        assert!(told, "the host never heard the title: {}", tail(&trace, 6));
        let text: Vec<u16> = "1. tone - Winamp".encode_utf16().chain(Some(0)).collect();
        // SAFETY: a terminated string; posting a key to the host's own window.
        unsafe {
            let host = FindWindowW(std::ptr::null(), text.as_ptr());
            assert!(!host.is_null(), "no host window with the track's title");
            PostMessageW(host, WM_KEYDOWN, b'V' as usize, 0);
        }
        let stopped = wait_for(Duration::from_secs(10), || {
            std::fs::read_to_string(&stderr_path)
                .unwrap_or_default()
                .contains("a plug-in asked for Stop")
        });
        note(
            &mut report,
            &out,
            format!("the key stopped the player: {stopped}"),
        );
        assert!(stopped, "player said: {}", tail(&stderr_path, 6));
        if preset_frame.is_some() {
            // The picture: the plug-in's window sits where the frame put it and
            // the frame draws its own chrome around the plug-in's area. Sample
            // inside that area rather than at a fixed place on the screen, or a
            // skinned title bar's tiles count as a picture and a black window
            // passes. The margins are the frame's chrome: 20 px of side rail and
            // 42 px of title bar.
            // The frame belongs to the plug-in host, and the host's process is
            // the only one a running case has. The title is not used to pick it:
            // `MainWindowTitle` is empty until Windows decides which window is
            // the process's main one, which raced with the screenshot.
            let host_pid: u32 = powershell(
                "(Get-Process -Name rustamp-plughost32 -ErrorAction SilentlyContinue | \
                 Sort-Object StartTime -Descending | \
                 Select-Object -First 1 -ExpandProperty Id)",
                15,
            )
            .trim()
            .parse()
            .unwrap_or(0);
            let frame = child_windows::find_window(host_pid, title_part, Duration::from_secs(15));
            assert!(
                !frame.is_null(),
                "no {title_part} frame: host pid {host_pid}, windows {title:?}"
            );
            let (win_left, win_top, win_right, win_bottom) = child_windows::outer_rect(frame);
            let (x0, y0) = (win_left + 20, win_top + 42);
            let (x1, y1) = (win_right - 20, win_bottom - 20);
            assert!(
                x1 > x0 && y1 > y0,
                "the {title_part} frame is too small to hold a picture: {win_left},{win_top}-{win_right},{win_bottom}"
            );
            let bright = powershell(
                &format!(
                    "Add-Type -AssemblyName System.Drawing; \
                     $b = New-Object System.Drawing.Bitmap '{}'; $n = 0; \
                     for ($y = {y0}; $y -lt {y1}; $y += 1) {{ for ($x = {x0}; $x -lt {x1}; $x += 1) {{ \
                     if ($b.GetPixel($x,$y).GetBrightness() -gt 0.2) {{ $n++ }} }} }}; $n",
                    png.display()
                ),
                60,
            );
            note(
                &mut report,
                &out,
                format!("the plug-in's own area is {x0},{y0}-{x1},{y1}; bright pixels: {bright}"),
            );
            let bright: u32 = bright.trim().parse().unwrap_or(0);
            assert!(
                bright > 50,
                "{plugin} drew nothing: {bright} bright pixels in its own area"
            );
        }
        assert!(
            rendering,
            "the toggle never reached a render: {}",
            tail(&trace, 8)
        );
        assert!(
            title.contains(title_part),
            "no {title_part} window: {title:?}; player said: {}",
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
    // The tests of this file overwrite one report: each keeps its own evidence.
    let _ = std::fs::copy(&trace, out.join(format!("{plugin}.trace.txt")));
    let _ = std::fs::copy(&stderr_path, out.join(format!("{plugin}.stderr.txt")));
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
