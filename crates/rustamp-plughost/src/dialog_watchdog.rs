//! Keeps a dialog raised by plug-in code from freezing the host.
//!
//! A plug-in's `DllMain` or `init` runs on the host's UI thread. If it (or the
//! C runtime it links) shows a **modal** dialog — the VC++ runtime does when
//! its side-by-side assembly is missing (`R6034`, "Microsoft Visual C++
//! Runtime Library") — the thread sits in the dialog's message loop until
//! somebody clicks, and the host answers nothing: not the load request, not a
//! ping. Nobody is there to click in a headless run, and a user should not have
//! to either.
//!
//! [`LoadWatchdog`] runs next to such a call. After a grace period it looks for
//! dialogs (`#32770`) owned by the UI thread, **records their title and text**,
//! and dismisses them (OK, then close), so the call returns and the host can
//! answer with a refusal that says what the plug-in complained about instead
//! of staying silent.
//!
//! The watchdog never touches a window of the host's own: it only looks at
//! standard dialog windows of the thread it was started on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long a load may run before the watchdog starts dismissing dialogs.
///
/// A plug-in that loads normally needs a fraction of a second; a dialog that
/// is still up after this is stuck. Own choice.
pub const DIALOG_GRACE: Duration = Duration::from_secs(3);

/// The button commands the watchdog posts to a dialog, in this order: Cancel,
/// No, OK, Ignore, Abort. **Never Yes or Retry**: "Yes" can start something
/// (MilkDrop's missing-DirectX box offers to open a download page), and a retry
/// would only raise the same dialog again. A dialog that offers none of these
/// is closed with its close box afterwards.
pub const DISMISS_COMMANDS: [usize; 5] = [2, 7, 1, 5, 3];

/// How often the watchdog looks, once the grace period is over.
const POLL: Duration = Duration::from_millis(150);

/// A watchdog for one plug-in call on the thread that created it.
pub struct LoadWatchdog {
    done: Arc<AtomicBool>,
    said: Arc<Mutex<Vec<String>>>,
    thread: Option<JoinHandle<()>>,
}

impl LoadWatchdog {
    /// Starts watching the **calling** thread, `after` the moment of the call.
    pub fn start(after: Duration) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let said = Arc::new(Mutex::new(Vec::new()));
        let ui_thread = current_thread_id();
        let thread = std::thread::Builder::new()
            .name("load-watchdog".to_string())
            .spawn({
                let done = Arc::clone(&done);
                let said = Arc::clone(&said);
                move || {
                    let started = Instant::now();
                    while !done.load(Ordering::Acquire) {
                        std::thread::sleep(POLL);
                        if started.elapsed() < after {
                            continue;
                        }
                        for line in dismiss_dialogs(ui_thread) {
                            if let Ok(mut said) = said.lock() {
                                said.push(line);
                            }
                        }
                    }
                }
            })
            .ok();
        Self { done, said, thread }
    }

    /// Stops watching and returns what the dismissed dialogs said, one
    /// `title: text` line each (empty when none appeared).
    pub fn finish(mut self) -> Vec<String> {
        self.stop();
        self.said
            .lock()
            .map(|said| said.clone())
            .unwrap_or_default()
    }

    fn stop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for LoadWatchdog {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One line for the refusal: what the dismissed dialogs said, or `None`.
pub fn describe(dialogs: &[String]) -> Option<String> {
    if dialogs.is_empty() {
        None
    } else {
        Some(format!(
            "the plug-in raised a dialog while loading, which the host dismissed: {}",
            dialogs.join(" | ")
        ))
    }
}

#[cfg(windows)]
fn current_thread_id() -> u32 {
    // SAFETY: no arguments, no preconditions.
    unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }
}

#[cfg(not(windows))]
fn current_thread_id() -> u32 {
    0
}

#[cfg(not(windows))]
fn dismiss_dialogs(_ui_thread: u32) -> Vec<String> {
    Vec::new()
}

/// Finds the standard dialogs of `ui_thread`, records them and dismisses them.
#[cfg(windows)]
fn dismiss_dialogs(ui_thread: u32) -> Vec<String> {
    use windows::Win32::Foundation::{HWND, LPARAM, TRUE, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, EnumThreadWindows, GetClassNameW, GetWindowTextW, IsWindowVisible,
        PostMessageW, WM_CLOSE, WM_COMMAND,
    };

    fn text_of(hwnd: HWND) -> String {
        let mut buffer = [0u16; 512];
        // SAFETY: a valid buffer of the length passed.
        let len = unsafe { GetWindowTextW(hwnd, &mut buffer) };
        String::from_utf16_lossy(&buffer[..usize::try_from(len).unwrap_or(0)])
    }

    fn class_of(hwnd: HWND) -> String {
        let mut buffer = [0u16; 64];
        // SAFETY: a valid buffer of the length passed.
        let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
        String::from_utf16_lossy(&buffer[..usize::try_from(len).unwrap_or(0)])
    }

    unsafe extern "system" fn collect_windows(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
        // SAFETY: `lparam` is the `Vec<HWND>` the caller passed and outlives
        // the enumeration.
        let found = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        found.push(hwnd);
        TRUE
    }

    let mut windows_of_thread: Vec<HWND> = Vec::new();
    // SAFETY: the callback only pushes into the vector behind `lparam`.
    let _ = unsafe {
        EnumThreadWindows(
            ui_thread,
            Some(collect_windows),
            LPARAM(&mut windows_of_thread as *mut Vec<HWND> as isize),
        )
    };

    let mut said = Vec::new();
    for hwnd in windows_of_thread {
        // SAFETY: a handle the enumeration just returned.
        let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
        if !visible || class_of(hwnd) != "#32770" {
            continue;
        }
        let mut children: Vec<HWND> = Vec::new();
        // SAFETY: as above, for the dialog's children.
        let _ = unsafe {
            EnumChildWindows(
                Some(hwnd),
                Some(collect_windows),
                LPARAM(&mut children as *mut Vec<HWND> as isize),
            )
        };
        let body: Vec<String> = children
            .into_iter()
            .filter(|child| class_of(*child) == "Static")
            .map(text_of)
            .filter(|text| !text.is_empty())
            .collect();
        said.push(format!("{:?}: {}", text_of(hwnd), body.join(" ")));
        // The buttons that decline or acknowledge, then the close box.
        // SAFETY: posting to a window of this process.
        unsafe {
            for command in DISMISS_COMMANDS {
                let _ = PostMessageW(Some(hwnd), WM_COMMAND, WPARAM(command), LPARAM(0));
            }
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_watchdog_that_saw_nothing_returns_quickly_and_empty() {
        let started = Instant::now();
        let watchdog = LoadWatchdog::start(Duration::from_secs(60));
        assert!(watchdog.finish().is_empty());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn the_watchdog_never_presses_yes_or_retry() {
        // IDYES = 6 and IDRETRY = 4.
        assert!(!DISMISS_COMMANDS.contains(&6));
        assert!(!DISMISS_COMMANDS.contains(&4));
        // Declining comes before acknowledging.
        assert_eq!(&DISMISS_COMMANDS[..2], &[2, 7]);
    }

    #[test]
    fn the_refusal_names_what_the_dialog_said() {
        assert_eq!(describe(&[]), None);
        let text = describe(&["\"Runtime\": R6034 An application".to_string()]).unwrap();
        assert!(text.contains("R6034"), "{text}");
        assert!(text.contains("dismissed"), "{text}");
    }
}
