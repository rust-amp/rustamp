// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The native file dialogs (wave122, wave154).
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
//!
//! The **directory** picker (wave154) is the same idea one step further. The
//! shell's folder dialog is a COM object, so its answer is not a buffer but
//! an `HRESULT` plus a shell item. [`directory_result`] is that decision - a
//! cancelled dialog is `Ok(None)`, a failed one is an error - and
//! [`DIRECTORY_DIALOG_OPTIONS`] is the set of option bits the dialog is
//! opened with. Both are plain data, so both are unit-tested without a
//! desktop.

use std::path::PathBuf;

/// The shell's folder dialog, used by the directory picker below.
///
/// The common file dialogs need no COM object; the folder dialog is one.
#[cfg(windows)]
use windows::Win32::UI::Shell::IFileOpenDialog;

/// Why a native file dialog could not be shown.
#[derive(Debug, thiserror::Error)]
pub enum FileDialogError {
    /// The dialog needs the owner window, and the handle was null.
    #[error("a native file dialog needs the owner window's handle")]
    NullOwner,
    /// The dialog call failed (not a cancellation).
    ///
    /// Carries the code the call reported: `CommDlgExtendedError` for the
    /// common file dialogs, the `HRESULT` for the shell's folder dialog. A
    /// cancelled dialog is not an error and returns an empty selection
    /// instead.
    #[error("the file dialog failed (code {0:#06x})")]
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

/// The file types the load-playlist dialog offers.
pub const PLAYLIST_OPEN_FILTERS: &[(&str, &str)] = &[
    ("Playlists", "*.m3u;*.m3u8;*.pls;*.xspf"),
    ("All files", "*.*"),
];

/// Shows the file-open dialog for one playlist file (S06 §4: LIST OPTS opens a
/// `#32770` dialog titled `Load Playlist`).
///
/// Returns `None` when the user cancelled. Same contract as
/// [`pick_media_files`]: call on the UI thread.
pub fn pick_playlist_file(
    raw_owner: isize,
    title: &str,
) -> Result<Option<PathBuf>, FileDialogError> {
    Ok(pick_files(raw_owner, title, PLAYLIST_OPEN_FILTERS, false)?
        .into_iter()
        .next())
}

/// The file types the Winamp configuration import dialog offers.
///
/// Own choice: the `.ini` files first, `All files` last so a file the user
/// named otherwise stays reachable.
pub const INI_FILTERS: &[(&str, &str)] = &[("Winamp configuration", "*.ini"), ("All files", "*.*")];

/// Shows the file-open dialog for one Winamp configuration file (`*.ini`).
///
/// The import reads the file the user chooses and never writes to it, so
/// this is the plain open dialog: there is nothing to confirm and nothing to
/// overwrite.
///
/// Returns `None` when the user cancelled. Same contract as
/// [`pick_media_files`]: call on the UI thread.
pub fn pick_winamp_ini_file(
    raw_owner: isize,
    title: &str,
) -> Result<Option<PathBuf>, FileDialogError> {
    Ok(pick_files(raw_owner, title, INI_FILTERS, false)?
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

/// The file types the HTML-playlist save dialog offers, in order, as
/// (label, patterns).
///
/// Own choice: the generated document is one HTML file, so `*.html` is what
/// the dialog offers. `All files` is last so a file the user names
/// differently stays reachable.
pub const HTML_SAVE_FILTERS: &[(&str, &str)] = &[("HTML playlist", "*.html"), ("All files", "*.*")];

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
    save_file(raw_owner, title, default_name, PLAYLIST_SAVE_FILTERS, "m3u")
}

/// Shows the standard file-save dialog for an HTML document (`*.html`).
///
/// The same dialog as [`save_playlist_file`] with the HTML filter and the
/// `html` default extension: a cancelled dialog is `Ok(None)`, and the
/// chosen path comes back as it is.
#[cfg(windows)]
pub fn save_html_playlist_file(
    raw_owner: isize,
    title: &str,
    default_name: &str,
) -> Result<Option<PathBuf>, FileDialogError> {
    save_file(raw_owner, title, default_name, HTML_SAVE_FILTERS, "html")
}

/// Stub: there is no native file dialog off Windows yet (milestone M6).
#[cfg(not(windows))]
pub fn save_playlist_file(
    _raw_owner: isize,
    _title: &str,
    _default_name: &str,
) -> Result<Option<PathBuf>, FileDialogError> {
    Err(FileDialogError::Unsupported)
}

/// Stub: there is no native file dialog off Windows yet (milestone M6).
#[cfg(not(windows))]
pub fn save_html_playlist_file(
    _raw_owner: isize,
    _title: &str,
    _default_name: &str,
) -> Result<Option<PathBuf>, FileDialogError> {
    Err(FileDialogError::Unsupported)
}

/// The save dialog itself, with the filter list and the default extension
/// the caller wants.
#[cfg(windows)]
fn save_file(
    raw_owner: isize,
    title: &str,
    default_name: &str,
    filters: &[(&str, &str)],
    default_extension: &str,
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
    let mut filter = filter_string(filters);
    let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
    let extension: Vec<u16> = default_extension.encode_utf16().chain([0]).collect();
    let name_units = default_name.encode_utf16().count();
    let mut selection: Vec<u16> = default_name
        .encode_utf16()
        .chain([0])
        .chain(std::iter::repeat_n(
            0,
            SELECTION_BUFFER_UNITS.saturating_sub(name_units + 1),
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
        nFilterIndex: 1, // The first filter is the default one.
        lpstrDefExt: PCWSTR(extension.as_ptr()),
        Flags: flags,
        ..Default::default()
    };
    // SAFETY: every pointer in `options` is to a live local buffer that
    // outlives this call, `nMaxFile` is the real length of `selection`, and
    // the filter, the title and the extension are NUL-terminated. The call is
    // synchronous.
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

// ---------------------------------------------------------------------------
// The directory picker
// ---------------------------------------------------------------------------

/// `FOS_PICKFOLDERS` (0x20): the dialog picks folders, not files.
const FOS_PICKFOLDERS: u32 = 0x0000_0020;
/// `FOS_FORCEFILESYSTEM` (0x40): the dialog stays in the file system, so
/// what comes back is a real directory and not a virtual shell folder.
const FOS_FORCEFILESYSTEM: u32 = 0x0000_0040;
/// `FOS_NOCHANGEDIR` (0x08): the dialog does not move the process's current
/// directory under the player.
const FOS_NOCHANGEDIR: u32 = 0x0000_0008;
/// `FOS_PATHMUSTEXIST` (0x800): the chosen path has to exist.
const FOS_PATHMUSTEXIST: u32 = 0x0000_0800;
/// `FOS_FILEMUSTEXIST` (0x1000): the chosen item has to exist.
const FOS_FILEMUSTEXIST: u32 = 0x0000_1000;
/// `FOS_ALLOWMULTISELECT` (0x200): the dialog would let the user pick more
/// than one folder. The directory picker wants one, so the flag is taken
/// away again.
const FOS_ALLOWMULTISELECT: u32 = 0x0000_0200;

/// The option bits the directory dialog is opened with.
///
/// `FOS_PICKFOLDERS` is the one that matters: it turns the open dialog into
/// the shell's "select a folder" dialog. The others make the answer
/// usable - the folder has to exist, the process's current directory must
/// not move under the player, and the dialog stays in the file system, so
/// the result is a real path and not a virtual shell folder.
///
/// The bits are the SDK's own numbers, spelled out here so that the set can
/// be checked in a test without a desktop; the Windows-only test below pins
/// every one of them to the SDK constant it stands for.
pub const DIRECTORY_DIALOG_OPTIONS: u32 =
    FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR | FOS_PATHMUSTEXIST | FOS_FILEMUSTEXIST;

/// The option set the directory dialog is opened with.
///
/// `defaults` is what the shell's dialog reports for itself
/// (`IFileDialog::GetOptions`). The folder flags are added to it, and the
/// multiple-selection flag is taken away again: the caller asked for one
/// directory, and the shell would answer with an array if it were left on.
///
/// Pure, so the set can be checked in a test without a desktop.
pub fn directory_dialog_options(defaults: u32) -> u32 {
    (defaults | DIRECTORY_DIALOG_OPTIONS) & !FOS_ALLOWMULTISELECT
}

/// The `HRESULT` the shell reports when the user closes the dialog without
/// choosing anything (`HRESULT_FROM_WIN32(ERROR_CANCELLED)`).
pub const CANCELLED_HRESULT: u32 = 0x8007_04C7;

/// The bit that marks an `HRESULT` as a failure.
const HRESULT_FAILED: u32 = 0x8000_0000;

/// Decides what the directory dialog's answer means for the caller.
///
/// `hresult` is the value the dialog's `Show` call returned and `picked` is
/// the folder it produced, if it produced one.
///
/// The rule the caller depends on: **a cancelled dialog is `Ok(None)`, not
/// an error.** The user closed the dialog, which is not a failure of the
/// player - the same contract [`pick_media_files`] and
/// [`save_playlist_file`] have. A failed dialog carries its `HRESULT` into
/// [`FileDialogError::Failed`].
///
/// Pure, so the contract is unit-tested without a desktop.
pub fn directory_result(
    hresult: u32,
    picked: Option<PathBuf>,
) -> Result<Option<PathBuf>, FileDialogError> {
    if hresult == CANCELLED_HRESULT {
        return Ok(None);
    }
    if hresult & HRESULT_FAILED != 0 {
        return Err(FileDialogError::Failed(hresult));
    }
    Ok(picked)
}

/// Decodes the UTF-16 path the shell handed out into a path.
///
/// The buffer is NUL-terminated; the terminator is optional, so a slice
/// without one is read to its end. An empty path is not a directory, so the
/// answer is `None`.
///
/// Pure, so the decoding is unit-tested without a desktop.
pub fn parse_wide_path(units: &[u16]) -> Option<PathBuf> {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    let text = String::from_utf16_lossy(&units[..end]);
    if text.is_empty() {
        return None;
    }
    Some(PathBuf::from(text))
}

/// Shows the shell's folder dialog and returns the directory the user chose.
///
/// `raw_owner` is the owner window's handle (the main window, as
/// `rustamp-app` holds it); the dialog is modal to it. `title` is the
/// dialog's title; an empty title leaves the shell's own.
///
/// This is the **modern** folder dialog (`IFileOpenDialog` with
/// `FOS_PICKFOLDERS`), not the old `SHBrowseForFolder`, which is deprecated
/// and looks like 1995.
///
/// A cancelled dialog is **not** an error: it returns `None`, so the caller
/// adds nothing. Same contract as [`pick_media_files`].
///
/// Call on the UI thread: the dialog runs its own modal message loop.
#[cfg(windows)]
pub fn pick_directory(raw_owner: isize, title: &str) -> Result<Option<PathBuf>, FileDialogError> {
    use std::ffi::c_void;

    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Shell::{FILEOPENDIALOGOPTIONS, FileOpenDialog};
    use windows::core::{IUnknown, PCWSTR};

    if raw_owner == 0 {
        return Err(FileDialogError::NullOwner);
    }

    // The shell's dialog is a COM object, so the thread needs an apartment.
    let _apartment = ComApartment::enter();

    // SAFETY: the class id and the interface id are the shell's own, and no
    // outer object is asked for.
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None::<&IUnknown>, CLSCTX_INPROC_SERVER) }
            .map_err(|error| FileDialogError::Failed(error.code().0 as u32))?;

    if !title.is_empty() {
        let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
        // SAFETY: the title is NUL-terminated and outlives the call.
        unsafe { dialog.SetTitle(PCWSTR(title.as_ptr())) }
            .map_err(|error| FileDialogError::Failed(error.code().0 as u32))?;
    }

    // `SetOptions` replaces the whole set, so the shell's own defaults are
    // read first and the folder flags are added to them.
    let current = unsafe { dialog.GetOptions() }.unwrap_or_default();
    // SAFETY: the options are a plain bit set.
    unsafe { dialog.SetOptions(FILEOPENDIALOGOPTIONS(directory_dialog_options(current.0))) }
        .map_err(|error| FileDialogError::Failed(error.code().0 as u32))?;

    // SAFETY: the owner is the player's own main window, the same contract
    // `TrayIcon::attach` documents. The call is modal: it returns when the
    // dialog is closed.
    let hresult = match unsafe { dialog.Show(Some(HWND(raw_owner as *mut c_void))) } {
        Ok(()) => 0,
        Err(error) => error.code().0 as u32,
    };

    // A successful `Show` means the dialog was accepted, so its answer is
    // there to be read; a failed one means the user cancelled or the shell
    // gave up, and the seam below says which.
    let picked = if hresult & HRESULT_FAILED == 0 {
        match chosen_directory(&dialog) {
            Ok(path) => Some(path),
            Err(code) => return Err(FileDialogError::Failed(code)),
        }
    } else {
        None
    };
    directory_result(hresult, picked)
}

/// Stub: there is no native directory dialog off Windows yet (milestone M6).
#[cfg(not(windows))]
pub fn pick_directory(_raw_owner: isize, _title: &str) -> Result<Option<PathBuf>, FileDialogError> {
    Err(FileDialogError::Unsupported)
}

/// Reads the directory the user chose out of the dialog.
///
/// Returns the `HRESULT` of the call that failed, if one did.
#[cfg(windows)]
fn chosen_directory(dialog: &IFileOpenDialog) -> Result<PathBuf, u32> {
    use std::ffi::c_void;

    use windows::Win32::Foundation::E_FAIL;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::SIGDN_FILESYSPATH;

    // SAFETY: the dialog produced a result item, on the thread that showed
    // the dialog.
    let item = unsafe { dialog.GetResult() }.map_err(|error| error.code().0 as u32)?;
    // SAFETY: the item is a shell item; the name is read into a buffer the
    // shell allocates and that is freed below.
    let name =
        unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(|error| error.code().0 as u32)?;
    // SAFETY: the pointer the shell handed out is valid until it is freed.
    let units = unsafe { name.as_wide() };
    let path = parse_wide_path(units);
    // SAFETY: the shell allocated this buffer and asked for it to be freed
    // with `CoTaskMemFree`.
    unsafe { CoTaskMemFree(Some(name.0.cast_const().cast::<c_void>())) };
    // An empty name is not a directory, and the shell has no code for it.
    path.ok_or(E_FAIL.0 as u32)
}

/// The COM apartment the shell's dialog needs.
///
/// The folder dialog is a COM object, so the thread must have an apartment.
/// `CoInitializeEx` succeeds twice: `S_OK` when it created one and `S_FALSE`
/// when the thread already had one in the same model - both have to be
/// balanced with a `CoUninitialize`. When the thread is already in a
/// *different* model the call fails with `RPC_E_CHANGED_MODE`; the existing
/// apartment is then used as it is and nothing is balanced.
#[cfg(windows)]
struct ComApartment {
    balanced: bool,
}

#[cfg(windows)]
impl ComApartment {
    fn enter() -> Self {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

        // SAFETY: no apartment is passed and the concurrency model is a
        // constant; the call only sets up the calling thread.
        let hresult = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self {
            balanced: hresult.is_ok(),
        }
    }
}

#[cfg(windows)]
impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.balanced {
            use windows::Win32::System::Com::CoUninitialize;

            // SAFETY: the matching `CoInitializeEx` succeeded on this thread.
            unsafe { CoUninitialize() };
        }
    }
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
                PathBuf::from("C:\\Music").join("a.mp3"),
                PathBuf::from("C:\\Music").join("b with spaces.mp3"),
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

    /// The Winamp configuration dialog offers the `.ini` files, and keeps
    /// `All files` last so a file named otherwise stays reachable.
    #[test]
    fn the_ini_filters_offer_the_configuration_and_keep_all_files_last() {
        assert_eq!(INI_FILTERS[0], ("Winamp configuration", "*.ini"));
        assert!(
            INI_FILTERS
                .iter()
                .all(|(label, patterns)| !label.is_empty() && patterns.contains('*'))
        );
        assert_eq!(INI_FILTERS.last().map(|(_, p)| *p), Some("*.*"));
    }

    /// The HTML dialog offers the one file type the document is, and keeps
    /// `All files` last so a file named otherwise stays reachable.
    #[test]
    fn the_html_filters_offer_the_document_and_keep_all_files_last() {
        assert_eq!(HTML_SAVE_FILTERS[0], ("HTML playlist", "*.html"));
        assert!(
            HTML_SAVE_FILTERS
                .iter()
                .all(|(label, patterns)| !label.is_empty() && patterns.contains('*'))
        );
        assert_eq!(HTML_SAVE_FILTERS.last().map(|(_, p)| *p), Some("*.*"));
    }

    /// The playlist dialog keeps its own formats: the HTML filter is a
    /// sibling of it, not a replacement.
    #[test]
    fn the_playlist_filters_are_the_playlist_formats_only() {
        assert_eq!(PLAYLIST_SAVE_FILTERS.len(), 4);
        assert!(
            PLAYLIST_SAVE_FILTERS
                .iter()
                .all(|(_, patterns)| !patterns.contains("html"))
        );
    }

    /// The case the whole directory contract turns on: the user closed the
    /// dialog, which is not a failure of the player.
    #[test]
    fn a_cancelled_dialog_is_no_directory_and_not_an_error() {
        assert!(matches!(
            directory_result(CANCELLED_HRESULT, None),
            Ok(None)
        ));
    }

    /// A dialog that failed for another reason is an error, and it carries
    /// the code the shell reported.
    #[test]
    fn a_failed_dialog_carries_its_code() {
        match directory_result(0x8000_0001, None) {
            Err(FileDialogError::Failed(code)) => assert_eq!(code, 0x8000_0001),
            other => panic!("a failed dialog must be an error: {other:?}"),
        }
    }

    #[test]
    fn a_chosen_directory_is_the_path_the_shell_gave() {
        let path = PathBuf::from("C:\\Music");
        match directory_result(0, Some(path.clone())) {
            Ok(Some(picked)) => assert_eq!(picked, path),
            other => panic!("the chosen directory must come back: {other:?}"),
        }
    }

    /// The flag that makes it a folder dialog at all, plus the ones that
    /// make the answer usable.
    #[test]
    fn the_directory_dialog_picks_folders_that_exist() {
        assert_ne!(
            DIRECTORY_DIALOG_OPTIONS & FOS_PICKFOLDERS,
            0,
            "the dialog must pick folders"
        );
        assert_ne!(DIRECTORY_DIALOG_OPTIONS & FOS_PATHMUSTEXIST, 0);
        assert_ne!(DIRECTORY_DIALOG_OPTIONS & FOS_FILEMUSTEXIST, 0);
        assert_ne!(DIRECTORY_DIALOG_OPTIONS & FOS_NOCHANGEDIR, 0);
        assert_ne!(DIRECTORY_DIALOG_OPTIONS & FOS_FORCEFILESYSTEM, 0);
    }

    /// One directory at a time: the menu command adds one, so a multiple
    /// selection would only make the answer harder to use.
    #[test]
    fn the_directory_dialog_does_not_allow_a_multiple_selection() {
        assert_eq!(directory_dialog_options(0) & FOS_ALLOWMULTISELECT, 0);
        assert_eq!(
            directory_dialog_options(FOS_ALLOWMULTISELECT) & FOS_ALLOWMULTISELECT,
            0
        );
    }

    /// The shell's own defaults are kept; only the multiple-selection flag
    /// is taken away.
    #[test]
    fn the_directory_options_keep_the_shells_own_defaults() {
        const FOS_STRICTFILETYPES: u32 = 0x0000_0004;
        assert_eq!(directory_dialog_options(0), DIRECTORY_DIALOG_OPTIONS);
        assert_eq!(
            directory_dialog_options(FOS_STRICTFILETYPES) & FOS_STRICTFILETYPES,
            FOS_STRICTFILETYPES
        );
    }

    /// The bits above are the SDK's own numbers; this pins each of them to
    /// the SDK constant it stands for, so a wrong number cannot slip in.
    #[cfg(windows)]
    #[test]
    fn the_directory_options_are_the_sdk_constants() {
        use windows::Win32::UI::Shell::{
            FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_PATHMUSTEXIST,
            FOS_PICKFOLDERS,
        };
        assert_eq!(FOS_PICKFOLDERS.0, super::FOS_PICKFOLDERS);
        assert_eq!(FOS_FORCEFILESYSTEM.0, super::FOS_FORCEFILESYSTEM);
        assert_eq!(FOS_NOCHANGEDIR.0, super::FOS_NOCHANGEDIR);
        assert_eq!(FOS_PATHMUSTEXIST.0, super::FOS_PATHMUSTEXIST);
        assert_eq!(FOS_FILEMUSTEXIST.0, super::FOS_FILEMUSTEXIST);
    }

    /// The cancelled code is the shell's own, not a number of our own.
    #[cfg(windows)]
    #[test]
    fn the_cancelled_code_is_the_shells_own() {
        use windows::Win32::Foundation::ERROR_CANCELLED;
        use windows::core::HRESULT;
        assert_eq!(
            CANCELLED_HRESULT,
            HRESULT::from_win32(ERROR_CANCELLED.0).0 as u32
        );
    }

    #[test]
    fn a_wide_path_is_decoded_into_a_path() {
        let units: Vec<u16> = "C:\\Music".encode_utf16().chain([0]).collect();
        assert_eq!(parse_wide_path(&units), Some(PathBuf::from("C:\\Music")));
    }

    /// The shell's name may come without a terminator; the slice is then
    /// read to its end.
    #[test]
    fn a_wide_path_without_a_terminator_is_decoded_too() {
        let units: Vec<u16> = "C:\\Music".encode_utf16().collect();
        assert_eq!(parse_wide_path(&units), Some(PathBuf::from("C:\\Music")));
    }

    /// An empty path is not a directory.
    #[test]
    fn an_empty_wide_path_is_no_directory() {
        assert_eq!(parse_wide_path(&[]), None);
        assert_eq!(parse_wide_path(&[0]), None);
    }
}
