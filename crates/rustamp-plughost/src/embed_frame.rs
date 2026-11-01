// The sprite rectangles and the frame's arrangement in this file are ported from
// webamp, commit 79117c66d79fdac690fc732a0ce8df543ccd97a0
// (packages/webamp/js/skinSprites.ts, skinParser.js, constants.ts,
// components/GenWindow/index.tsx, css/gen-window.css):
//
// MIT License
//
// Copyright (c) 2018 Jordan Eldredge
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

//! The frame a plug-in embeds its window in (IPC 505, S10 §13).
//!
//! The answer to IPC 505 is a function the plug-in calls with a state block; the
//! function makes a frame window and returns it, and the plug-in creates its own
//! window as a child of the frame. `vis_avs` has no other way to get a window:
//! without a frame its child window has no parent and cannot be created.
//!
//! The block's layout: the frame at 0, flags at 4, the placement as
//! left, top, right, bottom at 8.
//!
//! # The skin
//!
//! The frame is drawn from the skin's `gen.bmp`, which the player hands over as
//! pixels ([`rustamp_ipc::Message::EmbedSkin`]): the player decodes skins and
//! this process only draws, so the host needs neither the skin's file formats
//! nor their code. The placement a plug-in asks for is the frame's outer
//! rectangle, and the plug-in's window fills what the chrome leaves.
//!
//! A process the player sends no sheet (the helpers of the audio path), and a
//! skin without the sheet, get a plain system window. A frame keeps the kind it
//! was created with; a skinned one takes a new sheet at once.
//!
//! Own choices: the frame sizes in the steps and from the minimum of the sheet's
//! tiles (webamp's), and the lower right corner is the handle for it.

use std::sync::{Arc, Mutex};

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

/// The frame's size when a plug-in asks for none (own choice: the default of the
/// classic visualization window).
const DEFAULT_SIZE: (i32, i32) = (275, 348);

/// The smallest skinned frame.
const MIN_SIZE: (i32, i32) = (275, 116);
/// A skinned frame's size changes in these steps.
const SIZE_STEP: (i32, i32) = (25, 29);

/// A rectangle of the sheet: x, y, width, height.
type Rect = (i32, i32, i32, i32);

const TOP_LEFT: [Rect; 2] = [(0, 21, 25, 20), (0, 0, 25, 20)];
const TOP_LEFT_END: [Rect; 2] = [(26, 21, 25, 20), (26, 0, 25, 20)];
const TOP_CENTER_FILL: [Rect; 2] = [(52, 21, 25, 20), (52, 0, 25, 20)];
const TOP_RIGHT_END: [Rect; 2] = [(78, 21, 25, 20), (78, 0, 25, 20)];
const TOP_FILL: [Rect; 2] = [(104, 21, 25, 20), (104, 0, 25, 20)];
const TOP_RIGHT: [Rect; 2] = [(130, 21, 25, 20), (130, 0, 25, 20)];
const BOTTOM_LEFT: Rect = (0, 42, 125, 14);
const BOTTOM_RIGHT: Rect = (0, 57, 125, 14);
const BOTTOM_FILL: Rect = (127, 72, 25, 14);
const MIDDLE_LEFT: Rect = (127, 42, 11, 29);
const MIDDLE_LEFT_BOTTOM: Rect = (158, 42, 11, 24);
const MIDDLE_RIGHT: Rect = (139, 42, 8, 29);
const MIDDLE_RIGHT_BOTTOM: Rect = (170, 42, 8, 24);
const CLOSE_PRESSED: Rect = (148, 42, 9, 9);

const TOP_HEIGHT: i32 = 20;
const BOTTOM_HEIGHT: i32 = 14;
const LEFT_WIDTH: i32 = 11;
const RIGHT_WIDTH: i32 = 8;
const CORNER_WIDTH: i32 = 25;
/// The close button's place from the top right corner, and its size.
const CLOSE_FROM_RIGHT: i32 = 11;
const CLOSE_TOP: i32 = 3;
const CLOSE_SIZE: i32 = 9;
/// The size of the corner that resizes.
const RESIZE_CORNER: i32 = 20;

/// The rows of the sheet that hold the title's letters, per activation.
const LETTER_ROWS: [i32; 2] = [96, 88];
const LETTER_HEIGHT: i32 = 7;
/// The width of what has no letter.
const SPACE_WIDTH: i32 = 5;
const TITLE_TOP: i32 = 2;
const TITLE_PADDING: (i32, i32) = (4, 3);

/// The skin's sheet, with the letters found in it.
struct Sheet {
    width: i32,
    height: i32,
    rgba: Vec<u8>,
    /// Where each of A to Z sits, per activation: x and width.
    letters: [[(i32, i32); 26]; 2],
}

impl Sheet {
    fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        let (width, height) = (i32::try_from(width).ok()?, i32::try_from(height).ok()?);
        let expected = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?
            .checked_mul(4)?;
        if expected == 0 || rgba.len() != expected {
            return None;
        }
        let mut sheet = Sheet {
            width,
            height,
            rgba,
            letters: [[(0, 0); 26]; 2],
        };
        sheet.letters = [
            sheet.letters_in_row(LETTER_ROWS[0]),
            sheet.letters_in_row(LETTER_ROWS[1]),
        ];
        Some(sheet)
    }

    fn pixel(&self, x: i32, y: i32) -> Option<&[u8]> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return None;
        }
        let at = (y as usize * self.width as usize + x as usize) * 4;
        self.rgba.get(at..at + 4)
    }

    /// The letters stand side by side in a row, each followed by one pixel of
    /// the colour the row starts with.
    fn letters_in_row(&self, y: i32) -> [(i32, i32); 26] {
        let mut letters = [(0, 0); 26];
        let background = self.pixel(0, y);
        let mut x = 1;
        for letter in &mut letters {
            let mut end = x;
            while end < self.width && self.pixel(end, y) != background {
                end += 1;
            }
            *letter = (x, end - x);
            x = end + 1;
        }
        letters
    }

    /// Where `letter` sits in the sheet, when it is a letter of the row.
    fn letter(&self, letter: char, active: bool) -> Option<Rect> {
        let index = (letter.to_ascii_uppercase() as usize).checked_sub('A' as usize)?;
        let (x, width) = *self.letters[usize::from(active)].get(index)?;
        (width > 0).then_some((x, LETTER_ROWS[usize::from(active)], width, LETTER_HEIGHT))
    }

    fn title_width(&self, title: &str, active: bool) -> i32 {
        title
            .chars()
            .map(|c| self.letter(c, active).map_or(SPACE_WIDTH, |rect| rect.2))
            .sum()
    }
}

/// A picture of a frame: `0x00RRGGBB` per pixel, top row first.
struct Canvas {
    width: i32,
    height: i32,
    pixels: Vec<u32>,
}

impl Canvas {
    fn new(width: i32, height: i32) -> Self {
        let count = width.max(0) as usize * height.max(0) as usize;
        Canvas {
            width,
            height,
            pixels: vec![0; count],
        }
    }

    /// Copies `from` to (`x`, `y`). Both sides clip: a skin's sheet may be
    /// smaller than the classic one.
    fn blit(&mut self, sheet: &Sheet, from: Rect, x: i32, y: i32) {
        let (from_x, from_y, width, height) = from;
        for row in 0..height {
            for column in 0..width {
                let (to_x, to_y) = (x + column, y + row);
                if to_x < 0 || to_y < 0 || to_x >= self.width || to_y >= self.height {
                    continue;
                }
                if let Some(pixel) = sheet.pixel(from_x + column, from_y + row) {
                    let at = to_y as usize * self.width as usize + to_x as usize;
                    self.pixels[at] =
                        u32::from(pixel[0]) << 16 | u32::from(pixel[1]) << 8 | u32::from(pixel[2]);
                }
            }
        }
    }

    /// Tiles `from` over the columns `x..end`, cutting the last tile.
    fn tile_across(&mut self, sheet: &Sheet, from: Rect, x: i32, end: i32, y: i32) {
        let mut at = x;
        while at < end {
            let width = from.2.min(end - at);
            self.blit(sheet, (from.0, from.1, width, from.3), at, y);
            at += from.2;
        }
    }

    /// Tiles `from` over the rows `y..end`, cutting the last tile.
    fn tile_down(&mut self, sheet: &Sheet, from: Rect, x: i32, y: i32, end: i32) {
        let mut at = y;
        while at < end {
            let height = from.3.min(end - at);
            self.blit(sheet, (from.0, from.1, from.2, height), x, at);
            at += from.3;
        }
    }
}

/// Draws the frame's chrome. What the plug-in's window covers stays black.
fn draw_frame(
    sheet: &Sheet,
    width: i32,
    height: i32,
    active: bool,
    title: &str,
    close_pressed: bool,
) -> Canvas {
    let mut canvas = Canvas::new(width, height);
    let state = usize::from(active);

    // The top: the title stands in the middle between two tiled stretches.
    let inner = (width - 4 * CORNER_WIDTH).max(0);
    let title_width = title_area_width(sheet.title_width(title, active), inner);
    let fill_left = (inner - title_width) / 2;
    let left_end = CORNER_WIDTH + fill_left;
    let title_at = left_end + CORNER_WIDTH;
    let right_end = title_at + title_width;
    canvas.blit(sheet, TOP_LEFT[state], 0, 0);
    canvas.tile_across(sheet, TOP_FILL[state], CORNER_WIDTH, left_end, 0);
    canvas.blit(sheet, TOP_LEFT_END[state], left_end, 0);
    canvas.tile_across(sheet, TOP_CENTER_FILL[state], title_at, right_end, 0);
    canvas.blit(sheet, TOP_RIGHT_END[state], right_end, 0);
    canvas.tile_across(
        sheet,
        TOP_FILL[state],
        right_end + CORNER_WIDTH,
        width - CORNER_WIDTH,
        0,
    );
    canvas.blit(sheet, TOP_RIGHT[state], width - CORNER_WIDTH, 0);
    let mut x = title_at + TITLE_PADDING.0;
    for letter in title.chars() {
        match sheet.letter(letter, active) {
            Some(rect) if x + rect.2 <= right_end - TITLE_PADDING.1 => {
                canvas.blit(sheet, rect, x, TITLE_TOP);
                x += rect.2;
            }
            Some(_) => break,
            None => x += SPACE_WIDTH,
        }
    }

    // The sides, with their lower ends at the bottom.
    let (middle_end, side_bottom) = (height - BOTTOM_HEIGHT, MIDDLE_LEFT_BOTTOM.3);
    canvas.tile_down(sheet, MIDDLE_LEFT, 0, TOP_HEIGHT, middle_end - side_bottom);
    canvas.blit(sheet, MIDDLE_LEFT_BOTTOM, 0, middle_end - side_bottom);
    let right = width - RIGHT_WIDTH;
    canvas.tile_down(
        sheet,
        MIDDLE_RIGHT,
        right,
        TOP_HEIGHT,
        middle_end - side_bottom,
    );
    canvas.blit(sheet, MIDDLE_RIGHT_BOTTOM, right, middle_end - side_bottom);

    // The bottom.
    canvas.tile_across(
        sheet,
        BOTTOM_FILL,
        BOTTOM_LEFT.2,
        width - BOTTOM_RIGHT.2,
        middle_end,
    );
    canvas.blit(sheet, BOTTOM_LEFT, 0, middle_end);
    canvas.blit(sheet, BOTTOM_RIGHT, width - BOTTOM_RIGHT.2, middle_end);

    if close_pressed {
        canvas.blit(sheet, CLOSE_PRESSED, width - CLOSE_FROM_RIGHT, CLOSE_TOP);
    }
    canvas
}

/// The width the title's tiles take for a text of `text` pixels, in `inner`.
fn title_area_width(text: i32, inner: i32) -> i32 {
    (text + TITLE_PADDING.0 + TITLE_PADDING.1).min(inner)
}

/// What the plug-in's window fills in a skinned frame: left, top, width, height.
fn content_rect(width: i32, height: i32) -> (i32, i32, i32, i32) {
    (
        LEFT_WIDTH,
        TOP_HEIGHT,
        (width - LEFT_WIDTH - RIGHT_WIDTH).max(0),
        (height - TOP_HEIGHT - BOTTOM_HEIGHT).max(0),
    )
}

/// What a point of a skinned frame does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Close,
    Caption,
    Resize,
    Inside,
}

fn hit_test(width: i32, height: i32, x: i32, y: i32) -> Hit {
    let close_x = width - CLOSE_FROM_RIGHT;
    if (close_x..close_x + CLOSE_SIZE).contains(&x)
        && (CLOSE_TOP..CLOSE_TOP + CLOSE_SIZE).contains(&y)
    {
        return Hit::Close;
    }
    if x >= width - RESIZE_CORNER && y >= height - RESIZE_CORNER {
        return Hit::Resize;
    }
    let content = content_rect(width, height);
    let in_content = (content.0..content.0 + content.2).contains(&x)
        && (content.1..content.1 + content.3).contains(&y);
    if in_content {
        Hit::Inside
    } else {
        Hit::Caption
    }
}

/// The nearest size a skinned frame takes for a wish.
fn snapped_size(width: i32, height: i32) -> (i32, i32) {
    let snap = |wish: i32, least: i32, step: i32| {
        least + ((wish.max(least) - least + step / 2) / step) * step
    };
    (
        snap(width, MIN_SIZE.0, SIZE_STEP.0),
        snap(height, MIN_SIZE.1, SIZE_STEP.1),
    )
}

/// The sheet frames draw from, and the frames that show it.
static SKIN: Mutex<Option<Arc<Sheet>>> = Mutex::new(None);
#[cfg(windows)]
static FRAMES: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// The frame's own message that fits the child once more, after a child's
/// creation returned (`WM_APP + 0x51`, reserved for this module).
#[cfg(windows)]
const FIT_AGAIN: u32 = 0x8000 + 0x51;

fn current_sheet() -> Option<Arc<Sheet>> {
    SKIN.lock().ok().and_then(|skin| skin.clone())
}

/// Takes the skin's sheet: empty pixels mean the skin has none. Runs on the
/// thread of the frames.
pub fn set_skin(width: u32, height: u32, rgba: Vec<u8>) {
    let sheet = Sheet::new(width, height, rgba).map(Arc::new);
    let have_sheet = sheet.is_some();
    if let Ok(mut skin) = SKIN.lock() {
        *skin = sheet;
    }
    #[cfg(windows)]
    if let Ok(frames) = FRAMES.lock() {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Gdi::InvalidateRect;
        for frame in frames.iter() {
            let window = HWND(*frame as *mut _);
            // A frame opened before the sheet existed still wears the system
            // frame; it takes the skinned kind now, before it repaints.
            if have_sheet {
                win::make_skinned(window);
            }
            // SAFETY: a frame of this thread; a frame's end takes it off the list.
            let _ = unsafe { InvalidateRect(Some(window), None, false) };
        }
    }
}

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
mod win {
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, DIB_RGB_COLORS, EndPaint, InvalidateRect,
        PAINTSTRUCT, RGBQUAD, SetDIBitsToDevice,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetActiveWindow, ReleaseCapture, SetCapture,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, GW_CHILD, GWL_STYLE, GWL_USERDATA, GetClientRect, GetWindow,
        GetWindowLongW, GetWindowRect, GetWindowTextW, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT,
        MINMAXINFO, MoveWindow, PostMessageW, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, SetWindowLongW, SetWindowPos, WM_ACTIVATE, WM_CLOSE, WM_CREATE,
        WM_ERASEBKGND, WM_GETMINMAXINFO, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_NCCALCSIZE, WM_NCDESTROY,
        WM_NCHITTEST, WM_PAINT, WM_PARENTNOTIFY, WM_SETTEXT, WM_SIZE, WM_SIZING,
        WM_WINDOWPOSCHANGED, WS_CLIPCHILDREN, WS_POPUP, WS_THICKFRAME, WS_VISIBLE,
    };

    use super::{
        FRAMES, Hit, MIN_SIZE, content_rect, current_sheet, draw_frame, hit_test, snapped_size,
    };

    /// A frame that draws itself is the one made without a system caption.
    fn is_skinned(window: HWND) -> bool {
        // SAFETY: our own window.
        let style = unsafe { GetWindowLongW(window, GWL_STYLE) } as u32;
        style & WS_POPUP.0 != 0
    }

    fn size_of(window: HWND) -> (i32, i32) {
        let mut client = RECT::default();
        // SAFETY: our own window, a writable rectangle. On failure the size is 0.
        let _ = unsafe { GetClientRect(window, &mut client) };
        (client.right, client.bottom)
    }

    /// The client coordinates of a mouse message's parameter.
    fn client_point(lparam: LPARAM) -> (i32, i32) {
        (
            i32::from((lparam.0 & 0xFFFF) as u16 as i16),
            i32::from(((lparam.0 >> 16) & 0xFFFF) as u16 as i16),
        )
    }

    fn paint(window: HWND) {
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: our own window, in its paint message.
        let device = unsafe { BeginPaint(window, &mut paint) };
        let (width, height) = size_of(window);
        if let Some(sheet) = current_sheet()
            && width > 0
            && height > 0
        {
            let mut title = [0u16; 256];
            // SAFETY: our own window, a writable buffer.
            let length = unsafe { GetWindowTextW(window, &mut title) };
            let title = String::from_utf16_lossy(&title[..length.max(0) as usize]);
            // SAFETY: plain queries of our own window.
            let active = unsafe { GetActiveWindow() } == window;
            let pressed = unsafe { GetWindowLongW(window, GWL_USERDATA) } != 0;
            let canvas = draw_frame(&sheet, width, height, active, &title, pressed);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of_header(),
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                bmiColors: [RGBQUAD::default()],
            };
            // SAFETY: the pixels hold `width` x `height` values and live through
            // the call; the device is the paint message's.
            unsafe {
                SetDIBitsToDevice(
                    device,
                    0,
                    0,
                    width as u32,
                    height as u32,
                    0,
                    0,
                    0,
                    height as u32,
                    canvas.pixels.as_ptr().cast(),
                    &info,
                    DIB_RGB_COLORS,
                );
            }
        }
        // SAFETY: the pair of the call above.
        let _ = unsafe { EndPaint(window, &paint) };
    }

    fn size_of_header() -> u32 {
        std::mem::size_of::<BITMAPINFOHEADER>() as u32
    }

    /// Puts the plug-in's window where the chrome leaves room.
    ///
    /// `child` is the window the caller already knows about (a child's birth
    /// notice names it); without one the frame's first child is used.
    fn fit_child(window: HWND, child: Option<HWND>) {
        let (width, height) = size_of(window);
        let (x, y, width, height) = if is_skinned(window) {
            content_rect(width, height)
        } else {
            (0, 0, width, height)
        };
        let child = match child {
            Some(child) => child,
            // SAFETY: our own window.
            None => match unsafe { GetWindow(window, GW_CHILD) } {
                Ok(child) => child,
                Err(_) => return,
            },
        };
        // SAFETY: the plug-in's child of our own window. A failed move leaves
        // the child where it was.
        let _ = unsafe { MoveWindow(child, x, y, width, height, true) };
    }

    /// Gives a frame that was made before the sheet arrived the skinned kind.
    ///
    /// A skin can reach the host after a visualization window is already open
    /// (the player sends its sheet when it starts and when the skin changes),
    /// and the kind of a window cannot be chosen later by itself: the style is
    /// what `is_skinned` answers, so the frame is switched over and fitted
    /// again before it repaints.
    pub(super) fn make_skinned(window: HWND) {
        if is_skinned(window) || current_sheet().is_none() {
            return;
        }
        // SAFETY: our own window, on its own thread.
        unsafe {
            let style = WS_POPUP.0 | WS_THICKFRAME.0 | WS_VISIBLE.0 | WS_CLIPCHILDREN.0;
            SetWindowLongW(window, GWL_STYLE, style as i32);
            let _ = SetWindowPos(
                window,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            );
        }
        fit_child(window, None);
    }

    /// What a message means to a frame that draws itself; `None` passes it on.
    fn skinned_message(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        match message {
            // The whole window is client area: the sheet draws the chrome.
            WM_NCCALCSIZE if wparam.0 != 0 => Some(LRESULT(0)),
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_PAINT => {
                paint(window);
                Some(LRESULT(0))
            }
            WM_ACTIVATE | WM_SETTEXT => {
                // SAFETY: our own window.
                let _ = unsafe { InvalidateRect(Some(window), None, false) };
                None
            }
            WM_NCHITTEST => {
                let mut outer = RECT::default();
                // SAFETY: our own window, a writable rectangle.
                let _ = unsafe { GetWindowRect(window, &mut outer) };
                let (x, y) = client_point(lparam);
                let hit = hit_test(
                    outer.right - outer.left,
                    outer.bottom - outer.top,
                    x - outer.left,
                    y - outer.top,
                );
                Some(LRESULT(match hit {
                    Hit::Close | Hit::Inside => HTCLIENT,
                    Hit::Caption => HTCAPTION,
                    Hit::Resize => HTBOTTOMRIGHT,
                } as isize))
            }
            WM_LBUTTONDOWN => {
                let (width, height) = size_of(window);
                let (x, y) = client_point(lparam);
                if hit_test(width, height, x, y) == Hit::Close {
                    // SAFETY: our own window, on its own thread.
                    unsafe {
                        SetWindowLongW(window, GWL_USERDATA, 1);
                        SetCapture(window);
                        let _ = InvalidateRect(Some(window), None, false);
                    }
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                // SAFETY: our own window, on its own thread.
                let pressed = unsafe { GetWindowLongW(window, GWL_USERDATA) } != 0;
                if pressed {
                    let (width, height) = size_of(window);
                    let (x, y) = client_point(lparam);
                    // SAFETY: as above. A failed post leaves the frame open.
                    unsafe {
                        SetWindowLongW(window, GWL_USERDATA, 0);
                        let _ = ReleaseCapture();
                        let _ = InvalidateRect(Some(window), None, false);
                        if hit_test(width, height, x, y) == Hit::Close {
                            let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
                        }
                    }
                }
                Some(LRESULT(0))
            }
            WM_GETMINMAXINFO => {
                // SAFETY: Windows hands a writable block with this message.
                unsafe {
                    let info = lparam.0 as *mut MINMAXINFO;
                    (*info).ptMinTrackSize = POINT {
                        x: MIN_SIZE.0,
                        y: MIN_SIZE.1,
                    };
                }
                Some(LRESULT(0))
            }
            WM_SIZING => {
                // SAFETY: Windows hands a writable rectangle with this message.
                unsafe {
                    let wish = lparam.0 as *mut RECT;
                    let (width, height) =
                        snapped_size((*wish).right - (*wish).left, (*wish).bottom - (*wish).top);
                    (*wish).right = (*wish).left + width;
                    (*wish).bottom = (*wish).top + height;
                }
                Some(LRESULT(1))
            }
            _ => None,
        }
    }

    pub(super) unsafe extern "system" fn frame_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // A panic must not unwind through the system's callback boundary: that
        // aborts the whole host and takes every loaded plug-in with it. A
        // message that fails leaves the frame as it was.
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: the caller's own arguments, passed on unchanged.
            unsafe { frame_message(window, message, wparam, lparam) }
        }));
        match answer {
            Ok(answer) => answer,
            Err(_) => {
                crate::fault::trace_event(
                    "a message of an embed frame panicked; the frame keeps its last picture",
                );
                // SAFETY: the default handling of a message of our own window.
                unsafe { DefWindowProcW(window, message, wparam, lparam) }
            }
        }
    }

    /// One message of a frame, with the panic boundary taken off.
    ///
    /// # Safety
    ///
    /// Win32 passes a message of this frame.
    unsafe fn frame_message(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // The plug-in decides what closing means: `vis_avs` stops and destroys
        // the frame itself. A frame without a child closes as any window does.
        if message == WM_CLOSE {
            // SAFETY: our own window.
            if let Ok(child) = unsafe { GetWindow(window, GW_CHILD) } {
                // SAFETY: the plug-in's child of our own window. Posted, not
                // sent: `vis_avs` destroys the frame from inside its own close
                // handling, and a sent message would return into a dead window.
                unsafe {
                    let _ = PostMessageW(Some(child), WM_CLOSE, WPARAM(0), LPARAM(0));
                };
                return LRESULT(0);
            }
        }
        if message == WM_NCDESTROY
            && let Ok(mut frames) = FRAMES.lock()
        {
            frames.retain(|frame| *frame != window.0 as usize);
        }
        // A plug-in creates its window after the frame, at a size of its own:
        // the birth of the child is the frame's first chance to fit it, and its
        // own handle comes with that notice. A host-side size change, a move
        // that changes the client and the frame's own fit message follow.
        let child_born = message == WM_PARENTNOTIFY && (wparam.0 & 0xFFFF) as u32 == WM_CREATE;
        let born_child = if child_born {
            Some(HWND(lparam.0 as *mut _))
        } else {
            None
        };
        if message == WM_SIZE
            || message == WM_WINDOWPOSCHANGED
            || message == super::FIT_AGAIN
            || child_born
        {
            fit_child(window, born_child);
        }
        if child_born {
            // Fit once more after the plug-in's own creation has returned: a
            // child is still being made while its parent's notice runs.
            // SAFETY: our own window.
            unsafe {
                let _ = PostMessageW(Some(window), super::FIT_AGAIN, WPARAM(0), LPARAM(0));
            };
        }
        if is_skinned(window)
            && let Some(answer) = skinned_message(window, message, wparam, lparam)
        {
            return answer;
        }
        // SAFETY: the default handling of a message of our own window.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }
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
        AdjustWindowRect, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, RegisterClassW,
        WINDOW_EX_STYLE, WNDCLASSW, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_THICKFRAME,
        WS_VISIBLE,
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
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(win::frame_proc),
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
    let placed = x > 0 || y > 0;
    let skinned = current_sheet().is_some();
    let (style, outer_width, outer_height, left, top) = if skinned {
        // A popup has no default position; the plug-in's next move places it.
        let (left, top) = if placed { (x, y) } else { (100, 100) };
        let style = WS_POPUP | WS_THICKFRAME | WS_VISIBLE | WS_CLIPCHILDREN;
        (
            style,
            width.max(MIN_SIZE.0),
            height.max(MIN_SIZE.1),
            left,
            top,
        )
    } else {
        let style = WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN;
        let mut outer = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        // SAFETY: a writable rectangle. On failure the client size stands in.
        let _ = unsafe { AdjustWindowRect(&mut outer, style, false) };
        (
            style,
            outer.right - outer.left,
            outer.bottom - outer.top,
            if placed { x } else { CW_USEDEFAULT },
            if placed { y } else { CW_USEDEFAULT },
        )
    };
    // SAFETY: the class and title live through the call. The frame belongs to
    // the calling thread, which is the plug-in's and pumps its messages.
    let frame = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            w!("Visualization"),
            style,
            left,
            top,
            outer_width,
            outer_height,
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
    if let Ok(mut frames) = FRAMES.lock() {
        frames.push(handle);
    }
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

    /// A sheet of the classic size whose every pixel names its place, with a
    /// run of letters in each letter row: A is 3 wide, B 5 wide, the rest 1.
    fn test_sheet() -> Sheet {
        let (width, height) = (194usize, 109usize);
        let mut rgba = vec![0u8; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let at = (y * width + x) * 4;
                rgba[at] = x as u8;
                rgba[at + 1] = y as u8;
                rgba[at + 2] = 7;
                rgba[at + 3] = 255;
            }
        }
        for row in LETTER_ROWS {
            let row = row as usize;
            let background = [9u8, 9, 9, 255];
            for x in 0..width {
                rgba[(row * width + x) * 4..][..4].copy_from_slice(&background);
            }
            let mut x = 1;
            for letter_width in std::iter::once(3).chain(Some(5)).chain([1; 24]) {
                for column in x..x + letter_width {
                    rgba[(row * width + column) * 4..][..4].copy_from_slice(&[1, 2, 3, 255]);
                }
                x += letter_width + 1;
            }
        }
        Sheet::new(width as u32, height as u32, rgba).unwrap()
    }

    fn sheet_place(canvas: &Canvas, x: i32, y: i32) -> (u32, u32) {
        let pixel = canvas.pixels[y as usize * canvas.width as usize + x as usize];
        (pixel >> 16, (pixel >> 8) & 0xFF)
    }

    #[test]
    fn a_sheet_of_the_wrong_length_or_none_is_refused() {
        assert!(Sheet::new(0, 0, Vec::new()).is_none());
        assert!(Sheet::new(2, 2, vec![0; 15]).is_none());
        assert!(Sheet::new(2, 2, vec![0; 16]).is_some());
    }

    #[test]
    fn the_letters_are_found_by_the_rows_background() {
        let sheet = test_sheet();
        assert_eq!(sheet.letter('A', true), Some((1, 88, 3, 7)));
        assert_eq!(sheet.letter('b', true), Some((5, 88, 5, 7)));
        assert_eq!(sheet.letter('B', false), Some((5, 96, 5, 7)));
        assert_eq!(sheet.letter('1', true), None);
        assert_eq!(sheet.title_width("AB C", true), 3 + 5 + SPACE_WIDTH + 1);
    }

    #[test]
    fn the_chrome_stands_at_the_corners_and_leaves_the_middle_black() {
        let sheet = test_sheet();
        let canvas = draw_frame(&sheet, 300, 145, true, "A", false);
        // The corners of the top, active row of the sheet.
        assert_eq!(sheet_place(&canvas, 0, 0), (0, 0));
        assert_eq!(sheet_place(&canvas, 299, 19), (154, 19));
        // The sides.
        assert_eq!(sheet_place(&canvas, 0, 20), (127, 42));
        assert_eq!(sheet_place(&canvas, 292, 20), (139, 42));
        // The lower ends of the sides stand on the bottom.
        assert_eq!(sheet_place(&canvas, 0, 145 - 14 - 24), (158, 42));
        // The bottom row's corners.
        assert_eq!(sheet_place(&canvas, 0, 131), (0, 42));
        assert_eq!(sheet_place(&canvas, 299, 144), (124, 70));
        // What the plug-in fills.
        assert_eq!(content_rect(300, 145), (11, 20, 281, 111));
        assert_eq!(canvas.pixels[60 * 300 + 150], 0);
        // An inactive frame reads the other row.
        let canvas = draw_frame(&sheet, 300, 145, false, "A", false);
        assert_eq!(sheet_place(&canvas, 0, 0), (0, 21));
    }

    #[test]
    fn the_pressed_close_button_is_drawn_from_its_own_place() {
        let sheet = test_sheet();
        let canvas = draw_frame(&sheet, 275, 116, true, "", true);
        assert_eq!(sheet_place(&canvas, 275 - 11, 3), (148, 42));
    }

    #[test]
    fn a_small_sheet_and_a_long_title_stay_in_bounds() {
        let sheet = Sheet::new(4, 4, vec![255; 64]).unwrap();
        let long = "THE TITLE ".repeat(50);
        let canvas = draw_frame(&sheet, 275, 116, true, &long, true);
        assert_eq!(canvas.pixels.len(), 275 * 116);
        let sheet = test_sheet();
        let canvas = draw_frame(&sheet, 275, 116, true, &long, false);
        // The title stops before the right end: its tile is still there.
        assert_eq!(canvas.pixels.len(), 275 * 116);
    }

    #[test]
    fn a_point_does_what_it_shows() {
        assert_eq!(hit_test(275, 116, 275 - 11, 3), Hit::Close);
        assert_eq!(hit_test(275, 116, 100, 5), Hit::Caption);
        assert_eq!(hit_test(275, 116, 2, 50), Hit::Caption);
        assert_eq!(hit_test(275, 116, 274, 115), Hit::Resize);
        assert_eq!(hit_test(275, 116, 100, 50), Hit::Inside);
    }

    #[test]
    fn the_size_snaps_to_the_tiles_and_not_below_the_minimum() {
        assert_eq!(snapped_size(10, 10), MIN_SIZE);
        assert_eq!(snapped_size(275, 116), (275, 116));
        assert_eq!(snapped_size(288, 131), (300, 145));
        assert_eq!(snapped_size(286, 129), (275, 116));
    }

    #[test]
    fn the_rectangles_are_the_ones_of_the_skin_tables() {
        // The same numbers stand in the player's table of the sheet.
        assert_eq!(TOP_LEFT[1], (0, 0, 25, 20));
        assert_eq!(TOP_RIGHT[0], (130, 21, 25, 20));
        assert_eq!(BOTTOM_FILL, (127, 72, 25, 14));
        assert_eq!(MIDDLE_RIGHT_BOTTOM, (170, 42, 8, 24));
    }
}
