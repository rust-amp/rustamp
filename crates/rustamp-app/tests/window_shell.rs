// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The window frame, pinned without a desktop.
//!
//! The defect this file exists for: every window of the player opened with
//! the standard Windows caption and its minimise/maximise/close boxes,
//! because `winit`'s [`WindowAttributes`](winit::window::WindowAttributes)
//! defaults `decorations` to `true` and no creation site said otherwise. No
//! automated check caught it: the geometry tests measure `inner_size()`, the
//! **client** area, and a native frame does not change the client area — a
//! window can be exactly the right size and still be wrong to look at.
//!
//! What can be checked without a desktop is the *decision*: the attributes
//! every window is created from. Two things are pinned here:
//!
//! 1. every window of the player asks for a borderless frame, with the client
//!    size the specs measure (the per-window unit tests in
//!    `rustamp-app/src/window.rs` and `rustamp-ui/src/shell.rs`), and
//! 2. there is exactly **one** place in the two window-owning crates that
//!    builds window attributes from `winit`'s defaults — the shared shell
//!    (below). A new creation site that forgot the frame is how the defect
//!    would come back, and this is the check that fails when one appears.
//!
//! What cannot be checked here: that the window the OS actually shows has no
//! caption. That needs a window, so it needs a desktop; the VM probes
//! (`tests/eq_window.rs`, `--ignored`) assert the observable consequence —
//! `outer_size() == inner_size()`, which is only true without a frame.

use std::fs;
use std::path::{Path, PathBuf};

/// The workspace root, from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate lives at <root>/crates/rustamp-app")
        .to_path_buf()
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(rust_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
    found.sort();
    found
}

/// The one file allowed to build attributes from `winit`'s defaults.
const SHELL: &str = "shell.rs";

/// Builders that produce a fresh [`winit::window::WindowAttributes`], i.e. a
/// *decorated* one: `decorations` defaults to `true`.
const DEFAULT_BUILDERS: [&str; 2] = ["default_attributes()", "WindowAttributes::default()"];

#[test]
fn only_the_shared_shell_builds_window_attributes_from_winits_defaults() {
    let root = workspace_root();
    let mut offenders = Vec::new();
    for crate_name in ["rustamp-app", "rustamp-ui", "rustamp-platform"] {
        for dir in ["src", "examples"] {
            let searched = root.join("crates").join(crate_name).join(dir);
            for file in rust_files(&searched) {
                if file.file_name().is_some_and(|name| name == SHELL) {
                    continue;
                }
                let text = fs::read_to_string(&file).expect("read a source file");
                for (number, line) in text.lines().enumerate() {
                    // Doc comments may name the builder; only code counts.
                    if line.trim_start().starts_with("//") {
                        continue;
                    }
                    if DEFAULT_BUILDERS
                        .iter()
                        .any(|builder| line.contains(builder))
                    {
                        offenders.push(format!(
                            "{}:{}: {}",
                            file.strip_prefix(&root).unwrap_or(&file).display(),
                            number + 1,
                            line.trim()
                        ));
                    }
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a window built from winit's defaults is a *decorated* window, and the classic \
         windows are borderless (their title bar is part of the skin). Build it through \
         `rustamp_ui::shell::skinned_window_attributes` instead. Offending lines:\n{}",
        offenders.join("\n")
    );
}

/// The guard above is only worth something while the shell is actually there
/// and still says "no frame": a shell that lost `with_decorations(false)`
/// would make every site decorated again while the scan stayed green.
#[test]
fn the_shell_the_guard_points_at_still_refuses_the_native_frame() {
    let attributes = rustamp_ui::skinned_window_attributes(
        "Rustamp",
        (275, 116),
        rustamp_ui::shell::TaskbarRole::Application,
    );
    assert!(
        !attributes.decorations,
        "the shared shell must still refuse the native frame (SKINNED_DECORATIONS is {})",
        rustamp_ui::SKINNED_DECORATIONS
    );
    assert_eq!(
        rustamp_ui::attributes_client_size(&attributes),
        Some((275, 116)),
        "the client area the geometry tests measure must pass through untouched"
    );
}
