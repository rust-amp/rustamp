// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The native preferences dialog.
//!
//! The reference's preferences window is an ordinary Windows dialog: a tree of
//! pages on the left with a `Close` button under it, and the selected page's
//! group boxes, check boxes and lists on the right, all in the system font.
//! The player used to paint a frame of its own with the skin's bitmap font and
//! no controls at all, which is why it looked wrong and did nothing.
//!
//! This module is that dialog, in the layer that owns the Win32 bindings.
//!
//! # The headless seam
//!
//! The window needs a desktop. What does not is the whole *model*: the page
//! tree ([`TREE`]), what each page contains ([`page_items`]), which control is
//! bound to which setting ([`Bind`]) and what a click turns into
//! ([`PrefsAction`]). These are plain data and are unit-tested below.
//!
//! # Honesty
//!
//! A control exists only where the player really has the setting behind it.
//! Pages without any setting say so in words instead of showing dead controls.

/// One node of the page tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrefsNode {
    /// `Setup`.
    Setup,
    /// `Setup > File types`.
    FileTypes,
    /// `Options`.
    Options,
    /// `Options > Display`.
    Display,
    /// `Options > Visualization`.
    OptionsVisualization,
    /// `Options > Shuffle`.
    Shuffle,
    /// `Plug-ins`.
    PlugIns,
    /// `Plug-ins > Input`.
    Input,
    /// `Plug-ins > Output`.
    Output,
    /// `Plug-ins > Visualization`.
    PlugInVisualization,
    /// `Plug-ins > DSP/Effect`.
    DspEffect,
    /// `Plug-ins > General Purpose`.
    GeneralPurpose,
    /// `Skins`.
    Skins,
    /// `Bookmarks`.
    Bookmarks,
}

/// One row of the page tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeEntry {
    /// The page this row selects.
    pub node: PrefsNode,
    /// The row text.
    pub label: &'static str,
    /// Nesting depth: `0` is a root row, `1` a child of the root row above.
    pub depth: u8,
}

const fn entry(node: PrefsNode, label: &'static str, depth: u8) -> TreeEntry {
    TreeEntry { node, label, depth }
}

/// The page tree, top to bottom (parents come before their children).
pub const TREE: &[TreeEntry] = &[
    entry(PrefsNode::Setup, "Setup", 0),
    entry(PrefsNode::FileTypes, "File types", 1),
    entry(PrefsNode::Options, "Options", 0),
    entry(PrefsNode::Display, "Display", 1),
    entry(PrefsNode::OptionsVisualization, "Visualization", 1),
    entry(PrefsNode::Shuffle, "Shuffle", 1),
    entry(PrefsNode::PlugIns, "Plug-ins", 0),
    entry(PrefsNode::Input, "Input", 1),
    entry(PrefsNode::Output, "Output", 1),
    entry(PrefsNode::PlugInVisualization, "Visualization", 1),
    entry(PrefsNode::DspEffect, "DSP/Effect", 1),
    entry(PrefsNode::GeneralPurpose, "General Purpose", 1),
    entry(PrefsNode::Skins, "Skins", 0),
    entry(PrefsNode::Bookmarks, "Bookmarks", 0),
];

/// A rectangle in unscaled (96 DPI) pixels, relative to the page area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left.
    pub x: i32,
    /// Top.
    pub y: i32,
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
}

const fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect { x, y, w, h }
}

/// A setting a check box is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bind {
    /// Keep the main window above other windows.
    AlwaysOnTop,
    /// Drag the windows from anywhere.
    EasyMove,
    /// Draw the windows at double size.
    DoubleSize,
    /// Play in random order.
    Shuffle,
    /// Repeat the playlist.
    Repeat,
}

/// One element of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A group box with a caption.
    Group {
        /// Caption.
        text: &'static str,
        /// Placement.
        rect: Rect,
    },
    /// A line of static text.
    Label {
        /// Text.
        text: &'static str,
        /// Placement.
        rect: Rect,
    },
    /// A check box bound to a setting.
    Check {
        /// Text.
        text: &'static str,
        /// Placement.
        rect: Rect,
        /// The setting it shows and changes.
        bind: Bind,
    },
    /// The list of loadable skins; choosing a row loads it.
    SkinList {
        /// Placement.
        rect: Rect,
    },
}

/// Width and height of the page area.
pub const PAGE_SIZE: (i32, i32) = (262, 284);

fn note_page(group: &'static str, text: &'static str) -> Vec<Item> {
    vec![
        Item::Group {
            text: group,
            rect: rect(0, 0, PAGE_SIZE.0, 80),
        },
        Item::Label {
            text,
            rect: rect(12, 22, PAGE_SIZE.0 - 24, 48),
        },
    ]
}

/// What `node`'s page contains.
pub fn page_items(node: PrefsNode) -> Vec<Item> {
    let w = PAGE_SIZE.0;
    match node {
        PrefsNode::Setup => note_page(
            "Rustamp setup",
            "Language: English.\nSettings are saved to the player's configuration file when it closes.",
        ),
        PrefsNode::FileTypes => note_page(
            "File types",
            "Associating file types with Rustamp is not available yet.",
        ),
        PrefsNode::Options => vec![
            Item::Group {
                text: "Windows",
                rect: rect(0, 0, w, 76),
            },
            Item::Check {
                text: "Always on top",
                rect: rect(12, 22, w - 24, 20),
                bind: Bind::AlwaysOnTop,
            },
            Item::Check {
                text: "Easy move (drag windows from anywhere)",
                rect: rect(12, 46, w - 24, 20),
                bind: Bind::EasyMove,
            },
        ],
        PrefsNode::Display => vec![
            Item::Group {
                text: "Display",
                rect: rect(0, 0, w, 52),
            },
            Item::Check {
                text: "Double size",
                rect: rect(12, 22, w - 24, 20),
                bind: Bind::DoubleSize,
            },
        ],
        PrefsNode::OptionsVisualization => note_page(
            "Visualization",
            "The visualization options are not configurable yet.",
        ),
        PrefsNode::Shuffle => vec![
            Item::Group {
                text: "Playback order",
                rect: rect(0, 0, w, 76),
            },
            Item::Check {
                text: "Shuffle",
                rect: rect(12, 22, w - 24, 20),
                bind: Bind::Shuffle,
            },
            Item::Check {
                text: "Repeat",
                rect: rect(12, 46, w - 24, 20),
                bind: Bind::Repeat,
            },
        ],
        PrefsNode::PlugIns => {
            note_page("Plug-ins", "Choose a plug-in type in the list on the left.")
        }
        PrefsNode::Input => note_page("Input plug-ins", "Input plug-ins are not configurable yet."),
        PrefsNode::Output => note_page(
            "Output plug-ins",
            "The output device is chosen automatically.",
        ),
        PrefsNode::PlugInVisualization => note_page(
            "Visualization plug-in",
            "Visualization plug-ins are not configurable yet.",
        ),
        PrefsNode::DspEffect => note_page(
            "DSP/Effect plug-ins",
            "DSP/effect plug-ins are not configurable yet.",
        ),
        PrefsNode::GeneralPurpose => note_page(
            "General purpose plug-ins",
            "General purpose plug-ins are not configurable yet.",
        ),
        PrefsNode::Skins => vec![
            Item::Group {
                text: "Select skin",
                rect: rect(0, 0, w, PAGE_SIZE.1),
            },
            Item::SkinList {
                rect: rect(10, 20, w - 20, PAGE_SIZE.1 - 30),
            },
        ],
        PrefsNode::Bookmarks => note_page("Bookmarks", "Bookmarks are not available yet."),
    }
}

/// The settings the dialog shows, pushed in by the player.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrefsSettings {
    /// Always on top.
    pub always_on_top: bool,
    /// Easy move.
    pub easy_move: bool,
    /// Double size.
    pub double_size: bool,
    /// Shuffle.
    pub shuffle: bool,
    /// Repeat.
    pub repeat: bool,
    /// The skins that can be loaded, one row each.
    pub skins: Vec<String>,
    /// The row of the skin in use.
    pub skin_selected: Option<usize>,
}

impl PrefsSettings {
    /// The current value of `bind`.
    pub fn get(&self, bind: Bind) -> bool {
        match bind {
            Bind::AlwaysOnTop => self.always_on_top,
            Bind::EasyMove => self.easy_move,
            Bind::DoubleSize => self.double_size,
            Bind::Shuffle => self.shuffle,
            Bind::Repeat => self.repeat,
        }
    }
}

/// What the user did in the dialog; the player turns it into a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefsAction {
    /// A check box was set to this value.
    Set(Bind, bool),
    /// A skin row was chosen.
    LoadSkin(usize),
}

/// Why the dialog could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum PrefsError {
    /// A Win32 call refused.
    #[error("preferences dialog: {0}")]
    Win32(String),
    /// There is no native dialog on this platform yet.
    #[error("a native preferences dialog is only implemented on Windows")]
    Unsupported,
}

#[cfg(windows)]
pub use win32::PrefsWindow;

#[cfg(windows)]
mod win32 {
    use std::cell::RefCell;
    use std::ffi::c_void;

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        COLOR_BTNFACE, CreateFontIndirectW, DeleteObject, GetSysColorBrush, HFONT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Controls::{
        ICC_STANDARD_CLASSES, ICC_TREEVIEW_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
        NMHDR, NMTREEVIEWW, TVE_EXPAND, TVGN_CARET, TVI_LAST, TVIF_PARAM, TVIF_TEXT,
        TVINSERTSTRUCTW, TVITEMW, TVM_EXPAND, TVM_INSERTITEMW, TVM_SELECTITEM, TVN_SELCHANGEDW,
        WC_TREEVIEWW,
    };
    use windows::Win32::UI::HiDpi::{
        AdjustWindowRectExForDpi, GetDpiForWindow, SystemParametersInfoForDpi,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        BM_GETCHECK, BM_SETCHECK, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
        HMENU, IDC_ARROW, IsWindow, LB_ADDSTRING, LB_GETCURSEL, LB_SETCURSEL, LoadCursorW,
        NONCLIENTMETRICSW, RegisterClassExW, SPI_GETNONCLIENTMETRICS, SW_SHOW, SendMessageW,
        SetForegroundWindow, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_DESTROY,
        WM_NOTIFY, WM_SETFONT, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_EX_CLIENTEDGE, WS_SYSMENU,
        WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
    };
    use windows::core::{PCWSTR, PWSTR};

    use super::{
        Bind, Item, PrefsAction, PrefsError, PrefsNode, PrefsSettings, Rect, TREE, page_items,
    };

    const CLASS_NAME: &str = "RustampPreferences";
    /// Control ids: the close button, then page controls from `FIRST_PAGE_ID`.
    const ID_CLOSE: u16 = 1;
    const FIRST_PAGE_ID: u16 = 100;
    const ID_TREE: u16 = 50;
    const ID_SKINS: u16 = 60;

    /// Layout of the frame in unscaled pixels (client area).
    const CLIENT: (i32, i32) = (440, 300);
    const TREE_RECT: (i32, i32, i32, i32) = (8, 8, 150, 252);
    const CLOSE_RECT: (i32, i32, i32, i32) = (8, 266, 150, 26);
    const PAGE_ORIGIN: (i32, i32) = (170, 8);

    /// Button style bits (`BS_*`), which the bindings type differently per
    /// version, kept as plain numbers.
    const BS_PUSHBUTTON: u32 = 0x0;
    const BS_AUTOCHECKBOX: u32 = 0x3;
    const BS_GROUPBOX: u32 = 0x7;
    const SS_LEFT: u32 = 0x0;
    const TVS_HASBUTTONS: u32 = 0x1;
    const TVS_HASLINES: u32 = 0x2;
    const TVS_LINESATROOT: u32 = 0x4;
    const TVS_SHOWSELALWAYS: u32 = 0x20;
    const LBS_NOTIFY: u32 = 0x1;
    const LBN_SELCHANGE: u32 = 1;
    const BN_CLICKED: u32 = 0;

    /// The dialog's live state. The dialog is a singleton per thread (the UI
    /// thread), so the window procedure and the handle share it here instead
    /// of through a raw pointer stored in the window.
    struct Inner {
        hwnd: HWND,
        tree: HWND,
        font: HFONT,
        dpi: u32,
        node: PrefsNode,
        settings: PrefsSettings,
        /// Controls of the current page: (window, binding).
        page: Vec<HWND>,
        checks: Vec<(HWND, u16, Bind)>,
        skin_list: Option<HWND>,
        actions: Vec<PrefsAction>,
        /// Tree item handles, indexed like [`TREE`].
        items: Vec<isize>,
        /// Set while the code (not the user) changes the selection.
        quiet: bool,
    }

    thread_local! {
        static STATE: RefCell<Option<Inner>> = const { RefCell::new(None) };
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    fn scale(value: i32, dpi: u32) -> i32 {
        (i64::from(value) * i64::from(dpi) / 96) as i32
    }

    /// The handle to the open dialog. All state lives with the window; the
    /// handle only addresses it.
    #[derive(Debug, Clone, Copy)]
    pub struct PrefsWindow {
        hwnd: isize,
    }

    impl PrefsWindow {
        /// Opens the dialog, owned by `raw_owner`, showing `node`. Call on
        /// the UI thread, outside a window procedure.
        pub fn open(
            raw_owner: isize,
            node: PrefsNode,
            settings: &PrefsSettings,
        ) -> Result<Self, PrefsError> {
            register_class()?;
            let owner = HWND(raw_owner as *mut c_void);
            let class = wide(CLASS_NAME);
            let title = wide("Preferences");
            let style = WS_CAPTION | WS_SYSMENU;
            // SAFETY: plain window creation with NUL-terminated strings that
            // outlive the call; the owner is the player's main window.
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
                    if raw_owner == 0 { None } else { Some(owner) },
                    None,
                    None,
                    None,
                )
            }
            .map_err(|e| PrefsError::Win32(e.to_string()))?;
            // SAFETY: `hwnd` was just created on this thread.
            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
            let font = message_font(dpi);

            // Size the window so the client area is exactly CLIENT.
            let mut frame = RECT {
                left: 0,
                top: 0,
                right: scale(CLIENT.0, dpi),
                bottom: scale(CLIENT.1, dpi),
            };
            // SAFETY: `frame` is a valid RECT.
            let _ = unsafe {
                AdjustWindowRectExForDpi(&mut frame, style, false, WINDOW_EX_STYLE(0), dpi)
            };
            // SAFETY: `hwnd` is ours; this only moves and sizes it.
            let _ = unsafe {
                windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    frame.right - frame.left,
                    frame.bottom - frame.top,
                    windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
                )
            };

            let tree = child(
                hwnd,
                WC_TREEVIEWW,
                "",
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_TABSTOP.0
                    | TVS_HASBUTTONS
                    | TVS_HASLINES
                    | TVS_LINESATROOT
                    | TVS_SHOWSELALWAYS,
                WS_EX_CLIENTEDGE.0,
                (TREE_RECT.0, TREE_RECT.1, TREE_RECT.2, TREE_RECT.3),
                ID_TREE,
                dpi,
                font,
            )?;
            child(
                hwnd,
                windows::core::w!("BUTTON"),
                "Close",
                WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_PUSHBUTTON,
                0,
                (CLOSE_RECT.0, CLOSE_RECT.1, CLOSE_RECT.2, CLOSE_RECT.3),
                ID_CLOSE,
                dpi,
                font,
            )?;

            let items = fill_tree(tree);
            STATE.with(|state| {
                *state.borrow_mut() = Some(Inner {
                    hwnd,
                    tree,
                    font,
                    dpi,
                    node,
                    settings: settings.clone(),
                    page: Vec::new(),
                    checks: Vec::new(),
                    skin_list: None,
                    actions: Vec::new(),
                    items,
                    quiet: false,
                });
            });
            let handle = PrefsWindow {
                hwnd: hwnd.0 as isize,
            };
            handle.show_page(node);
            // SAFETY: `hwnd` is ours.
            let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
            // SAFETY: `hwnd` is ours.
            let _ = unsafe { SetForegroundWindow(hwnd) };
            Ok(handle)
        }

        /// Whether the window still exists (the user may have closed it).
        pub fn is_open(&self) -> bool {
            // SAFETY: `IsWindow` accepts any handle value.
            unsafe { IsWindow(Some(HWND(self.hwnd as *mut c_void))) }.as_bool()
                && STATE.with(|state| state.borrow().is_some())
        }

        /// Brings the window to the front.
        pub fn raise(&self) {
            // SAFETY: `SetForegroundWindow` accepts any handle value.
            let _ = unsafe { SetForegroundWindow(HWND(self.hwnd as *mut c_void)) };
        }

        /// Closes the window.
        pub fn close(&self) {
            // SAFETY: destroys our own window on its thread.
            let _ = unsafe { DestroyWindow(HWND(self.hwnd as *mut c_void)) };
        }

        /// Shows `node`'s page and selects its row in the tree.
        pub fn show_page(&self, node: PrefsNode) {
            let Some((hwnd, tree, item, dpi, font, settings)) = STATE.with(|state| {
                let mut guard = state.try_borrow_mut().ok()?;
                let inner = guard.as_mut()?;
                inner.node = node;
                inner.quiet = true;
                let index = TREE.iter().position(|e| e.node == node)?;
                Some((
                    inner.hwnd,
                    inner.tree,
                    inner.items.get(index).copied()?,
                    inner.dpi,
                    inner.font,
                    inner.settings.clone(),
                ))
            }) else {
                return;
            };
            // SAFETY: selecting a row of our own tree view.
            unsafe {
                SendMessageW(
                    tree,
                    TVM_SELECTITEM,
                    Some(WPARAM(TVGN_CARET as usize)),
                    Some(LPARAM(item)),
                );
            }
            build_page(hwnd, node, &settings, dpi, font);
            STATE.with(|state| {
                if let Ok(mut guard) = state.try_borrow_mut()
                    && let Some(inner) = guard.as_mut()
                {
                    inner.quiet = false;
                }
            });
        }

        /// Pushes the player's current settings into the open controls.
        pub fn set_settings(&self, settings: &PrefsSettings) {
            let Ok(controls) = STATE.with(|state| {
                let mut guard = state.try_borrow_mut().map_err(|_| ())?;
                let inner = guard.as_mut().ok_or(())?;
                inner.settings = settings.clone();
                Ok::<_, ()>((inner.checks.clone(), inner.skin_list))
            }) else {
                return;
            };
            for (control, _, bind) in controls.0 {
                set_check(control, settings.get(bind));
            }
            if let (Some(list), Some(row)) = (controls.1, settings.skin_selected) {
                // SAFETY: selecting a row of our own list box.
                unsafe {
                    SendMessageW(list, LB_SETCURSEL, Some(WPARAM(row)), Some(LPARAM(0)));
                }
            }
        }

        /// What the user did since the last call.
        pub fn take_actions(&self) -> Vec<PrefsAction> {
            STATE.with(|state| {
                state
                    .try_borrow_mut()
                    .ok()
                    .and_then(|mut guard| guard.as_mut().map(|i| std::mem::take(&mut i.actions)))
                    .unwrap_or_default()
            })
        }
    }

    fn set_check(control: HWND, on: bool) {
        // SAFETY: a check state on our own button.
        unsafe {
            SendMessageW(
                control,
                BM_SETCHECK,
                Some(WPARAM(usize::from(on))),
                Some(LPARAM(0)),
            );
        }
    }

    fn register_class() -> Result<(), PrefsError> {
        let class = wide(CLASS_NAME);
        // SAFETY: standard class registration; the name outlives the call.
        unsafe {
            let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_TREEVIEW_CLASSES | ICC_STANDARD_CLASSES,
            });
            let module = GetModuleHandleW(None).map_err(|e| PrefsError::Win32(e.to_string()))?;
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: module.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            // A second registration fails with "class exists", which is fine.
            let _ = RegisterClassExW(&wc);
        }
        Ok(())
    }

    fn message_font(dpi: u32) -> HFONT {
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        // SAFETY: `metrics` is a valid, correctly sized out-parameter.
        let ok = unsafe {
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                metrics.cbSize,
                Some((&mut metrics as *mut NONCLIENTMETRICSW).cast()),
                0,
                dpi,
            )
        };
        if ok.is_err() {
            return HFONT::default();
        }
        // SAFETY: creates a font from the logfont the OS just filled in.
        unsafe { CreateFontIndirectW(&metrics.lfMessageFont) }
    }

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
        font: HFONT,
    ) -> Result<HWND, PrefsError> {
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
        .map_err(|e| PrefsError::Win32(e.to_string()))?;
        // SAFETY: setting the font of a control we own.
        unsafe {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        Ok(control)
    }

    /// Fills the tree view; returns each row's item handle, in [`TREE`] order.
    fn fill_tree(tree: HWND) -> Vec<isize> {
        let mut handles: Vec<isize> = Vec::new();
        let mut last_root = 0isize;
        for (index, entry) in TREE.iter().enumerate() {
            let mut text = wide(entry.label);
            let parent = if entry.depth == 0 { 0 } else { last_root };
            let insert = TVINSERTSTRUCTW {
                hParent: windows::Win32::UI::Controls::HTREEITEM(parent),
                hInsertAfter: TVI_LAST,
                Anonymous: windows::Win32::UI::Controls::TVINSERTSTRUCTW_0 {
                    item: TVITEMW {
                        mask: TVIF_TEXT | TVIF_PARAM,
                        pszText: PWSTR(text.as_mut_ptr()),
                        lParam: LPARAM(index as isize),
                        ..Default::default()
                    },
                },
            };
            // SAFETY: the insert struct and its text outlive the call.
            let item = unsafe {
                SendMessageW(
                    tree,
                    TVM_INSERTITEMW,
                    Some(WPARAM(0)),
                    Some(LPARAM(&insert as *const TVINSERTSTRUCTW as isize)),
                )
            }
            .0;
            if entry.depth == 0 {
                last_root = item;
            }
            handles.push(item);
        }
        // Open every root so the whole tree is visible, like the reference.
        for (index, entry) in TREE.iter().enumerate() {
            if entry.depth == 0 {
                // SAFETY: expanding a row of our own tree view.
                unsafe {
                    SendMessageW(
                        tree,
                        TVM_EXPAND,
                        Some(WPARAM(TVE_EXPAND.0 as usize)),
                        Some(LPARAM(handles[index])),
                    );
                }
            }
        }
        handles
    }

    /// Replaces the page area's controls with `node`'s page.
    fn build_page(hwnd: HWND, node: PrefsNode, settings: &PrefsSettings, dpi: u32, font: HFONT) {
        let old = STATE.with(|state| {
            state
                .try_borrow_mut()
                .ok()
                .and_then(|mut g| {
                    g.as_mut().map(|i| {
                        i.checks.clear();
                        i.skin_list = None;
                        std::mem::take(&mut i.page)
                    })
                })
                .unwrap_or_default()
        });
        for control in old {
            // SAFETY: destroying a control we created.
            let _ = unsafe { DestroyWindow(control) };
        }
        let shift = |r: Rect| (r.x + PAGE_ORIGIN.0, r.y + PAGE_ORIGIN.1, r.w, r.h);
        let mut page = Vec::new();
        let mut checks = Vec::new();
        let mut skin_list = None;
        for (next_id, item) in (FIRST_PAGE_ID..).zip(page_items(node)) {
            match item {
                Item::Group { text, rect } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("BUTTON"),
                        text,
                        WS_CHILD.0 | WS_VISIBLE.0 | BS_GROUPBOX,
                        0,
                        shift(rect),
                        next_id,
                        dpi,
                        font,
                    ) {
                        page.push(c);
                    }
                }
                Item::Label { text, rect } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("STATIC"),
                        text,
                        WS_CHILD.0 | WS_VISIBLE.0 | SS_LEFT,
                        0,
                        shift(rect),
                        next_id,
                        dpi,
                        font,
                    ) {
                        page.push(c);
                    }
                }
                Item::Check { text, rect, bind } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("BUTTON"),
                        text,
                        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_AUTOCHECKBOX,
                        0,
                        shift(rect),
                        next_id,
                        dpi,
                        font,
                    ) {
                        set_check(c, settings.get(bind));
                        page.push(c);
                        checks.push((c, next_id, bind));
                    }
                }
                Item::SkinList { rect } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("LISTBOX"),
                        "",
                        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | WS_VSCROLL.0 | LBS_NOTIFY,
                        WS_EX_CLIENTEDGE.0,
                        shift(rect),
                        ID_SKINS,
                        dpi,
                        font,
                    ) {
                        for skin in &settings.skins {
                            let text = wide(skin);
                            // SAFETY: the text outlives the call.
                            unsafe {
                                SendMessageW(
                                    c,
                                    LB_ADDSTRING,
                                    Some(WPARAM(0)),
                                    Some(LPARAM(text.as_ptr() as isize)),
                                );
                            }
                        }
                        if let Some(row) = settings.skin_selected {
                            // SAFETY: selecting a row of our own list box.
                            unsafe {
                                SendMessageW(c, LB_SETCURSEL, Some(WPARAM(row)), Some(LPARAM(0)));
                            }
                        }
                        page.push(c);
                        skin_list = Some(c);
                    }
                }
            }
        }
        STATE.with(|state| {
            if let Ok(mut guard) = state.try_borrow_mut()
                && let Some(inner) = guard.as_mut()
            {
                inner.page = page;
                inner.checks = checks;
                inner.skin_list = skin_list;
            }
        });
    }

    /// Records a user action from the window procedure (queue only: no
    /// player state is touched here).
    fn push_action(action: PrefsAction) {
        STATE.with(|state| {
            if let Ok(mut guard) = state.try_borrow_mut()
                && let Some(inner) = guard.as_mut()
            {
                inner.actions.push(action);
            }
        });
    }

    // SAFETY: Windows calls this with OS-provided values only; pointers in
    // `lparam` are valid for the matching notification and read as such.
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
                if id == ID_CLOSE && code == BN_CLICKED {
                    // SAFETY: closes our own window.
                    let _ = unsafe { DestroyWindow(hwnd) };
                    return LRESULT(0);
                }
                if id == ID_SKINS && code == LBN_SELCHANGE {
                    let list = HWND(lparam.0 as *mut c_void);
                    // SAFETY: querying our own list box.
                    let row = unsafe {
                        SendMessageW(list, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0)))
                    }
                    .0;
                    if row >= 0 {
                        push_action(PrefsAction::LoadSkin(row as usize));
                    }
                    return LRESULT(0);
                }
                if code == BN_CLICKED {
                    let hit = STATE.with(|state| {
                        state.try_borrow().ok().and_then(|g| {
                            g.as_ref().and_then(|i| {
                                i.checks.iter().find(|(_, cid, _)| *cid == id).copied()
                            })
                        })
                    });
                    if let Some((control, _, bind)) = hit {
                        // SAFETY: reading the state of our own check box.
                        let on = unsafe {
                            SendMessageW(control, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0)))
                        }
                        .0 == 1;
                        push_action(PrefsAction::Set(bind, on));
                        return LRESULT(0);
                    }
                }
            }
            WM_NOTIFY => {
                // SAFETY: for a tree-view notification `lparam` points at an
                // NMTREEVIEWW that lives for the duration of the message.
                let header = unsafe { &*(lparam.0 as *const NMHDR) };
                if header.idFrom == usize::from(ID_TREE) && header.code == TVN_SELCHANGEDW {
                    let notify = unsafe { &*(lparam.0 as *const NMTREEVIEWW) };
                    let index = notify.itemNew.lParam.0 as usize;
                    let quiet = STATE.with(|state| {
                        state
                            .try_borrow()
                            .ok()
                            .and_then(|g| g.as_ref().map(|i| i.quiet))
                            .unwrap_or(true)
                    });
                    if !quiet && let Some(entry) = TREE.get(index) {
                        // Rebuilding destroys controls, which can send this
                        // window messages: do it from a clean borrow state.
                        let info = STATE.with(|state| {
                            state.try_borrow().ok().and_then(|g| {
                                g.as_ref().map(|i| (i.dpi, i.font, i.settings.clone()))
                            })
                        });
                        if let Some((dpi, font, settings)) = info {
                            STATE.with(|state| {
                                if let Ok(mut g) = state.try_borrow_mut()
                                    && let Some(i) = g.as_mut()
                                {
                                    i.node = entry.node;
                                    i.quiet = true;
                                }
                            });
                            build_page(hwnd, entry.node, &settings, dpi, font);
                            STATE.with(|state| {
                                if let Ok(mut g) = state.try_borrow_mut()
                                    && let Some(i) = g.as_mut()
                                {
                                    i.quiet = false;
                                }
                            });
                        }
                    }
                }
            }
            WM_DESTROY => {
                let font = STATE.with(|state| {
                    state
                        .try_borrow_mut()
                        .ok()
                        .and_then(|mut g| g.take().map(|i| i.font))
                });
                if let Some(font) = font
                    && !font.is_invalid()
                {
                    // SAFETY: deleting the font this dialog created.
                    let _ = unsafe { DeleteObject(font.into()) };
                }
                return LRESULT(0);
            }
            _ => {}
        }
        // SAFETY: default handling for everything else.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tree_lists_the_reference_pages_with_children_after_their_parent() {
        let labels: Vec<_> = TREE.iter().map(|e| e.label).collect();
        assert_eq!(
            labels,
            [
                "Setup",
                "File types",
                "Options",
                "Display",
                "Visualization",
                "Shuffle",
                "Plug-ins",
                "Input",
                "Output",
                "Visualization",
                "DSP/Effect",
                "General Purpose",
                "Skins",
                "Bookmarks",
            ]
        );
        assert_eq!(TREE[0].depth, 0, "the first row is a root");
        assert!(TREE.iter().all(|e| e.depth <= 1));
    }

    #[test]
    fn every_node_has_exactly_one_row_and_a_page() {
        for entry in TREE {
            assert_eq!(
                TREE.iter().filter(|e| e.node == entry.node).count(),
                1,
                "{:?}",
                entry.node
            );
            assert!(!page_items(entry.node).is_empty(), "{:?}", entry.node);
        }
    }

    #[test]
    fn every_item_fits_inside_the_page_area() {
        for entry in TREE {
            for item in page_items(entry.node) {
                let r = match item {
                    Item::Group { rect, .. }
                    | Item::Label { rect, .. }
                    | Item::Check { rect, .. }
                    | Item::SkinList { rect } => rect,
                };
                assert!(r.x >= 0 && r.y >= 0, "{:?}: {r:?}", entry.node);
                assert!(r.x + r.w <= PAGE_SIZE.0, "{:?}: {r:?}", entry.node);
                assert!(r.y + r.h <= PAGE_SIZE.1, "{:?}: {r:?}", entry.node);
            }
        }
    }

    #[test]
    fn each_setting_has_exactly_one_check_box() {
        let mut seen: Vec<Bind> = Vec::new();
        for entry in TREE {
            for item in page_items(entry.node) {
                if let Item::Check { bind, .. } = item {
                    assert!(!seen.contains(&bind), "{bind:?} appears twice");
                    seen.push(bind);
                }
            }
        }
        for bind in [
            Bind::AlwaysOnTop,
            Bind::EasyMove,
            Bind::DoubleSize,
            Bind::Shuffle,
            Bind::Repeat,
        ] {
            assert!(seen.contains(&bind), "{bind:?} has no control");
        }
    }

    #[test]
    fn settings_read_back_through_their_binding() {
        let settings = PrefsSettings {
            always_on_top: true,
            repeat: true,
            ..PrefsSettings::default()
        };
        assert!(settings.get(Bind::AlwaysOnTop));
        assert!(settings.get(Bind::Repeat));
        assert!(!settings.get(Bind::EasyMove));
        assert!(!settings.get(Bind::DoubleSize));
        assert!(!settings.get(Bind::Shuffle));
    }
}
