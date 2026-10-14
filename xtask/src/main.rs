//! `cargo xtask <command>` - repository automation.
//!
//! Commands:
//! * `golden [--bless] [--skin <name>]` - run golden image tests.
//! * `capture [--list] [--class C] [--title T] [--out F.png] [--client-only]`
//!   - generic Windows window capture.
//! * `plughost [--release]` - build the 32-bit plugin host.
//! * `package` - not implemented yet.

mod capture;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  golden [--bless] [--skin <name>]   run golden tests (test fns named `golden_*`)
  capture [options]                  capture a window to PNG (Windows only); see `capture --help`
  plughost [--release]               build rustamp-plughost for i686-pc-windows-msvc
  package                            (not implemented)";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = args.next();
    let rest: Vec<String> = args.collect();
    let result = match cmd.as_deref() {
        Some("golden") => golden(&rest),
        Some("capture") => capture::run(&rest),
        Some("plughost") => plughost(&rest),
        // TODO(M2+): NSIS installer packaging.
        Some("package") => {
            eprintln!("xtask package: not implemented yet");
            Ok(())
        }
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

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn run_status(cmd: &mut Command) -> Result<(), String> {
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

/// `cargo xtask golden [--bless] [--skin <name>]`
///
/// Convention: golden tests are ordinary `#[test]` functions whose names start
/// with `golden_`. By convention they embed the skin namespace, i.e.
/// `golden_lookalike_<case>` and `golden_classic_base_<case>`, so `--skin`
/// narrows the libtest filter to `golden_<skin with '-' -> '_'>`.
fn golden(args: &[String]) -> Result<(), String> {
    let mut bless = false;
    let mut skin: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--bless" => bless = true,
            "--skin" => skin = Some(it.next().ok_or("--skin needs a value")?.clone()),
            other => return Err(format!("unknown golden option `{other}`")),
        }
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
            "--no-fail-fast",
            "--",
        ])
        .arg(&filter);
    if bless {
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
    result
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
