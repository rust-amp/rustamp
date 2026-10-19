// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The native file-open dialog (wave122).
//!
//! The playlist's ADD button and the main window's open-media actions ask the
//! user for files. They used to open a window of our own that wanted a typed
//! path or a drop, because `rustamp-ui` is `forbid(unsafe_code)` and has no
//! Win32 binding. That is not what a Windows player does: it opens the
//! **standard file-open dialog**, with multiple selection allowed, and adds
//! what the user chose.
//!
//! This module is that dialog, in the layer that already owns the Win32
//! bindings (the tray icon, the global hotkeys and the DPI calls live here
//! too). It takes the owner window's raw handle exactly as
//! [`TrayIcon::attach`](crate::tray::TrayIcon::attach) does, because the
//! handle only exists once `rustamp-app` has built the OS window.
//!
//! # The headless seam
//!
//! The dialog itself needs a desktop. What does not is the part that is easy
//! to get wrong: `GetOpenFileNameW` reports a multiple selection as a
//! directory followed by bare file names, all NUL-separated in one buffer,
//! and as a single full path when one file was picked.
//! [`parse_selection`] is that decoding, as a pure function over the buffer,
//! and it is unit-tested below on every platform.

use std::path::PathBuf;

/// Why a native file dialog could not be shown.
#[derive(Debug, thiserror::Error)]
pub enum FileDialogError {
    /// The dialog needs the owner window, and the handle was null.
    #[error("a native file dialog needs the owner window's handle")]
    NullOwner,
    /// The common-dialog call failed (not a cancellation).
    ///
    /// Carries `CommDlgExtendedError`; a cancelled dialog is not an error and
    /// returns an empty selection instead.
    #[error("the common file dialog failed (CommDlgExtendedError {0:#06x})")]
    Failed(u32),
    /// There is no native file dialog on this platform yet.
    #[error("a native file dialog is only implemented on Windows")]
    Unsupported,
}

/// The file types the open dialog offers, in order, as (label, patterns).
///
/// Own choice, stated plainly: no spec covers the dialog's filter list (S04
/// is not started for the open-media dialogs). The audio patterns are the
/// container extensions the player is expected to take; `All files` is last
/// so nothing the user has is unreachable. Playlist files are deliberately
/// **not** a filter here: this dialog adds entries, and loading a playlist is
/// the playlist editor's LIST OPTS path (RFC 0010).
pub const MEDIA_FILTERS: &[(&str, &str)] = &[
    (
        "Audio files",
        "*.mp3;*.ogg;*.oga;*.opus;*.flac;*.wav;*.m4a;*.aac;*.wma;*.mp2;*.mpc;*.ape",
    ),
    ("All files", "*.*"),
];

/// Decodes the buffer `GetOpenFileNameW` filled into absolute paths.
///
/// Two shapes, both NUL-separated and terminated by an empty entry:
///
/// * one file: a single full path (`C:\Music\a.mp3`);
/// * several files (`OFN_ALLOWMULTISELECT` with `OFN_EXPLORER`): the
///   directory first, then one bare file name per file
///   (`C:\Music`, `a.mp3`, `b.mp3`).
///
/// The one-entry case is therefore returned as it stands, and the
/// several-entry case is joined onto the directory. Empty entries end the
/// list - the buffer is longer than the data and the rest is zeros. A path
/// that is already absolute is kept even in the several-entry case, because
/// `Path::join` does that anyway and some shells report full names.
///
/// Returns an empty vector for an empty buffer, which is also what a
/// cancelled dialog leaves behind.
pub fn parse_selection(buffer: &[u16]) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    for chunk in buffer.split(|unit| *unit == 0) {
        if chunk.is_empty() {
            break;
        }
        entries.push(String::from_utf16_lossy(chunk));
    }
    match entries.split_first() {
        None => Vec::new(),
        Some((single, [])) => vec![PathBuf::from(single)],
        Some((directory, names)) => {
            let directory = PathBuf::from(directory);
            names.iter().map(|name| directory.join(name)).collect()
        }
    }
}

/// Builds the NUL-separated, double-NUL-terminated filter string
/// `OPENFILENAMEW::lpstrFilter` expects.
fn filter_string(filters: &[(&str, &str)]) -> Vec<u16> {
    let mut encoded = Vec::new();
    for (label, patterns) in filters {
        encoded.extend(label.encode_utf16());
        encoded.push(0);
        encoded.extend(patterns.encode_utf16());
        encoded.push(0);
    }
    encoded.push(0);
    encoded
}

/// How many UTF-16 units the selection buffer holds.
///
/// A multiple selection is one buffer for every path, so it has to be large:
/// 32768 units is 64 KiB, which holds hundreds of files and is the size the
/// common-dialog documentation suggests for multi-select.
const SELECTION_BUFFER_UNITS: usize = 32 * 1024;

/// Shows the standard file-open dialog and returns what the user chose.
///
/// `raw_owner` is the owner window's handle (the main window, as
/// `rustamp-app` holds it); the dialog is modal to it. `multiple` allows a
/// multiple selection. A cancelled dialog is **not** an error: it returns an
/// empty vector, so the caller adds nothing.
///
/// Call on the UI thread: the dialog runs its own modal message loop, exactly
/// as the tray's popup menu does.
pub fn pick_media_files(
    raw_owner: isize,
    title: &str,
    multiple: bool,
) -> Result<Vec<PathBuf>, FileDialogError> {
    pick_files(raw_owner, title, MEDIA_FILTERS, multiple)
}

/// The file types the equalizer-preset open dialog offers.
///
/// Own choice: the `.eqf` library first, `All files` last so a renamed
/// library stays reachable.
pub const EQF_FILTERS: &[(&str, &str)] = &[("EQ library files", "*.eqf"), ("All files", "*.*")];

/// Shows the file-open dialog for one equalizer preset library (`*.eqf`).
///
/// Returns `None` when the user cancelled. Same contract as
/// [`pick_media_files`]: call on the UI thread.
pub fn pick_eqf_file(raw_owner: isize, title: &str) -> Result<Option<PathBuf>, FileDialogError> {
    Ok(pick_files(raw_owner, title, EQF_FILTERS, false)?
        .into_iter()
        .next())
}

#[cfg(windows)]
fn pick_files(
    raw_owner: isize,
    title: &str,
    filters: &[(&str, &str)],
    multiple: bool,
) -> Result<Vec<PathBuf>, FileDialogError> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, GetOpenFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER,
        OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use windows::core::{PCWSTR, PWSTR};

    if raw_owner == 0 {
        return Err(FileDialogError::NullOwner);
    }
    let mut filter = filter_string(filters);
    let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
    let mut selection = vec![0u16; SELECTION_BUFFER_UNITS];
    // `OFN_NOCHANGEDIR` keeps the dialog from moving the process's current
    // directory under the player; `OFN_EXPLORER` is what makes a multiple
    // selection arrive in the directory-then-names shape `parse_selection`
    // decodes.
    let mut flags =
        OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
    if multiple {
        flags |= OFN_ALLOWMULTISELECT;
    }
    let mut options = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        // SAFETY of the cast: the caller passes the live main window's
        // handle, the same contract `TrayIcon::attach` documents.
        hwndOwner: HWND(raw_owner as *mut core::ffi::c_void),
        lpstrFilter: PCWSTR(filter.as_mut_ptr()),
        lpstrFile: PWSTR(selection.as_mut_ptr()),
        nMaxFile: SELECTION_BUFFER_UNITS as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        Flags: flags,
        ..Default::default()
    };
    // SAFETY: every pointer in `options` is to a live local buffer that
    // outlives this call, `nMaxFile` is the real length of `selection`, and
    // the filter and title are NUL-terminated. The call is synchronous: it
    // returns before the buffers are dropped.
    let chosen = unsafe { GetOpenFileNameW(&mut options) }.as_bool();
    if !chosen {
        // SAFETY: read immediately after the call, before any other OS call.
        // Zero means the user cancelled, which is not a failure.
        let error = unsafe { CommDlgExtendedError() };
        if error.0 == 0 {
            return Ok(Vec::new());
        }
        return Err(FileDialogError::Failed(error.0));
    }
    Ok(parse_selection(&selection))
}

/// Stub: there is no native file dialog off Windows yet (milestone M6).
#[cfg(not(windows))]
fn pick_files(
    _raw_owner: isize,
    _title: &str,
    _filters: &[(&str, &str)],
    _multiple: bool,
) -> Result<Vec<PathBuf>, FileDialogError> {
    Err(FileDialogError::Unsupported)
}

/// The file types the save-playlist dialog offers, in order, as (label, patterns).
///
/// Own choice: M3U, M3U8, PLS, and XSPF are the formats `rustamp_playlist::write_playlist`
/// supports. The default is M3U (`.m3u` extension).
pub const PLAYLIST_SAVE_FILTERS: &[(&str, &str)] = &[
    ("M3U Playlist", "*.m3u"),
    ("M3U8 Playlist (UTF-8)", "*.m3u8"),
    ("PLS Playlist", "*.pls"),
    ("XSPF Playlist", "*.xspf"),
];

/// Shows the standard file-save dialog and returns the path the user chose.
///
/// `raw_owner` is the owner window's handle; the dialog is modal to it.
/// `default_name` is the initial filename suggestion. A cancelled dialog is
/// **not** an error: it returns `None`.
///
/// Call on the UI thread: the dialog runs its own modal message loop.
#[cfg(windows)]
pub fn save_playlist_file(
    raw_owner: isize,
    title: &str,
    default_name: &str,
) -> Result<Option<PathBuf>, FileDialogError> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, GetSaveFileNameW, OFN_HIDEREADONLY, OFN_NOCHANGEDIR,
        OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use windows::core::{PCWSTR, PWSTR};

    if raw_owner == 0 {
        return Err(FileDialogError::NullOwner);
    }
    let mut filter = filter_string(PLAYLIST_SAVE_FILTERS);
    let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
    let mut selection: Vec<u16> = default_name
        .encode_utf16()
        .chain([0])
        .chain(std::iter::repeat_n(
            0,
            SELECTION_BUFFER_UNITS - default_name.len() - 1,
        ))
        .collect();
    let flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
    let mut options = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: HWND(raw_owner as *mut core::ffi::c_void),
        lpstrFilter: PCWSTR(filter.as_mut_ptr()),
        lpstrFile: PWSTR(selection.as_mut_ptr()),
        nMaxFile: SELECTION_BUFFER_UNITS as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        nFilterIndex: 1, // Default to M3U
        lpstrDefExt: PCWSTR(['m' as u16, '3' as u16, 'u' as u16, 0].as_ptr()),
        Flags: flags,
        ..Default::default()
    };
    // SAFETY: every pointer in `options` is to a live local buffer that
    // outlives this call, `nMaxFile` is the real length of `selection`, and
    // the filter and title are NUL-terminated. The call is synchronous.
    let chosen = unsafe { GetSaveFileNameW(&mut options) }.as_bool();
    if !chosen {
        // SAFETY: read immediately after the call, before any other OS call.
        // Zero means the user cancelled, which is not a failure.
        let error = unsafe { CommDlgExtendedError() };
        if error.0 == 0 {
            return Ok(None);
        }
        return Err(FileDialogError::Failed(error.0));
    }
    // Parse the single selected path (save dialog never returns multiple)
    let paths = parse_selection(&selection);
    Ok(paths.first().cloned())
}

/// Stub: there is no native save dialog off Windows yet (milestone M6).
#[cfg(not(windows))]
pub fn save_playlist_file(
    _raw_owner: isize,
    _title: &str,
    _default_name: &str,
) -> Result<Option<PathBuf>, FileDialogError> {
    Err(FileDialogError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A buffer of UTF-16 units from NUL-separated parts, padded with zeros
    /// the way the OS leaves the rest of the buffer.
    fn buffer(parts: &[&str]) -> Vec<u16> {
        let mut units = Vec::new();
        for part in parts {
            units.extend(part.encode_utf16());
            units.push(0);
        }
        units.resize(units.len() + 16, 0);
        units
    }

    #[test]
    fn one_picked_file_is_the_whole_path() {
        assert_eq!(
            parse_selection(&buffer(&["C:\\Music\\a.mp3"])),
            vec![PathBuf::from("C:\\Music\\a.mp3")]
        );
    }

    /// The shape that makes multiple selection worth a test: the directory is
    /// the first entry and the files are bare names after it.
    #[test]
    fn several_picked_files_are_joined_onto_the_directory() {
        assert_eq!(
            parse_selection(&buffer(&["C:\\Music", "a.mp3", "b with spaces.mp3"])),
            vec![
                PathBuf::from("C:\\Music\\a.mp3"),
                PathBuf::from("C:\\Music\\b with spaces.mp3"),
            ]
        );
    }

    #[test]
    fn a_cancelled_or_empty_dialog_selects_nothing() {
        assert!(parse_selection(&[]).is_empty());
        assert!(parse_selection(&[0, 0, 0]).is_empty());
    }

    #[test]
    fn the_filter_string_is_nul_separated_and_double_nul_terminated() {
        let encoded = filter_string(&[("Audio files", "*.mp3"), ("All files", "*.*")]);
        let text = String::from_utf16_lossy(&encoded);
        assert_eq!(text, "Audio files\0*.mp3\0All files\0*.*\0\0");
        assert_eq!(encoded.last(), Some(&0));
    }

    /// The offered filters must stay usable: a label and a pattern each, and
    /// `All files` last so nothing is unreachable.
    #[test]
    fn the_media_filters_end_with_all_files() {
        assert!(
            MEDIA_FILTERS
                .iter()
                .all(|(label, patterns)| !label.is_empty() && patterns.contains('*'))
        );
        assert_eq!(MEDIA_FILTERS.last().map(|(_, p)| *p), Some("*.*"));
    }
}
