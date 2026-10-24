// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Giving the release binary a console to write to.
//!
//! The release build is linked as a **GUI** program (`windows_subsystem`), so
//! double-clicking `rustamp.exe` does not open a console window (owner report,
//! 2026-10-21). The player still writes its notes to stderr, and `--console`
//! makes them visible: the process either attaches to the terminal that started
//! it or opens a console of its own.
//!
//! This lives here, and not in the binary, because the application crate forbids
//! `unsafe`; the platform crate is where the Win32 calls belong.

/// Makes this process own a console so the player's notes are visible.
///
/// Attaching to the parent process's console is the common case (a launch from
/// a terminal); a new console is the fallback (a launch from Explorer with
/// `--console`). Both calls change nothing but this process's console
/// attachment, so there is no state to restore - closing the console ends it.
#[cfg(windows)]
pub fn ensure_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
        fn AllocConsole() -> i32;
    }

    /// `ATTACH_PARENT_PROCESS`: the console of the process that launched us.
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;

    // SAFETY: both functions only change this process's console attachment.
    // A failure is reported by the return value and is not an error here: a
    // process that already owns a console keeps it.
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            let _ = AllocConsole();
        }
    }
}

/// Nothing to do away from Windows: a console, if any, is already inherited.
#[cfg(not(windows))]
pub fn ensure_console() {}
