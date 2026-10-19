//! Where the clean-room general-purpose plugin fixture lives, and how the
//! acceptance tests find it.
//!
//! The fixture is a **separate cdylib artifact**
//! (`crates/rustamp-plughost/examples/gen_rustamp_fixture.rs`), built for
//! `i686-pc-windows-msvc` and staged next to the test binaries by
//! `docs/notes/vm-run-tests.ps1`. It is never linked into a test binary: the
//! tests load it through the OS loader, which is the whole point of the
//! exercise.
//!
//! Shared by `tests/gen_acceptance.rs` (in-process) and
//! `tests/control_load_acceptance.rs` (through the production host process).

use std::path::PathBuf;

/// The file name of the fixture DLL.
pub const FIXTURE_FILE_NAME: &str = "gen_rustamp_fixture.dll";

/// Find the fixture DLL, or refuse loudly.
///
/// The search starts at the test executable's own directory, because the
/// runner stages the fixture beside the test binaries. A missing fixture must
/// never read as a pass or a quiet skip.
pub fn find_fixture_dll() -> PathBuf {
    let mut checked = Vec::new();

    // 1. Explicit override by the runner.
    if let Some(value) = std::env::var_os("RUSTAMP_GEN_FIXTURE") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return path;
        }
        checked.push(path);
    }

    // 2. Next to the test binary, and in the usual cargo output layout.
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let mut candidates = vec![dir.join(FIXTURE_FILE_NAME)];
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join(FIXTURE_FILE_NAME));
            if let Some(root) = parent.parent() {
                candidates.push(
                    root.join("i686-pc-windows-msvc")
                        .join("debug")
                        .join("examples")
                        .join(FIXTURE_FILE_NAME),
                );
            }
        }
        for candidate in candidates {
            if candidate.is_file() {
                return candidate;
            }
            checked.push(candidate);
        }
    }

    // 3. The runner's staged tree inside the guest.
    for candidate in [
        PathBuf::from(r"C:\rustamp\code\Rustamp\target\i686-pc-windows-msvc\debug\examples")
            .join(FIXTURE_FILE_NAME),
        PathBuf::from(r"C:\rustamp\code\Rustamp\target\debug\deps").join(FIXTURE_FILE_NAME),
    ] {
        if candidate.is_file() {
            return candidate;
        }
        checked.push(candidate);
    }

    panic!(
        "REFUSING LOUDLY: missing the general-purpose fixture {FIXTURE_FILE_NAME}. Checked {checked:?}. \
         Build it with `cargo build -p rustamp-plughost --example gen_rustamp_fixture --target \
         i686-pc-windows-msvc` and stage it beside the test binaries. This test refuses rather \
         than passing vacuously."
    );
}
