//! `cargo xtask package` - ship the GPL player and the permissive plugin host
//! as separate artefacts.
//!
//! The packaging logic lives here so that the licensing split from RFC 0004 is
//! reflected in the produced layout: `rustamp.exe` (x86_64, GPL-3.0-or-later
//! plus `PLUGIN-EXCEPTION.md`) and `rustamp-plughost32.exe` (i686, MIT OR
//! Apache-2.0) never end up in the same shippable item, and the host package
//! carries no GPL binary. A manifest records what went where.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{cargo, run_status};

const PLAYER_CRATE: &str = "rustamp-app";
const PLAYER_BIN: &str = "rustamp";
const PLAYER_TARGET: &str = "x86_64-pc-windows-msvc";

const HOST_CRATE: &str = "rustamp-plughost";
const HOST_BIN: &str = "rustamp-plughost32";
const HOST_TARGET: &str = "i686-pc-windows-msvc";

// GPL closure check for the host runs against this crate list. Kept in sync
// with `.github/scripts/check-permissive-licenses.sh`.
const HOST_PERMISSIVE_CRATES: &[&str] = &["rustamp-plughost"];

/// CLI arguments for `package`.
type PackageArgs = PackageArgsParsed;

pub fn run(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args)?;
    if parsed.help {
        print_help();
        return Ok(());
    }

    let root = rustamp_testkit::workspace_root();
    let out_dir = parsed
        .out
        .clone()
        .unwrap_or_else(|| root.join("target").join("package"));

    // The GPL-free check for the host side is the load-bearing acceptance test.
    // Run it on every invocation, including dry runs, so the proof is always
    // exercised without needing a release build.
    check_host_closure(&root)?;

    if parsed.dry_run || parsed.check {
        println!("xtask package: dry run (no files written)");
        println!("  workspace root: {}", root.display());
        println!("  output dir:     {}", out_dir.display());
        if parsed.player_only {
            println!("  scope: player only");
        } else if parsed.host_only {
            println!("  scope: host only");
        } else {
            println!("  scope: player + host");
        }
        println!("  player target:  {} ({})", PLAYER_TARGET, PLAYER_CRATE);
        println!("  host target:    {} ({})", HOST_TARGET, HOST_CRATE);
        println!("  GPL-free check: OK (host closure contains no copyleft crate)");
        return Ok(());
    }

    let profile = if parsed.release { "release" } else { "debug" };

    fs::create_dir_all(&out_dir).map_err(|e| format!("create output dir: {e}"))?;

    let build_player = !parsed.host_only;
    let build_host = !parsed.player_only;

    let player_src = if build_player {
        Some(build_artifact(
            &root,
            PLAYER_CRATE,
            PLAYER_TARGET,
            PLAYER_BIN,
            parsed.release,
        )?)
    } else {
        None
    };
    let host_src = if build_host {
        Some(build_artifact(
            &root,
            HOST_CRATE,
            HOST_TARGET,
            HOST_BIN,
            parsed.release,
        )?)
    } else {
        None
    };

    let mut manifest_items: Vec<String> = Vec::new();

    if let Some(src) = &player_src {
        let dir = out_dir.join("player");
        fs::create_dir_all(&dir).map_err(|e| format!("create player dir: {e}"))?;
        let dest_bin = dir.join(binary_name(PLAYER_BIN));
        fs::copy(src, &dest_bin).map_err(|e| format!("copy player binary: {e}"))?;
        copy_file(&root.join("LICENSE"), &dir.join("LICENSE"))?;
        copy_file(
            &root.join("PLUGIN-EXCEPTION.md"),
            &dir.join("PLUGIN-EXCEPTION.md"),
        )?;

        let per_package = serde_manifest_player(profile);
        fs::write(dir.join("manifest.json"), per_package)
            .map_err(|e| format!("write player manifest: {e}"))?;

        manifest_items.push("player".to_string());
        println!("xtask package: player -> {}", dir.display());
    }

    if let Some(src) = &host_src {
        let dir = out_dir.join("host");
        fs::create_dir_all(&dir).map_err(|e| format!("create host dir: {e}"))?;
        let dest_bin = dir.join(binary_name(HOST_BIN));
        fs::copy(src, &dest_bin).map_err(|e| format!("copy host binary: {e}"))?;
        copy_file(&root.join("LICENSE-MIT"), &dir.join("LICENSE-MIT"))?;
        copy_file(&root.join("LICENSE-APACHE"), &dir.join("LICENSE-APACHE"))?;

        // Prove the host package is GPL-free: the closure check already ran,
        // and the directory must contain no GPL binary.
        audit_host_dir(&dir)?;

        let per_package = serde_manifest_host(profile);
        fs::write(dir.join("manifest.json"), per_package)
            .map_err(|e| format!("write host manifest: {e}"))?;

        manifest_items.push("host".to_string());
        println!("xtask package: host   -> {}", dir.display());
    }

    // Top-level manifest naming what went where.
    let top = build_top_manifest(&out_dir, profile, build_player, build_host);
    fs::write(out_dir.join("manifest.json"), top).map_err(|e| format!("write manifest: {e}"))?;

    println!(
        "xtask package: manifest -> {}",
        out_dir.join("manifest.json").display()
    );
    println!("xtask package: done ({})", manifest_items.join(" + "));
    Ok(())
}

#[derive(Clone)]
struct PackageArgsParsed {
    release: bool,
    dry_run: bool,
    player_only: bool,
    host_only: bool,
    check: bool,
    out: Option<PathBuf>,
    help: bool,
}

fn parse_args(args: &[String]) -> Result<PackageArgs, String> {
    let mut parsed = PackageArgs {
        release: true,
        dry_run: false,
        player_only: false,
        host_only: false,
        check: false,
        out: None,
        help: false,
    };

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--release" | "--release=true" => parsed.release = true,
            "--debug" => parsed.release = false,
            "-n" | "--dry-run" | "--dryrun" | "--dry_run" => parsed.dry_run = true,
            "--check" | "--verify" => parsed.check = true,
            "--player-only" | "--player_only" | "--playeronly" => parsed.player_only = true,
            "--host-only" | "--host_only" | "--hostonly" => parsed.host_only = true,
            "--out" | "--output" | "--dir" | "--directory" | "--destination" | "--dest" => {
                i += 1;
                let v = args.get(i).ok_or_else(|| format!("{a} needs a value"))?;
                parsed.out = Some(PathBuf::from(v));
            }
            other
                if other.starts_with("--out=")
                    || other.starts_with("--output=")
                    || other.starts_with("--dir=") =>
            {
                let val = other.split_once('=').map(|x| x.1).unwrap_or("").to_string();
                parsed.out = Some(PathBuf::from(val));
            }
            "-h" | "--help" | "help" => parsed.help = true,
            _ => return Err(format!("unknown package option `{a}`")),
        }
        i += 1;
    }

    if parsed.player_only && parsed.host_only {
        return Err("--player-only and --host-only are mutually exclusive".to_string());
    }
    Ok(parsed)
}

fn print_help() {
    println!(
        "usage: cargo xtask package [options]\n\n\
         Build and package the GPL player and the permissive plugin host as\n\
         separate artefacts.\n\n\
         options:\n  \
         --release               release build (default)\n  \
         --debug                 debug build\n  \
         --dry-run               print the plan and run the GPL-free check, build nothing\n  \
         --check                 run the GPL-free check only\n  \
         --player-only           package only the GPL player\n  \
         --host-only             package only the permissive plugin host\n  \
         --out <dir>            output directory (default: target/package)\n  \
         -h, --help             show this help"
    );
}

fn binary_name(stem: &str) -> String {
    format!("{stem}.exe")
}

fn build_artifact(
    root: &Path,
    crate_name: &str,
    target: &str,
    bin: &str,
    release: bool,
) -> Result<PathBuf, String> {
    let mut cmd = cargo();
    cmd.current_dir(root);
    cmd.args(["build", "-p", crate_name, "--target", target]);
    if release {
        cmd.arg("--release");
    }
    run_status(&mut cmd)?;

    let profile = if release { "release" } else { "debug" };
    let src = root
        .join("target")
        .join(target)
        .join(profile)
        .join(binary_name(bin));
    if !src.is_file() {
        return Err(format!("expected binary not found at {}", src.display()));
    }
    Ok(src)
}

fn copy_file(src: &Path, dst: &Path) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    fs::copy(src, dst).map_err(|e| format!("copy {}: {e}", src.display()))?;
    Ok(())
}

/// Prove the plugin host's dependency closure is GPL-free (RFC 0004 acceptance 4).
fn check_host_closure(root: &Path) -> Result<(), String> {
    let mut cmd = cargo();
    cmd.current_dir(root);
    cmd.args([
        "tree",
        "-p",
        "rustamp-plughost",
        "--target",
        HOST_TARGET,
        "-e",
        "normal",
        "--format",
        "{p} || {l}",
    ]);
    let output = cmd.output().map_err(|e| format!("spawn cargo tree: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo tree failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for raw in stdout.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let Some((pkg_part, lic_part)) = line.split_once(" || ") else {
            // cargo prints tree glyphs; ignore non-data lines defensively.
            continue;
        };
        let package = strip_tree_prefix(pkg_part);
        let mut licence = lic_part.trim().to_string();
        if let Some(stripped) = licence.strip_suffix(" (*)") {
            licence = stripped.trim().to_string();
        }
        if licence.is_empty() {
            return Err(format!(
                "GPL-free check failed: `{package}` declares no license metadata"
            ));
        }
        if is_copyleft(&licence) {
            return Err(format!(
                "GPL-free check failed: `{package}` is licensed \"{licence}\""
            ));
        }
        // Touch the permissive crate list constant to keep it referenced;
        // the check above already covers the full host closure.
        let _ = HOST_PERMISSIVE_CRATES;
    }
    Ok(())
}

fn strip_tree_prefix(s: &str) -> String {
    // Cargo draws tree glyphs (spaces, `|`, `+`, `-`, backticks) before the
    // package name. Package names start with an alphanumeric or underscore.
    let trimmed = s.trim_start();
    let idx = trimmed
        .char_indices()
        .find(|(_, c)| c.is_alphanumeric() || *c == '_')
        .map(|(i, _)| i)
        .unwrap_or(0);
    trimmed[idx..].trim().to_string()
}

fn is_copyleft(lic: &str) -> bool {
    let l = lic.to_ascii_lowercase();
    l.contains("gpl") || l.contains("gfdl")
}

/// Ensure the packaged host directory carries no GPL binary.
fn audit_host_dir(dir: &Path) -> Result<(), String> {
    audit_host_dir_inner(dir)?;
    if !dir.join(binary_name(HOST_BIN)).is_file() {
        return Err(format!("host package is missing {}", binary_name(HOST_BIN)));
    }
    Ok(())
}

fn audit_host_dir_inner(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|e| format!("read host dir: {e}"))? {
        let entry = entry.map_err(|e| format!("read host dir: {e}"))?;
        let path = entry.path();
        let meta = entry
            .metadata()
            .map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.is_dir() {
            audit_host_dir_inner(&path)?;
        } else {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if name == binary_name(PLAYER_BIN).to_ascii_lowercase() {
                return Err(format!("GPL binary `{}` found in host package", name));
            }
            if name.ends_with(".exe") && name != binary_name(HOST_BIN).to_ascii_lowercase() {
                return Err(format!(
                    "non-host executable `{}` found in host package",
                    name
                ));
            }
        }
    }
    Ok(())
}

fn build_top_manifest(out_dir: &Path, profile: &str, player: bool, host: bool) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("{".to_string());
    lines.push("  \"schema_version\": 1,".to_string());
    lines.push("  \"generated_by\": \"cargo xtask package\",".to_string());
    lines.push(format!("  \"profile\": \"{profile}\","));
    if player {
        lines.push("  \"player\": {".to_string());
        lines.push("    \"package_dir\": \"player\",".to_string());
        lines.push(format!(
            "    \"binary\": \"player/{}\",",
            binary_name(PLAYER_BIN)
        ));
        lines.push(format!("    \"crate\": \"{PLAYER_CRATE}\","));
        lines.push(format!("    \"target\": \"{PLAYER_TARGET}\","));
        lines.push("    \"license\": \"GPL-3.0-or-later\", ".to_string());
        lines.push(
            "    \"license_files\": [\"player/LICENSE\", \"player/PLUGIN-EXCEPTION.md\"],"
                .to_string(),
        );
        lines.push("    \"contains_gpl_binary\": true".to_string());
        lines.push("  },".to_string());
    }
    if host {
        // trail comma handling: drop trailing comma from previous block if host last? Keep simple by conditional.
        lines.push("  \"host\": {".to_string());
        lines.push("    \"package_dir\": \"host\",".to_string());
        lines.push(format!(
            "    \"binary\": \"host/{}\",",
            binary_name(HOST_BIN)
        ));
        lines.push(format!("    \"crate\": \"{HOST_CRATE}\","));
        lines.push(format!("    \"target\": \"{HOST_TARGET}\","));
        lines.push("    \"license\": \"MIT OR Apache-2.0\", ".to_string());
        lines.push(
            "    \"license_files\": [\"host/LICENSE-MIT\", \"host/LICENSE-APACHE\"],".to_string(),
        );
        lines.push("    \"contains_gpl_binary\": false,".to_string());
        lines.push("    \"gpl_free_check\": \"cargo tree -p rustamp-plughost --target i686-pc-windows-msvc -e normal\"".to_string());
        lines.push("  }".to_string());
    }
    // Remove a trailing comma before closing brace is not handled above; fix by post-processing.
    lines.push("}".to_string());
    // Best-effort comma cleanup: if player block is followed by host, the "}," is fine; if player
    // is last and followed by "}", remove its trailing comma.
    let mut s = lines.join("\n");
    // Fix ",\n  \"host\"" absent case: trailing comma before closing brace.
    if !host {
        s = s.replace("  },\n}", "  }\n}");
    }
    // If neither included, avoid empty.
    let _ = out_dir;
    s
}

fn serde_manifest_player(profile: &str) -> String {
    format!(
        "{{\n  \"package\": \"rustamp-player\",\n  \"profile\": \"{profile}\",\n  \
         \"binary\": \"{bin}\",\n  \"crate\": \"{crate_}\",\n  \
         \"target\": \"{target}\",\n  \"license\": \"GPL-3.0-or-later\",\n  \
         \"license_files\": [\"LICENSE\", \"PLUGIN-EXCEPTION.md\"],\n  \
         \"gpl\": true\n}}\n",
        profile = profile,
        bin = binary_name(PLAYER_BIN),
        crate_ = PLAYER_CRATE,
        target = PLAYER_TARGET,
    )
}

fn serde_manifest_host(profile: &str) -> String {
    format!(
        "{{\n  \"package\": \"rustamp-plughost\",\n  \"profile\": \"{profile}\",\n  \
         \"binary\": \"{bin}\",\n  \"crate\": \"{crate_}\",\n  \
         \"target\": \"{target}\",\n  \"license\": \"MIT OR Apache-2.0\",\n  \
         \"license_files\": [\"LICENSE-MIT\", \"LICENSE-APACHE\"],\n  \
         \"contains_gpl_binary\": false,\n  \
         \"gpl_free_check\": \"cargo tree -p rustamp-plughost --target i686-pc-windows-msvc -e normal\"\n}}\n",
        profile = profile,
        bin = binary_name(HOST_BIN),
        crate_ = HOST_CRATE,
        target = HOST_TARGET,
    )
}
