//! `cargo xtask <command>` - repository automation.
//!
//! Commands:
//! * `golden [--bless] [--skin <name>]` - run golden image tests.
//! * `capture [--list] [--class C] [--title T] [--out F.png] [--client-only]`
//!   - generic Windows window capture.
//! * `ring-cross` - 32-bit/64-bit shared-memory ring check.
//! * `wake-cross` - 32-bit/64-bit blocking wake check (RFC 0007).
//! * `plughost [--release]` - build the 32-bit plugin host.
//! * `package [options]` - package the player (x86_64) and the plugin host
//!   (i686) as separate artefacts, bundling the host beside the player.
//! * `pe-imports [--allow a.dll,b.dll] [--machine x64|x86] <file>...` - parse
//!   PE import tables offline and fail on any non-system DLL.

mod capture;
mod package;
mod pe_imports;
mod ring_cross;
mod wake_cross;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  golden [--bless] [--check] [--skin <name>]
                                     run golden tests (test fns named `golden_*`)
  golden-check [--skin <name>]       verify golden images match (exits non-zero on mismatch/missing)
  capture [options]                  capture a window to PNG (Windows only); see `capture --help`
  ring-cross                         cross-bitness shared-memory ring check (Windows only)
  wake-cross                         cross-bitness blocking wake check (RFC 0007, Windows only)
  plughost [--release]               build rustamp-plughost for i686-pc-windows-msvc
  package [options]                  package player (x86_64) and plugin host (i686) separately
  pe-imports [options] <file>...     offline PE import check: only Windows system DLLs allowed";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = args.next();
    let rest: Vec<String> = args.collect();
    let result = match cmd.as_deref() {
        Some("golden") => golden(&rest),
        Some("golden-check" | "check-golden" | "check-goldens") => golden_check(&rest),
        Some("capture") => capture::run(&rest),
        Some("ring-cross") => ring_cross::run(&rest),
        Some("wake-cross") => wake_cross::run(&rest),
        Some("plughost") => plughost(&rest),
        Some("package") => package::run(&rest),
        Some("pe-imports") => pe_imports::run(&rest),
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `CreateProcess` flag: run the child with a console but **no console window**.
///
/// A console-subsystem child spawned without it gets a console window of its
/// own when the parent has no console — and a window on the owner's desktop is
/// a rule violation (`docs/HANDOFF.md`, the GUI rule), even when it is "only" a
/// console. Nothing graphical may run on the host.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A [`Command`] for `program` that cannot raise a window.
pub(crate) fn no_window_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

pub(crate) fn cargo() -> Command {
    no_window_command(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

pub(crate) fn run_status(cmd: &mut Command) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn {cmd:?}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("command failed: {status}"))
    }
}

/// `cargo xtask plughost [--release]`
fn plughost(args: &[String]) -> Result<(), String> {
    let mut cmd = cargo();
    cmd.args([
        "build",
        "-p",
        "rustamp-plughost",
        "--target",
        "i686-pc-windows-msvc",
    ]);
    for a in args {
        match a.as_str() {
            "--release" => {
                cmd.arg("--release");
            }
            other => return Err(format!("unknown plughost option `{other}`")),
        }
    }
    run_status(&mut cmd)
}

/// `cargo xtask golden [--bless] [--check] [--skin <name>]`
///
/// Convention: golden tests are ordinary `#[test]` functions whose names start
/// with `golden_`. By convention they embed the skin namespace, i.e.
/// `golden_lookalike_<case>` and `golden_classic_base_<case>`, so `--skin`
/// narrows the libtest filter to `golden_<skin with '-' -> '_'>`.
fn golden(args: &[String]) -> Result<(), String> {
    let mut bless = false;
    let mut check = false;
    let mut skin: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--bless" => bless = true,
            "--check" => check = true,
            "--skin" => skin = Some(it.next().ok_or("--skin needs a value")?.clone()),
            other => return Err(format!("unknown golden option `{other}`")),
        }
    }
    if check && bless {
        return Err(
            "--check and --bless are mutually exclusive; check mode refuses to overwrite goldens"
                .to_string(),
        );
    }
    let filter = match &skin {
        Some(s) => format!("golden_{}", s.replace('-', "_")),
        None => "golden_".to_string(),
    };
    let root = rustamp_testkit::workspace_root();
    let out_dir = root.join("target").join("golden-out");
    let _ = std::fs::remove_dir_all(&out_dir);

    let mut cmd = cargo();
    cmd.current_dir(&root)
        .args([
            "test",
            "--workspace",
            "--all-features",
            "--tests",
            "--no-fail-fast",
            "--",
        ])
        .arg(&filter);
    if check {
        cmd.env_remove("RUSTAMP_BLESS");
        cmd.env("RUSTAMP_BLESS", "0");
        println!("xtask golden: check mode (verifying committed images; blessing disabled)");
    } else if bless {
        cmd.env("RUSTAMP_BLESS", "1");
        println!("xtask golden: blessing enabled (RUSTAMP_BLESS=1)");
    }
    if let Some(s) = &skin {
        cmd.env("RUSTAMP_GOLDEN_SKIN", s);
    }
    if rustamp_testkit::testdata_dir().is_none() {
        println!("xtask golden: RUSTAMP_TESTDATA unset; classic-base tests will be skipped");
    }
    let result = run_status(&mut cmd);

    let mut files = Vec::new();
    collect_files(&out_dir, &mut files);
    files.sort();
    println!("\n== target/golden-out ==");
    if files.is_empty() {
        println!("(no mismatch artefacts)");
    }
    for f in &files {
        println!("  {}", f.strip_prefix(&root).unwrap_or(f).display());
    }

    if check {
        if result.is_err() || !files.is_empty() {
            return Err(format!(
                "golden check failed: {} mismatch artifact(s) found or test exited with error",
                files.len()
            ));
        }
        println!("xtask golden: check passed (all golden images matched)");
        return Ok(());
    }

    result
}

/// `cargo xtask golden-check [--skin <name>]`
///
/// Runs the golden comparison in non-mutating check mode and exits non-zero on
/// any mismatch or missing image. Blessing is disallowed.
fn golden_check(args: &[String]) -> Result<(), String> {
    let mut check_args = vec!["--check".to_string()];
    check_args.extend_from_slice(args);
    golden(&check_args)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_files(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_check_and_bless_are_mutually_exclusive() {
        let err = golden(&["--check".to_string(), "--bless".to_string()]).unwrap_err();
        assert!(err.contains("mutually exclusive"));
    }

    #[test]
    fn golden_check_subcommand_rejects_bless() {
        let err = golden_check(&["--bless".to_string()]).unwrap_err();
        assert!(err.contains("mutually exclusive"));
    }

    #[test]
    fn golden_unknown_option_fails() {
        let err = golden(&["--invalid-flag".to_string()]).unwrap_err();
        assert!(err.contains("unknown golden option"));
    }
}
