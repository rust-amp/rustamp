//! Generic window capture tool (no application-specific logic).
//!
//! ```text
//! cargo xtask capture --list
//! cargo xtask capture [--class <window class>] [--title <substring>]
//!                     [--out <png>] [--client-only]
//! ```

const HELP: &str = "\
usage: cargo xtask capture --list
       cargo xtask capture [--class <class>] [--title <substring>] [--out <png>] [--client-only]

  --list         print visible top-level windows (hwnd, class, title, rect)
  --class C      match window class exactly (case-insensitive)
  --title T      match window title containing T (case-insensitive)
  --out F        output PNG path (default: target/capture.png)
  --client-only  capture only the client area";

#[cfg(not(windows))]
pub fn run(_args: &[String]) -> Result<(), String> {
    println!("xtask capture: unsupported on this OS");
    Ok(())
}

#[cfg(windows)]
pub fn run(args: &[String]) -> Result<(), String> {
    imp::run(args)
}

#[cfg(windows)]
mod imp {
    use super::HELP;
    use rustamp_testkit::RgbaImage;
    use std::ffi::c_void;
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, ClientToScreen, CreateCompatibleDC,
        CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, ReleaseDC,
        SRCCOPY, SelectObject,
    };
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetClientRect, GetWindowRect, GetWindowTextW, IsWindowVisible,
        SetProcessDPIAware,
    };
    use windows::core::BOOL;

    const PW_CLIENTONLY: u32 = 1;
    const PW_RENDERFULLCONTENT: u32 = 2;

    struct WinInfo {
        hwnd: HWND,
        class: String,
        title: String,
        rect: RECT,
    }

    fn info(hwnd: HWND) -> WinInfo {
        let mut cls = [0u16; 256];
        let mut txt = [0u16; 512];
        let mut rect = RECT::default();
        // SAFETY: hwnd came from EnumWindows; buffers and rect are valid for writes
        // and their lengths match the slices passed.
        unsafe {
            let n = GetClassNameW(hwnd, &mut cls).max(0) as usize;
            let m = GetWindowTextW(hwnd, &mut txt).max(0) as usize;
            let _ = GetWindowRect(hwnd, &mut rect);
            WinInfo {
                hwnd,
                class: String::from_utf16_lossy(&cls[..n]),
                title: String::from_utf16_lossy(&txt[..m]),
                rect,
            }
        }
    }

    fn visible_windows() -> Vec<WinInfo> {
        unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: lparam is the pointer to the Vec passed by `visible_windows`,
            // alive and exclusively used for the duration of EnumWindows.
            let v = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
            // SAFETY: hwnd is a valid handle supplied by EnumWindows.
            if unsafe { IsWindowVisible(hwnd) }.as_bool() {
                v.push(hwnd);
            }
            BOOL(1)
        }
        let mut handles: Vec<HWND> = Vec::new();
        // SAFETY: the callback only dereferences the Vec pointer we pass, which
        // outlives the call.
        let _ = unsafe { EnumWindows(Some(cb), LPARAM(&mut handles as *mut _ as isize)) };
        handles.into_iter().map(info).collect()
    }

    pub fn run(args: &[String]) -> Result<(), String> {
        let (mut list, mut client_only) = (false, false);
        let (mut class, mut title, mut out) = (None, None, "target/capture.png".to_string());
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut val = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
            match a.as_str() {
                "--list" => list = true,
                "--client-only" => client_only = true,
                "--class" => class = Some(val("--class")?),
                "--title" => title = Some(val("--title")?),
                "--out" => out = val("--out")?,
                "-h" | "--help" => {
                    println!("{HELP}");
                    return Ok(());
                }
                other => return Err(format!("unknown capture option `{other}`\n{HELP}")),
            }
        }
        // SAFETY: no preconditions; makes coordinates physical pixels.
        let _ = unsafe { SetProcessDPIAware() };

        let wins = visible_windows();
        if list {
            for w in wins
                .iter()
                .filter(|w| !w.title.is_empty() || !w.class.is_empty())
            {
                println!(
                    "hwnd=0x{:X} class={:?} title={:?} rect=({},{})-({},{}) {}x{}",
                    w.hwnd.0 as usize,
                    w.class,
                    w.title,
                    w.rect.left,
                    w.rect.top,
                    w.rect.right,
                    w.rect.bottom,
                    w.rect.right - w.rect.left,
                    w.rect.bottom - w.rect.top
                );
            }
            return Ok(());
        }
        if class.is_none() && title.is_none() {
            return Err(format!("need --list, --class and/or --title\n{HELP}"));
        }
        let matches: Vec<&WinInfo> = wins
            .iter()
            .filter(|w| {
                class
                    .as_ref()
                    .is_none_or(|c| w.class.eq_ignore_ascii_case(c))
                    && title
                        .as_ref()
                        .is_none_or(|t| w.title.to_lowercase().contains(&t.to_lowercase()))
            })
            .collect();
        let target = matches.first().ok_or("no matching visible window")?;
        if matches.len() > 1 {
            eprintln!(
                "note: {} windows matched; using the first (hwnd=0x{:X})",
                matches.len(),
                target.hwnd.0 as usize
            );
        }
        let img = capture(target.hwnd, client_only)?;
        img.save_png(&out).map_err(|e| e.to_string())?;
        println!(
            "captured hwnd=0x{:X} class={:?} title={:?} -> {} ({}x{})",
            target.hwnd.0 as usize,
            target.class,
            target.title,
            out,
            img.width(),
            img.height()
        );
        Ok(())
    }

    /// Captures `hwnd` via PrintWindow, falling back to a screen BitBlt.
    fn capture(hwnd: HWND, client_only: bool) -> Result<RgbaImage, String> {
        let mut rect = RECT::default();
        // SAFETY: hwnd is a window handle; rect is a valid out pointer.
        unsafe {
            if client_only {
                GetClientRect(hwnd, &mut rect).map_err(|e| e.to_string())?;
            } else {
                GetWindowRect(hwnd, &mut rect).map_err(|e| e.to_string())?;
            }
        }
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        if w <= 0 || h <= 0 {
            return Err(format!("window has empty size {w}x{h}"));
        }
        // SAFETY: all GDI objects created below are released before returning;
        // the DIB pointer is valid for w*h*4 bytes while the bitmap lives.
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let dib: Result<HBITMAP, _> =
                CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
            let result = match dib {
                Err(e) => Err(format!("CreateDIBSection: {e}")),
                Ok(dib) => {
                    let old = SelectObject(mem, dib.into());
                    let len = w as usize * h as usize * 4;
                    let buf = std::slice::from_raw_parts_mut(bits as *mut u8, len);
                    buf.fill(0);
                    let flags = PW_RENDERFULLCONTENT | if client_only { PW_CLIENTONLY } else { 0 };
                    let ok = PrintWindow(hwnd, mem, PRINT_WINDOW_FLAGS(flags)).as_bool();
                    if !ok || buf.iter().all(|&b| b == 0) {
                        eprintln!(
                            "note: PrintWindow produced nothing; falling back to screen BitBlt"
                        );
                        blit_from_screen(hwnd, client_only, rect, screen, mem, w, h)?;
                    }
                    let mut rgba = Vec::with_capacity(len);
                    for px in buf.chunks_exact(4) {
                        rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                    }
                    SelectObject(mem, old);
                    let _ = DeleteObject(dib.into());
                    RgbaImage::from_raw(w as u32, h as u32, rgba).map_err(|e| e.to_string())
                }
            };
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            result
        }
    }

    /// # Safety
    /// `screen` and `mem` must be valid DCs with a `w`x`h` bitmap selected in `mem`.
    unsafe fn blit_from_screen(
        hwnd: HWND,
        client_only: bool,
        rect: RECT,
        screen: HDC,
        mem: HDC,
        w: i32,
        h: i32,
    ) -> Result<(), String> {
        let mut origin = POINT {
            x: rect.left,
            y: rect.top,
        };
        if client_only {
            origin = POINT { x: 0, y: 0 };
            // SAFETY: hwnd valid, origin is a valid out pointer.
            let _ = unsafe { ClientToScreen(hwnd, &mut origin) };
        }
        // SAFETY: guaranteed by the caller contract.
        unsafe { BitBlt(mem, 0, 0, w, h, Some(screen), origin.x, origin.y, SRCCOPY) }
            .map_err(|e| format!("BitBlt: {e}"))
    }
}
