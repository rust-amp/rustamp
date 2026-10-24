// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The operating system's own popup menu (wave122).
//!
//! The reference's menus **are** system menus: spec S04 §1 settles the window
//! class of every popup it measured as `#32768`, which is the window class
//! Windows itself uses for menus. A Winamp 2.95 menu is therefore the plain
//! light-grey system menu: the system font, separator lines between groups, a
//! small arrow on the right of every entry that opens a submenu, accelerator
//! text right-aligned on the same line, and tick marks on the toggles.
//!
//! The player used to draw the main menu itself, into a window of its own
//! with a bitmap font. That produced the two defects the owner saw: a popup
//! with a standard window caption (it was an ordinary top-level window), and
//! text that could not be read (the `text.bmp` glyph sheet at the wrong
//! metrics for a 35-px menu row). Both disappear by letting the OS draw the
//! menu, which is also the faithful answer.
//!
//! This module is the thin Win32 half: a [`PopupRow`] tree in, the chosen
//! id out. It is the same shape as the tray's popup (which already used
//! `TrackPopupMenu`), generalised to carry separators, submenus,
//! accelerators and check marks.
//!
//! # The headless seam
//!
//! Showing a menu needs a desktop. What does not is the row tree and the item
//! text: [`item_text`] is the `label\taccelerator` form that makes Windows
//! right-align an accelerator, and [`PopupRow`] is an ordinary data structure
//! the caller can assert over. `rustamp-ui` builds the player's menu as such
//! a tree (`rustamp_ui::menu::native_main_menu`) and its tests check the
//! separators, the arrows, the accelerators and the ticks without opening
//! anything.

/// One row of a native popup menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupRow {
    /// A separator line between two groups.
    Separator,
    /// A selectable entry. `id` is what [`show_popup_menu`] returns when the
    /// user picks it, and must be non-zero (zero is "nothing was picked").
    Entry {
        /// The id returned when this entry is chosen.
        id: u32,
        /// The entry's text.
        label: String,
        /// Accelerator text, right-aligned by the OS on the same line.
        accelerator: Option<String>,
        /// Whether the entry shows a tick mark.
        checked: bool,
        /// Whether the entry can be chosen (a greyed entry cannot).
        enabled: bool,
    },
    /// A parent entry: the OS draws the submenu arrow and opens `rows`.
    Submenu {
        /// The parent entry's text.
        label: String,
        /// The submenu's own rows.
        rows: Vec<PopupRow>,
    },
}

impl PopupRow {
    /// A plain entry with no accelerator, no tick and enabled.
    pub fn entry(id: u32, label: impl Into<String>) -> Self {
        PopupRow::Entry {
            id,
            label: label.into(),
            accelerator: None,
            checked: false,
            enabled: true,
        }
    }
}

/// Why a native popup menu could not be shown.
#[derive(Debug, thiserror::Error)]
pub enum PopupMenuError {
    /// A popup menu is owned by a window, and the handle was null.
    #[error("a popup menu needs the owner window's handle")]
    NullOwner,
    /// An entry carried id zero, which `TrackPopupMenu` cannot report.
    #[error("a popup menu entry must have a non-zero id")]
    ZeroId,
    /// A Win32 call refused.
    #[error("popup menu: {0}")]
    Win32(String),
    /// There is no native popup menu on this platform yet.
    #[error("a native popup menu is only implemented on Windows")]
    Unsupported,
}

/// The text one menu entry carries, in the form Windows expects.
///
/// A tab separates the label from its accelerator, and the OS right-aligns
/// everything after the tab in the menu's accelerator column - which is what
/// the reference screenshot shows (`Alt+3`, `Alt+W`, `F1`, `Alt+F4`). Without
/// the tab the accelerator would run on after the label.
pub fn item_text(label: &str, accelerator: Option<&str>) -> String {
    match accelerator {
        Some(accelerator) if !accelerator.is_empty() => format!("{label}\t{accelerator}"),
        _ => label.to_string(),
    }
}

/// Shows a native popup menu at screen position `at` and returns the id of
/// the chosen entry, or `None` when the user dismissed it.
///
/// `raw_owner` is the owner window's handle, exactly as
/// [`TrayIcon::attach`](crate::tray::TrayIcon::attach) takes it. Call on the
/// UI thread and outside a window procedure: the menu runs its own modal
/// message loop.
#[cfg(windows)]
pub fn show_popup_menu(
    raw_owner: isize,
    rows: &[PopupRow],
    at: (i32, i32),
) -> Result<Option<u32>, PopupMenuError> {
    use windows::Win32::Foundation::{ERROR_SUCCESS, GetLastError, HWND, SetLastError};
    use windows::Win32::UI::WindowsAndMessaging::{
        SetForegroundWindow, TPM_LEFTALIGN, TPM_LEFTBUTTON, TPM_NONOTIFY, TPM_RETURNCMD,
        TPM_TOPALIGN, TrackPopupMenu,
    };

    if raw_owner == 0 {
        return Err(PopupMenuError::NullOwner);
    }
    // SAFETY of the cast: the caller passes the live main window's handle,
    // the same contract the tray attach documents.
    let owner = HWND(raw_owner as *mut core::ffi::c_void);
    let menu = imp::build_menu(rows)?;
    // Foreground is best-effort: a popup of a background window would
    // otherwise not take the dismissal click.
    // SAFETY: the owner is live on this thread.
    let _ = unsafe { SetForegroundWindow(owner) };
    // SAFETY: cleared so a dismissal can be told apart from a failure.
    unsafe { SetLastError(ERROR_SUCCESS) };
    // SAFETY: a live menu and owner and screen coordinates. RETURNCMD gives
    // the id back instead of posting `WM_COMMAND`, and NONOTIFY keeps the
    // tracking messages out of `winit` and the app.
    let chosen = unsafe {
        TrackPopupMenu(
            menu.0,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTBUTTON | TPM_LEFTALIGN | TPM_TOPALIGN,
            at.0,
            at.1,
            None,
            owner,
            None,
        )
    }
    .0 as u32;
    // SAFETY: read immediately after tracking, before another OS call.
    let error = unsafe { GetLastError() };
    if chosen == 0 {
        if error != ERROR_SUCCESS {
            return Err(PopupMenuError::Win32(format!(
                "TrackPopupMenu failed ({})",
                error.0
            )));
        }
        return Ok(None); // Dismissed, which is not a failure.
    }
    Ok(Some(chosen))
}

/// Stub: there is no native popup menu off Windows yet (milestone M6).
#[cfg(not(windows))]
pub fn show_popup_menu(
    _raw_owner: isize,
    _rows: &[PopupRow],
    _at: (i32, i32),
) -> Result<Option<u32>, PopupMenuError> {
    Err(PopupMenuError::Unsupported)
}

#[cfg(windows)]
mod imp {
    //! The Win32 menu build: one `HMENU` per level, owned by its parent.

    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DestroyMenu, HMENU, MF_CHECKED, MF_GRAYED, MF_POPUP,
        MF_SEPARATOR, MF_STRING,
    };
    use windows::core::PCWSTR;

    use super::{PopupMenuError, PopupRow, item_text};

    /// An owned popup menu: `Drop` destroys it, and destroying a menu
    /// destroys the submenus appended into it, so only the root is guarded.
    pub(super) struct OwnedMenu(pub(super) HMENU);

    impl Drop for OwnedMenu {
        fn drop(&mut self) {
            // SAFETY: exclusively owns the menu from `CreatePopupMenu`; any
            // submenu appended with `MF_POPUP` belongs to it and goes too.
            let _ = unsafe { DestroyMenu(self.0) };
        }
    }

    fn win32(error: windows::core::Error) -> PopupMenuError {
        PopupMenuError::Win32(error.to_string())
    }

    /// Builds the menu tree. The returned root owns every submenu below it.
    pub(super) fn build_menu(rows: &[PopupRow]) -> Result<OwnedMenu, PopupMenuError> {
        // SAFETY: creates an owned popup menu; the guard releases it on
        // every path, including the error paths below.
        let menu = OwnedMenu(unsafe { CreatePopupMenu() }.map_err(win32)?);
        for row in rows {
            match row {
                PopupRow::Separator => {
                    // SAFETY: the menu is live; a separator takes no text.
                    unsafe { AppendMenuW(menu.0, MF_SEPARATOR, 0, PCWSTR::null()) }
                        .map_err(win32)?;
                }
                PopupRow::Entry {
                    id,
                    label,
                    accelerator,
                    checked,
                    enabled,
                } => {
                    if *id == 0 {
                        return Err(PopupMenuError::ZeroId);
                    }
                    let text: Vec<u16> = item_text(label, accelerator.as_deref())
                        .encode_utf16()
                        .chain([0])
                        .collect();
                    let mut flags = MF_STRING;
                    if *checked {
                        flags |= MF_CHECKED;
                    }
                    if !*enabled {
                        flags |= MF_GRAYED;
                    }
                    // SAFETY: the menu is live, the id is non-zero by the
                    // check above, and the NUL-terminated text outlives the
                    // call (Windows copies it).
                    unsafe { AppendMenuW(menu.0, flags, *id as usize, PCWSTR(text.as_ptr())) }
                        .map_err(win32)?;
                }
                PopupRow::Submenu { label, rows } => {
                    let child = build_menu(rows)?;
                    let text: Vec<u16> = label.encode_utf16().chain([0]).collect();
                    // SAFETY: both menus are live and the text outlives the
                    // call. `MF_POPUP` takes the child's handle as the id and
                    // transfers its ownership to `menu`, so the child's guard
                    // must not destroy it afterwards - hence the `forget`.
                    unsafe {
                        AppendMenuW(menu.0, MF_POPUP, child.0.0 as usize, PCWSTR(text.as_ptr()))
                    }
                    .map_err(win32)?;
                    std::mem::forget(child);
                }
            }
        }
        Ok(menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one piece of the native menu that is ours to get right: an
    /// accelerator is right-aligned only when a tab separates it.
    #[test]
    fn an_accelerator_is_separated_from_its_label_by_a_tab() {
        assert_eq!(
            item_text("View file info", Some("Alt+3")),
            "View file info\tAlt+3"
        );
        assert_eq!(item_text("Exit", Some("Alt+F4")), "Exit\tAlt+F4");
        assert_eq!(item_text("Play", None), "Play");
        // An empty accelerator is no accelerator: a bare tab would leave an
        // empty right column.
        assert_eq!(item_text("Play", Some("")), "Play");
    }

    #[test]
    fn a_plain_entry_has_no_accelerator_no_tick_and_is_enabled() {
        assert_eq!(
            PopupRow::entry(7, "Skins"),
            PopupRow::Entry {
                id: 7,
                label: "Skins".to_string(),
                accelerator: None,
                checked: false,
                enabled: true,
            }
        );
    }
}
