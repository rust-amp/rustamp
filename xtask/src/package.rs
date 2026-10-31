//! `cargo xtask package` - ship the GPL player and the permissive plugin host
//! as separate portable artefacts, with the host also bundled beside the player,
//! complete license inventory, and portable release ZIP with SHA-256 checksums.
//!
//! The packaging logic lives here so that the licensing split from RFC 0004 is
//! reflected in the produced layout: `rustamp.exe` (x86_64, GPL-3.0-or-later
//! plus `PLUGIN-EXCEPTION.md`) and `rustamp-plughost32.exe` (i686, MIT OR
//! Apache-2.0) stay packageable independently, and the host-only artefact
//! carries no GPL binary. On top of that, the **player** artefact bundles a
//! copy of the host binary **beside `rustamp.exe`** — the exact packaged
//! layout RFC 0006 §8 requires and the first candidate `supervise.rs`
//! resolves — so the packaged product supervises without a heuristic search.
//! The bundled copy changes no licence file on either side: the player
//! directory keeps exactly the GPL set, the host directory exactly the
//! permissive set.
//!
//! Both packaged binaries use the static MSVC CRT; ordinary workspace builds
//! are unaffected. Windows system DLLs (and WOW64 for the host) are still needed.
//!
//! Release layout ships:
//! - Root: manifest.json (schema 3), QUICKSTART.md, README.md, THIRD_PARTY.md.
//! - Player: rustamp.exe, residents, bundled host, LICENSE, PLUGIN-EXCEPTION.md,
//!   THIRD_PARTY.md (with GPL source offer, static CRT terms, webamp MIT, symphonia MPL),
//!   manifest.json, QUICKSTART.md.
//! - Host: rustamp-plughost32.exe, LICENSE-MIT, LICENSE-APACHE, THIRD_PARTY.md
//!   (permissive closure inventory only, no GPL/copyleft), manifest.json.
//! - Portable ZIP archive beside the output directory with a .sha256 checksum file.
//! - With `--installer`: an NSIS installer (`xtask/installer/rustamp.nsi`) beside the ZIP,
//!   also with a .sha256 checksum file.
//!
//! With `SIGNTOOL_CERT_FILE` or `SIGNTOOL_CERT_THUMBPRINT` set, every shipped EXE and the
//! installer are signed and verified with signtool; otherwise the run says it is unsigned.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{cargo, no_window_command, pe_imports, run_status};

const PLAYER_CRATE: &str = "rustamp-app";
const PLAYER_BIN: &str = "rustamp";
const PLAYER_TARGET: &str = "x86_64-pc-windows-msvc";

const HOST_CRATE: &str = "rustamp-plughost";
const HOST_BIN: &str = "rustamp-plughost32";
const HOST_TARGET: &str = "i686-pc-windows-msvc";

const DECHOST_CRATE: &str = "rustamp-dechost";
const DECHOST_BIN: &str = "rustamp-dechost-symphonia";
const DECHOST_TARGET: &str = "x86_64-pc-windows-msvc";

const DSPHOST_CRATE: &str = "rustamp-dsphost";
const DSPHOST_BIN: &str = "rustamp-dsphost-stages";
const DSPHOST_TARGET: &str = "x86_64-pc-windows-msvc";

const NSIS_SCRIPT: &str = "xtask/installer/rustamp.nsi";

pub use rustamp_ipc::PROTOCOL_VERSION;
pub const CONFIG_VERSION: u32 = 1;
pub const MANIFEST_SCHEMA_VERSION: u32 = 3;

// Encoded flags take precedence over inherited RUSTFLAGS and Cargo target
// rustflags. Own the complete package-build flags rather than allowing a caller
// to silently negate the runtime policy. Explicit --target keeps these flags
// off host-side proc macros/build scripts.
const PACKAGE_RUSTFLAGS: &str = "-C\x1ftarget-feature=+crt-static";

/// The `runtime` object of every manifest. `artifact_imports_verified` is true
/// only when the PE import tables of every shipped executable were parsed and
/// named nothing but Windows system DLLs (see `pe_imports`).
fn runtime_manifest(meta: &ReleaseMetadata) -> String {
    format!(
        r#"{{"crt":"msvc","crt_linkage":"static","rustflags":["-C","target-feature=+crt-static"],"vc_redist_required":false,"bundled_runtime_dlls":[],"system_dlls_required":true,"artifact_imports_verified":{}}}"#,
        meta.imports_verified
    )
}

// GPL closure check for the host runs against this crate list. Kept in sync
// with `.github/scripts/check-permissive-licenses.sh`.
const HOST_PERMISSIVE_CRATES: &[&str] = &["rustamp-plughost"];

/// Stamped version and identity metadata for a packaged release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseMetadata {
    pub product_version: String,
    pub commit: String,
    pub commit_short: String,
    pub release_date: String,
    pub release_id: String,
    pub protocol_version: u32,
    pub config_version: u32,
    pub schema_version: u32,
    /// Set once the PE import inspection passed for every shipped executable.
    pub imports_verified: bool,
}

impl ReleaseMetadata {
    /// Discover version, commit, date, and release identity from tree and git.
    pub fn discover(root: &Path, player: bool, host: bool) -> Self {
        let product_version = get_product_version(root);
        let (commit, commit_short) = get_git_commit(root);
        let release_date = get_release_date(root);
        let prefix = if player && host {
            "rustamp"
        } else if player {
            "rustamp-player"
        } else {
            "rustamp-plughost"
        };
        let release_id = format!("{prefix}-{product_version}+{release_date}.{commit_short}");
        Self {
            product_version,
            commit,
            commit_short,
            release_date,
            release_id,
            protocol_version: PROTOCOL_VERSION,
            config_version: CONFIG_VERSION,
            schema_version: MANIFEST_SCHEMA_VERSION,
            imports_verified: false,
        }
    }

    /// Fixed metadata for unit testing and deterministic verification.
    pub fn for_testing() -> Self {
        Self {
            product_version: "0.0.1".to_string(),
            commit: "d2c8e5d4928f61d8198de427ef012ffbd11d5157".to_string(),
            commit_short: "d2c8e5d".to_string(),
            release_date: "2026-10-18".to_string(),
            release_id: "rustamp-0.0.1+2026-10-18.d2c8e5d".to_string(),
            protocol_version: PROTOCOL_VERSION,
            config_version: CONFIG_VERSION,
            schema_version: MANIFEST_SCHEMA_VERSION,
            imports_verified: false,
        }
    }
}

fn get_product_version(root: &Path) -> String {
    if let Ok(content) = fs::read_to_string(root.join("Cargo.toml")) {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(val) = trimmed.strip_prefix("version = ") {
                return val.trim().trim_matches('"').to_string();
            }
        }
    }
    env!("CARGO_PKG_VERSION").to_string()
}

fn get_git_commit(root: &Path) -> (String, String) {
    let mut cmd = no_window_command("git");
    cmd.current_dir(root).args(["rev-parse", "HEAD"]);
    if let Ok(output) = cmd.output() {
        if !output.status.success() {
            return ("unknown".to_string(), "unknown".to_string());
        }
        let full = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let short = if full.len() >= 7 {
            full[..7].to_string()
        } else {
            full.clone()
        };
        if !full.is_empty() {
            return (full, short);
        }
    }
    ("unknown".to_string(), "unknown".to_string())
}

fn get_release_date(root: &Path) -> String {
    let mut cmd = no_window_command("git");
    cmd.current_dir(root)
        .args(["log", "-1", "--format=%cd", "--date=format:%Y-%m-%d"]);
    if let Ok(output) = cmd.output() {
        if !output.status.success() {
            return current_date_utc();
        }
        let date = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if date.len() == 10 && date.chars().all(|c| c.is_ascii_digit() || c == '-') {
            return date;
        }
    }
    current_date_utc()
}

fn current_date_utc() -> String {
    let now = std::time::SystemTime::now();
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86400) as i64;
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1024 + doe / 1461 - doe / 142424) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// CLI arguments for `package`.
type PackageArgs = PackageArgsParsed;

pub fn run(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args)?;
    if parsed.help {
        print_help();
        return Ok(());
    }

    let signing = Signing::from_env(|name| std::env::var(name).ok())?;
    if let Some(file) = &parsed.sign_file {
        // The hook NSIS calls for the uninstaller it generates.
        let signing = signing.ok_or("--sign-file needs a configured certificate")?;
        return signing.sign_and_verify(file);
    }

    let root = rustamp_testkit::workspace_root();
    let build_player = !parsed.host_only;
    let build_host = !parsed.player_only;

    // Discover release metadata from the workspace root and git.
    let mut meta = ReleaseMetadata::discover(&root, build_player, build_host);

    let out_dir = parsed
        .out
        .clone()
        .unwrap_or_else(|| root.join("target").join("package").join(&meta.release_id));

    // The GPL-free check for the host side is the load-bearing acceptance test.
    // Run it on every invocation, including dry runs, so the proof is always
    // exercised without needing a release build.
    check_host_closure(&root)?;
    check_runtime_contract(&root)?;

    let archive_stem = out_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&meta.release_id);
    let zip_name = format!("{archive_stem}.zip");
    let zip_path = out_dir.parent().unwrap_or(&out_dir).join(&zip_name);
    let checksum_path = out_dir
        .parent()
        .unwrap_or(&out_dir)
        .join(format!("{zip_name}.sha256"));
    let installer_path = out_dir
        .parent()
        .unwrap_or(&out_dir)
        .join(format!("{archive_stem}-setup.exe"));

    if parsed.dry_run || parsed.check {
        println!("xtask package: dry run (no files written)");
        println!("  workspace root: {}", root.display());
        println!("  release ID:     {}", meta.release_id);
        println!("  product version:{}", meta.product_version);
        println!("  commit:         {} ({})", meta.commit, meta.commit_short);
        println!("  output dir:     {}", out_dir.display());
        println!("  portable zip:   {}", zip_path.display());
        println!("  checksum file:  {}", checksum_path.display());
        if parsed.installer {
            println!("  installer:      {}", installer_path.display());
        }
        match &signing {
            Some(_) => println!("  signing:        signtool (certificate configured)"),
            None => println!("  signing:        none (no certificate configured)"),
        }
        if parsed.player_only {
            println!("  scope: player only");
        } else if parsed.host_only {
            println!("  scope: host only");
        } else {
            println!("  scope: player + host");
        }
        println!("  player target:  {} ({})", PLAYER_TARGET, PLAYER_CRATE);
        println!("  decoder host:   {} ({})", DECHOST_TARGET, DECHOST_CRATE);
        println!("  DSP host:       {} ({})", DSPHOST_TARGET, DSPHOST_CRATE);
        println!("  host target:    {} ({})", HOST_TARGET, HOST_CRATE);
        println!("  GPL-free check: OK (host closure contains no copyleft crate)");
        println!("  runtime contract: OK (all targets: static MSVC CRT, no VC redist/DLL payload)");
        println!(
            "  prerequisites: Windows x64; WOW64 for the i686 host; system DLLs remain required"
        );
        println!(
            "  output: generated README.md, QUICKSTART.md, THIRD_PARTY.md, and manifests (schema 3)"
        );
        println!(
            "  imports: every shipped EXE is parsed offline and must import only Windows system \
             DLLs (a real packaging run, or --check on an existing layout); a dry run inspects nothing"
        );
        println!("  not proven: first launch on a clean OS");
        if !parsed.host_only {
            println!(
                "  player ships residents: {} and {} beside {}",
                binary_name(DECHOST_BIN),
                binary_name(DSPHOST_BIN),
                binary_name(PLAYER_BIN)
            );
        }
        if !parsed.player_only && !parsed.host_only {
            println!(
                "  player bundles host: yes ({} beside {}: RFC 0006 §8, \
                 the first `supervise.rs` candidate)",
                binary_name(HOST_BIN),
                binary_name(PLAYER_BIN)
            );
            println!(
                "  host-only artefact: unchanged ({} + LICENSE-MIT/APACHE/THIRD_PARTY.md, \
                 GPL-free proof as before)",
                binary_name(HOST_BIN)
            );
        } else if parsed.player_only {
            println!(
                "  player bundles host: no (--player-only builds no host; the \
                 supervisor falls back to the dev-tree heuristic and stays \
                 detached when it cannot resolve)"
            );
        } else {
            println!(
                "  host-only artefact: unchanged ({} + LICENSE-MIT/APACHE/THIRD_PARTY.md, \
                 GPL-free proof as before; no GPL side included)",
                binary_name(HOST_BIN)
            );
        }
        if parsed.check && out_dir.exists() {
            // Inspect first: a layout whose binaries fail the import check must
            // not be audited as if it were verified.
            inspect_imports(&staged_binaries(
                &out_dir,
                !parsed.host_only,
                !parsed.player_only,
            ))?;
            let verified = ReleaseMetadata {
                imports_verified: true,
                ..meta.clone()
            };
            audit_version_consistency(&out_dir, &verified)?;
            audit_package_layout(
                &out_dir,
                if parsed.release { "release" } else { "debug" },
                !parsed.host_only,
                !parsed.player_only,
                &verified,
            )?;
            println!(
                "  existing layout: OK (files/manifests/notices and PE import tables; no binary executed)"
            );
            if zip_path.is_file() {
                audit_zip_archive(&zip_path, &meta)?;
                println!("  existing zip archive: OK (format and checksum match)");
            }
            if parsed.installer {
                audit_installer(&installer_path)?;
                println!("  existing installer: OK (PE file and checksum match)");
            }
        }
        return Ok(());
    }

    let profile = if parsed.release { "release" } else { "debug" };

    // Before the long builds: a missing tool should not cost a release build.
    let makensis = if parsed.installer {
        Some(find_makensis()?)
    } else {
        None
    };
    if signing.is_none() {
        println!(
            "xtask package: UNSIGNED build: no SIGNTOOL_CERT_FILE or SIGNTOOL_CERT_THUMBPRINT set"
        );
    }

    // Never silently retain old CRT DLLs, wrong-side licences or a stale host.
    // Refuse rather than delete somebody else's output.
    require_empty_output(&out_dir)?;
    fs::create_dir_all(&out_dir).map_err(|e| format!("create output dir: {e}"))?;

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
    let dechost_src = if build_player {
        Some(build_artifact(
            &root,
            DECHOST_CRATE,
            DECHOST_TARGET,
            DECHOST_BIN,
            parsed.release,
        )?)
    } else {
        None
    };
    let dsphost_src = if build_player {
        Some(build_artifact(
            &root,
            DSPHOST_CRATE,
            DSPHOST_TARGET,
            DSPHOST_BIN,
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

    // Offline PE import inspection of the built binaries, before anything is
    // staged: a non-system import (MSVCP140.dll, VCRUNTIME140.dll, ...) or an
    // unreadable binary stops the run. Only a pass lets the manifests say
    // `artifact_imports_verified: true`.
    let mut built: Vec<(PathBuf, u16)> = Vec::new();
    for (src, machine) in [
        (&player_src, pe_imports::MACHINE_X64),
        (&dechost_src, pe_imports::MACHINE_X64),
        (&dsphost_src, pe_imports::MACHINE_X64),
        (&host_src, pe_imports::MACHINE_X86),
    ] {
        if let Some(path) = src {
            built.push((path.clone(), machine));
        }
    }
    inspect_imports(&built)?;
    meta.imports_verified = true;

    let mut manifest_items: Vec<String> = Vec::new();

    if let Some(src) = &player_src {
        let dir = out_dir.join("player");
        fs::create_dir_all(&dir).map_err(|e| format!("create player dir: {e}"))?;
        let dest_bin = dir.join(binary_name(PLAYER_BIN));
        fs::copy(src, &dest_bin).map_err(|e| format!("copy player binary: {e}"))?;
        if let Some(dechost) = &dechost_src {
            let dest_dechost = dir.join(binary_name(DECHOST_BIN));
            fs::copy(dechost, &dest_dechost)
                .map_err(|e| format!("copy decoder resident binary: {e}"))?;
        }
        if let Some(dsphost) = &dsphost_src {
            let dest_dsphost = dir.join(binary_name(DSPHOST_BIN));
            fs::copy(dsphost, &dest_dsphost)
                .map_err(|e| format!("copy DSP resident binary: {e}"))?;
        }
        copy_file(&root.join("LICENSE"), &dir.join("LICENSE"))?;
        copy_file(
            &root.join("PLUGIN-EXCEPTION.md"),
            &dir.join("PLUGIN-EXCEPTION.md"),
        )?;

        // Ship player-side third-party notices (GPL source offer, static CRT terms,
        // webamp MIT, Symphonia MPL-2.0, permissive licenses, dependency inventory).
        let player_notices = player_third_party_notices(&root, &meta)?;
        fs::write(dir.join("THIRD_PARTY.md"), player_notices)
            .map_err(|e| format!("write player THIRD_PARTY.md: {e}"))?;

        // RFC 0006 §8 packaged layout: the helper ships next to `rustamp.exe`,
        // which is exactly the first candidate `supervise.rs` resolves. The
        // copy changes no licence file on either side: this directory keeps
        // exactly the GPL set (LICENSE + PLUGIN-EXCEPTION.md + THIRD_PARTY.md),
        // and the host-only artefact below is untouched.
        let mut bundled_host = false;
        if let Some(host) = &host_src {
            let dest_host = dir.join(binary_name(HOST_BIN));
            fs::copy(host, &dest_host).map_err(|e| format!("copy bundled host binary: {e}"))?;
            bundled_host = true;
        }

        let per_package = serde_manifest_player(profile, bundled_host, &meta);
        fs::write(dir.join("manifest.json"), per_package)
            .map_err(|e| format!("write player manifest: {e}"))?;
        fs::write(
            dir.join("QUICKSTART.md"),
            quickstart(profile, true, bundled_host, &meta),
        )
        .map_err(|e| format!("write player quickstart: {e}"))?;

        manifest_items.push("player".to_string());
        println!("xtask package: player -> {}", dir.display());
        println!(
            "xtask package: player includes residents: {} and {} beside {}",
            binary_name(DECHOST_BIN),
            binary_name(DSPHOST_BIN),
            binary_name(PLAYER_BIN)
        );
        if bundled_host {
            println!(
                "xtask package: player bundles {} beside {} \
                 (RFC 0006 §8; the first `supervise.rs` candidate, \
                 licence files unchanged)",
                binary_name(HOST_BIN),
                binary_name(PLAYER_BIN)
            );
        } else {
            println!(
                "xtask package: note: player has no bundled host \
                 (--player-only); the supervisor falls back to the dev-tree \
                 heuristic and stays detached when it cannot resolve"
            );
        }
    }

    if let Some(src) = &host_src {
        let dir = out_dir.join("host");
        fs::create_dir_all(&dir).map_err(|e| format!("create host dir: {e}"))?;
        let dest_bin = dir.join(binary_name(HOST_BIN));
        fs::copy(src, &dest_bin).map_err(|e| format!("copy host binary: {e}"))?;
        copy_file(&root.join("LICENSE-MIT"), &dir.join("LICENSE-MIT"))?;
        copy_file(&root.join("LICENSE-APACHE"), &dir.join("LICENSE-APACHE"))?;

        // Ship permissive host third-party notices (permissive closure inventory only,
        // static CRT terms, permissive licenses, strictly no GPL/copyleft).
        let host_notices = host_third_party_notices(&root, &meta)?;
        fs::write(dir.join("THIRD_PARTY.md"), &host_notices)
            .map_err(|e| format!("write host THIRD_PARTY.md: {e}"))?;

        // Prove the host package is GPL-free: the closure check already ran,
        // and the directory must contain no GPL binary or copyleft notice.
        audit_host_dir(&dir)?;

        let per_package = serde_manifest_host(profile, &meta);
        fs::write(dir.join("manifest.json"), per_package)
            .map_err(|e| format!("write host manifest: {e}"))?;

        manifest_items.push("host".to_string());
        println!("xtask package: host   -> {}", dir.display());
    }

    // Top-level files naming what went where and explaining the portable release.
    let bundled_host = player_src.is_some() && host_src.is_some();
    let top = build_top_manifest(profile, build_player, build_host, bundled_host, &meta);
    fs::write(out_dir.join("manifest.json"), top).map_err(|e| format!("write manifest: {e}"))?;
    fs::write(
        out_dir.join("QUICKSTART.md"),
        quickstart(profile, build_player, bundled_host, &meta),
    )
    .map_err(|e| format!("write quickstart: {e}"))?;
    fs::write(
        out_dir.join("README.md"),
        readme(profile, build_player, build_host, &meta),
    )
    .map_err(|e| format!("write readme: {e}"))?;

    // Copy repository-wide THIRD_PARTY.md to root of package.
    copy_file(
        &root.join("THIRD_PARTY.md"),
        &out_dir.join("THIRD_PARTY.md"),
    )?;

    // The staged copies are what ships: inspect them too, not only the build outputs.
    inspect_imports(&staged_binaries(&out_dir, build_player, build_host))?;
    audit_package_layout(&out_dir, profile, build_player, build_host, &meta)?;
    if let Some(signing) = &signing {
        for (file, _) in staged_binaries(&out_dir, build_player, build_host) {
            signing.sign_and_verify(&file)?;
        }
    }

    println!(
        "xtask package: manifest -> {}",
        out_dir.join("manifest.json").display()
    );

    // G4: Produce portable ZIP archive and SHA-256 checksum file beside out_dir.
    if parsed.zip {
        println!("xtask package: archiving -> {}", zip_path.display());
        create_zip_archive(&zip_path, &out_dir, archive_stem)?;
        write_checksum_file(&zip_path, &checksum_path)?;
        audit_zip_archive(&zip_path, &meta)?;
        println!("xtask package: archive  -> {}", zip_path.display());
        println!("xtask package: checksum -> {}", checksum_path.display());
    }

    if let Some(makensis) = &makensis {
        println!("xtask package: installer -> {}", installer_path.display());
        run_status(&mut makensis_command(
            makensis,
            &root,
            &out_dir,
            &installer_path,
            &meta,
            signing.is_some(),
        )?)?;
        if let Some(signing) = &signing {
            signing.sign_and_verify(&installer_path)?;
        }
        write_checksum_file(&installer_path, &sha256_path(&installer_path))?;
        audit_installer(&installer_path)?;
        println!(
            "xtask package: checksum -> {}",
            sha256_path(&installer_path).display()
        );
    }

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
    zip: bool,
    installer: bool,
    sign_file: Option<PathBuf>,
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
        zip: true,
        installer: false,
        sign_file: None,
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
            "--zip" => parsed.zip = true,
            "--no-zip" | "--nozip" => parsed.zip = false,
            "--installer" => parsed.installer = true,
            "--sign-file" => {
                i += 1;
                let v = args.get(i).ok_or_else(|| format!("{a} needs a value"))?;
                parsed.sign_file = Some(PathBuf::from(v));
            }
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
    if parsed.installer && (parsed.player_only || parsed.host_only || !parsed.release) {
        return Err("--installer needs the full release package".to_string());
    }
    Ok(parsed)
}

fn print_help() {
    println!(
        "usage: cargo xtask package [options]\n\n\
         Build and package the GPL player and the permissive plugin host as\n\
         separate portable artefacts, producing a release folder, portable ZIP,\n\
         and SHA-256 checksum file.\n\n\
         The player artefact carries a copy of the plugin host binary next\n\
         to `rustamp.exe` (RFC 0006 §8: the first `supervise.rs` candidate),\n\
         with the licence files of each side unchanged; the host-only\n\
         artefact stays GPL-free and shippable alone.\n\n\
         options:\n  \
         --release               release build (default)\n  \
         --debug                 debug build\n  \
         --dry-run               print the plan and run the GPL-free check, build nothing\n  \
         --check                 check closure/runtime contract; audit output if present; build nothing\n  \
         --player-only           package only the GPL player\n  \
         --host-only             package only the permissive plugin host\n  \
         --zip                   produce portable ZIP and checksum (default: true)\n  \
         --no-zip                skip producing portable ZIP archive\n  \
         --installer             also build the NSIS installer and its checksum\n                          \
         (needs makensis on PATH or in MAKENSIS)\n  \
         --out <dir>             empty output directory (default: target/package/<release-id>)\n  \
         -h, --help              show this help\n\n\
         signing: set SIGNTOOL_CERT_FILE (with SIGNTOOL_CERT_PASSWORD) or\n\
         SIGNTOOL_CERT_THUMBPRINT; optional SIGNTOOL_TIMESTAMP_URL and SIGNTOOL_PATH.\n\
         Without a certificate the build says it is unsigned and goes on."
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
    let mut cmd = artifact_command(root, crate_name, target, bin, release);
    run_status(&mut cmd)?;

    let profile = if release { "release" } else { "debug" };
    let src = package_build_dir(root)
        .join(target)
        .join(profile)
        .join(binary_name(bin));
    if !src.is_file() {
        return Err(format!("expected binary not found at {}", src.display()));
    }
    Ok(src)
}

fn package_build_dir(root: &Path) -> PathBuf {
    // A separate cache prevents accidental staging of ordinary dynamic-CRT
    // workspace outputs and ignores a caller's external CARGO_TARGET_DIR.
    root.join("target").join("package-build")
}

fn artifact_command(
    root: &Path,
    crate_name: &str,
    target: &str,
    bin: &str,
    release: bool,
) -> Command {
    let mut cmd = cargo();
    cmd.current_dir(root);
    cmd.args([
        "build",
        "--offline",
        "--locked",
        "-p",
        crate_name,
        "--bin",
        bin,
        "--target",
        target,
    ]);
    cmd.env_remove("RUSTFLAGS");
    cmd.env("CARGO_ENCODED_RUSTFLAGS", PACKAGE_RUSTFLAGS);
    cmd.env("CARGO_TARGET_DIR", package_build_dir(root));
    if release {
        cmd.arg("--release");
    }
    cmd
}

/// The certificate settings for signtool, read from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Signing {
    signtool: String,
    /// The arguments that name the certificate.
    certificate: Vec<String>,
    timestamp_url: Option<String>,
}

impl Signing {
    /// `None` means no certificate is configured. A password without a
    /// certificate file is a mistake, not an unsigned build.
    fn from_env(get: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let password = get("SIGNTOOL_CERT_PASSWORD");
        let certificate = match (get("SIGNTOOL_CERT_FILE"), get("SIGNTOOL_CERT_THUMBPRINT")) {
            (Some(_), Some(_)) => {
                return Err(
                    "set SIGNTOOL_CERT_FILE or SIGNTOOL_CERT_THUMBPRINT, not both".to_string(),
                );
            }
            (Some(file), None) => {
                let mut args = vec!["/f".to_string(), file];
                if let Some(password) = password {
                    args.extend(["/p".to_string(), password]);
                }
                args
            }
            (None, Some(thumbprint)) if password.is_none() => {
                vec!["/sha1".to_string(), thumbprint]
            }
            (None, None) if password.is_none() => return Ok(None),
            (None, _) => {
                return Err("SIGNTOOL_CERT_PASSWORD needs SIGNTOOL_CERT_FILE".to_string());
            }
        };
        Ok(Some(Self {
            signtool: get("SIGNTOOL_PATH").unwrap_or_else(|| "signtool".to_string()),
            certificate,
            timestamp_url: get("SIGNTOOL_TIMESTAMP_URL"),
        }))
    }

    fn sign_args(&self, file: &Path) -> Vec<String> {
        let mut args = vec!["sign".to_string(), "/fd".to_string(), "SHA256".to_string()];
        args.extend(self.certificate.iter().cloned());
        if let Some(url) = &self.timestamp_url {
            args.extend([
                "/tr".to_string(),
                url.clone(),
                "/td".to_string(),
                "SHA256".to_string(),
            ]);
        }
        args.push(file.display().to_string());
        args
    }

    fn sign_and_verify(&self, file: &Path) -> Result<(), String> {
        // Not `run_status`: its error text would print a password argument.
        let sign = no_window_command(&self.signtool)
            .args(self.sign_args(file))
            .status()
            .map_err(|e| format!("could not start {}: {e}", self.signtool))?;
        if !sign.success() {
            return Err(format!("signtool could not sign {}", file.display()));
        }
        let verify = no_window_command(&self.signtool)
            .args(["verify", "/pa"])
            .arg(file)
            .status()
            .map_err(|e| format!("could not start {}: {e}", self.signtool))?;
        if !verify.success() {
            return Err(format!("signtool could not verify {}", file.display()));
        }
        println!("xtask package: signed and verified {}", file.display());
        Ok(())
    }
}

/// `MAKENSIS`, else `makensis` on PATH; proved by asking it for its version.
fn find_makensis() -> Result<PathBuf, String> {
    let tool = std::env::var_os("MAKENSIS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("makensis"));
    let found = no_window_command(&tool)
        .arg("/VERSION")
        .output()
        .is_ok_and(|output| output.status.success());
    if found {
        Ok(tool)
    } else {
        Err(format!(
            "--installer needs NSIS: {} did not run. Put makensis on PATH or set MAKENSIS \
             (portable zip: https://sourceforge.net/projects/nsis/files/NSIS%203/)",
            tool.display()
        ))
    }
}

fn makensis_command(
    makensis: &Path,
    root: &Path,
    out_dir: &Path,
    installer: &Path,
    meta: &ReleaseMetadata,
    signed: bool,
) -> Result<Command, String> {
    let mut cmd = no_window_command(makensis);
    cmd.args(["/V2", "/WX"]);
    cmd.arg(format!("/DVERSION={}", meta.product_version));
    cmd.arg(format!("/DPLAYER_DIR={}", out_dir.join("player").display()));
    cmd.arg(format!("/DHOST_DIR={}", out_dir.join("host").display()));
    cmd.arg(format!("/DOUTFILE={}", installer.display()));
    if signed {
        let xtask = std::env::current_exe().map_err(|e| format!("own path: {e}"))?;
        cmd.arg(format!("/DSIGN_UNINSTALLER={}", xtask.display()));
    }
    cmd.arg(root.join(NSIS_SCRIPT));
    Ok(cmd)
}

/// Offline: the file is a PE image and its checksum file matches. Nothing is run.
fn audit_installer(installer: &Path) -> Result<(), String> {
    let bytes =
        fs::read(installer).map_err(|e| format!("read installer {}: {e}", installer.display()))?;
    if !bytes.starts_with(b"MZ") {
        return Err(format!("{} is not a PE file", installer.display()));
    }
    let checksum_path = sha256_path(installer);
    let content = fs::read_to_string(&checksum_path)
        .map_err(|e| format!("read checksum {}: {e}", checksum_path.display()))?;
    if content.split_whitespace().next() != Some(Sha256::digest_hex(&bytes).as_str()) {
        return Err(format!("checksum mismatch for {}", installer.display()));
    }
    Ok(())
}

fn sha256_path(file: &Path) -> PathBuf {
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
    file.with_file_name(format!("{name}.sha256"))
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
        "--offline",
        "--locked",
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
    l.contains("gpl") || l.contains("gfdl") || l.contains("mpl")
}

fn contains_copyleft_license(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    for token in [
        "gpl-",
        "gplv",
        "gpl 1",
        "gpl 2",
        "gpl 3",
        "gpl_v",
        "general public license",
        "mpl-",
        "mpl 2",
        "mpl v",
        "mozilla public license",
        "lgpl",
        "agpl",
        "gfdl",
        "copyleft license",
    ] {
        if lower.contains(token) {
            return true;
        }
    }
    false
}

/// Ensure the packaged host directory carries no GPL binary and no copyleft notices.
fn audit_host_dir(dir: &Path) -> Result<(), String> {
    audit_host_dir_inner(dir)?;
    if !dir.join(binary_name(HOST_BIN)).is_file() {
        return Err(format!("host package is missing {}", binary_name(HOST_BIN)));
    }
    let third_party = dir.join("THIRD_PARTY.md");
    if third_party.is_file() {
        let text = fs::read_to_string(&third_party)
            .map_err(|e| format!("read {}: {e}", third_party.display()))?;
        if contains_copyleft_license(&text) {
            return Err("copyleft/MPL license terms found in host THIRD_PARTY.md".to_string());
        }
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

fn build_top_manifest(
    profile: &str,
    player: bool,
    host: bool,
    bundled_host: bool,
    meta: &ReleaseMetadata,
) -> String {
    let runtime = runtime_manifest(meta);
    let mut lines: Vec<String> = Vec::new();
    lines.push("{".to_string());
    lines.push(format!("  \"schema_version\": {},", meta.schema_version));
    lines.push("  \"generated_by\": \"cargo xtask package\",".to_string());
    lines.push(format!(
        "  \"product_version\": \"{}\",",
        meta.product_version
    ));
    lines.push(format!("  \"commit\": \"{}\",", meta.commit));
    lines.push(format!("  \"commit_short\": \"{}\",", meta.commit_short));
    lines.push(format!("  \"release_id\": \"{}\",", meta.release_id));
    lines.push(format!(
        "  \"protocol_version\": {},",
        meta.protocol_version
    ));
    lines.push(format!("  \"config_version\": {},", meta.config_version));
    lines.push(format!("  \"profile\": \"{profile}\","));
    lines.push("  \"readme\": \"README.md\",".to_string());
    lines.push("  \"quickstart\": \"QUICKSTART.md\",".to_string());
    lines.push("  \"third_party_notices\": \"THIRD_PARTY.md\",".to_string());
    lines.push(format!("  \"runtime\": {runtime},"));
    if player {
        lines.push("  \"player\": {".to_string());
        lines.push("    \"package_dir\": \"player\",".to_string());
        lines.push(format!(
            "    \"binary\": \"player/{}\",",
            binary_name(PLAYER_BIN)
        ));
        if bundled_host {
            lines.push(format!(
                "    \"bundled_host\": \"player/{}\",",
                binary_name(HOST_BIN)
            ));
            lines.push(
                "    \"host_resolution\": \"RFC 0006 §8: beside rustamp.exe \
                 (the first supervise.rs candidate)\","
                    .to_string(),
            );
        } else {
            lines.push("    \"bundled_host\": null,".to_string());
        }
        lines.push(format!("    \"crate\": \"{PLAYER_CRATE}\","));
        lines.push(format!("    \"target\": \"{PLAYER_TARGET}\","));
        lines.push(format!("    \"runtime\": {runtime},"));
        lines.push(format!(
            "    \"decoder_host\": \"player/{}\",",
            binary_name(DECHOST_BIN)
        ));
        lines.push(format!("    \"decoder_host_crate\": \"{DECHOST_CRATE}\","));
        lines.push(format!(
            "    \"decoder_host_target\": \"{DECHOST_TARGET}\","
        ));
        lines.push(format!("    \"decoder_host_runtime\": {runtime},"));
        lines.push(format!(
            "    \"dsp_host\": \"player/{}\",",
            binary_name(DSPHOST_BIN)
        ));
        lines.push(format!("    \"dsp_host_crate\": \"{DSPHOST_CRATE}\","));
        lines.push(format!("    \"dsp_host_target\": \"{DSPHOST_TARGET}\","));
        lines.push(format!("    \"dsp_host_runtime\": {runtime},"));
        if bundled_host {
            lines.push(format!("    \"bundled_host_runtime\": {runtime},"));
            lines.push("    \"bundled_host_requires_wow64_on_x64\": true,".to_string());
        }
        lines.push("    \"license\": \"GPL-3.0-or-later\", ".to_string());
        lines.push(
            "    \"license_files\": [\"player/LICENSE\", \"player/PLUGIN-EXCEPTION.md\", \"player/THIRD_PARTY.md\"],"
                .to_string(),
        );
        lines.push("    \"contains_gpl_binary\": true".to_string());
        lines.push("  },".to_string());
    }
    if host {
        lines.push("  \"host\": {".to_string());
        lines.push("    \"package_dir\": \"host\",".to_string());
        lines.push(format!(
            "    \"binary\": \"host/{}\",",
            binary_name(HOST_BIN)
        ));
        lines.push(format!("    \"crate\": \"{HOST_CRATE}\","));
        lines.push(format!("    \"target\": \"{HOST_TARGET}\","));
        lines.push(format!("    \"runtime\": {runtime},"));
        lines.push("    \"requires_wow64_on_x64\": true,".to_string());
        lines.push("    \"license\": \"MIT OR Apache-2.0\", ".to_string());
        lines.push(
            "    \"license_files\": [\"host/LICENSE-MIT\", \"host/LICENSE-APACHE\", \"host/THIRD_PARTY.md\"],".to_string(),
        );
        lines.push("    \"contains_gpl_binary\": false,".to_string());
        lines.push("    \"gpl_free_check\": \"cargo tree -p rustamp-plughost --target i686-pc-windows-msvc -e normal\"".to_string());
        lines.push("  }".to_string());
    }
    lines.push("}".to_string());
    let mut s = lines.join("\n");
    if !host {
        s = s.replace("  },\n}", "  }\n}");
    }
    s
}

fn serde_manifest_player(profile: &str, bundled_host: bool, meta: &ReleaseMetadata) -> String {
    let runtime = runtime_manifest(meta);
    let bundled_line = if bundled_host {
        format!(
            "\n  \"bundled_host\": \"{bin}\",\n  \
             \"host_resolution\": \"RFC 0006 §8: beside rustamp.exe \
             (the first supervise.rs candidate)\",\n  \
             \"bundled_host_runtime\": {runtime},\n  \
             \"bundled_host_requires_wow64_on_x64\": true,",
            bin = binary_name(HOST_BIN),
        )
    } else {
        "\n  \"bundled_host\": null,".to_string()
    };
    format!(
        "{{\n  \"package\": \"rustamp-player\",\n  \
         \"schema_version\": {schema_version},\n  \
         \"product_version\": \"{product_version}\",\n  \
         \"commit\": \"{commit}\",\n  \
         \"commit_short\": \"{commit_short}\",\n  \
         \"release_id\": \"{release_id}\",\n  \
         \"protocol_version\": {protocol_version},\n  \
         \"config_version\": {config_version},\n  \
         \"profile\": \"{profile}\",\n  \
         \"binary\": \"{bin}\",{bundled}\n  \
         \"decoder_host\": \"{dechost_bin}\",\n  \
         \"decoder_host_crate\": \"{dechost_crate}\",\n  \
         \"decoder_host_target\": \"{dechost_target}\",\n  \
         \"decoder_host_runtime\": {runtime},\n  \
         \"dsp_host\": \"{dsphost_bin}\",\n  \
         \"dsp_host_crate\": \"{dsphost_crate}\",\n  \
         \"dsp_host_target\": \"{dsphost_target}\",\n  \
         \"dsp_host_runtime\": {runtime},\n  \
         \"crate\": \"{crate_}\",\n  \
         \"target\": \"{target}\",\n  \"license\": \"GPL-3.0-or-later\",\n  \
         \"runtime\": {runtime},\n  \"quickstart\": \"QUICKSTART.md\",\n  \
         \"third_party_notices\": \"THIRD_PARTY.md\",\n  \
         \"license_files\": [\"LICENSE\", \"PLUGIN-EXCEPTION.md\", \"THIRD_PARTY.md\"],\n  \
         \"gpl\": true\n}}\n",
        schema_version = meta.schema_version,
        product_version = meta.product_version,
        commit = meta.commit,
        commit_short = meta.commit_short,
        release_id = meta.release_id,
        protocol_version = meta.protocol_version,
        config_version = meta.config_version,
        profile = profile,
        bin = binary_name(PLAYER_BIN),
        bundled = bundled_line,
        dechost_bin = binary_name(DECHOST_BIN),
        dechost_crate = DECHOST_CRATE,
        dechost_target = DECHOST_TARGET,
        dsphost_bin = binary_name(DSPHOST_BIN),
        dsphost_crate = DSPHOST_CRATE,
        dsphost_target = DSPHOST_TARGET,
        crate_ = PLAYER_CRATE,
        target = PLAYER_TARGET,
    )
}

fn serde_manifest_host(profile: &str, meta: &ReleaseMetadata) -> String {
    let runtime = runtime_manifest(meta);
    format!(
        "{{\n  \"package\": \"rustamp-plughost\",\n  \
         \"schema_version\": {schema_version},\n  \
         \"product_version\": \"{product_version}\",\n  \
         \"commit\": \"{commit}\",\n  \
         \"commit_short\": \"{commit_short}\",\n  \
         \"release_id\": \"{release_id}\",\n  \
         \"protocol_version\": {protocol_version},\n  \
         \"profile\": \"{profile}\",\n  \
         \"binary\": \"{bin}\",\n  \"crate\": \"{crate_}\",\n  \
         \"target\": \"{target}\",\n  \"license\": \"MIT OR Apache-2.0\",\n  \
         \"runtime\": {runtime},\n  \"requires_wow64_on_x64\": true,\n  \
         \"third_party_notices\": \"THIRD_PARTY.md\",\n  \
         \"license_files\": [\"LICENSE-MIT\", \"LICENSE-APACHE\", \"THIRD_PARTY.md\"],\n  \
         \"contains_gpl_binary\": false,\n  \
         \"gpl_free_check\": \"cargo tree -p rustamp-plughost --target i686-pc-windows-msvc -e normal\"\n}}\n",
        schema_version = meta.schema_version,
        product_version = meta.product_version,
        commit = meta.commit,
        commit_short = meta.commit_short,
        release_id = meta.release_id,
        protocol_version = meta.protocol_version,
        profile = profile,
        bin = binary_name(HOST_BIN),
        crate_ = HOST_CRATE,
        target = HOST_TARGET,
    )
}

fn quickstart(profile: &str, player: bool, bundled_host: bool, meta: &ReleaseMetadata) -> String {
    if !player {
        return format!(
            "# Rustamp plugin host ({})\n\n\
             This is the standalone i686 helper, not the player. Do not launch it directly.\n\
             Use the full player package for listening to music.\n\n\
             Runtime policy: static MSVC CRT; no VC++ redistributable or runtime DLLs\n\
             are shipped or required by this policy. Windows system DLLs and WOW64\n\
             on x64 Windows are still required. {} See host/manifest.json.\n",
            meta.release_id,
            if meta.imports_verified {
                "The import table was inspected at packaging time (system DLLs only);\n\
                 launch on a clean OS has not been certified."
            } else {
                "DLL imports and clean-OS launch must be verified by the release process."
            }
        );
    }
    let mut text = format!(
        r#"# Rustamp quick start ({release_id})

Use Windows 10/11 x64 (WOW64 is needed for the 32-bit helper). Package policy:
packaged EXEs use the static MSVC CRT, so no VC++ redistributable installation or
app-local CRT DLLs are needed. Windows system DLLs are still required. {imports}

Zip: unzip the archive to any directory; no registry keys or services are used.
To remove it, delete the folder.
Installer (`*-setup.exe`): installs for the current user, without administrator
rights, into %LOCALAPPDATA%\Programs\Rustamp with a Start menu shortcut. Close
Rustamp before upgrading. Remove it through Windows' installed apps. Both keep
your settings in %APPDATA%\Rustamp, and files you added to the folder stay.

Open PowerShell in the folder containing rustamp.exe, then run:

```powershell
.\rustamp.exe "C:\Music\song.mp3"
```

One local media file is supported per launch; it is queued and played. With no
file, the player starts empty. Use a known-good MP3, FLAC or WAV first.

Useful first-run switches:
- `--skin "C:\Skins\example.wsz"`: classic player artwork/layout (.wsz/.zip/folder).
  Optional; built-in default is the drawn Rust theme (`--skin lookalike` selectable).
- `--no-audio`: open the window without an audio device, intentionally silent.
- `--volume 200`: set volume (0..=200, 100 = unity).

Example: `.\rustamp.exe "C:\Music\song.mp3" --skin "C:\Skins\example.wsz"`

If nothing plays: check the file exists, Windows default playback device and
mixer volume; omit --no-audio/--null, and try `--volume 200`. Restart after a
device change. Startup may fail before any window if no audio device is usable.
Run from PowerShell with `2> rustamp-startup.log` appended and keep that log.
For a no-window/no-sound decoder diagnostic, use:
`.\rustamp.exe "C:\Music\song.mp3" --null --seconds 5`.
GUI errors are not all visible yet; apparent playback is not proof of sound.

No configuration file is needed initially. It reads %APPDATA%\Rustamp\rustamp.toml
(or the RUSTAMP_CONFIG override); settings and volume are saved on clean exit.
The helper binaries (rustamp-plughost32.exe, rustamp-dechost-symphonia.exe,
rustamp-dsphost-stages.exe) are internal background processes, not user entry points.
No automatic updates: download new versions from
https://gitlab.com/rustamp/rustamp/-/releases

## MilkDrop

Rustamp ships no plug-ins and no MilkDrop data, because their licence is not
clearly free. To use MilkDrop 2, take these from your own Winamp 5 installation:

1. Beside rustamp.exe, create a folder named `Plugins` and copy `vis_milk2.dll`
   and the whole `Milkdrop2` folder from Winamp's `Plugins` into it. It must hold
   `Milkdrop2\data\*.fx` (eight files: MilkDrop does not start properly without
   them) and `Milkdrop2\presets\*.milk` (without presets it only draws its
   default). Keep `Milkdrop2\textures` too.
2. MilkDrop also needs a `d3dx9` library: install Microsoft's DirectX End-User
   Runtime, or put `d3dx9_31.dll` beside `vis_milk2.dll`. Copy Winamp's
   `Microsoft.VC90.CRT` folder beside rustamp.exe as well.
3. Approve that exact file: `.\rustamp.exe --approve-plugin "<path to vis_milk2.dll>"`,
   switch it on under Preferences, Plug-ins, Visualization, restart Rustamp, then
   start it with Ctrl+Shift+K.

When the data or the presets are missing, the player says so and names the folder.
"#,
        release_id = meta.release_id,
        imports = if meta.imports_verified {
            "The import tables of every EXE were inspected at packaging\ntime and name only Windows system DLLs; launch on a truly clean OS is not certified."
        } else {
            "Final DLL\nimports and launch on a truly clean OS still need release validation."
        },
    );
    if !bundled_host {
        text.push_str(
            "\nPlayer-only package: no helper is bundled; plugin supervision stays detached.\n",
        );
    }
    if profile == "debug" {
        text.push_str("\nDebug build: the GUI safety gate normally refuses outside the test VM.\nUse a release package for first-user launch; do not bypass the host GUI rule.\n");
    }
    text
}

fn readme(profile: &str, player: bool, host: bool, meta: &ReleaseMetadata) -> String {
    let s = format!(
        r#"# Rustamp {version} — Portable Release ({release_id})

Rustamp is a clean-room audio player for 64-bit Windows.

This package is a **portable ZIP release**. It requires no installer, creates no
registry keys, installs no background services, and makes no automatic network
connections. A separate installer (`*-setup.exe`) is published next to it.

## 1. Quick Start

1. Extract the `.zip` archive into any directory.
2. Open PowerShell in the `player` folder.
3. Run:
   ```powershell
   .\rustamp.exe "C:\Music\song.mp3"
   ```
   Or launch without arguments to start empty.

To uninstall: simply delete the extracted folder. User settings in
`%APPDATA%\Rustamp\rustamp.toml` may be deleted separately if desired.

## 2. Release & Integrity Metadata

- **Product Version:** `{version}`
- **Source Git Commit:** `{commit}` (`{commit_short}`)
- **Release Identifier:** `{release_id}`
- **IPC Protocol Version:** {protocol_version}
- **Config Schema Version:** {config_version}
- **Build Profile:** `{profile}`
- **Runtime:** MSVC C runtime statically linked (`-C target-feature=+crt-static`).
  No Microsoft VC++ Redistributable package is required.

### Integrity Verification
A SHA-256 checksum file is provided beside the archive (`<archive>.zip.sha256`).
Verify before extracting:
```powershell
Get-FileHash -Path .\{release_id}.zip -Algorithm SHA256
```

## 3. Important User Disclosures (G5 & G6)

### Unsigned Binaries (G5)
- Binaries are code-signed only when the build had a certificate; a build without one says so and the files show no Digital Signatures tab.
- For an unsigned file, Windows Defender SmartScreen will show:
  *"Windows protected your PC — Microsoft Defender SmartScreen prevented an unrecognized app from starting."*
- To proceed: Click **"More info"**, then select **"Run anyway"**.
- Web browsers may flag the download as unrecognized. Verify the SHA-256 hash to ensure integrity.

### No Automatic Updates (G6)
- Rustamp contains **no automatic update checker**, background service, or telemetry.
- It will never connect to the internet to check for or download updates.
- To upgrade: Manually download a new release archive from https://gitlab.com/rustamp/rustamp/-/releases.
- Settings in `%APPDATA%\Rustamp\rustamp.toml` are preserved across manual upgrades.

## 4. What This Release Can and Cannot Claim

### Proven & Functional:
- Audio playback for local MP3, FLAC, and WAV files via Symphonia and RIFF/WAVE decoders.
- Transport controls: play, pause, stop, frame-accurate seeking, and gapless track advance.
- 10-band Equalizer and preamp in the DSP chain (flat/unity by default).
- Built-in default drawn Rust theme skin, selectable lookalike theme (`--skin lookalike`),
  and external classic skin archives (.wsz, .zip, directories).
- Single-instance command-line forwarding via Windows named pipes.
- Configuration loaded from and saved to `%APPDATA%\Rustamp\rustamp.toml` on clean exit.
- Supervised background helpers (32-bit plugin host, decoder resident, DSP resident).
- Shuffle and repeat toggles drive the playback engine; the engine owns the shuffle plan.
- Built-in visualizer (spectrum analyzer and oscilloscope) is fed from the audio the player is playing.

### Unproven / Not Implemented:
- **Clean Machine Proof:** {clean_machine}
- **Physical Audio Hardware:** Tested in VM environments; physical device-loss timing, diverse sound card compatibility, and hardware latency remain uncertified.
- **Winamp Plugins:** The 32-bit helper and verified-byte approval store exist, but user-facing UI wiring is not yet implemented. The helper is not a sandbox. Winamp visualization plugins are not hosted.
- **Visualizer Fidelity:** The built-in visualizer's response (scaling, decay, refresh cadence) is not matched against the reference player.

## 5. Licensing & Third-Party Notices

Rustamp is clean-room software:
- Player executables are licensed under **GPL-3.0-or-later** with a **Section 7 Plugin Exception** (`player/PLUGIN-EXCEPTION.md`).
- Standalone plugin host (`rustamp-plughost32.exe`) is licensed under **MIT OR Apache-2.0** (`host/LICENSE-MIT`, `host/LICENSE-APACHE`).
- Source Code Offer: In accordance with GPL-3.0 Section 6, complete corresponding source code is available at https://gitlab.com/rustamp/rustamp and released on GitLab.
- See `THIRD_PARTY.md` and `player/THIRD_PARTY.md` for complete third-party notices, webamp skin table attribution, static MSVC CRT terms, and dependency inventories.
"#,
        version = meta.product_version,
        release_id = meta.release_id,
        commit = meta.commit,
        commit_short = meta.commit_short,
        protocol_version = meta.protocol_version,
        config_version = meta.config_version,
        profile = profile,
        clean_machine = if meta.imports_verified {
            "`artifact_imports_verified: true`: the import tables of every shipped EXE were parsed offline at packaging time and name only Windows system DLLs (no MSVCP140.dll, VCRUNTIME140.dll or other runtime). This is a static check; launch on a truly clean OS has not been certified."
        } else {
            "`artifact_imports_verified: false`: the import tables were not inspected for this package; launch on a truly clean OS without developer runtimes has not been certified."
        },
    );
    let _ = (player, host);
    s
}

/// Full Mozilla Public License 2.0 text (MPL-2.0 §3.2 / §3.1 require it to
/// accompany Symphonia in binary form). Verbatim copy of the SPDX license text,
/// stored in `xtask/licenses/MPL-2.0.txt`.
const MPL_2_0_TEXT: &str = include_str!("../licenses/MPL-2.0.txt");

/// Full Apache License 2.0 text, taken from the repository's own `LICENSE-APACHE`.
const APACHE_2_0_TEXT: &str = include_str!("../../LICENSE-APACHE");

fn ensure_trailing_newline(text: &str) -> String {
    let mut s = text.to_string();
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Generate third-party notices for the player package.
///
/// Contains:
/// 1. GPL-3.0 §6 Source Offer linking to repo and release.
/// 2. Static Microsoft MSVC CRT redistribution terms.
/// 3. Webamp classic skin tables MIT attribution.
/// 4. Symphonia MPL-2.0 notice, source availability, and full license text.
/// 5. Canonical permissive open source licenses (MIT, Apache-2.0, BSD-2/3, ISC, Zlib).
/// 6. Complete Cargo.lock license inventory from the repository records.
fn player_third_party_notices(root: &Path, meta: &ReleaseMetadata) -> Result<String, String> {
    let inventory = extract_cargo_inventory(root)?;
    let mut out = String::new();
    out.push_str(&format!(
        "# Rustamp Player — Third-Party Notices and Licenses\n\n\
         Release: {release_id} (commit {commit})\n\n\
         This file documents all third-party software components, source code offers,\n\
         static runtime terms, and dependency licenses distributed with the Rustamp player.\n\n\
         ---\n\n\
         ## 1. Source Code Offer (GNU GPL v3 §6)\n\n\
         The player and resident helper executables (`rustamp.exe`,\n\
         `rustamp-dechost-symphonia.exe`, `rustamp-dsphost-stages.exe`) are licensed under\n\
         the GNU General Public License v3 or later (GPL-3.0-or-later) with the\n\
         RFC 0004 Section 7 Plugin Exception (`PLUGIN-EXCEPTION.md`).\n\n\
         In accordance with Section 6 of the GNU GPL v3, the complete machine-readable\n\
         Corresponding Source code for this binary distribution is publicly available at:\n\n\
         - Repository: https://gitlab.com/rustamp/rustamp\n\
         - Release: https://gitlab.com/rustamp/rustamp/-/releases/v{version}\n\
         - Commit: {commit}\n\n\
         Recipients may also request the Corresponding Source directly from the maintainers.\n\n\
         ---\n\n\
         ## 2. Static Microsoft MSVC C Runtime Terms\n\n\
         All executables in this package are built with the static Microsoft Visual C++\n\
         runtime (`-C target-feature=+crt-static`), incorporating statically linked C/C++\n\
         runtime components (VCRuntime and UCRT). Redistribution in binary form is\n\
         governed by the Microsoft Visual Studio Software License Terms (REDIST.TXT).\n\
         No separate Microsoft Visual C++ redistributable installer is required.\n\
         Standard Windows system DLLs (kernel32.dll, user32.dll, etc.) remain required.\n\n\
         ---\n\n\
         ## 3. Webamp Classic Skin Tables (MIT)\n\n\
         Portions of skin layout tables and sprite coordinate definitions are derived from\n\
         Webamp (https://github.com/captbaritone/webamp), commit 79117c66d79fdac690fc732a0ce8df543ccd97a0:\n\n\
         ```text\n\
         Copyright (c) 2014 Jordan Eldredge\n\n\
         Permission is hereby granted, free of charge, to any person obtaining a copy\n\
         of this software and associated documentation files (the \"Software\"), to deal\n\
         in the Software without restriction, including without limitation the rights\n\
         to use, copy, modify, merge, publish, distribute, sublicense, and/or sell\n\
         copies of the Software, and to permit persons to whom the Software is\n\
         furnished to do so, subject to the following conditions:\n\n\
         The above copyright notice and this permission notice shall be included in all\n\
         copies or substantial portions of the Software.\n\n\
         THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR\n\
         IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,\n\
         FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE\n\
         AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\n\
         LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,\n\
         OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE\n\
         SOFTWARE.\n\
         ```\n\n\
         ---\n\n\
         ## 4. Symphonia Audio Decoders and Demuxers (MPL-2.0)\n\n\
         The audio decoding framework includes Symphonia (https://github.com/pdeljanov/Symphonia),\n\
         licensed under the Mozilla Public License, v. 2.0 (MPL-2.0).\n\n\
         Copyright (c) 2019-2022 The Symphonia Developers.\n\n\
         In accordance with Section 3.2 of the Mozilla Public License v2.0, the Source Code Form\n\
         of Symphonia is available at https://github.com/pdeljanov/Symphonia and via crates.io.\n\
         The terms of the Mozilla Public License, v. 2.0 apply to Symphonia. The full\n\
         license text follows (Mozilla Public License Version 2.0, also available at\n\
         https://www.mozilla.org/en-US/MPL/2.0/):\n\n\
         ```text\n\
         {mpl}\
         ```\n\n\
         ---\n\n\
         ## 5. Permissive Open Source License Texts\n\n\
         ### The MIT License\n\n\
         Permission is hereby granted, free of charge, to any person obtaining a copy\n\
         of this software and associated documentation files (the \"Software\"), to deal\n\
         in the Software without restriction, including without limitation the rights\n\
         to use, copy, modify, merge, publish, distribute, sublicense, and/or sell\n\
         copies of the Software, and to permit persons to whom the Software is\n\
         furnished to do so, subject to the following conditions:\n\n\
         The above copyright notice and this permission notice shall be included in all\n\
         copies or substantial portions of the Software.\n\n\
         THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR\n\
         IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,\n\
         FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE\n\
         AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\n\
         LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,\n\
         OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE\n\
         SOFTWARE.\n\n\
         ### Apache License, Version 2.0 (full text)\n\n\
         ```text\n\
         {apache}\
         ```\n\n\
         ### The 2-Clause BSD License\n\n\
         Redistribution and use in source and binary forms, with or without\n\
         modification, are permitted provided that the following conditions are met:\n\n\
         1. Redistributions of source code must retain the above copyright notice,\n\
            this list of conditions and the following disclaimer.\n\
         2. Redistributions in binary form must reproduce the above copyright notice,\n\
            this list of conditions and the following disclaimer in the documentation\n\
            and/or other materials provided with the distribution.\n\n\
         THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS \"AS IS\"\n\
         AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE\n\
         IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE\n\
         DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE\n\
         FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL\n\
         DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR\n\
         SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER\n\
         CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,\n\
         OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE\n\
         OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.\n\n\
         ### The 3-Clause BSD License\n\n\
         Redistribution and use in source and binary forms, with or without\n\
         modification, are permitted provided that the following conditions are met:\n\n\
         1. Redistributions of source code must retain the above copyright notice,\n\
            this list of conditions and the following disclaimer.\n\
         2. Redistributions in binary form must reproduce the above copyright notice,\n\
            this list of conditions and the following disclaimer in the documentation\n\
            and/or other materials provided with the distribution.\n\
         3. Neither the name of the copyright holder nor the names of its contributors\n\
            may be used to endorse or promote products derived from this software\n\
            without specific prior written permission.\n\n\
         THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS \"AS IS\"\n\
         AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE\n\
         IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE\n\
         DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE\n\
         FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL\n\
         DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR\n\
         SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER\n\
         CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,\n\
         OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE\n\
         OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.\n\n\
         ### The zlib License\n\n\
         This software is provided 'as-is', without any express or implied warranty.\n\
         In no event will the authors be held liable for any damages arising from\n\
         the use of this software.\n\n\
         Permission is granted to anyone to use this software for any purpose,\n\
         including commercial applications, and to alter it and redistribute it freely,\n\
         subject to the following restrictions:\n\n\
         1. The origin of this software must not be misrepresented; you must not\n\
            claim that you wrote the original software. If you use this software in\n\
            a product, an acknowledgment in the product documentation would be\n\
            appreciated but is not required.\n\
         2. Altered source versions must be plainly marked as such, and must not\n\
            be misrepresented as being the original software.\n\
         3. This notice may not be removed or altered from any source distribution.\n\n\
         ---\n\n\
         ## 6. Complete Cargo Dependency Inventory\n\n\
         {inventory}\n\n\
         ---\n\n\
         ## 7. Installer\n\n\
         The Windows installer (`*-setup.exe`) and its `uninstall.exe` are built with NSIS\n\
         (https://nsis.sourceforge.io), Copyright (C) 1999-2026 Contributors, under the\n\
         zlib/libpng license; its LZMA module is under the Common Public License 1.0 with\n\
         the NSIS linking exception.\n",
        release_id = meta.release_id,
        commit = meta.commit,
        version = meta.product_version,
        inventory = inventory,
        mpl = ensure_trailing_newline(&MPL_2_0_TEXT.replace("\r\n", "\n")),
        apache = ensure_trailing_newline(&APACHE_2_0_TEXT.replace("\r\n", "\n")),
    ));
    Ok(out)
}

/// Generate third-party notices for the standalone plugin host package.
///
/// Must be completely GPL-free, copyleft-free, and MPL-free.
fn host_third_party_notices(root: &Path, meta: &ReleaseMetadata) -> Result<String, String> {
    let _ = root;
    let mut out = String::new();
    out.push_str(&format!(
        "# Rustamp Plugin Host — Third-Party Notices and Licenses\n\n\
         Release: {release_id} (commit {commit})\n\n\
         `rustamp-plughost32.exe` is the 32-bit standalone helper process for hosting\n\
         classic Winamp plugins (RFC 0004 / RFC 0006). It is licensed under MIT OR Apache-2.0\n\
         (see LICENSE-MIT and LICENSE-APACHE).\n\n\
         **Permissive Scope Confirmation:** This package contains exclusively permissive\n\
         material under MIT or Apache-2.0 terms. Its complete normal dependency closure has been\n\
         verified by automated audit against an allowlist containing exclusively permissive licenses.\n\
         No reciprocal copyleft terms apply to this standalone helper.\n\n\
         ---\n\n\
         ## 1. Static Microsoft MSVC C Runtime Terms\n\n\
         `rustamp-plughost32.exe` is compiled for `i686-pc-windows-msvc` with `-C target-feature=+crt-static`,\n\
         incorporating statically linked Microsoft Visual C++ runtime components under\n\
         the Microsoft Visual Studio Software License Terms (REDIST.TXT).\n\
         WOW64 is required to run on 64-bit Windows.\n\n\
         ---\n\n\
         ## 2. Permissive License Texts\n\n\
         ### The MIT License\n\n\
         Permission is hereby granted, free of charge, to any person obtaining a copy\n\
         of this software and associated documentation files (the \"Software\"), to deal\n\
         in the Software without restriction, including without limitation the rights\n\
         to use, copy, modify, merge, publish, distribute, sublicense, and/or sell\n\
         copies of the Software, and to permit persons to whom the Software is\n\
         furnished to do so, subject to the following conditions:\n\n\
         The above copyright notice and this permission notice shall be included in all\n\
         copies or substantial portions of the Software.\n\n\
         THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR\n\
         IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,\n\
         FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE\n\
         AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\n\
         LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,\n\
         OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE\n\
         SOFTWARE.\n\n\
         ### Apache License, Version 2.0\n\n\
         Licensed under the Apache License, Version 2.0 (the \"License\");\n\
         you may not use this file except in compliance with the License.\n\
         You may obtain a copy of the License at:\n\n\
         http://www.apache.org/licenses/LICENSE-2.0\n\n\
         Unless required by applicable law or agreed to in writing, software\n\
         distributed under the License is distributed on an \"AS IS\" BASIS,\n\
         WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.\n\
         See the License for the specific language governing permissions and\n\
         limitations under the License.\n\n\
         ---\n\n\
         ## 3. Host Dependency Closure Inventory\n\n\
         | Package | Declared license | Normal Closure |\n\
         |---|---|---|\n\
         | `bitflags` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `byteorder` | `Unlicense OR MIT` | `rustamp-plughost` |\n\
         | `cobs` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `crossbeam-channel` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `crossbeam-utils` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `hash32` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `heapless` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `postcard` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `proc-macro2` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `quote` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `rustamp-ipc` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `rustamp-plugin-abi` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `rustamp-plughost` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `rustamp-proto` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `serde` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `serde_core` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `serde_derive` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `stable_deref_trait` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `syn` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `thiserror` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `thiserror-impl` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `unicode-ident` | `(MIT OR Apache-2.0) AND Unicode-3.0` | `rustamp-plughost` |\n\
         | `windows` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `windows-collections` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `windows-core` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `windows-future` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `windows-implement` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `windows-interface` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `windows-link` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `windows-numerics` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `windows-result` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `windows-strings` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n\
         | `windows-threading` | `MIT OR Apache-2.0` | `rustamp-plughost` |\n",
        release_id = meta.release_id,
        commit = meta.commit,
    ));
    Ok(out)
}

fn extract_cargo_inventory(root: &Path) -> Result<String, String> {
    let third_party_path = root.join("THIRD_PARTY.md");
    if let Ok(content) = fs::read_to_string(&third_party_path) {
        const BEGIN: &str = "<!-- BEGIN CARGO LICENSE INVENTORY -->";
        const END: &str = "<!-- END CARGO LICENSE INVENTORY -->";
        if let (Some(b), Some(e)) = (content.find(BEGIN), content.find(END)) {
            return Ok(content[b..e + END.len()].to_string());
        }
    }
    // Fallback if THIRD_PARTY.md cannot be read in a test fixture
    Ok("<!-- BEGIN CARGO LICENSE INVENTORY -->\n(inventory metadata)\n<!-- END CARGO LICENSE INVENTORY -->".to_string())
}

/// The executables a staged layout ships, with the machine type each must have.
fn staged_binaries(out: &Path, player: bool, host: bool) -> Vec<(PathBuf, u16)> {
    let mut files = Vec::new();
    if player {
        let dir = out.join("player");
        files.push((dir.join(binary_name(PLAYER_BIN)), pe_imports::MACHINE_X64));
        files.push((dir.join(binary_name(DECHOST_BIN)), pe_imports::MACHINE_X64));
        files.push((dir.join(binary_name(DSPHOST_BIN)), pe_imports::MACHINE_X64));
        if host {
            files.push((dir.join(binary_name(HOST_BIN)), pe_imports::MACHINE_X86));
        }
    }
    if host {
        files.push((
            out.join("host").join(binary_name(HOST_BIN)),
            pe_imports::MACHINE_X86,
        ));
    }
    files
}

/// Parse the import tables of every file and require Windows system DLLs only.
///
/// Reports every file before failing, so one run shows the whole picture. An
/// empty list, an unreadable file, a non-PE file or a wrong machine type is an
/// error, never a pass. No binary is executed.
fn inspect_imports(files: &[(PathBuf, u16)]) -> Result<(), String> {
    if files.is_empty() {
        return Err("PE import inspection has no binary to inspect; refusing".to_string());
    }
    let mut failures = Vec::new();
    for (path, machine) in files {
        match pe_imports::verify_file(path, Some(*machine), None) {
            Ok(imports) => println!(
                "xtask package: imports OK   {}",
                pe_imports::summary(path, &imports)
            ),
            Err(e) => {
                eprintln!("xtask package: imports FAIL {e}");
                failures.push(e);
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "PE import inspection failed for {} of {} binaries: {}",
            failures.len(),
            files.len(),
            failures.join("; ")
        ))
    }
}

/// Read-only headless guard, run even without artifacts by `package --check`.
/// It checks the actual command factory and generated policy, not PE imports.
fn check_runtime_contract(root: &Path) -> Result<(), String> {
    for (crate_name, target, bin) in [
        (PLAYER_CRATE, PLAYER_TARGET, PLAYER_BIN),
        (HOST_CRATE, HOST_TARGET, HOST_BIN),
        (DECHOST_CRATE, DECHOST_TARGET, DECHOST_BIN),
        (DSPHOST_CRATE, DSPHOST_TARGET, DSPHOST_BIN),
    ] {
        for release in [false, true] {
            let cmd = artifact_command(root, crate_name, target, bin, release);
            let env: Vec<_> = cmd.get_envs().collect();
            let flags_ok = env.iter().any(|(key, value)| {
                *key == "CARGO_ENCODED_RUSTFLAGS"
                    && *value == Some(std::ffi::OsStr::new("-C\x1ftarget-feature=+crt-static"))
            });
            let cache = package_build_dir(root);
            let cache_ok = env.iter().any(|(key, value)| {
                *key == "CARGO_TARGET_DIR" && *value == Some(cache.as_os_str())
            });
            if !flags_ok || !cache_ok {
                return Err(format!(
                    "package command contract broken for {crate_name}:{bin} (release={release})"
                ));
            }
        }
    }
    let dummy_meta = ReleaseMetadata::for_testing();
    for manifest in [
        serde_manifest_player("release", true, &dummy_meta),
        serde_manifest_player("release", false, &dummy_meta),
        serde_manifest_host("release", &dummy_meta),
        build_top_manifest("release", true, true, true, &dummy_meta),
        build_top_manifest("release", true, false, false, &dummy_meta),
        build_top_manifest("release", false, true, false, &dummy_meta),
    ] {
        for required in [
            "\"crt_linkage\":\"static\"",
            "target-feature=+crt-static",
            "\"vc_redist_required\":false",
            "\"bundled_runtime_dlls\":[]",
            "\"artifact_imports_verified\":false",
            "\"schema_version\": 3",
            "\"product_version\": \"0.0.1\"",
            &format!("\"protocol_version\": {PROTOCOL_VERSION}"),
        ] {
            if !manifest.contains(required) {
                return Err(format!("runtime manifest contract missing {required}"));
            }
        }
    }
    let verified_meta = ReleaseMetadata {
        imports_verified: true,
        ..dummy_meta.clone()
    };
    for manifest in [
        serde_manifest_player("release", true, &verified_meta),
        serde_manifest_host("release", &verified_meta),
        build_top_manifest("release", true, true, true, &verified_meta),
    ] {
        if !manifest.contains("\"artifact_imports_verified\":true")
            || manifest.contains("\"artifact_imports_verified\":false")
        {
            return Err("runtime manifest contract: verified imports not recorded".to_string());
        }
    }
    let guide = quickstart("release", true, true, &dummy_meta);
    if ![
        "--skin",
        "--no-audio",
        "--null",
        "static MSVC CRT",
        "Milkdrop2",
    ]
    .iter()
    .all(|required| guide.contains(required))
    {
        return Err("quickstart runtime/CLI contract failed".to_string());
    }
    Ok(())
}

fn require_empty_output(dir: &Path) -> Result<(), String> {
    if dir.exists()
        && fs::read_dir(dir)
            .map_err(|e| format!("read output dir: {e}"))?
            .next()
            .is_some()
    {
        return Err(format!(
            "output {} is not empty; choose a new --out directory (nothing removed)",
            dir.display()
        ));
    }
    Ok(())
}

/// Exact allowlists preserve the licence split and reject stale runtime DLLs.
/// No executable is loaded or run; non-empty files are only layout evidence.
fn audit_directory(dir: &Path, files: &[&str], dirs: &[&str]) -> Result<(), String> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("read package entry: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type().map_err(|e| format!("stat {name}: {e}"))?;
        let expected = (kind.is_file() && files.contains(&name.as_str()))
            || (kind.is_dir() && dirs.contains(&name.as_str()));
        if !expected {
            return Err(format!(
                "unexpected package entry: {}",
                entry.path().display()
            ));
        }
        if kind.is_file()
            && entry
                .metadata()
                .map_err(|e| format!("stat {name}: {e}"))?
                .len()
                == 0
        {
            return Err(format!("empty package file: {}", entry.path().display()));
        }
        found.push(name);
    }
    for name in files.iter().chain(dirs) {
        if !found.iter().any(|f| f == name) {
            return Err(format!(
                "missing package entry: {}",
                dir.join(name).display()
            ));
        }
    }
    Ok(())
}

fn audit_text(path: &Path, expected: &str) -> Result<(), String> {
    let actual = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if actual != expected {
        return Err(format!(
            "generated package metadata differs: {}",
            path.display()
        ));
    }
    Ok(())
}

fn audit_package_layout(
    out: &Path,
    profile: &str,
    player: bool,
    host: bool,
    meta: &ReleaseMetadata,
) -> Result<(), String> {
    let root = rustamp_testkit::workspace_root();
    let mut dirs = Vec::new();
    if player {
        dirs.push("player");
        let mut files = vec![
            "rustamp.exe",
            "rustamp-dechost-symphonia.exe",
            "rustamp-dsphost-stages.exe",
            "LICENSE",
            "PLUGIN-EXCEPTION.md",
            "THIRD_PARTY.md",
            "manifest.json",
            "QUICKSTART.md",
        ];
        if host {
            files.push("rustamp-plughost32.exe");
        }
        let dir = out.join("player");
        audit_directory(&dir, &files, &[])?;
        audit_text(
            &dir.join("manifest.json"),
            &serde_manifest_player(profile, host, meta),
        )?;
        audit_text(
            &dir.join("QUICKSTART.md"),
            &quickstart(profile, true, host, meta),
        )?;
        audit_text(
            &dir.join("THIRD_PARTY.md"),
            &player_third_party_notices(&root, meta)?,
        )?;
    }
    if host {
        dirs.push("host");
        let dir = out.join("host");
        audit_directory(
            &dir,
            &[
                "rustamp-plughost32.exe",
                "LICENSE-MIT",
                "LICENSE-APACHE",
                "THIRD_PARTY.md",
                "manifest.json",
            ],
            &[],
        )?;
        audit_text(
            &dir.join("manifest.json"),
            &serde_manifest_host(profile, meta),
        )?;
        audit_text(
            &dir.join("THIRD_PARTY.md"),
            &host_third_party_notices(&root, meta)?,
        )?;
        audit_host_dir(&dir)?;
    }
    audit_directory(
        out,
        &[
            "manifest.json",
            "QUICKSTART.md",
            "README.md",
            "THIRD_PARTY.md",
        ],
        &dirs,
    )?;
    audit_text(
        &out.join("manifest.json"),
        &build_top_manifest(profile, player, host, player && host, meta),
    )?;
    audit_text(
        &out.join("QUICKSTART.md"),
        &quickstart(profile, player, player && host, meta),
    )?;
    audit_text(&out.join("README.md"), &readme(profile, player, host, meta))?;
    // Top-level THIRD_PARTY.md must match repository root THIRD_PARTY.md
    if root.join("THIRD_PARTY.md").is_file() {
        let expected_top_notices = fs::read_to_string(root.join("THIRD_PARTY.md"))
            .map_err(|e| format!("read root THIRD_PARTY.md: {e}"))?;
        audit_text(&out.join("THIRD_PARTY.md"), &expected_top_notices)?;
    }
    Ok(())
}

fn audit_version_consistency(out_dir: &Path, meta: &ReleaseMetadata) -> Result<(), String> {
    let manifest_path = out_dir.join("manifest.json");
    if !manifest_path.is_file() {
        return Err(format!(
            "missing top manifest at {}",
            manifest_path.display()
        ));
    }
    let content = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("read manifest {}: {e}", manifest_path.display()))?;

    let expected_ver = format!("\"product_version\": \"{}\"", meta.product_version);
    if !content.contains(&expected_ver) {
        return Err(format!(
            "manifest version mismatch: expected `{expected_ver}`"
        ));
    }
    let expected_commit = format!("\"commit\": \"{}\"", meta.commit);
    if !content.contains(&expected_commit) {
        return Err(format!(
            "manifest commit mismatch: expected `{expected_commit}`"
        ));
    }
    if !content.contains("\"schema_version\": 3") {
        return Err("manifest schema_version mismatch: expected 3".to_string());
    }

    let dir_name = out_dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if dir_name.starts_with("rustamp-") {
        if !dir_name.contains(&meta.product_version) {
            return Err(format!(
                "output folder name `{dir_name}` disagrees with product version `{}`",
                meta.product_version
            ));
        }
        if meta.commit_short != "unknown" && !dir_name.contains(&meta.commit_short) {
            return Err(format!(
                "output folder name `{dir_name}` disagrees with commit `{}`",
                meta.commit_short
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Pure-Rust Clean-Room SHA-256 Hasher (FIPS 180-4)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total_len = self
            .total_len
            .checked_add(data.len() as u64)
            .expect("image size must fit in u64");

        if self.buf_len > 0 {
            let needed = 64 - self.buf_len;
            if data.len() >= needed {
                self.buffer[self.buf_len..64].copy_from_slice(&data[..needed]);
                let chunk = self.buffer;
                Self::process_block(&mut self.state, &chunk);
                self.buf_len = 0;
                data = &data[needed..];
            } else {
                self.buffer[self.buf_len..self.buf_len + data.len()].copy_from_slice(data);
                self.buf_len += data.len();
                return;
            }
        }

        while data.len() >= 64 {
            let chunk: &[u8; 64] = data[..64].try_into().unwrap();
            Self::process_block(&mut self.state, chunk);
            data = &data[64..];
        }

        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self
            .total_len
            .checked_mul(8)
            .expect("bit length must not overflow u64");
        self.buffer[self.buf_len] = 0x80;
        self.buf_len += 1;

        if self.buf_len > 56 {
            self.buffer[self.buf_len..64].fill(0);
            let chunk = self.buffer;
            Self::process_block(&mut self.state, &chunk);
            self.buf_len = 0;
        }

        self.buffer[self.buf_len..56].fill(0);
        self.buffer[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let chunk = self.buffer;
        Self::process_block(&mut self.state, &chunk);

        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    pub fn digest(data: &[u8]) -> [u8; 32] {
        let mut hasher = Self::new();
        hasher.update(data);
        hasher.finalize()
    }

    pub fn digest_hex(data: &[u8]) -> String {
        let d = Self::digest(data);
        let mut s = String::with_capacity(64);
        for byte in d {
            use std::fmt::Write;
            let _ = write!(&mut s, "{:02x}", byte);
        }
        s
    }

    #[inline]
    fn process_block(state: &mut [u32; 8], block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];

        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..(i + 1) * 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust IEEE 802.3 CRC-32 (Standard ZIP Checksum)
// ---------------------------------------------------------------------------

const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut j = 0;
        while j < 8 {
            if c & 1 != 0 {
                c = 0xEDB8_8320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            j += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = CRC32_TABLE[((c ^ (b as u32)) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

// ---------------------------------------------------------------------------
// Pure-Rust Portable ZIP Writer (APPNOTE.TXT Compliant)
// ---------------------------------------------------------------------------

struct ZipFileEntry {
    rel_path: String,
    data: Vec<u8>,
    crc32: u32,
    offset: u32,
}

fn collect_dir_files(
    dir: &Path,
    base: &Path,
    entries: &mut Vec<(PathBuf, String)>,
) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("read dir entry: {e}"))?;
        let path = entry.path();
        let meta = entry
            .metadata()
            .map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.is_dir() {
            collect_dir_files(&path, base, entries)?;
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(base)
                .map_err(|e| format!("rel path: {e}"))?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            entries.push((path, rel_str));
        }
    }
    Ok(())
}

fn create_zip_archive(zip_path: &Path, source_dir: &Path, root_prefix: &str) -> Result<(), String> {
    use std::io::{Seek, Write};
    let mut file_list = Vec::new();
    collect_dir_files(source_dir, source_dir, &mut file_list)?;
    file_list.sort_by(|a, b| a.1.cmp(&b.1));

    if let Some(parent) = zip_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create parent for zip: {e}"))?;
    }
    let mut zip_file = fs::File::create(zip_path).map_err(|e| format!("create zip file: {e}"))?;

    let mut entries = Vec::new();

    // 2026-10-18 MS-DOS date: ((2026 - 1980) << 9) | (10 << 5) | 18 = 0x5D52
    // 12:00:00 MS-DOS time: (12 << 11) = 0x6000
    const DOS_DATE: u16 = 0x5D52;
    const DOS_TIME: u16 = 0x6000;

    for (abs_path, rel_str) in file_list {
        let data = fs::read(&abs_path).map_err(|e| format!("read {}: {e}", abs_path.display()))?;
        let crc = crc32(&data);
        let name = if root_prefix.is_empty() {
            rel_str
        } else {
            format!("{root_prefix}/{rel_str}")
        };
        let offset = zip_file
            .stream_position()
            .map_err(|e| format!("stream pos: {e}"))? as u32;

        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len() as u16;
        let size = data.len() as u32;

        // Local header (30 bytes)
        zip_file
            .write_all(&0x04034b50u32.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&20u16.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?; // version needed 2.0
        zip_file
            .write_all(&0x0800u16.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?; // UTF-8 flag
        zip_file
            .write_all(&0u16.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?; // method 0 (stored)
        zip_file
            .write_all(&DOS_TIME.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&DOS_DATE.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&crc.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&size.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&size.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&name_len.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&0u16.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?; // extra field len
        zip_file
            .write_all(name_bytes)
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&data)
            .map_err(|e| format!("write zip: {e}"))?;

        entries.push(ZipFileEntry {
            rel_path: name,
            data,
            crc32: crc,
            offset,
        });
    }

    let cd_offset = zip_file
        .stream_position()
        .map_err(|e| format!("stream pos: {e}"))? as u32;

    for entry in &entries {
        let name_bytes = entry.rel_path.as_bytes();
        let name_len = name_bytes.len() as u16;
        let size = entry.data.len() as u32;

        // Central directory header (46 bytes)
        zip_file
            .write_all(&0x02014b50u32.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&20u16.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?; // version made by
        zip_file
            .write_all(&20u16.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?; // version needed
        zip_file
            .write_all(&0x0800u16.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?; // UTF-8
        zip_file
            .write_all(&0u16.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?; // method 0
        zip_file
            .write_all(&DOS_TIME.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&DOS_DATE.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&entry.crc32.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&size.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&size.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&name_len.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(&0u16.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?; // extra field len
        zip_file
            .write_all(&0u16.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?; // comment len
        zip_file
            .write_all(&0u16.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?; // disk number
        zip_file
            .write_all(&0u16.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?; // internal attrs
        zip_file
            .write_all(&0u32.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?; // external attrs
        zip_file
            .write_all(&entry.offset.to_le_bytes())
            .map_err(|e| format!("write zip: {e}"))?;
        zip_file
            .write_all(name_bytes)
            .map_err(|e| format!("write zip: {e}"))?;
    }

    let cd_end = zip_file
        .stream_position()
        .map_err(|e| format!("stream pos: {e}"))? as u32;
    let cd_size = cd_end - cd_offset;
    let total_records = entries.len() as u16;

    // End of central directory record (22 bytes)
    zip_file
        .write_all(&0x06054b50u32.to_le_bytes())
        .map_err(|e| format!("write zip: {e}"))?;
    zip_file
        .write_all(&0u16.to_le_bytes())
        .map_err(|e| format!("write zip: {e}"))?; // disk number
    zip_file
        .write_all(&0u16.to_le_bytes())
        .map_err(|e| format!("write zip: {e}"))?; // start disk
    zip_file
        .write_all(&total_records.to_le_bytes())
        .map_err(|e| format!("write zip: {e}"))?;
    zip_file
        .write_all(&total_records.to_le_bytes())
        .map_err(|e| format!("write zip: {e}"))?;
    zip_file
        .write_all(&cd_size.to_le_bytes())
        .map_err(|e| format!("write zip: {e}"))?;
    zip_file
        .write_all(&cd_offset.to_le_bytes())
        .map_err(|e| format!("write zip: {e}"))?;
    zip_file
        .write_all(&0u16.to_le_bytes())
        .map_err(|e| format!("write zip: {e}"))?; // comment len
    zip_file.flush().map_err(|e| format!("flush zip: {e}"))?;
    Ok(())
}

fn write_checksum_file(zip_path: &Path, checksum_path: &Path) -> Result<(), String> {
    let bytes = fs::read(zip_path).map_err(|e| format!("read zip for checksum: {e}"))?;
    let hash = Sha256::digest_hex(&bytes);
    let zip_filename = zip_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archive.zip");
    let content = format!("{hash} *{zip_filename}\n");
    fs::write(checksum_path, content).map_err(|e| format!("write checksum file: {e}"))?;
    Ok(())
}

fn audit_zip_archive(zip_path: &Path, meta: &ReleaseMetadata) -> Result<(), String> {
    if !zip_path.is_file() {
        return Err(format!(
            "expected zip archive missing at {}",
            zip_path.display()
        ));
    }
    let bytes = fs::read(zip_path).map_err(|e| format!("read zip: {e}"))?;
    if bytes.len() < 22 {
        return Err(format!("zip archive too small: {} bytes", bytes.len()));
    }
    // Verify EOCD signature in last 22 bytes
    let eocd_sig = &bytes[bytes.len() - 22..bytes.len() - 18];
    if eocd_sig != 0x06054b50u32.to_le_bytes() {
        return Err("invalid zip archive: missing EOCD signature".to_string());
    }

    // Check checksum file if beside archive
    let checksum_name = format!(
        "{}.sha256",
        zip_path.file_name().and_then(|n| n.to_str()).unwrap_or("")
    );
    let checksum_path = zip_path.with_file_name(checksum_name);
    if checksum_path.is_file() {
        let content = fs::read_to_string(&checksum_path)
            .map_err(|e| format!("read checksum {}: {e}", checksum_path.display()))?;
        let actual_hash = Sha256::digest_hex(&bytes);
        let expected_hash = content
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if actual_hash != expected_hash {
            return Err(format!(
                "zip checksum mismatch: actual {actual_hash} != expected {expected_hash}"
            ));
        }
    }
    let _ = meta;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_four_package_commands_and_manifests_enforce_static_crt() {
        check_runtime_contract(Path::new("workspace")).expect("static CRT contract");
    }

    /// Filesystem-only fixtures: do not execute either stub binary.
    #[test]
    fn layout_check_rejects_missing_guide_stale_crt_and_wrong_licences() {
        let base = std::env::temp_dir().join(format!(
            "rustamp-package-layout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = rustamp_testkit::workspace_root();
        let meta = ReleaseMetadata::for_testing();
        fs::create_dir(&base).unwrap();
        for (player, host) in [(true, true), (true, false), (false, true)] {
            let out = base.join(format!("{player}-{host}"));
            fs::create_dir(&out).unwrap();
            let guide = quickstart("release", player, player && host, &meta);
            fs::write(out.join("QUICKSTART.md"), &guide).unwrap();
            fs::write(
                out.join("README.md"),
                readme("release", player, host, &meta),
            )
            .unwrap();
            if root.join("THIRD_PARTY.md").is_file() {
                copy_file(&root.join("THIRD_PARTY.md"), &out.join("THIRD_PARTY.md")).unwrap();
            } else {
                fs::write(out.join("THIRD_PARTY.md"), b"third party notices").unwrap();
            }
            fs::write(
                out.join("manifest.json"),
                build_top_manifest("release", player, host, player && host, &meta),
            )
            .unwrap();
            for (enabled, dir_name, files, manifest) in [
                (
                    player,
                    "player",
                    &[
                        "rustamp.exe",
                        "rustamp-dechost-symphonia.exe",
                        "rustamp-dsphost-stages.exe",
                        "LICENSE",
                        "PLUGIN-EXCEPTION.md",
                    ][..],
                    serde_manifest_player("release", host, &meta),
                ),
                (
                    host,
                    "host",
                    &["rustamp-plughost32.exe", "LICENSE-MIT", "LICENSE-APACHE"][..],
                    serde_manifest_host("release", &meta),
                ),
            ] {
                if enabled {
                    let dir = out.join(dir_name);
                    fs::create_dir(&dir).unwrap();
                    for file in files {
                        fs::write(dir.join(file), b"layout fixture, not executable").unwrap();
                    }
                    if dir_name == "player" {
                        fs::write(
                            dir.join("THIRD_PARTY.md"),
                            player_third_party_notices(&root, &meta).unwrap(),
                        )
                        .unwrap();
                    } else {
                        fs::write(
                            dir.join("THIRD_PARTY.md"),
                            host_third_party_notices(&root, &meta).unwrap(),
                        )
                        .unwrap();
                    }
                    fs::write(dir.join("manifest.json"), manifest).unwrap();
                }
            }
            if player {
                fs::write(out.join("player/QUICKSTART.md"), &guide).unwrap();
                if host {
                    fs::write(out.join("player/rustamp-plughost32.exe"), b"host stub").unwrap();
                }
            }
            assert!(audit_package_layout(&out, "release", player, host, &meta).is_ok());
            assert!(audit_version_consistency(&out, &meta).is_ok());
            assert!(require_empty_output(&out).is_err());

            // Missing QUICKSTART fails
            fs::remove_file(out.join("QUICKSTART.md")).unwrap();
            assert!(audit_package_layout(&out, "release", player, host, &meta).is_err());
            fs::write(out.join("QUICKSTART.md"), &guide).unwrap();

            // Missing README fails
            fs::remove_file(out.join("README.md")).unwrap();
            assert!(audit_package_layout(&out, "release", player, host, &meta).is_err());
            fs::write(
                out.join("README.md"),
                readme("release", player, host, &meta),
            )
            .unwrap();

            // Missing THIRD_PARTY.md fails
            fs::remove_file(out.join("THIRD_PARTY.md")).unwrap();
            assert!(audit_package_layout(&out, "release", player, host, &meta).is_err());
            // Same guarded restore as the fixture setup above: the audit only
            // compares against the repository file when it exists.
            if root.join("THIRD_PARTY.md").is_file() {
                copy_file(&root.join("THIRD_PARTY.md"), &out.join("THIRD_PARTY.md")).unwrap();
            } else {
                fs::write(out.join("THIRD_PARTY.md"), b"third party notices").unwrap();
            }

            let dir = out.join(if player { "player" } else { "host" });
            for stale in ["vcruntime140.dll", "unexpected.exe", "LICENSE-GPL"] {
                fs::write(dir.join(stale), b"stale").unwrap();
                assert!(audit_package_layout(&out, "release", player, host, &meta).is_err());
                fs::remove_file(dir.join(stale)).unwrap();
            }
            let wrong_licence = if player { "LICENSE-MIT" } else { "LICENSE" };
            fs::write(dir.join(wrong_licence), b"wrong side").unwrap();
            assert!(audit_package_layout(&out, "release", player, host, &meta).is_err());
            fs::remove_file(dir.join(wrong_licence)).unwrap();
            fs::write(dir.join("manifest.json"), b"dynamic CRT").unwrap();
            assert!(audit_package_layout(&out, "release", player, host, &meta).is_err());
        }
        // Best-effort cleanup after every claim above has passed or failed on
        // its own merits: a locked temp file on Windows must not flip a green
        // test red, and unique temp names make leftovers harmless.
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn bundled_host_name_matches_the_packaged_candidate() {
        assert_eq!(binary_name(HOST_BIN), "rustamp-plughost32.exe");
        assert_eq!(binary_name(PLAYER_BIN), "rustamp.exe");
        let player_dir = Path::new("player");
        let bundled = player_dir.join(binary_name(HOST_BIN));
        assert_eq!(bundled, Path::new("player").join("rustamp-plughost32.exe"));
        assert_ne!(binary_name(HOST_BIN), binary_name(PLAYER_BIN));
    }

    #[test]
    fn resident_binary_names_match_the_crates() {
        assert_eq!(binary_name(DECHOST_BIN), "rustamp-dechost-symphonia.exe");
        assert_eq!(binary_name(DSPHOST_BIN), "rustamp-dsphost-stages.exe");
        assert_eq!(DECHOST_CRATE, "rustamp-dechost");
        assert_eq!(DSPHOST_CRATE, "rustamp-dsphost");
        assert_eq!(DECHOST_TARGET, "x86_64-pc-windows-msvc");
        assert_eq!(DSPHOST_TARGET, "x86_64-pc-windows-msvc");
    }

    #[test]
    fn player_manifest_records_the_bundled_host_and_notices() {
        let meta = ReleaseMetadata::for_testing();
        let with = serde_manifest_player("release", true, &meta);
        assert!(
            with.contains("\"bundled_host\": \"rustamp-plughost32.exe\""),
            "{with}"
        );
        assert!(with.contains("RFC 0006"), "{with}");
        assert!(
            with.contains(
                "\"license_files\": [\"LICENSE\", \"PLUGIN-EXCEPTION.md\", \"THIRD_PARTY.md\"]"
            ),
            "{with}"
        );
        assert!(
            with.contains("\"third_party_notices\": \"THIRD_PARTY.md\""),
            "{with}"
        );
        assert!(with.contains("\"schema_version\": 3"), "{with}");
        assert!(with.contains("\"product_version\": \"0.0.1\""), "{with}");
        assert!(
            with.contains(&format!("\"protocol_version\": {PROTOCOL_VERSION}")),
            "{with}"
        );
        assert!(with.contains("\"config_version\": 1"), "{with}");
        assert!(!with.contains("LICENSE-MIT"), "{with}");
        assert!(!with.contains("LICENSE-APACHE"), "{with}");

        let without = serde_manifest_player("release", false, &meta);
        assert!(without.contains("\"bundled_host\": null"), "{without}");
        assert!(!without.contains("rustamp-plughost32"), "{without}");
    }

    #[test]
    fn player_manifest_records_the_residents() {
        let meta = ReleaseMetadata::for_testing();
        let manifest = serde_manifest_player("release", true, &meta);
        assert!(
            manifest.contains("\"decoder_host\": \"rustamp-dechost-symphonia.exe\""),
            "{manifest}"
        );
        assert!(
            manifest.contains("\"decoder_host_crate\": \"rustamp-dechost\""),
            "{manifest}"
        );
        assert!(
            manifest.contains("\"dsp_host\": \"rustamp-dsphost-stages.exe\""),
            "{manifest}"
        );
        assert!(
            manifest.contains("\"dsp_host_crate\": \"rustamp-dsphost\""),
            "{manifest}"
        );
    }

    #[test]
    fn host_manifest_stays_gpl_free_and_player_free() {
        let meta = ReleaseMetadata::for_testing();
        let manifest = serde_manifest_host("release", &meta);
        assert!(
            manifest.contains("\"contains_gpl_binary\": false"),
            "{manifest}"
        );
        assert!(
            manifest.contains(
                "\"license_files\": [\"LICENSE-MIT\", \"LICENSE-APACHE\", \"THIRD_PARTY.md\"]"
            ),
            "{manifest}"
        );
        assert!(
            manifest.contains("\"third_party_notices\": \"THIRD_PARTY.md\""),
            "{manifest}"
        );
        assert!(manifest.contains("\"schema_version\": 3"), "{manifest}");
        assert!(!manifest.contains("rustamp.exe"), "{manifest}");
        assert!(!manifest.contains("PLUGIN-EXCEPTION"), "{manifest}");
        assert!(!manifest.contains("rustamp-dechost"), "{manifest}");
        assert!(!manifest.contains("rustamp-dsphost"), "{manifest}");
    }

    #[test]
    fn top_manifest_names_the_bundled_host_only_when_bundled() {
        let meta = ReleaseMetadata::for_testing();
        let both = build_top_manifest("release", true, true, true, &meta);
        assert!(
            both.contains("\"binary\": \"player/rustamp.exe\""),
            "{both}"
        );
        assert!(
            both.contains("\"bundled_host\": \"player/rustamp-plughost32.exe\""),
            "{both}"
        );
        assert!(
            both.contains("\"binary\": \"host/rustamp-plughost32.exe\""),
            "{both}"
        );
        assert!(both.contains("\"schema_version\": 3"), "{both}");
        assert!(both.contains("\"product_version\": \"0.0.1\""), "{both}");
        assert!(
            both.contains(&format!("\"protocol_version\": {PROTOCOL_VERSION}")),
            "{both}"
        );

        let player_only = build_top_manifest("release", true, false, false, &meta);
        assert!(
            player_only.contains("\"bundled_host\": null"),
            "{player_only}"
        );
        assert!(!player_only.contains("\"host\""), "{player_only}");

        let host_only = build_top_manifest("release", false, true, false, &meta);
        assert!(!host_only.contains("rustamp.exe"), "{host_only}");
        assert!(
            host_only.contains("\"binary\": \"host/rustamp-plughost32.exe\""),
            "{host_only}"
        );
        assert!(!host_only.contains("bundled_host"), "{host_only}");
    }

    #[test]
    fn top_manifest_records_the_residents() {
        let meta = ReleaseMetadata::for_testing();
        let manifest = build_top_manifest("release", true, true, true, &meta);
        assert!(
            manifest.contains("\"decoder_host\": \"player/rustamp-dechost-symphonia.exe\""),
            "{manifest}"
        );
        assert!(
            manifest.contains("\"dsp_host\": \"player/rustamp-dsphost-stages.exe\""),
            "{manifest}"
        );
    }

    #[test]
    fn layout_audit_rejects_missing_residents() {
        let base = std::env::temp_dir().join(format!(
            "rustamp-package-layout-missing-res-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = rustamp_testkit::workspace_root();
        let meta = ReleaseMetadata::for_testing();
        let out = base.join("test");
        fs::create_dir_all(out.join("player")).unwrap();
        let guide = quickstart("release", true, true, &meta);
        fs::write(out.join("QUICKSTART.md"), &guide).unwrap();
        fs::write(out.join("README.md"), readme("release", true, true, &meta)).unwrap();
        if root.join("THIRD_PARTY.md").is_file() {
            copy_file(&root.join("THIRD_PARTY.md"), &out.join("THIRD_PARTY.md")).unwrap();
        } else {
            fs::write(out.join("THIRD_PARTY.md"), b"notices").unwrap();
        }
        fs::write(out.join("player/QUICKSTART.md"), &guide).unwrap();
        fs::write(
            out.join("player/THIRD_PARTY.md"),
            player_third_party_notices(&root, &meta).unwrap(),
        )
        .unwrap();
        fs::write(
            out.join("manifest.json"),
            build_top_manifest("release", true, true, true, &meta),
        )
        .unwrap();
        fs::write(
            out.join("player/manifest.json"),
            serde_manifest_player("release", true, &meta),
        )
        .unwrap();
        for file in [
            "rustamp.exe",
            "rustamp-plughost32.exe",
            "LICENSE",
            "PLUGIN-EXCEPTION.md",
        ] {
            fs::write(out.join("player").join(file), b"stub").unwrap();
        }
        // Missing rustamp-dechost-symphonia.exe and rustamp-dsphost-stages.exe
        assert!(audit_package_layout(&out, "release", true, true, &meta).is_err());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn host_third_party_contains_no_copyleft_terms() {
        let root = rustamp_testkit::workspace_root();
        let meta = ReleaseMetadata::for_testing();
        let notices = host_third_party_notices(&root, &meta).expect("host notices");
        assert!(
            !contains_copyleft_license(&notices),
            "host notices must contain no copyleft license identifiers or texts"
        );
    }

    #[test]
    fn player_third_party_contains_source_offer_and_mpl_notice() {
        let root = rustamp_testkit::workspace_root();
        let meta = ReleaseMetadata::for_testing();
        let notices = player_third_party_notices(&root, &meta).expect("player notices");
        assert!(notices.contains("Source Code Offer (GNU GPL v3 §6)"));
        assert!(notices.contains("Static Microsoft MSVC C Runtime Terms"));
        assert!(notices.contains("Webamp Classic Skin Tables (MIT)"));
        assert!(notices.contains("Symphonia Audio Decoders and Demuxers (MPL-2.0)"));
        assert!(notices.contains("BEGIN CARGO LICENSE INVENTORY"));
    }

    #[test]
    fn player_third_party_carries_the_full_mpl_and_apache_texts() {
        let root = rustamp_testkit::workspace_root();
        let meta = ReleaseMetadata::for_testing();
        let notices = player_third_party_notices(&root, &meta).expect("player notices");
        // MPL-2.0: first line, a middle clause, the last exhibit.
        assert!(notices.contains("Mozilla Public License Version 2.0\n=========="));
        assert!(notices.contains("3.2. Distribution of Executable Form"));
        assert!(notices.contains("Exhibit B - \"Incompatible With Secondary Licenses\" Notice"));
        // Apache-2.0: the terms and conditions, not only the source header.
        assert!(notices.contains("TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION"));
        assert!(notices.contains("4. Redistribution."));
        assert!(notices.contains("END OF TERMS AND CONDITIONS"));
        // The host notices must stay free of the MPL text.
        let host = host_third_party_notices(&root, &meta).expect("host notices");
        assert!(!host.contains("Mozilla Public License Version 2.0"));
    }

    #[test]
    fn readme_no_longer_calls_shuffle_repeat_or_the_visualizer_placebos() {
        let meta = ReleaseMetadata::for_testing();
        let text = readme("release", true, true, &meta);
        assert!(!text.contains("contracted to zero"));
        assert!(!text.contains("not yet connected to the playback engine"));
        assert!(text.contains("`artifact_imports_verified: false`"));
        let verified = ReleaseMetadata {
            imports_verified: true,
            ..meta
        };
        assert!(
            readme("release", true, true, &verified).contains("`artifact_imports_verified: true`")
        );
    }

    #[test]
    fn sha256_and_crc32_match_test_vectors() {
        assert_eq!(
            Sha256::digest_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            Sha256::digest_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
    }

    #[test]
    fn zip_archive_creation_and_checksum_verification() {
        let base = std::env::temp_dir().join(format!(
            "rustamp-zip-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let src = base.join("content");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), b"file a").unwrap();
        fs::write(src.join("sub/b.txt"), b"file b in sub").unwrap();

        let zip_path = base.join("test.zip");
        let checksum_path = base.join("test.zip.sha256");
        let meta = ReleaseMetadata::for_testing();

        create_zip_archive(&zip_path, &src, "root-prefix").unwrap();
        assert!(zip_path.is_file());
        write_checksum_file(&zip_path, &checksum_path).unwrap();
        assert!(checksum_path.is_file());

        assert!(audit_zip_archive(&zip_path, &meta).is_ok());

        // Checksum mismatch triggers error
        fs::write(
            &checksum_path,
            b"0000000000000000000000000000000000000000000000000000000000000000 *test.zip\n",
        )
        .unwrap();
        assert!(audit_zip_archive(&zip_path, &meta).is_err());

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn arg_scopes_stay_mutually_exclusive_with_sane_defaults() {
        let parsed = parse_args(&[]).expect("empty args parse");
        assert!(parsed.release);
        assert!(parsed.zip);
        assert!(!parsed.player_only && !parsed.host_only);
        let player_only = parse_args(&["--player-only".to_string()]).expect("player-only parses");
        assert!(player_only.player_only);
        assert!(parse_args(&["--player-only".to_string(), "--host-only".to_string()]).is_err());
        let no_zip = parse_args(&["--no-zip".to_string()]).expect("no-zip parses");
        assert!(!no_zip.zip);
    }

    #[test]
    fn installer_option_needs_the_full_release_package() {
        let args = |list: &[&str]| -> Vec<String> { list.iter().map(|a| a.to_string()).collect() };
        assert!(
            parse_args(&args(&["--release", "--installer"]))
                .unwrap()
                .installer
        );
        for partial in ["--player-only", "--host-only", "--debug"] {
            assert!(parse_args(&args(&["--installer", partial])).is_err());
        }
    }

    /// The script lists its files by hand: every file `xtask package` stages
    /// beside the player must be installed and removed again.
    #[test]
    fn nsis_script_installs_and_removes_the_packaged_files() {
        let script = include_str!("../installer/rustamp.nsi");
        for file in [
            "rustamp.exe",
            "rustamp-dechost-symphonia.exe",
            "rustamp-dsphost-stages.exe",
            "rustamp-plughost32.exe",
            "LICENSE",
            "PLUGIN-EXCEPTION.md",
            "THIRD_PARTY.md",
            "QUICKSTART.md",
            "manifest.json",
        ] {
            assert!(
                script.contains(&format!("File \"${{PLAYER_DIR}}\\{file}\"")),
                "{file} is not installed"
            );
            assert!(
                script.contains(&format!("Delete \"$INSTDIR\\{file}\"")),
                "{file} is not removed"
            );
        }
        for file in ["LICENSE-MIT", "LICENSE-APACHE", "THIRD_PARTY.md"] {
            assert!(script.contains(&format!("File \"${{HOST_DIR}}\\{file}\"")));
            assert!(script.contains(&format!("Delete \"$INSTDIR\\licenses\\plughost\\{file}\"")));
        }
    }

    #[test]
    fn signing_follows_the_environment() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        assert_eq!(Signing::from_env(env(&[])).unwrap(), None);
        assert!(Signing::from_env(env(&[("SIGNTOOL_CERT_PASSWORD", "x")])).is_err());
        assert!(
            Signing::from_env(env(&[
                ("SIGNTOOL_CERT_FILE", "a.pfx"),
                ("SIGNTOOL_CERT_THUMBPRINT", "ab"),
            ]))
            .is_err()
        );

        let file = Signing::from_env(env(&[
            ("SIGNTOOL_CERT_FILE", "a.pfx"),
            ("SIGNTOOL_CERT_PASSWORD", "x"),
            ("SIGNTOOL_TIMESTAMP_URL", "http://ts"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(file.signtool, "signtool");
        assert_eq!(
            file.sign_args(Path::new("r.exe")),
            [
                "sign",
                "/fd",
                "SHA256",
                "/f",
                "a.pfx",
                "/p",
                "x",
                "/tr",
                "http://ts",
                "/td",
                "SHA256",
                "r.exe"
            ]
        );

        let store = Signing::from_env(env(&[("SIGNTOOL_CERT_THUMBPRINT", "ab")]))
            .unwrap()
            .unwrap();
        assert_eq!(
            store.sign_args(Path::new("r.exe")),
            ["sign", "/fd", "SHA256", "/sha1", "ab", "r.exe"]
        );
    }

    #[test]
    fn installer_audit_checks_the_image_and_the_checksum() {
        let dir =
            std::env::temp_dir().join(format!("rustamp-installer-audit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let installer = dir.join("setup.exe");
        fs::write(&installer, b"MZ fixture, not executable").unwrap();
        assert!(audit_installer(&installer).is_err(), "no checksum file");
        write_checksum_file(&installer, &sha256_path(&installer)).unwrap();
        assert!(audit_installer(&installer).is_ok());
        fs::write(&installer, b"MZ changed").unwrap();
        assert!(audit_installer(&installer).is_err(), "stale checksum");
        fs::write(&installer, b"not a PE").unwrap();
        write_checksum_file(&installer, &sha256_path(&installer)).unwrap();
        assert!(audit_installer(&installer).is_err(), "not a PE file");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn host_audit_still_rejects_any_gpl_side_binary() {
        let base = std::env::temp_dir().join(format!(
            "rustamp-package-audit-{}-{}",
            std::process::id(),
            "gpl-side"
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("create temp audit dir");
        fs::write(base.join(binary_name(HOST_BIN)), b"host").expect("write host stub");
        assert!(audit_host_dir(&base).is_ok());
        fs::write(base.join(binary_name(PLAYER_BIN)), b"player").expect("write player stub");
        assert!(audit_host_dir(&base).is_err());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn copyleft_probe_still_catches_gpl_without_flagging_permissive() {
        assert!(is_copyleft("GPL-3.0-only"));
        assert!(is_copyleft("Apache-2.0 AND GPL-2.0"));
        assert!(!is_copyleft("MIT OR Apache-2.0"));
        assert!(!is_copyleft("MIT"));
    }
}
