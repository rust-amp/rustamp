//! The frame a plug-in embeds its window in (IPC 505, S10 §13).
//!
//! The answer to IPC 505 is a function the plug-in calls with a state block; the
//! function makes a frame window and returns it, and the plug-in creates its own
//! window as a child of the frame. `vis_avs` has no other way to get a window:
//! without a frame its child window has no parent and cannot be created.
//!
//! The block's layout is what `vis_avs.dll` reads and writes (static reading, the
//! owner's approval of 2026-10-31): the frame at 0, flags at 4, the placement as
//! left, top, right, bottom at 8.
//!
//! Own choice: the frame is a plain system window, not yet the skin's `gen.bmp`
//! frame of S11, which also has no rectangles to draw from.

#[cfg(windows)]
use rustamp_plugin_abi::common::CLASS_GEN;

/// The state block's head. The plug-in owns the block.
#[repr(C)]
pub struct EmbedState {
    /// The frame, written here.
    pub frame: usize,
    /// The plug-in's flags.
    pub flags: i32,
    /// The placement the plug-in asks for.
    pub left: i32,
    /// See `left`.
    pub top: i32,
    /// See `left`.
    pub right: i32,
    /// See `left`.
    pub bottom: i32,
}

/// The frame's client size when a plug-in asks for none (own choice: the
/// default of the classic visualization window).
const DEFAULT_SIZE: (i32, i32) = (275, 348);

/// The size to open with, from the placement the plug-in asks for.
fn frame_size(left: i32, top: i32, right: i32, bottom: i32) -> (i32, i32) {
    let (width, height) = (right.saturating_sub(left), bottom.saturating_sub(top));
    if width > 0 && height > 0 {
        (width, height)
    } else {
        DEFAULT_SIZE
    }
}

/// The function IPC 505 answers with.
#[cfg(windows)]
pub fn embed_function() -> usize {
    embed_window as unsafe extern "C" fn(*mut EmbedState) -> usize as usize
}

/// Without a window system there is nothing to embed in.
#[cfg(not(windows))]
pub fn embed_function() -> usize {
    0
}

/// IPC 505's other form: the parameter is the state block, and the answer the
/// frame.
#[cfg(windows)]
pub fn embed_now(state: usize) -> usize {
    // SAFETY: the plug-in passes its own block, as for the function it can ask
    // for instead.
    unsafe { embed_window(state as *mut EmbedState) }
}

/// See the Windows form.
#[cfg(not(windows))]
pub fn embed_now(_state: usize) -> usize {
    0
}

#[cfg(windows)]
unsafe extern "system" fn frame_proc(
    window: windows::Win32::Foundation::HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, GW_CHILD, GetClientRect, GetWindow, MoveWindow, SendMessageW, WM_CLOSE,
        WM_CREATE, WM_PARENTNOTIFY, WM_SIZE,
    };
    // The plug-in decides what closing means: `vis_avs` stops and destroys the
    // frame itself. A frame without a child closes as any window does.
    if message == WM_CLOSE {
        // SAFETY: our own window.
        if let Ok(child) = unsafe { GetWindow(window, GW_CHILD) } {
            // SAFETY: the plug-in's child of our own window.
            unsafe { SendMessageW(child, WM_CLOSE, None, None) };
            return windows::Win32::Foundation::LRESULT(0);
        }
    }
    // A plug-in creates its window after the frame, at a size of its own: the
    // birth of the child is the frame's first chance to fit it.
    let child_born = message == WM_PARENTNOTIFY && (wparam.0 & 0xFFFF) as u32 == WM_CREATE;
    if message == WM_SIZE || child_born {
        let mut client = RECT::default();
        // SAFETY: our own window, a writable rectangle.
        if unsafe { GetClientRect(window, &mut client) }.is_ok()
            && let Ok(child) = unsafe { GetWindow(window, GW_CHILD) }
        {
            // SAFETY: the plug-in's child of our own window. A failed move
            // leaves the child where it was.
            let _ = unsafe { MoveWindow(child, 0, 0, client.right, client.bottom, true) };
        }
    }
    // SAFETY: the default handling of a message of our own window.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// Makes the frame, writes it into the block and returns it; 0 when it cannot.
///
/// # Safety
///
/// `state` is the plug-in's block, null or writable for the head above.
#[cfg(windows)]
unsafe extern "C" fn embed_window(state: *mut EmbedState) -> usize {
    use windows::Win32::Foundation::{HINSTANCE, RECT};
    use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH};
    use windows::Win32::UI::WindowsAndMessaging::{
        AdjustWindowRect, CW_USEDEFAULT, CreateWindowExW, RegisterClassW, WINDOW_EX_STYLE,
        WNDCLASSW, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };
    use windows::core::{PCWSTR, w};

    if state.is_null() {
        return 0;
    }
    let class: Vec<u16> = CLASS_GEN
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| u16::from(*byte))
        .chain(Some(0))
        .collect();
    let description = WNDCLASSW {
        lpfnWndProc: Some(frame_proc),
        hInstance: HINSTANCE(std::ptr::null_mut()),
        // SAFETY: a stock object, which needs no release.
        hbrBackground: HBRUSH(unsafe { GetStockObject(BLACK_BRUSH) }.0),
        lpszClassName: PCWSTR(class.as_ptr()),
        ..Default::default()
    };
    // SAFETY: the name lives through the call. Registering twice answers 0 and
    // leaves the first class in place, which is the one wanted.
    let _ = unsafe { RegisterClassW(&description) };

    // SAFETY: the caller's block.
    let (x, y, width, height) = unsafe {
        let block = &*state;
        let (width, height) = frame_size(block.left, block.top, block.right, block.bottom);
        (block.left, block.top, width, height)
    };
    let style = WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN;
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    // SAFETY: a writable rectangle. On failure the client size stands in.
    let _ = unsafe { AdjustWindowRect(&mut outer, style, false) };
    let placed = x > 0 || y > 0;
    // SAFETY: the class and title live through the call. The frame belongs to
    // the calling thread, which is the plug-in's and pumps its messages.
    let frame = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            w!("Visualization"),
            style,
            if placed { x } else { CW_USEDEFAULT },
            if placed { y } else { CW_USEDEFAULT },
            outer.right - outer.left,
            outer.bottom - outer.top,
            None,
            None,
            None,
            None,
        )
    };
    let Ok(frame) = frame else {
        return 0;
    };
    let handle = frame.0 as usize;
    // SAFETY: the caller's block.
    unsafe { (*state).frame = handle };
    handle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_opens_at_what_the_plugin_asks_for_or_the_default() {
        assert_eq!(frame_size(10, 20, 410, 320), (400, 300));
        assert_eq!(frame_size(0, 0, 0, 0), DEFAULT_SIZE);
        assert_eq!(frame_size(50, 50, 40, 60), DEFAULT_SIZE);
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn the_head_of_the_block_has_the_layout_the_plugins_read() {
        assert_eq!(core::mem::offset_of!(EmbedState, frame), 0);
        assert_eq!(core::mem::offset_of!(EmbedState, flags), 4);
        assert_eq!(core::mem::offset_of!(EmbedState, left), 8);
    }
}
