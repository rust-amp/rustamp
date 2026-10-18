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
//!
//! Three further pure seams are headless too (F5 follow-up): [`TRAY_LEFT_CLICK`]
//! names what a left click means, `is_shell_restart` decides whether a
//! message is Explorer's `TaskbarCreated` broadcast, and [`art`] holds the
//! player's own committed icon artwork and its expansion to BGRA.

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

/// What a **left** click on the icon does (own choice; task card F5
/// follow-up): exactly the menu's show/hide row, so the tray still invents no
/// command and the two buttons share one model entry.
///
/// Named here, next to [`TRAY_MENU`], so the mapping itself is headlessly
/// testable: `window.rs` submits `TRAY_LEFT_CLICK.commands()` through the same
/// path a menu pick uses. A left click never opens the popup, and a right
/// click never toggles the window.
pub const TRAY_LEFT_CLICK: TrayMenuEntry = TrayMenuEntry::ToggleMainWindow;

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

/// Does `message` announce that the shell restarted and dropped our icon?
///
/// `registered` is what `RegisterWindowMessageW("TaskbarCreated")` returned.
/// **A failed registration returns 0, and 0 is `WM_NULL`** — the most common
/// message there is, and one the popup code itself posts to the owner. So an
/// unregistered id must match nothing: this is why the predicate is a pure
/// function with its own test rather than a bare `message == registered`.
#[cfg(any(windows, test))]
fn is_shell_restart(registered: u32, message: u32) -> bool {
    registered != 0 && message == registered
}

pub mod art {
    //! The tray icon's own artwork: a committed pixel map, expanded at
    //! runtime.
    //!
    //! # Provenance
    //!
    //! This glyph is **Rustamp's own trivial art**, written here as a palette
    //! plus sixteen rows of ASCII: a dark rounded plate with a green play
    //! triangle. It was drawn for this crate. No Winamp artwork, reference
    //! binary, extracted resource or third-party image was used, and it is
    //! deliberately *not* taken from `rustamp-skin` — that crate holds skin
    //! data, which is not product branding.
    //!
    //! # Why a source pixel map and not a resource
    //!
    //! The executable's own icon resource would need a resource compiler
    //! (`build.rs` plus a new build dependency) and a manifest/lockfile edit,
    //! which this lane does not own. A committed pixel map needs neither, and
    //! unlike a binary `.ico` blob it can be reviewed in the diff.
    //!
    //! # Format
    //!
    //! [`scaled_bgra`] expands the rows to **straight-alpha BGRA**, top-down,
    //! four bytes per pixel, nearest-neighbour scaled to the requested edge
    //! length so the plate's straight edges stay crisp at 150 % and 200 % DPI.
    //! Every palette entry is either fully transparent or fully opaque, so the
    //! straight and premultiplied forms of this art are byte-identical and the
    //! choice cannot show up as a dark fringe.

    /// The committed art is a square of this edge length, in pixels.
    pub const SOURCE_SIZE: u32 = 16;

    /// The palette: the glyph used in [`ROWS`] and its `[R, G, B, A]` colour.
    ///
    /// Own choice, all four entries: transparent surround, slate plate edge,
    /// near-black plate face, green transport accent.
    pub const PALETTE: [(char, [u8; 4]); 4] = [
        ('.', [0x00, 0x00, 0x00, 0x00]),
        ('o', [0x3B, 0x45, 0x54, 0xFF]),
        ('#', [0x15, 0x19, 0x20, 0xFF]),
        ('*', [0x6C, 0xD9, 0x4F, 0xFF]),
    ];

    /// The artwork, top row first. Each row is [`SOURCE_SIZE`] glyphs wide and
    /// uses only [`PALETTE`] glyphs; both are pinned by tests.
    pub const ROWS: [&str; SOURCE_SIZE as usize] = [
        "...oooooooooo...",
        ".oooooooooooooo.",
        "oo############oo",
        "o##############o",
        "o####*#########o",
        "o####***#######o",
        "o####*****#####o",
        "o####*******###o",
        "o####*******###o",
        "o####*****#####o",
        "o####***#######o",
        "o####*#########o",
        "o##############o",
        "oo############oo",
        ".oooooooooooooo.",
        "...oooooooooo...",
    ];

    /// The palette colour for a glyph. An unknown glyph is transparent; the
    /// tests pin that [`ROWS`] never contains one.
    pub fn colour_for(glyph: char) -> Option<[u8; 4]> {
        PALETTE
            .iter()
            .find_map(|(key, colour)| (*key == glyph).then_some(*colour))
    }

    /// The committed art as straight-alpha BGRA, top-down.
    pub fn source_bgra() -> Vec<u8> {
        let mut pixels = Vec::with_capacity((SOURCE_SIZE * SOURCE_SIZE * 4) as usize);
        for row in ROWS {
            let mut written = 0u32;
            for glyph in row.chars() {
                let [r, g, b, a] = colour_for(glyph).unwrap_or([0, 0, 0, 0]);
                pixels.extend_from_slice(&[b, g, r, a]);
                written += 1;
            }
            // A short row would otherwise skew every later row; pad it out so
            // the buffer length always matches the declared dimensions.
            for _ in written..SOURCE_SIZE {
                pixels.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
        pixels.truncate((SOURCE_SIZE * SOURCE_SIZE * 4) as usize);
        pixels
    }

    /// The art as straight-alpha BGRA at `size` × `size`, top-down,
    /// nearest-neighbour scaled. The result is always `size * size * 4` bytes.
    pub fn scaled_bgra(size: u32) -> Vec<u8> {
        let source = source_bgra();
        if size == SOURCE_SIZE {
            return source;
        }
        let mut pixels = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            let source_y = y * SOURCE_SIZE / size;
            for x in 0..size {
                let source_x = x * SOURCE_SIZE / size;
                let at = ((source_y * SOURCE_SIZE + source_x) * 4) as usize;
                pixels.extend_from_slice(&source[at..at + 4]);
            }
        }
        pixels
    }
}

#[cfg(windows)]
mod imp {
    //! Win32 attach: one `Shell_NotifyIconW` icon owned by the main window.
    //!
    //! The icon needs an `HWND` for its callback messages, which only exists
    //! once `window.rs` has built the OS window, not from `main.rs`. A subclass
    //! queues callback requests without touching App or winit. The UI pump
    //! opens the popup and routes the pick. Desktop execution is VM-only.

    use std::sync::{Mutex, OnceLock};

    use windows::Win32::Foundation::{
        ERROR_SUCCESS, GetLastError, HWND, LPARAM, LRESULT, POINT, SetLastError, WPARAM,
    };
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
        DeleteObject, HBITMAP, HGDIOBJ,
    };
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Shell::{
        DefSubclassProc, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
        RemoveWindowSubclass, SetWindowSubclass, Shell_NotifyIconW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos,
        GetSystemMetrics, GetWindowThreadProcessId, HICON, HMENU, ICONINFO, IDI_APPLICATION,
        LoadIconW, MF_STRING, PostMessageW, RegisterWindowMessageW, SM_CXSMICON,
        SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
        WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NCDESTROY, WM_NULL, WM_RBUTTONUP,
    };
    use windows::core::PCWSTR;

    use super::{
        TRAY_CALLBACK_MESSAGE, TRAY_TOOLTIP, TrayMenuEntry, art, is_shell_restart,
        menu_entry_for_id,
    };

    /// The tray icon's id within its owner window (own choice; one icon).
    const TRAY_UID: u32 = 1;
    /// Unique within our subclass procedure, not a public command id.
    const SUBCLASS_ID: usize = 1;

    /// Coalesced right-click/context-menu requests; no Rust pointer is stored
    /// in the window.
    static MENU_REQUESTS: Mutex<Vec<isize>> = Mutex::new(Vec::new());
    /// Coalesced left-click activations, kept **separate** from the menu
    /// requests: a left click must never open the popup, and a right click
    /// must never toggle the window.
    static LEFT_CLICKS: Mutex<Vec<isize>> = Mutex::new(Vec::new());
    /// Coalesced `TaskbarCreated` (shell restart) re-add requests.
    static SHELL_RESTARTS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

    /// `RegisterWindowMessageW("TaskbarCreated")`, registered once per
    /// process. A refused registration yields 0, which
    /// `is_shell_restart` deliberately matches against nothing.
    static SHELL_RESTART_MESSAGE: OnceLock<u32> = OnceLock::new();

    fn shell_restart_message() -> u32 {
        *SHELL_RESTART_MESSAGE.get_or_init(|| {
            let name: Vec<u16> = "TaskbarCreated".encode_utf16().chain([0]).collect();
            // SAFETY: the NUL-terminated name outlives the call; the OS
            // returns an id (or zero on failure) and touches nothing of ours.
            unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) }
        })
    }

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
            // We retain the original (not NOTIFYICON_VERSION_4) callback
            // format, so the mouse message arrives in lParam. A double click
            // arrives as down/up/dblclk/up; `WM_LBUTTONUP` and
            // `WM_LBUTTONDBLCLK` share one coalesced queue, so the UI pump
            // sees a single toggle rather than two or four.
            match lparam.0 as u32 {
                WM_RBUTTONUP | WM_CONTEXTMENU => queue_request(&MENU_REQUESTS, owner),
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => queue_request(&LEFT_CLICKS, owner),
                _ => {}
            }
            return LRESULT(0);
        }
        // The shell broadcasts `TaskbarCreated` to every top-level window
        // after Explorer restarts, so this subclass on the owner HWND sees it.
        // Queue a re-add and let the broadcast continue down the chain.
        if is_shell_restart(shell_restart_message(), message) {
            queue_request(&SHELL_RESTARTS, owner);
        }
        if message == WM_NCDESTROY {
            clear_requests(owner);
            // SAFETY: called on the owner thread, removing only our subclass.
            let _ = unsafe { RemoveWindowSubclass(owner, Some(tray_callback), SUBCLASS_ID) };
        }
        // SAFETY: forward every unrelated message along the existing chain.
        unsafe { DefSubclassProc(owner, message, wparam, lparam) }
    }

    /// Records at most one pending request per owner. Called from the window
    /// procedure: it touches only this queue, never App, winit or a command.
    fn queue_request(queue: &Mutex<Vec<isize>>, owner: HWND) {
        let mut pending = queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let raw = owner.0 as isize;
        if !pending.contains(&raw) {
            pending.push(raw);
        }
    }

    /// Removes the owner's pending request and reports whether there was one.
    fn take_request(queue: &Mutex<Vec<isize>>, owner: HWND) -> bool {
        let mut pending = queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = pending.len();
        pending.retain(|raw| *raw != owner.0 as isize);
        pending.len() != before
    }

    fn clear_requests(owner: HWND) {
        for queue in [&MENU_REQUESTS, &LEFT_CLICKS, &SHELL_RESTARTS] {
            take_request(queue, owner);
        }
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

    /// The icon handle plus who owns it: only our own icon may be destroyed,
    /// a stock `IDI_APPLICATION` handle must never be passed to `DestroyIcon`.
    #[derive(Debug, Clone, Copy)]
    struct OwnedIcon {
        handle: HICON,
        ours: bool,
    }

    /// A GDI bitmap released on every path out of the icon builder.
    struct Bitmap(HBITMAP);

    impl Drop for Bitmap {
        fn drop(&mut self) {
            // SAFETY: exclusively owns the bitmap it was constructed from;
            // `CreateIconIndirect` copies, so deleting here is correct.
            let _ = unsafe { DeleteObject(HGDIOBJ(self.0.0)) };
        }
    }

    /// The shell's small-icon edge length, so the glyph is crisp at 100 %,
    /// 150 % and 200 % DPI. A refused or absurd metric falls back to the
    /// committed art's own 16 px.
    fn small_icon_size() -> u32 {
        // SAFETY: a plain system-metric query; no handle or buffer involved.
        let metric = unsafe { GetSystemMetrics(SM_CXSMICON) };
        if (1..=512).contains(&metric) {
            metric as u32
        } else {
            art::SOURCE_SIZE
        }
    }

    /// Turns the committed artwork into an `HICON` at `size` × `size`.
    ///
    /// Only APIs already enabled in this crate's `windows` features are used,
    /// so no manifest, lockfile, `build.rs` or `.ico` is needed.
    fn build_own_icon(size: u32) -> Result<HICON, windows::core::Error> {
        let pixels = art::scaled_bgra(size);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                // Negative height: top-down rows, matching `scaled_bgra`.
                biHeight: -(size as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
        // SAFETY: `info` describes a 32-bpp top-down DIB and outlives the
        // call; `bits` is a writable out-parameter. A failure is reported as
        // `Err` and leaves nothing to release.
        let colour =
            Bitmap(unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }?);
        if bits.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        // SAFETY: `CreateDIBSection` succeeded for `size * size` 32-bpp
        // top-down pixels, so `bits` addresses exactly `pixels.len()`
        // writable bytes; the two regions cannot overlap.
        unsafe { core::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len()) };
        // The AND mask is ignored for a 32-bpp colour bitmap carrying alpha,
        // but `ICONINFO` still requires one. Supply explicit zero bits (rows
        // are WORD-aligned) rather than leaving its contents undefined.
        let mask_bits = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
        // SAFETY: the zeroed buffer is at least one monochrome bitmap of this
        // size and outlives the call, which copies it.
        let mask = Bitmap(unsafe {
            CreateBitmap(
                size as i32,
                size as i32,
                1,
                1,
                Some(mask_bits.as_ptr().cast()),
            )
        });
        if mask.0.is_invalid() {
            return Err(windows::core::Error::from_thread());
        }
        let icon_info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask.0,
            hbmColor: colour.0,
        };
        // SAFETY: both bitmaps are live for the call and `icon_info` outlives
        // it. `CreateIconIndirect` copies them, so the guards may delete them
        // when this function returns, by either path.
        unsafe { CreateIconIndirect(&icon_info) }
    }

    /// The `NOTIFYICONDATAW` both the initial add and the shell-restart re-add
    /// use, so there is one description of the icon and not two.
    fn notify_icon_data(owner: HWND, icon: HICON) -> NOTIFYICONDATAW {
        let mut tip = [0u16; 128];
        for (slot, unit) in tip.iter_mut().zip(TRAY_TOOLTIP.encode_utf16().chain([0])) {
            *slot = unit;
            if unit == 0 {
                break;
            }
        }
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: owner,
            uID: TRAY_UID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY_CALLBACK_MESSAGE,
            hIcon: icon,
            szTip: tip,
            ..NOTIFYICONDATAW::default()
        }
    }

    fn add_icon(owner: HWND, icon: HICON) -> Result<(), TrayError> {
        let data = notify_icon_data(owner, icon);
        // SAFETY: `data` is a fully initialised `NOTIFYICONDATAW` that
        // outlives the call; `cbSize` names its own size. The shell copies
        // what it needs.
        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
            Ok(())
        } else {
            Err(TrayError::AddRefused)
        }
    }

    /// The running player's tray icon.
    ///
    /// Dropping it removes the icon (`NIM_DELETE`) and destroys our own
    /// `HICON`, so neither can outlive the window that owns them.
    #[derive(Debug)]
    pub struct TrayIcon {
        owner: HWND,
        added: bool,
        icon: OwnedIcon,
    }

    impl TrayIcon {
        /// Adds the tray icon to the window `raw_owner` (the main window's
        /// raw handle, as `window.rs` holds it).
        ///
        /// The glyph is the player's own committed artwork (`art`), built at
        /// the shell's small-icon size. If that cannot be built the stock
        /// `IDI_APPLICATION` is used after a warning; if the stock icon also
        /// fails, [`TrayError::IconLoad`] is returned and the player runs with
        /// no tray icon. Startup and playback are never aborted for an icon.
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
            let icon = match build_own_icon(small_icon_size()) {
                Ok(handle) => OwnedIcon { handle, ours: true },
                Err(error) => {
                    eprintln!(
                        "rustamp: the tray icon artwork could not be built, \
                         falling back to the stock icon: {error}"
                    );
                    // SAFETY: `None` loads a stock icon; `IDI_APPLICATION`
                    // always names one. No window is created.
                    let handle = unsafe { LoadIconW(None, IDI_APPLICATION) }
                        .map_err(|_| TrayError::IconLoad)?;
                    OwnedIcon {
                        handle,
                        ours: false,
                    }
                }
            };
            // The guard owns the icon from here on, so every later failure
            // path destroys it (ours only) instead of leaking it.
            let mut guard = Self {
                owner,
                added: false,
                icon,
            };
            // Register the shell-restart broadcast before the callback can
            // see it, so the subclass only ever reads the cached id.
            let _ = shell_restart_message();
            // SAFETY: the owner is on this thread; the procedure stores no
            // pointers, and Drop removes it before the app drops the window.
            if !unsafe { SetWindowSubclass(owner, Some(tray_callback), SUBCLASS_ID, 0) }.as_bool() {
                return Err(TrayError::CallbackRefused);
            }
            add_icon(owner, icon.handle)?;
            guard.added = true;
            Ok(guard)
        }

        /// Takes one coalesced right-click/context-menu request on the UI
        /// thread. This never opens a popup or calls application code.
        pub fn take_menu_request(&self) -> bool {
            take_request(&MENU_REQUESTS, self.owner)
        }

        /// Takes one coalesced left-click activation on the UI thread. A
        /// double click coalesces into this single request.
        pub fn take_left_click(&self) -> bool {
            take_request(&LEFT_CLICKS, self.owner)
        }

        /// Takes one coalesced `TaskbarCreated` request on the UI thread.
        pub fn take_shell_restart(&self) -> bool {
            take_request(&SHELL_RESTARTS, self.owner)
        }

        /// Re-adds the icon after the shell restarted, through the same
        /// `NIM_ADD` body [`TrayIcon::attach`] uses.
        ///
        /// The subclass survives a shell restart and is deliberately *not*
        /// reinstalled. The previous icon went away with the previous
        /// taskbar, so this creates no second icon. A refusal is reported and
        /// not retried.
        pub fn readd(&self) -> Result<(), TrayError> {
            add_icon(self.owner, self.icon.handle)
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
            clear_requests(self.owner);
            if self.added {
                let data = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: self.owner,
                    uID: TRAY_UID,
                    ..NOTIFYICONDATAW::default()
                };
                // SAFETY: same invariant as `attach`; the owner window still
                // exists because the guard never outlives it. A refused
                // delete is not reported: there is nothing left to clean
                // through.
                let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
                self.added = false;
            }
            // Only our own icon may be destroyed: a stock `IDI_APPLICATION`
            // handle is shared process-wide and `DestroyIcon` must never see
            // it. Removing the icon first means the shell no longer refers to
            // the handle.
            if self.icon.ours {
                // SAFETY: the handle came from our `CreateIconIndirect` and
                // is destroyed exactly once, here.
                let _ = unsafe { DestroyIcon(self.icon.handle) };
                self.icon.ours = false;
            }
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
    fn a_left_click_resolves_to_the_menus_show_hide_row() {
        assert_eq!(TRAY_LEFT_CLICK, TrayMenuEntry::ToggleMainWindow);
        assert_eq!(
            TRAY_LEFT_CLICK, TRAY_MENU[0],
            "the left click reuses the menu's show/hide row, not a new entry"
        );
        assert_eq!(
            TRAY_LEFT_CLICK.commands(),
            vec![Command::ToggleWindow(WindowId::Main)],
            "the left click invents no command"
        );
        assert_eq!(
            TRAY_LEFT_CLICK.commands(),
            menu_entry_for_id(TRAY_MENU, 1).unwrap().commands(),
            "both buttons reach show/hide through the same model entry"
        );
    }

    #[test]
    fn a_left_click_and_a_right_click_stay_distinct() {
        // The right click's own effect is opening the popup, which dispatches
        // nothing by itself: popup id 0 (no selection) is no command at all.
        assert_eq!(menu_entry_for_id(TRAY_MENU, 0), None);
        // And the left click is not any other row's command.
        for entry in TRAY_MENU.iter().filter(|e| **e != TRAY_LEFT_CLICK) {
            assert_ne!(
                entry.commands(),
                TRAY_LEFT_CLICK.commands(),
                "{entry:?} must not be reachable by a left click"
            );
        }
        // Quit in particular must never be a left click away.
        assert_ne!(TRAY_LEFT_CLICK, TrayMenuEntry::ExitPlayer);
    }

    #[test]
    fn the_shell_restart_predicate_ignores_an_unregistered_id_and_wm_null() {
        // WM_NULL is 0 and so is a failed `RegisterWindowMessageW`.
        const WM_NULL: u32 = 0;
        for message in [WM_NULL, 1, 0x8000, 0xC000, u32::MAX] {
            assert!(
                !is_shell_restart(0, message),
                "an unregistered id must match nothing, not even {message:#x}"
            );
        }
        for registered in [1u32, 0xC123, u32::MAX] {
            assert!(is_shell_restart(registered, registered));
            assert!(!is_shell_restart(registered, WM_NULL));
            assert!(!is_shell_restart(registered, registered.wrapping_add(1)));
        }
    }

    #[test]
    fn the_committed_artwork_has_fixed_dimensions_and_only_palette_glyphs() {
        assert_eq!(art::ROWS.len(), art::SOURCE_SIZE as usize);
        for (y, row) in art::ROWS.iter().enumerate() {
            assert_eq!(
                row.chars().count(),
                art::SOURCE_SIZE as usize,
                "row {y} is not {} glyphs wide",
                art::SOURCE_SIZE
            );
            for glyph in row.chars() {
                assert!(
                    art::colour_for(glyph).is_some(),
                    "row {y} uses the unknown glyph {glyph:?}"
                );
            }
        }
        // Every palette entry is actually drawn, so none is dead weight.
        for (glyph, _) in art::PALETTE {
            assert!(
                art::ROWS.iter().any(|row| row.contains(glyph)),
                "the palette entry {glyph:?} is never used"
            );
        }
        let mut colours: Vec<[u8; 4]> = art::PALETTE.iter().map(|(_, c)| *c).collect();
        colours.sort_unstable();
        colours.dedup();
        assert_eq!(colours.len(), art::PALETTE.len(), "no colour is duplicated");
    }

    #[test]
    fn the_artwork_is_transparent_at_the_corners_and_opaque_on_the_plate() {
        let pixels = art::source_bgra();
        let size = art::SOURCE_SIZE as usize;
        assert_eq!(pixels.len(), size * size * 4);
        let alpha_at = |x: usize, y: usize| pixels[(y * size + x) * 4 + 3];
        let bgra_at = |x: usize, y: usize| {
            let at = (y * size + x) * 4;
            [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
        };
        // The four rounded corners are cut away.
        for (x, y) in [(0, 0), (15, 0), (0, 15), (15, 15), (1, 0), (0, 1)] {
            assert_eq!(alpha_at(x, y), 0, "({x},{y}) must be transparent");
        }
        // The plate is opaque across its whole middle band.
        for y in 2..14 {
            for x in 0..size {
                assert_eq!(alpha_at(x, y), 0xFF, "({x},{y}) must be opaque");
            }
        }
        // The accent glyph lands where the triangle was drawn, in BGRA order.
        let [r, g, b, a] = PALETTE_ACCENT;
        assert_eq!(bgra_at(7, 7), [b, g, r, a], "the triangle's body");
        assert_ne!(bgra_at(2, 7), [b, g, r, a], "the plate's face");
        // Straight alpha: no transparent pixel carries colour, so the
        // straight and premultiplied forms of this art are identical.
        for chunk in pixels.as_chunks::<4>().0 {
            if chunk[3] == 0 {
                assert_eq!(&chunk[..3], &[0, 0, 0]);
            } else {
                assert_eq!(chunk[3], 0xFF, "the art is fully opaque or fully clear");
            }
        }
    }

    /// The accent colour as committed, `[R, G, B, A]`.
    const PALETTE_ACCENT: [u8; 4] = [0x6C, 0xD9, 0x4F, 0xFF];

    #[test]
    fn the_scaled_buffer_has_the_expected_length_and_keeps_the_glyph() {
        for size in [0u32, 1, 8, 16, 20, 24, 32, 48, 64] {
            let pixels = art::scaled_bgra(size);
            assert_eq!(
                pixels.len(),
                (size * size * 4) as usize,
                "a {size}px icon must be {size}*{size}*4 bytes"
            );
        }
        assert_eq!(art::scaled_bgra(art::SOURCE_SIZE), art::source_bgra());
        // Nearest-neighbour doubling: the centre stays the accent colour and
        // the corner stays transparent at 200 % DPI.
        let doubled = art::scaled_bgra(32);
        let at = |x: usize, y: usize| {
            let i = (y * 32 + x) * 4;
            [doubled[i], doubled[i + 1], doubled[i + 2], doubled[i + 3]]
        };
        let [r, g, b, a] = PALETTE_ACCENT;
        assert_eq!(at(15, 15), [b, g, r, a], "the triangle survives scaling");
        assert_eq!(at(0, 0), [0, 0, 0, 0], "the cut corner survives scaling");
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
