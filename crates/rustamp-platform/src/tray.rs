// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The tray icon: a menu model that maps onto existing [`Command`]s, plus the
//! Win32 icon attach (task card F3).
//!
//! The player shows one tray icon while it runs. Its menu offers the expected
//! actions: show/hide the main window, play, pause/unpause, stop, next,
//! previous and exit. Every entry maps onto an **existing** [`Command`]
//! variant — no new command was invented for the tray.
//!
//! # What the spec settles and what is our choice
//!
//! * Settled (S03): the `WM_COMMAND` ids behind the transport and window
//!   actions — Play 40045, Pause/unpause 40046, Stop 40047, Next 40048,
//!   Previous 40044 — and through them the [`Command`] variants. S04 (menu
//!   trees) is not started, so no spec fixes any tray menu.
//! * Own choice: the menu's membership, order and labels ([`TRAY_MENU`],
//!   [`TrayMenuEntry::label`]); the single show/hide entry mapping onto
//!   [`Command::ToggleWindow`] of the main window (S03 records no
//!   toggle-main-window row — `Alt+W` 40258 is a one-way *show* — so this
//!   mapping is ours, stated plainly); the tray tooltip text; the callback
//!   message id.
//!
//! # Headless seam
//!
//! [`TrayMenuEntry::commands`] is pure and unit-tested without any OS object.
//! The Win32 half ([`TrayIcon::attach`]) needs the main window's `HWND`, which
//! only exists once `window.rs` has built the OS window. The app attaches the
//! icon there, polls its menu requests, and submits the selected entry's
//! commands. The native callback only queues requests: it never re-enters the
//! app. Constructing the menu model is headless; showing the native popup is
//! desktop-only (task card F5).

use rustamp_core::command::Command;
use rustamp_core::state::WindowId;

/// One entry of the tray menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrayMenuEntry {
    /// Show the main window when hidden, hide it when shown.
    ToggleMainWindow,
    /// Start playback ([`Command::Play`]).
    Play,
    /// Pause when playing, resume when paused ([`Command::TogglePause`]).
    PauseUnpause,
    /// Stop playback ([`Command::Stop`]).
    Stop,
    /// Next track ([`Command::Next`]).
    NextTrack,
    /// Previous track ([`Command::Prev`]).
    PreviousTrack,
    /// Quit the player ([`Command::Quit`]).
    ExitPlayer,
}

/// The tray menu in display order (own choice).
///
/// Separators are presenter detail (the popup is built in `window.rs`), so the
/// model carries only the commands.
pub const TRAY_MENU: &[TrayMenuEntry] = &[
    TrayMenuEntry::ToggleMainWindow,
    TrayMenuEntry::Play,
    TrayMenuEntry::PauseUnpause,
    TrayMenuEntry::Stop,
    TrayMenuEntry::NextTrack,
    TrayMenuEntry::PreviousTrack,
    TrayMenuEntry::ExitPlayer,
];

impl TrayMenuEntry {
    /// The menu label (own choice; S04 is not started).
    pub fn label(self) -> &'static str {
        match self {
            TrayMenuEntry::ToggleMainWindow => "Show/Hide main window",
            TrayMenuEntry::Play => "Play",
            TrayMenuEntry::PauseUnpause => "Pause/Unpause",
            TrayMenuEntry::Stop => "Stop",
            TrayMenuEntry::NextTrack => "Next track",
            TrayMenuEntry::PreviousTrack => "Previous track",
            TrayMenuEntry::ExitPlayer => "Exit",
        }
    }

    /// The public `WM_COMMAND` id (S03) behind the entry, if S03 records one
    /// for it.
    ///
    /// The show/hide toggle has none: S03's only main-window row here is the
    /// one-way show (`Alt+W`, 40258), and quitting has no accelerator row at
    /// all — both mappings stay [`Command`]s without an id, rather than
    /// borrowing an id that means something else.
    pub fn command_id(self) -> Option<u32> {
        match self {
            TrayMenuEntry::ToggleMainWindow => None,
            TrayMenuEntry::Play => Some(40045),
            TrayMenuEntry::PauseUnpause => Some(40046),
            TrayMenuEntry::Stop => Some(40047),
            TrayMenuEntry::NextTrack => Some(40048),
            TrayMenuEntry::PreviousTrack => Some(40044),
            TrayMenuEntry::ExitPlayer => None,
        }
    }

    /// The commands selecting this entry applies, in order.
    ///
    /// Every variant already exists: the tray invents no command.
    pub fn commands(self) -> Vec<Command> {
        vec![match self {
            TrayMenuEntry::ToggleMainWindow => Command::ToggleWindow(WindowId::Main),
            TrayMenuEntry::Play => Command::Play,
            TrayMenuEntry::PauseUnpause => Command::TogglePause,
            TrayMenuEntry::Stop => Command::Stop,
            TrayMenuEntry::NextTrack => Command::Next,
            TrayMenuEntry::PreviousTrack => Command::Prev,
            TrayMenuEntry::ExitPlayer => Command::Quit,
        }]
    }
}

/// Native popup ids are one-based positions, not borrowed S03 command ids.
/// Zero means cancellation; an unknown id must never dispatch a command.
#[cfg(any(windows, test))]
fn menu_entry_for_id(entries: &[TrayMenuEntry], id: u32) -> Option<TrayMenuEntry> {
    entries.get(id.checked_sub(1)? as usize).copied()
}

#[cfg(windows)]
mod imp {
    //! Win32 attach: one `Shell_NotifyIconW` icon owned by the main window.
    //!
    //! The icon needs an `HWND` for its callback messages, which only exists
    //! once `window.rs` has built the OS window, not from `main.rs`. A subclass
    //! queues callback requests without touching App or winit. The UI pump
    //! opens the popup and routes the pick. Desktop execution is VM-only.

    use std::sync::Mutex;

    use windows::Win32::Foundation::{
        ERROR_SUCCESS, GetLastError, HWND, LPARAM, LRESULT, POINT, SetLastError, WPARAM,
    };
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Shell::{
        DefSubclassProc, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
        RemoveWindowSubclass, SetWindowSubclass, Shell_NotifyIconW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetWindowThreadProcessId, HMENU,
        IDI_APPLICATION, LoadIconW, MF_STRING, PostMessageW, SetForegroundWindow, TPM_NONOTIFY,
        TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_CONTEXTMENU, WM_NCDESTROY, WM_NULL,
        WM_RBUTTONUP,
    };
    use windows::core::PCWSTR;

    use super::{TRAY_CALLBACK_MESSAGE, TRAY_TOOLTIP, TrayMenuEntry, menu_entry_for_id};

    /// The tray icon's id within its owner window (own choice; one icon).
    const TRAY_UID: u32 = 1;
    /// Unique within our subclass procedure, not a public command id.
    const SUBCLASS_ID: usize = 1;

    /// Coalesced menu requests; no Rust pointer is stored in the window.
    static MENU_REQUESTS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

    // SAFETY: Windows calls this only while our subclass is installed. All
    // parameters are OS values; no app/window Rust object is dereferenced.
    unsafe extern "system" fn tray_callback(
        owner: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if message == TRAY_CALLBACK_MESSAGE && wparam.0 == TRAY_UID as usize {
            // We retain the original (not NOTIFYICON_VERSION_4) callback format.
            if matches!(lparam.0 as u32, WM_RBUTTONUP | WM_CONTEXTMENU) {
                let mut pending = MENU_REQUESTS
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let raw = owner.0 as isize;
                if !pending.contains(&raw) {
                    pending.push(raw);
                }
            }
            return LRESULT(0);
        }
        if message == WM_NCDESTROY {
            clear_menu_request(owner);
            // SAFETY: called on the owner thread, removing only our subclass.
            let _ = unsafe { RemoveWindowSubclass(owner, Some(tray_callback), SUBCLASS_ID) };
        }
        // SAFETY: forward every unrelated message along the existing chain.
        unsafe { DefSubclassProc(owner, message, wparam, lparam) }
    }

    fn clear_menu_request(owner: HWND) -> bool {
        let mut pending = MENU_REQUESTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = pending.len();
        pending.retain(|raw| *raw != owner.0 as isize);
        pending.len() != before
    }

    /// Why the tray icon could not be attached.
    #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
    pub enum TrayError {
        /// The owner handle is null.
        #[error("the tray icon needs the main window's handle, got null")]
        NullOwner,
        /// The fallback application icon could not be loaded.
        #[error("could not load the tray icon")]
        IconLoad,
        /// `Shell_NotifyIconW(NIM_ADD)` refused the icon.
        #[error("the shell refused the tray icon (NIM_ADD failed)")]
        AddRefused,
        /// Attaching a subclass requires the live window's owning thread.
        #[error("the tray icon must be attached on the main window's owning thread")]
        OwnerThread,
        /// The callback could not be installed; no inert icon is left behind.
        #[error("could not install the tray callback")]
        CallbackRefused,
        /// Native popup creation, population or tracking failed.
        #[error("could not show the tray menu: {0}")]
        Menu(String),
    }

    /// The running player's tray icon.
    ///
    /// Dropping it removes the icon (`NIM_DELETE`), so the icon can never
    /// outlive the window that owns it.
    #[derive(Debug)]
    pub struct TrayIcon {
        owner: HWND,
        added: bool,
    }

    impl TrayIcon {
        /// Adds the tray icon to the window `raw_owner` (the main window's
        /// raw handle, as `window.rs` holds it).
        ///
        /// Uses the stock application icon: the player's own artwork is the
        /// skin lane's (J), not this crate's, and no icon file is invented
        /// here.
        pub fn attach(raw_owner: isize) -> Result<Self, TrayError> {
            if raw_owner == 0 {
                return Err(TrayError::NullOwner);
            }
            // SAFETY: `raw_owner` is non-null by the check above; the caller
            // (`window.rs`) guarantees it is the live main window, which
            // outlives the returned guard — `Drop` removes the icon first.
            let owner = HWND(raw_owner as *mut core::ffi::c_void);
            // SAFETY: both APIs accept plain OS handles/ids; an invalid HWND
            // yields thread id zero, which cannot equal the current thread.
            if unsafe { GetWindowThreadProcessId(owner, None) != GetCurrentThreadId() } {
                return Err(TrayError::OwnerThread);
            }
            // SAFETY: `None` loads a stock icon; `IDI_APPLICATION` always
            // names one. No window is created.
            let icon =
                unsafe { LoadIconW(None, IDI_APPLICATION) }.map_err(|_| TrayError::IconLoad)?;
            let mut tip = [0u16; 128];
            for (slot, unit) in tip.iter_mut().zip(TRAY_TOOLTIP.encode_utf16().chain([0])) {
                *slot = unit;
                if unit == 0 {
                    break;
                }
            }
            let data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: owner,
                uID: TRAY_UID,
                uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                uCallbackMessage: super::TRAY_CALLBACK_MESSAGE,
                hIcon: icon,
                szTip: tip,
                ..NOTIFYICONDATAW::default()
            };
            // SAFETY: the owner is on this thread; the procedure stores no
            // pointers, and Drop removes it before the app drops the window.
            if !unsafe { SetWindowSubclass(owner, Some(tray_callback), SUBCLASS_ID, 0) }.as_bool() {
                return Err(TrayError::CallbackRefused);
            }
            let mut guard = Self {
                owner,
                added: false,
            };
            // SAFETY: `data` is a fully initialised `NOTIFYICONDATAW` that
            // outlives the call; `cbSize` names its own size. The shell copies
            // what it needs.
            let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) };
            if !added.as_bool() {
                return Err(TrayError::AddRefused);
            }
            guard.added = true;
            Ok(guard)
        }

        /// Takes one coalesced right-click/context-menu request on the UI
        /// thread. This never opens a popup or calls application code.
        pub fn take_menu_request(&self) -> bool {
            clear_menu_request(self.owner)
        }

        /// Shows the native popup from the supplied model data and returns its
        /// selection. Call on the owner/UI thread; never from the callback.
        /// Cancellation returns `Ok(None)`. No command is applied here.
        pub fn show_menu(
            &self,
            entries: &[TrayMenuEntry],
        ) -> Result<Option<TrayMenuEntry>, TrayError> {
            // SAFETY: creates an owned popup menu, released on every path.
            let menu = PopupMenu(unsafe { CreatePopupMenu() }.map_err(menu_error)?);
            for (index, entry) in entries.iter().enumerate() {
                let label: Vec<u16> = entry.label().encode_utf16().chain([0]).collect();
                // SAFETY: menu is live, ids are nonzero positions and the
                // NUL-terminated label outlives the call (Windows copies it).
                unsafe { AppendMenuW(menu.0, MF_STRING, index + 1, PCWSTR(label.as_ptr())) }
                    .map_err(menu_error)?;
            }
            let mut position = POINT::default();
            // SAFETY: the writable point outlives this call.
            unsafe { GetCursorPos(&mut position) }.map_err(menu_error)?;
            // SAFETY: owner remains live on the UI thread. Foreground is
            // best-effort, notably when the main window is hidden.
            let _ = unsafe { SetForegroundWindow(self.owner) };
            // SAFETY: clear the error so cancellation can be distinguished
            // from a failed TrackPopupMenu call.
            unsafe { SetLastError(ERROR_SUCCESS) };
            // SAFETY: live menu and owner, screen coordinates; RETURNCMD and
            // NONOTIFY keep WM_COMMAND out of winit/the app during tracking.
            let selected = unsafe {
                TrackPopupMenu(
                    menu.0,
                    TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                    position.x,
                    position.y,
                    None,
                    self.owner,
                    None,
                )
            }
            .0 as u32;
            // SAFETY: read immediately after tracking, before another OS call.
            let error = unsafe { GetLastError() };
            // SAFETY: the no-op posted after tracking lets Windows finish
            // dismissing a notification-area popup; failure is non-fatal.
            let _ = unsafe { PostMessageW(Some(self.owner), WM_NULL, WPARAM(0), LPARAM(0)) };
            if selected == 0 && error != ERROR_SUCCESS {
                return Err(TrayError::Menu(format!(
                    "TrackPopupMenu failed ({})",
                    error.0
                )));
            }
            Ok(menu_entry_for_id(entries, selected))
        }
    }

    fn menu_error(error: windows::core::Error) -> TrayError {
        TrayError::Menu(error.to_string())
    }

    struct PopupMenu(HMENU);

    impl Drop for PopupMenu {
        fn drop(&mut self) {
            // SAFETY: exclusively owns the menu returned by CreatePopupMenu.
            let _ = unsafe { DestroyMenu(self.0) };
        }
    }

    impl Drop for TrayIcon {
        fn drop(&mut self) {
            // SAFETY: the guard stays on the UI thread and is dropped before
            // the owner's window. No Rust pointers are left if removal fails.
            let _ = unsafe { RemoveWindowSubclass(self.owner, Some(tray_callback), SUBCLASS_ID) };
            clear_menu_request(self.owner);
            if !self.added {
                return;
            }
            let data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.owner,
                uID: TRAY_UID,
                ..NOTIFYICONDATAW::default()
            };
            // SAFETY: same invariant as `attach`; the owner window still
            // exists because the guard never outlives it. A refused delete is
            // not reported: there is nothing left to clean through.
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
            self.added = false;
        }
    }
}

/// The tray tooltip (own choice).
pub const TRAY_TOOLTIP: &str = "Rustamp";

/// The tray icon's callback message: `WM_APP + 1` (own choice).
///
/// The `window.rs` follow-up pops the menu built from [`TRAY_MENU`] on this
/// message and applies the pick through [`TrayMenuEntry::commands`].
pub const TRAY_CALLBACK_MESSAGE: u32 = 32768 + 1;

#[cfg(windows)]
pub use imp::{TrayError, TrayIcon};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_popup_ids_select_the_model_entry_and_its_exact_command() {
        let expected = [
            Command::ToggleWindow(WindowId::Main),
            Command::Play,
            Command::TogglePause,
            Command::Stop,
            Command::Next,
            Command::Prev,
            Command::Quit,
        ];
        for (index, command) in expected.into_iter().enumerate() {
            let entry = menu_entry_for_id(TRAY_MENU, index as u32 + 1).unwrap();
            assert_eq!(entry, TRAY_MENU[index]);
            assert_eq!(entry.commands(), vec![command]);
        }
        for id in [0, TRAY_MENU.len() as u32 + 1, u32::MAX] {
            assert_eq!(menu_entry_for_id(TRAY_MENU, id), None);
        }
        assert_eq!(menu_entry_for_id(&[], 1), None);
    }

    #[test]
    fn the_menu_has_exactly_the_seven_expected_entries_in_order() {
        assert_eq!(
            TRAY_MENU,
            &[
                TrayMenuEntry::ToggleMainWindow,
                TrayMenuEntry::Play,
                TrayMenuEntry::PauseUnpause,
                TrayMenuEntry::Stop,
                TrayMenuEntry::NextTrack,
                TrayMenuEntry::PreviousTrack,
                TrayMenuEntry::ExitPlayer,
            ]
        );
    }

    #[test]
    fn every_entry_maps_onto_an_existing_command() {
        for entry in TRAY_MENU {
            assert_eq!(entry.commands().len(), 1, "{entry:?}");
        }
        assert_eq!(
            TrayMenuEntry::Play.commands(),
            vec![Command::Play],
            "play borrows no new variant"
        );
        assert_eq!(
            TrayMenuEntry::PauseUnpause.commands(),
            vec![Command::TogglePause],
            "pause/unpause is the S03 C-key toggle, not a new command"
        );
        assert_eq!(TrayMenuEntry::Stop.commands(), vec![Command::Stop]);
        assert_eq!(TrayMenuEntry::NextTrack.commands(), vec![Command::Next]);
        assert_eq!(TrayMenuEntry::PreviousTrack.commands(), vec![Command::Prev]);
        assert_eq!(
            TrayMenuEntry::ToggleMainWindow.commands(),
            vec![Command::ToggleWindow(WindowId::Main)],
            "show/hide toggles the existing window command"
        );
        assert_eq!(TrayMenuEntry::ExitPlayer.commands(), vec![Command::Quit]);
    }

    #[test]
    fn the_recorded_ids_are_the_s03_transport_ids() {
        let ids: Vec<Option<u32>> = TRAY_MENU.iter().map(|e| e.command_id()).collect();
        assert_eq!(
            ids,
            [
                None,
                Some(40045),
                Some(40046),
                Some(40047),
                Some(40048),
                Some(40044),
                None,
            ]
        );
        let mut distinct: Vec<u32> = ids.into_iter().flatten().collect();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), 5, "no id is borrowed twice");
    }

    #[test]
    fn labels_are_non_empty_and_distinct() {
        let mut labels: Vec<&str> = TRAY_MENU.iter().map(|e| e.label()).collect();
        assert!(labels.iter().all(|l| !l.is_empty()));
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), TRAY_MENU.len());
    }
}
