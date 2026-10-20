// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! VM-only helpers. Call the VM gate before either helper; neither is a GUI rail.

use std::time::{Duration, Instant};

/// winit permits only one EventLoop per process, even after it was dropped.
/// Run each lib-test probe in a fresh, exactly filtered harness process. The
/// parent still fails on timeout, panic, zero tests, or any child failure.
pub fn isolated(test: &str) -> bool {
    const MARKER: &str = "RUSTAMP_WINDOW_PROBE_CHILD";
    if std::env::var(MARKER).as_deref() == Ok(test) {
        return false;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .args([
            "--ignored",
            "--exact",
            test,
            "--nocapture",
            "--test-threads=1",
        ])
        .env(MARKER, test)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn isolated VM window probe");
    let deadline = Instant::now() + Duration::from_secs(30);
    let timed_out = loop {
        if child.try_wait().unwrap().is_some() {
            break false;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            break true;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !timed_out
            && output.status.success()
            && stdout.contains("running 1 test")
            && stdout.contains("1 passed"),
        "isolated {test}: timeout={timed_out}, status={}\n{stdout}\n{stderr}",
        output.status
    );
    eprintln!("isolated {test}:\n{stdout}\n{stderr}");
    true
}

#[cfg(windows)]
#[allow(unsafe_code)] // Test-only Win32 readback of a live system popup.
mod native {
    use super::*;
    use rustamp_platform::PopupRow;
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct MenuBarInfo {
        size: u32,
        rect: Rect,
        menu: *mut c_void,
        menu_window: *mut c_void,
        focused: i32,
    }

    #[repr(C)]
    struct MenuItemInfo {
        size: u32,
        mask: u32,
        kind: u32,
        state: u32,
        id: u32,
        submenu: *mut c_void,
        checked: *mut c_void,
        unchecked: *mut c_void,
        data: usize,
        text: *mut u16,
        text_len: u32,
        bitmap: *mut c_void,
    }

    #[test]
    fn popup_readback_layouts_match_the_win64_abi() {
        use std::mem::{offset_of, size_of};
        assert_eq!(size_of::<usize>(), 8, "the player is 64-bit");
        assert_eq!(size_of::<Rect>(), 16);
        assert_eq!(size_of::<MenuBarInfo>(), 48);
        assert_eq!(offset_of!(MenuBarInfo, menu), 24);
        assert_eq!(offset_of!(MenuBarInfo, focused), 40);
        assert_eq!(size_of::<MenuItemInfo>(), 80);
        assert_eq!(offset_of!(MenuItemInfo, submenu), 24);
        assert_eq!(offset_of!(MenuItemInfo, text), 56);
        assert_eq!(offset_of!(MenuItemInfo, bitmap), 72);
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumThreadWindows(
            thread: u32,
            callback: unsafe extern "system" fn(*mut c_void, isize) -> i32,
            data: isize,
        ) -> i32;
        fn GetWindowThreadProcessId(hwnd: *mut c_void, pid: *mut u32) -> u32;
        fn GetClassNameW(hwnd: *mut c_void, text: *mut u16, count: i32) -> i32;
        fn IsWindowVisible(hwnd: *mut c_void) -> i32;
        fn GetWindowRect(hwnd: *mut c_void, rect: *mut Rect) -> i32;
        fn GetWindowLongW(hwnd: *mut c_void, index: i32) -> i32;
        fn GetMenuBarInfo(hwnd: *mut c_void, object: i32, item: i32, info: *mut MenuBarInfo)
        -> i32;
        fn GetMenuItemCount(menu: *mut c_void) -> i32;
        fn GetMenuItemRect(hwnd: *mut c_void, menu: *mut c_void, item: u32, rect: *mut Rect)
        -> i32;
        fn GetMenuStringW(
            menu: *mut c_void,
            item: u32,
            text: *mut u16,
            count: i32,
            flags: u32,
        ) -> i32;
        fn GetMenuItemInfoW(
            menu: *mut c_void,
            item: u32,
            by_position: i32,
            info: *mut MenuItemInfo,
        ) -> i32;
        fn PostMessageW(hwnd: *mut c_void, message: u32, wparam: usize, lparam: isize) -> i32;
    }

    unsafe extern "system" fn collect(hwnd: *mut c_void, data: isize) -> i32 {
        // SAFETY: enumeration is synchronous and data is its live Vec.
        unsafe { &mut *(data as *mut Vec<isize>) }.push(hwnd as isize);
        1
    }

    fn measure(owner: isize, anchor: (i32, i32), rows: &[PopupRow]) -> Result<(), String> {
        let mut pid = 0;
        // SAFETY: owner stays alive until the observer is joined.
        let thread = unsafe { GetWindowThreadProcessId(owner as *mut c_void, &mut pid) };
        assert_ne!(thread, 0);
        let deadline = Instant::now() + Duration::from_secs(10);
        let popup = loop {
            let mut windows: Vec<isize> = Vec::new();
            // SAFETY: collect only pushes handles into the live Vec.
            unsafe { EnumThreadWindows(thread, collect, &mut windows as *mut _ as isize) };
            let mut found = Vec::new();
            for hwnd in windows {
                let mut class = [0u16; 256];
                // SAFETY: a live enumeration handle and a valid wide buffer.
                let n = unsafe {
                    GetClassNameW(hwnd as *mut c_void, class.as_mut_ptr(), class.len() as i32)
                };
                // SAFETY: only inspecting a handle returned by the OS.
                if String::from_utf16_lossy(&class[..n.max(0) as usize]) == "#32768"
                    && unsafe { IsWindowVisible(hwnd as *mut c_void) } != 0
                {
                    found.push(hwnd);
                }
            }
            if found.len() == 1 {
                break found[0] as *mut c_void;
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "expected one visible #32768 on owner thread {thread}, found {}",
                    found.len()
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut outer = Rect::default();
        // SAFETY: popup remains open until we post cancellation below.
        assert_ne!(unsafe { GetWindowRect(popup, &mut outer) }, 0);
        assert_eq!(
            (outer.left, outer.top),
            anchor,
            "the menu-button screen anchor"
        );
        // SAFETY: GWL_STYLE reads the system popup's style, no pointers cross.
        let style = unsafe { GetWindowLongW(popup, -16) } as u32;
        // WS_CAPTION is the combined WS_BORDER | WS_DLGFRAME mask. A system
        // menu may legitimately have its thin WS_BORDER without a caption.
        assert_ne!(
            style & 0x00c0_0000,
            0x00c0_0000,
            "no native caption on the menu"
        );
        assert_ne!(style & 0x8000_0000, 0, "a real WS_POPUP");
        let mut info = MenuBarInfo {
            size: std::mem::size_of::<MenuBarInfo>() as u32,
            rect: Rect::default(),
            menu: std::ptr::null_mut(),
            menu_window: std::ptr::null_mut(),
            focused: 0,
        };
        // SAFETY: OBJID_CLIENT is the documented object for a popup menu;
        // item 0 requests the whole menu, with a correctly sized out-buffer.
        assert_ne!(unsafe { GetMenuBarInfo(popup, -4, 0, &mut info) }, 0);
        assert!(!info.menu.is_null(), "the popup must carry a real HMENU");
        // SAFETY: the system owns this live HMENU while tracking.
        assert_eq!(unsafe { GetMenuItemCount(info.menu) }, rows.len() as i32);
        let mut previous_bottom = None;
        let mut union = Rect {
            left: i32::MAX,
            top: i32::MAX,
            right: i32::MIN,
            bottom: i32::MIN,
        };
        for (index, row) in rows.iter().enumerate() {
            let mut rect = Rect::default();
            // SAFETY: index is within the count checked above; the menu is live.
            // A popup menu permits a null HWND (unlike a window menu bar).
            assert_ne!(
                unsafe {
                    GetMenuItemRect(std::ptr::null_mut(), info.menu, index as u32, &mut rect)
                },
                0
            );
            assert!(
                rect.right > rect.left && rect.bottom > rect.top,
                "row {index}: {rect:?}"
            );
            assert!(
                rect.left >= outer.left
                    && rect.top >= outer.top
                    && rect.right <= outer.right
                    && rect.bottom <= outer.bottom,
                "row {index} outside popup: {rect:?}, {outer:?}"
            );
            if let Some(bottom) = previous_bottom {
                assert_eq!(rect.top, bottom, "rows must cover consecutive screen bands");
            }
            previous_bottom = Some(rect.bottom);
            union.left = union.left.min(rect.left);
            union.top = union.top.min(rect.top);
            union.right = union.right.max(rect.right);
            union.bottom = union.bottom.max(rect.bottom);
            let mut item = MenuItemInfo {
                size: std::mem::size_of::<MenuItemInfo>() as u32,
                mask: 0x105, // MIIM_FTYPE | MIIM_STATE | MIIM_SUBMENU
                kind: 0,
                state: 0,
                id: 0,
                submenu: std::ptr::null_mut(),
                checked: std::ptr::null_mut(),
                unchecked: std::ptr::null_mut(),
                data: 0,
                text: std::ptr::null_mut(),
                text_len: 0,
                bitmap: std::ptr::null_mut(),
            };
            // SAFETY: by-position query, valid menu/index and sized out-buffer.
            assert_ne!(
                unsafe { GetMenuItemInfoW(info.menu, index as u32, 1, &mut item) },
                0
            );
            assert_eq!(item.kind & 0x100, 0, "no owner-drawn bitmap text");
            assert_eq!(item.kind & 0x800 != 0, matches!(row, PopupRow::Separator));
            if matches!(row, PopupRow::Separator) {
                continue;
            }
            let (label, accelerator, submenu, checked) = match row {
                PopupRow::Entry {
                    label,
                    accelerator,
                    checked,
                    ..
                } => (label, accelerator.as_deref(), false, *checked),
                PopupRow::Submenu { label, .. } => (label, None, true, false),
                PopupRow::Separator => unreachable!(),
            };
            assert_eq!(!item.submenu.is_null(), submenu, "submenu arrow on {label}");
            assert_eq!(item.state & 8 != 0, checked, "tick on {label}");
            let mut text = [0u16; 256];
            // SAFETY: the wide buffer is valid and the menu entry is live.
            let n = unsafe {
                GetMenuStringW(
                    info.menu,
                    index as u32,
                    text.as_mut_ptr(),
                    text.len() as i32,
                    0x400,
                )
            };
            assert_eq!(
                String::from_utf16_lossy(&text[..n.max(0) as usize]),
                rustamp_platform::item_text(label, accelerator)
            );
        }
        // Measure the actual screen footprint from both independent OS APIs,
        // not the retired 150%-DPI screenshot's hard-coded 323 x 569 window.
        assert!(union.right > union.left && union.bottom > union.top);
        assert!(
            info.rect.left <= union.left
                && info.rect.top <= union.top
                && info.rect.right >= union.right
                && info.rect.bottom >= union.bottom,
            "menu bar bounds must contain every rendered row: {:?}, {union:?}",
            info.rect
        );
        eprintln!(
            "native popup: screen={outer:?}, menu={:?}, rows={union:?}",
            info.rect
        );
        Ok(())
    }

    /// Observe the menu on its actual owner thread, then dismiss it even when
    /// an assertion panics, so a failed measurement cannot strand the modal UI.
    pub fn observe_popup(
        owner: isize,
        anchor: (i32, i32),
        rows: Vec<PopupRow>,
    ) -> std::thread::JoinHandle<Result<(), String>> {
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| measure(owner, anchor, &rows));
            // SAFETY: the owner is retained by the caller until join; only a
            // pointer-free WM_CANCELMODE is posted, on success or failure.
            let cancelled = unsafe { PostMessageW(owner as *mut c_void, 0x001f, 0, 0) };
            assert_ne!(cancelled, 0, "could not dismiss the native popup");
            result.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        })
    }
}

#[cfg(windows)]
pub use native::observe_popup;
