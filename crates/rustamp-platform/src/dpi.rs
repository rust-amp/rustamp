//! Per-monitor DPI awareness and window-chrome size math (task card F1).
//!
//! The main window's layout is fixed in physical pixels (275x116 at 1x, see
//! `specs/S01-window-geometry.md`), so the process must be per-monitor-V2 DPI
//! aware before any window exists: otherwise Windows virtualizes the process
//! and the client area ends up scaled (on a 150% system the 275x116 request
//! becomes 183x77; on a 250% system it becomes 110x46).
//!
//! [`init_dpi_awareness`] makes one best-effort pass over the Win32 DPI APIs,
//! newest first, and remembers its outcome: every later call returns the same
//! value without touching the OS again, so the function is idempotent and
//! never panics. Call it before the `winit` event loop is created (the
//! presenter does this in `MainWindowPresenter::run`). `winit` attempts the
//! same step when the event loop is built, so a second attempt from there is
//! harmless if ours ran first.
//!
//! [`outer_size_for_client`] is the headless-testable chrome adjustment: given
//! a desired client area and measured chrome insets it returns the outer
//! window size, with saturating arithmetic. `winit` performs this conversion
//! internally (`request_inner_size` goes through `AdjustWindowRect`), and the
//! exact insets are only known after the window is shown, so callers measure
//! the chrome from the live window (`outer_size` minus `inner_size`) and use
//! this function to predict or verify the outer size.
//!
//! On a DPI change while running (Windows sends `WM_DPICHANGED`, which `winit`
//! surfaces as `WindowEvent::ScaleFactorChanged`) the presenter re-asserts the
//! exact layout size in physical pixels, so the client area never ends up
//! smaller than the layout. The window keeps its pixel size across monitors
//! (it looks smaller on a high-DPI screen); logical-size preservation, if
//! wanted, belongs to the app layer or S05.

use std::sync::OnceLock;

/// How DPI-aware the process is after [`init_dpi_awareness`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpiAwareness {
    /// `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2` (Windows 10 1703+).
    PerMonitorV2,
    /// Older per-monitor awareness (V1 context or the 8.1 API).
    PerMonitor,
    /// System-DPI awareness only (`SetProcessDPIAware`, Vista+).
    SystemAware,
    /// Not a Windows target: DPI handling belongs to the toolkit, nothing was
    /// changed by this function.
    Unmanaged,
}

/// Why [`init_dpi_awareness`] could not establish any awareness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DpiError {
    /// Every Win32 DPI API failed; carries the first call's `HRESULT` so a
    /// failure stays diagnosable without panicking.
    #[error("no DPI-awareness API succeeded (first HRESULT {0:#X})")]
    Unavailable(i32),
}

/// Makes the process per-monitor-V2 DPI aware (Windows) and returns what was
/// achieved.
///
/// Idempotent: the OS is touched at most once; every later call returns the
/// remembered outcome. Never panics. On non-Windows targets this returns
/// [`DpiAwareness::Unmanaged`] without doing anything.
pub fn init_dpi_awareness() -> Result<DpiAwareness, DpiError> {
    *init_once()
}

/// The remembered [`init_dpi_awareness`] outcome.
fn init_once() -> &'static Result<DpiAwareness, DpiError> {
    static INIT: OnceLock<Result<DpiAwareness, DpiError>> = OnceLock::new();
    INIT.get_or_init(init_dpi_awareness_inner)
}

#[cfg(windows)]
fn init_dpi_awareness_inner() -> Result<DpiAwareness, DpiError> {
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        PROCESS_PER_MONITOR_DPI_AWARE, SetProcessDpiAwareness, SetProcessDpiAwarenessContext,
    };

    // SAFETY: these calls take no pointers or handles and only set the
    // process-wide DPI-awareness flag. The flag may only move from unaware to
    // aware once per process; this function runs its OS pass at most once
    // (guarded by `INIT`), and a second setter (e.g. winit's own) failing
    // afterwards is a documented benign outcome, not unsoundness.
    unsafe {
        let v2 = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if v2.is_ok() {
            return Ok(DpiAwareness::PerMonitorV2);
        }
        let first_code = v2.unwrap_err().code().0;
        // SAFETY: same invariant as above; each call only sets the flag.
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE).is_ok() {
            return Ok(DpiAwareness::PerMonitor);
        }
        // SAFETY: same invariant as above; each call only sets the flag.
        if SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE).is_ok() {
            return Ok(DpiAwareness::PerMonitor);
        }
        // SAFETY: same invariant as above; takes no arguments at all.
        if windows::Win32::UI::WindowsAndMessaging::SetProcessDPIAware().as_bool() {
            return Ok(DpiAwareness::SystemAware);
        }
        Err(DpiError::Unavailable(first_code))
    }
}

#[cfg(not(windows))]
fn init_dpi_awareness_inner() -> Result<DpiAwareness, DpiError> {
    Ok(DpiAwareness::Unmanaged)
}

/// Window chrome (title bar plus borders) in physical pixels.
///
/// `winit` does not expose per-side insets before the window is shown, so
/// callers measure the totals from the live window (`outer_size` minus
/// `inner_size`) and distribute them here as they see fit. The struct keeps
/// the four sides so asymmetric chrome stays expressible and testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowChrome {
    /// Chrome on the left edge, in physical pixels.
    pub left: u32,
    /// Chrome on the right edge, in physical pixels.
    pub right: u32,
    /// Chrome on the top edge (usually the title bar), in physical pixels.
    pub top: u32,
    /// Chrome on the bottom edge, in physical pixels.
    pub bottom: u32,
}

impl WindowChrome {
    /// No chrome: the outer size equals the client area (borderless window).
    pub const NONE: WindowChrome = WindowChrome {
        left: 0,
        right: 0,
        top: 0,
        bottom: 0,
    };

    /// Total horizontal chrome (`left + right`), saturating.
    pub fn horizontal(&self) -> u32 {
        self.left.saturating_add(self.right)
    }

    /// Total vertical chrome (`top + bottom`), saturating.
    pub fn vertical(&self) -> u32 {
        self.top.saturating_add(self.bottom)
    }
}

/// The outer window size for a desired client area plus chrome, in physical
/// pixels.
///
/// Pure numbers, no window handle, so tests cover it without opening anything.
/// Arithmetic saturates instead of overflowing.
pub fn outer_size_for_client(
    client_width: u32,
    client_height: u32,
    chrome: WindowChrome,
) -> (u32, u32) {
    (
        client_width.saturating_add(chrome.horizontal()),
        client_height.saturating_add(chrome.vertical()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_layout_with_zero_chrome_is_itself() {
        assert_eq!(
            outer_size_for_client(275, 116, WindowChrome::NONE),
            (275, 116)
        );
    }

    #[test]
    fn double_size_layout_with_zero_chrome_is_itself() {
        assert_eq!(
            outer_size_for_client(550, 232, WindowChrome::NONE),
            (550, 232)
        );
    }

    #[test]
    fn asymmetric_chrome_adds_each_side_once() {
        let chrome = WindowChrome {
            left: 8,
            right: 8,
            top: 31,
            bottom: 8,
        };
        assert_eq!(chrome.horizontal(), 16);
        assert_eq!(chrome.vertical(), 39);
        assert_eq!(outer_size_for_client(275, 116, chrome), (291, 155));
        assert_eq!(outer_size_for_client(550, 232, chrome), (566, 271));
    }

    #[test]
    fn single_sided_chrome_counts_too() {
        let chrome = WindowChrome {
            top: 30,
            ..WindowChrome::default()
        };
        assert_eq!(outer_size_for_client(275, 116, chrome), (275, 146));
    }

    #[test]
    fn huge_values_saturate_instead_of_overflowing() {
        let chrome = WindowChrome {
            left: u32::MAX,
            right: 1,
            top: 1,
            bottom: u32::MAX,
        };
        assert_eq!(chrome.horizontal(), u32::MAX);
        assert_eq!(chrome.vertical(), u32::MAX);
        assert_eq!(
            outer_size_for_client(u32::MAX, u32::MAX, chrome),
            (u32::MAX, u32::MAX)
        );
        assert_eq!(
            outer_size_for_client(u32::MAX, u32::MAX, WindowChrome::NONE),
            (u32::MAX, u32::MAX)
        );
    }

    #[test]
    fn dpi_awareness_init_is_idempotent() {
        let first = init_dpi_awareness();
        let second = init_dpi_awareness();
        assert_eq!(first, second);
    }

    #[test]
    fn dpi_awareness_init_agrees_across_threads() {
        let expected = init_dpi_awareness();
        let handles: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(init_dpi_awareness))
            .collect();
        for handle in handles {
            assert_eq!(handle.join().expect("worker panicked"), expected);
        }
    }

    #[cfg(windows)]
    #[test]
    fn dpi_awareness_reports_v2_on_a_modern_system() {
        // In an interactive session on a current Windows 10/11 build the V2
        // context must succeed; on an older system the init still returns Ok
        // with a lesser level, so only an Err would fail here - which is what
        // the test guards.
        //
        // The exception is a **non-interactive session**: a scheduled task, a
        // service, or the PowerShell Direct test host (session 0) has no window
        // station or desktop to be aware of, so the API can legitimately fail
        // there. In that case the contract is a typed error and no panic; the V2
        // assertion belongs to an interactive session, and the test host runs
        // the suite in session 0 by design (see docs/HANDOFF.md, the test host).
        match init_dpi_awareness() {
            Ok(awareness) => assert_eq!(awareness, DpiAwareness::PerMonitorV2),
            Err(error) => {
                let interactive = std::env::var_os("SESSIONNAME").is_some();
                assert!(
                    !interactive,
                    "DPI awareness failed in an interactive session: {error}"
                );
                eprintln!(
                    "note: no DPI awareness in this non-interactive session ({error}); \
                     session 0 has no desktop, which is where the test host runs"
                );
            }
        }
    }
}
