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
//! only exists once `window.rs` has built the OS window, so attaching the icon
//! — and popping the menu on the callback message — is a `window.rs`
//! follow-up the card records. Nothing here opens a window: constructing the
//! menu model is headless.

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

#[cfg(windows)]
mod imp {
    //! Win32 attach: one `Shell_NotifyIconW` icon owned by the main window.
    //!
    //! The icon needs an `HWND` for its callback messages, which only exists
    //! once `window.rs` has built the OS window — so this runs as a
    //! `window.rs` follow-up (see the card), not from `main.rs`. Compiled here,
    //! exercised in the VM: a tray icon needs a real desktop.

    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{
        NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{IDI_APPLICATION, LoadIconW};

    use super::TRAY_TOOLTIP;

    /// The tray icon's id within its owner window (own choice; one icon).
    const TRAY_UID: u32 = 1;

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
            // SAFETY: `data` is a fully initialised `NOTIFYICONDATAW` that
            // outlives the call; `cbSize` names its own size. The shell copies
            // what it needs.
            let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) };
            if !added.as_bool() {
                return Err(TrayError::AddRefused);
            }
            Ok(Self { owner, added: true })
        }
    }

    impl Drop for TrayIcon {
        fn drop(&mut self) {
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
