// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The native dialogs of the playlist extraction (spec S20 §4).
//!
//! The extraction of the media library asks the user for two values before
//! it runs: the **path template** the file names are rendered from and the
//! **directory** the files are written into. Both are asked in one small
//! native window of our own: the player's own dialogs carry the skin of the
//! main window and are not a form, and the system has no dialog for "two
//! fields and two buttons".
//!
//! The second question of the extraction - whether an existing playlist is
//! overwritten - is a yes/no question, and the system's own message box asks
//! it.
//!
//! # The headless seam
//!
//! Both dialogs need a desktop: they open a window and run their own modal
//! message loop. What does **not** need one is the decision behind the
//! answers - which files are written, what the report says - and that is
//! where the logic lives (see `rustamp-app`'s `library_extract`).
//!
//! Call both on the UI thread.

use std::path::PathBuf;

use crate::file_dialog::FileDialogError;

/// The two values the configuration step of an extraction collects.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtractSettings {
    /// The path template the file names are rendered from.
    ///
    /// The language is the one of spec S20 §4.2: bare words, no `%name%` and
    /// no functions.
    pub template: String,
    /// The directory the files are written into.
    pub directory: PathBuf,
}

/// Asks the user for the settings of an extraction.
///
/// `template` and `directory` are the values the dialog starts with: the
/// template of the last extraction and the directory it wrote into, so the
/// second run of an extraction needs no typing.
///
/// A cancelled dialog is **not** an error: it returns `None`, and the caller
/// writes nothing. Same contract [`crate::file_dialog::pick_directory`] has.
///
/// Call on the UI thread: the dialog runs its own modal message loop.
#[cfg(windows)]
pub fn ask_extract_settings(
    raw_owner: isize,
    template: &str,
    directory: &str,
) -> Result<Option<ExtractSettings>, FileDialogError> {
    if raw_owner == 0 {
        return Err(FileDialogError::NullOwner);
    }
    win32::ask_extract_settings(raw_owner, template, directory)
}

/// Stub: there is no native dialog off Windows yet (milestone M6).
#[cfg(not(windows))]
pub fn ask_extract_settings(
    _raw_owner: isize,
    _template: &str,
    _directory: &str,
) -> Result<Option<ExtractSettings>, FileDialogError> {
    Err(FileDialogError::Unsupported)
}

/// Asks the user a yes/no question.
///
/// The question carries three answers: `Some(true)` is "yes", `Some(false)`
/// is "no" and `None` is the question the user closed without answering. A
/// question that was not answered is **not** an error: the caller decides
/// what an unanswered question means.
///
/// Call on the UI thread: the message box runs its own modal message loop.
#[cfg(windows)]
pub fn ask_yes_no_cancel(
    raw_owner: isize,
    question: &str,
) -> Result<Option<bool>, FileDialogError> {
    if raw_owner == 0 {
        return Err(FileDialogError::NullOwner);
    }
    Ok(win32::ask_yes_no_cancel(raw_owner, question))
}

/// Stub: there is no native dialog off Windows yet (milestone M6).
#[cfg(not(windows))]
pub fn ask_yes_no_cancel(
    _raw_owner: isize,
    _question: &str,
) -> Result<Option<bool>, FileDialogError> {
    Err(FileDialogError::Unsupported)
}

#[cfg(windows)]
mod win32 {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::path::PathBuf;

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{COLOR_BTNFACE, GetSysColorBrush};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
    use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
    use windows::Win32::UI::WindowsAndMessaging::{
        CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
        GetMessageW, GetWindowTextW, HMENU, IDC_ARROW, IDNO, IDYES, IsDialogMessageW, LoadCursorW,
        MB_ICONQUESTION, MB_YESNOCANCEL, MSG, MessageBoxW, PostQuitMessage, RegisterClassExW,
        SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos,
        SetWindowTextW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND,
        WM_DESTROY, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
    };
    use windows::core::{PCWSTR, w};

    use crate::file_dialog::{FileDialogError, pick_directory};

    use super::ExtractSettings;

    /// The name of the window class of the dialog.
    const CLASS_NAME: &str = "RustampExtractDialog";
    /// The caption of the dialog.
    const TITLE: &str = "Extract playlist";
    /// The caption of the question the system asks about an existing playlist.
    const QUESTION_CAPTION: &str = "Extract playlist";

    /// The control ids.
    ///
    /// The two buttons carry the dialog's own standard ids (`IDOK` and
    /// `IDCANCEL` of the system), so the system's keyboard handling maps the
    /// Enter and the Escape key onto them.
    const ID_OK: u16 = 1;
    const ID_CANCEL: u16 = 2;
    const ID_TEMPLATE: u16 = 3;
    const ID_DIRECTORY: u16 = 4;
    const ID_BROWSE: u16 = 5;
    const ID_TEMPLATE_LABEL: u16 = 6;
    const ID_DIRECTORY_LABEL: u16 = 7;

    /// The client area of the dialog, in unscaled pixels.
    const CLIENT: (i32, i32) = (420, 150);
    /// The layout of the controls, in unscaled pixels: (x, y, width, height).
    const TEMPLATE_LABEL: (i32, i32, i32, i32) = (10, 16, 100, 18);
    const TEMPLATE_FIELD: (i32, i32, i32, i32) = (115, 12, 285, 22);
    const DIRECTORY_LABEL: (i32, i32, i32, i32) = (10, 48, 100, 18);
    const DIRECTORY_FIELD: (i32, i32, i32, i32) = (115, 44, 190, 22);
    const BROWSE_BUTTON: (i32, i32, i32, i32) = (310, 43, 90, 24);
    const OK_BUTTON: (i32, i32, i32, i32) = (215, 110, 90, 26);
    const CANCEL_BUTTON: (i32, i32, i32, i32) = (315, 110, 90, 26);

    /// The style bits of the controls, as plain numbers.
    ///
    /// The bindings type the button and the edit styles differently per
    /// version, so the bits are spelled out here, as the preferences dialog
    /// does.
    const SS_LEFT: u32 = 0x0;
    const BS_PUSHBUTTON: u32 = 0x0;
    const BS_DEFPUSHBUTTON: u32 = 0x1;
    const ES_AUTOHSCROLL: u32 = 0x0080;

    /// The live state of the dialog.
    struct Inner {
        /// The field of the path template.
        template: HWND,
        /// The field of the target directory.
        directory: HWND,
        /// The answer of the dialog, when the user confirmed it.
        answer: Option<ExtractSettings>,
    }

    thread_local! {
        static STATE: RefCell<Option<Inner>> = const { RefCell::new(None) };
    }

    /// The text of a string, as the API wants it.
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    /// A length in unscaled pixels, at the scale of the monitor.
    fn scale(value: i32, dpi: u32) -> i32 {
        (i64::from(value) * i64::from(dpi) / 96) as i32
    }

    /// The text a field shows.
    fn read_edit_text(control: HWND) -> String {
        let mut buffer = [0u16; 2048];
        // SAFETY: the control is our own, and the buffer is big enough for
        // the text it holds.
        let length = unsafe { GetWindowTextW(control, &mut buffer) };
        if length <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buffer[..length as usize])
    }

    /// Makes a field show `text`.
    fn set_edit_text(control: HWND, text: &str) {
        let text = wide(text);
        // SAFETY: the text outlives the call, and the control is our own.
        let _ = unsafe { SetWindowTextW(control, PCWSTR(text.as_ptr())) };
    }

    /// Registers the window class of the dialog.
    ///
    /// A second registration fails with "class exists", which is fine: the
    /// class is registered once per process.
    fn register_class() -> Result<(), FileDialogError> {
        let class = wide(CLASS_NAME);
        // SAFETY: standard class registration; the name outlives the call.
        unsafe {
            let module = GetModuleHandleW(None)
                .map_err(|error| FileDialogError::Failed(error.code().0 as u32))?;
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: module.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            let _ = RegisterClassExW(&wc);
        }
        Ok(())
    }

    /// Creates a child control of the dialog.
    #[allow(clippy::too_many_arguments)]
    fn child(
        parent: HWND,
        class: PCWSTR,
        text: &str,
        style: u32,
        ex_style: u32,
        (x, y, w, h): (i32, i32, i32, i32),
        id: u16,
        dpi: u32,
    ) -> Result<HWND, FileDialogError> {
        let text = wide(text);
        // SAFETY: creating a child control of our own window.
        let control = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(ex_style),
                class,
                PCWSTR(text.as_ptr()),
                WINDOW_STYLE(style),
                scale(x, dpi),
                scale(y, dpi),
                scale(w, dpi),
                scale(h, dpi),
                Some(parent),
                Some(HMENU(usize::from(id) as *mut c_void)),
                None,
                None,
            )
        }
        .map_err(|error| FileDialogError::Failed(error.code().0 as u32))?;
        Ok(control)
    }

    /// The window procedure of the dialog.
    ///
    /// # Safety
    ///
    /// Windows calls this with OS-provided values only; the pointers in
    /// `lparam` are valid for the matching notification and are read as
    /// such.
    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_COMMAND => {
                let id = (wparam.0 & 0xffff) as u16;
                let code = ((wparam.0 >> 16) & 0xffff) as u32;
                if code != 0 {
                    // Only the clicks of the buttons are handled here.
                    return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
                }
                if id == ID_OK {
                    accept();
                    // SAFETY: closes our own window.
                    let _ = unsafe { DestroyWindow(hwnd) };
                    return LRESULT(0);
                }
                if id == ID_CANCEL {
                    // SAFETY: closes our own window.
                    let _ = unsafe { DestroyWindow(hwnd) };
                    return LRESULT(0);
                }
                if id == ID_BROWSE {
                    browse(hwnd);
                    return LRESULT(0);
                }
            }
            WM_DESTROY => {
                // The modal loop below ends when the quit message arrives.
                // SAFETY: the message carries no pointer.
                unsafe { PostQuitMessage(0) };
                return LRESULT(0);
            }
            _ => {}
        }
        // SAFETY: default handling for everything else.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    /// Reads the two fields and keeps them as the answer of the dialog.
    fn accept() {
        STATE.with(|state| {
            let Ok(mut guard) = state.try_borrow_mut() else {
                return;
            };
            let Some(inner) = guard.as_mut() else {
                return;
            };
            let template = read_edit_text(inner.template);
            let directory = read_edit_text(inner.directory);
            inner.answer = Some(ExtractSettings {
                template,
                directory: PathBuf::from(directory),
            });
        });
    }

    /// Asks the user for a directory and puts it into the directory field.
    fn browse(hwnd: HWND) {
        let field = STATE.with(|state| {
            state
                .try_borrow()
                .ok()
                .and_then(|guard| guard.as_ref().map(|inner| inner.directory))
        });
        let Some(field) = field else {
            return;
        };
        // The folder dialog needs a directory that exists, so the text the
        // user typed is only a starting point: the dialog itself asks.
        if let Ok(Some(directory)) = pick_directory(hwnd.0 as isize, "Target directory") {
            set_edit_text(field, &directory.display().to_string());
        }
    }

    /// Asks the user for the settings of an extraction.
    pub fn ask_extract_settings(
        raw_owner: isize,
        template: &str,
        directory: &str,
    ) -> Result<Option<ExtractSettings>, FileDialogError> {
        register_class()?;
        let owner = HWND(raw_owner as *mut c_void);
        let class = wide(CLASS_NAME);
        let title = wide(TITLE);
        let style = WS_CAPTION | WS_SYSMENU;
        // SAFETY: plain window creation with NUL-terminated strings that
        // outlive the call; the owner is the player's own window.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR(title.as_ptr()),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CLIENT.0,
                CLIENT.1,
                Some(owner),
                None,
                None,
                None,
            )
        }
        .map_err(|error| FileDialogError::Failed(error.code().0 as u32))?;

        // SAFETY: the window was just created on this thread.
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);

        // The window is sized so that its client area is exactly CLIENT.
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: scale(CLIENT.0, dpi),
            bottom: scale(CLIENT.1, dpi),
        };
        // SAFETY: `frame` is a valid RECT.
        let _ =
            unsafe { AdjustWindowRectExForDpi(&mut frame, style, false, WINDOW_EX_STYLE(0), dpi) };
        // SAFETY: the window is ours; this only moves and sizes it.
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                frame.right - frame.left,
                frame.bottom - frame.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };

        child(
            hwnd,
            w!("STATIC"),
            "Path template:",
            WS_CHILD.0 | WS_VISIBLE.0 | SS_LEFT,
            0,
            TEMPLATE_LABEL,
            ID_TEMPLATE_LABEL,
            dpi,
        )?;
        let template_field = child(
            hwnd,
            w!("EDIT"),
            "",
            WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL,
            0,
            TEMPLATE_FIELD,
            ID_TEMPLATE,
            dpi,
        )?;
        child(
            hwnd,
            w!("STATIC"),
            "Target directory:",
            WS_CHILD.0 | WS_VISIBLE.0 | SS_LEFT,
            0,
            DIRECTORY_LABEL,
            ID_DIRECTORY_LABEL,
            dpi,
        )?;
        let directory_field = child(
            hwnd,
            w!("EDIT"),
            "",
            WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL,
            0,
            DIRECTORY_FIELD,
            ID_DIRECTORY,
            dpi,
        )?;
        child(
            hwnd,
            w!("BUTTON"),
            "Browse...",
            WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_PUSHBUTTON,
            0,
            BROWSE_BUTTON,
            ID_BROWSE,
            dpi,
        )?;
        child(
            hwnd,
            w!("BUTTON"),
            "Extract",
            WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_DEFPUSHBUTTON,
            0,
            OK_BUTTON,
            ID_OK,
            dpi,
        )?;
        child(
            hwnd,
            w!("BUTTON"),
            "Cancel",
            WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_PUSHBUTTON,
            0,
            CANCEL_BUTTON,
            ID_CANCEL,
            dpi,
        )?;

        set_edit_text(template_field, template);
        set_edit_text(directory_field, directory);

        STATE.with(|state| {
            *state.borrow_mut() = Some(Inner {
                template: template_field,
                directory: directory_field,
                answer: None,
            });
        });

        // SAFETY: the window is ours.
        let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
        // SAFETY: the window is ours.
        let _ = unsafe { SetForegroundWindow(hwnd) };
        // SAFETY: the field is ours; it takes the focus, so the template can
        // be typed right away.
        let _ = unsafe { SetFocus(Some(template_field)) };
        // SAFETY: the owner is the player's own window; it is disabled while
        // the modal dialog is up.
        let _ = unsafe { EnableWindow(owner, false) };

        // The modal loop: the dialog runs until it is closed.
        let mut message = MSG::default();
        loop {
            // SAFETY: the message struct is a valid out-parameter.
            let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
            if status.0 <= 0 {
                // The quit message of the destroyed window, or an error.
                break;
            }
            // SAFETY: the message was filled by the call above.
            unsafe {
                // The dialog's own keyboard handling: the Enter key confirms
                // and the Escape key cancels.
                if IsDialogMessageW(hwnd, &message).as_bool() {
                    continue;
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        // SAFETY: the owner is the player's own window; it is enabled again.
        let _ = unsafe { EnableWindow(owner, true) };

        let answer = STATE.with(|state| state.borrow_mut().take());
        Ok(answer.and_then(|inner| inner.answer))
    }

    /// Asks the user a yes/no question.
    pub fn ask_yes_no_cancel(raw_owner: isize, question: &str) -> Option<bool> {
        let owner = HWND(raw_owner as *mut c_void);
        let question = wide(question);
        let caption = wide(QUESTION_CAPTION);
        // SAFETY: the strings are NUL-terminated and outlive the call; the
        // owner is the player's own window.
        let answer = unsafe {
            MessageBoxW(
                Some(owner),
                PCWSTR(question.as_ptr()),
                PCWSTR(caption.as_ptr()),
                MB_YESNOCANCEL | MB_ICONQUESTION,
            )
        };
        if answer.0 == IDYES.0 {
            return Some(true);
        }
        if answer.0 == IDNO.0 {
            return Some(false);
        }
        None
    }
}
