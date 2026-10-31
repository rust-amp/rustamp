// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Text drawn by the operating system's own GDI.
//!
//! The classic player's list font is a GDI font: comparing Rustamp against
//! Winamp 2.95 on one machine showed that only GDI gives the same glyphs, the
//! same widths and the same smoothing (ClearType where the user has it on).
//! GDI blends the glyph's edges against the colour under the text, so a call
//! names the background as well as the ink: both come from the caller's skin.
//!
//! A text is drawn in **one** face. When the requested face lacks a character,
//! the first of the caller's fallback families that has all of them draws the
//! whole text, so a title with a Japanese name does not have a Latin extension
//! in another font.
//!
//! The size is GDI's positive height: the character cell, which is what a row
//! pitch is measured in.

/// What a text is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GdiFont<'a> {
    /// The family name. A family the host does not have falls back to
    /// `fallbacks`.
    pub family: &'a str,
    /// The cell height in pixels.
    pub size: u32,
    /// Whether a bold face is wanted.
    pub bold: bool,
    /// Whether an italic face is wanted.
    pub italic: bool,
}

#[cfg(windows)]
pub use imp::{render_text, text_width};

/// Stub: there is no GDI off Windows.
#[cfg(not(windows))]
pub fn text_width(
    _font: GdiFont<'_>,
    _fallbacks: &[&str],
    _whole: &str,
    _text: &str,
) -> Option<u32> {
    None
}

/// Stub: there is no GDI off Windows.
#[cfg(not(windows))]
#[allow(clippy::too_many_arguments)]
pub fn render_text(
    _font: GdiFont<'_>,
    _fallbacks: &[&str],
    _whole: &str,
    _text: &str,
    _width: u32,
    _height: u32,
    _ink: [u8; 3],
    _background: [u8; 3],
) -> Option<Vec<u8>> {
    None
}

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateCompatibleDC,
        CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH, DEFAULT_QUALITY,
        DIB_RGB_COLORS, DRAW_TEXT_FORMAT, DT_CALCRECT, DT_LEFT, DT_NOCLIP, DT_NOPREFIX,
        DT_SINGLELINE, DT_TOP, DeleteDC, DeleteObject, DrawTextW, FF_DONTCARE, FW_BOLD, FW_NORMAL,
        GGI_MARK_NONEXISTING_GLYPHS, GetGlyphIndicesW, GetTextFaceW, HBITMAP, HDC, HFONT, HGDIOBJ,
        OUT_DEFAULT_PRECIS, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    };
    use windows::core::PCWSTR;

    use super::GdiFont;

    type FontKey = (String, u32, bool, bool);

    thread_local! {
        /// The fonts this thread asked for: creating one is the expensive part,
        /// and a list draws many rows in the same few.
        static FONTS: RefCell<HashMap<FontKey, HFONT>> = RefCell::new(HashMap::new());
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn font_handle(font: GdiFont<'_>, family: &str) -> HFONT {
        let key = (family.to_string(), font.size, font.bold, font.italic);
        FONTS.with(|fonts| {
            *fonts.borrow_mut().entry(key).or_insert_with(|| {
                let mut name = wide(family);
                name.push(0);
                // SAFETY: `name` is NUL-terminated and outlives the call.
                unsafe {
                    CreateFontW(
                        font.size as i32,
                        0,
                        0,
                        0,
                        if font.bold { FW_BOLD.0 } else { FW_NORMAL.0 } as i32,
                        u32::from(font.italic),
                        0,
                        0,
                        DEFAULT_CHARSET,
                        OUT_DEFAULT_PRECIS,
                        CLIP_DEFAULT_PRECIS,
                        DEFAULT_QUALITY,
                        DEFAULT_PITCH.0 as u32 | FF_DONTCARE.0 as u32,
                        PCWSTR(name.as_ptr()),
                    )
                }
            })
        })
    }

    /// How many characters of `text` the font selected into `dc` lacks.
    fn missing(dc: HDC, text: &[u16]) -> usize {
        let mut glyphs = vec![0u16; text.len()];
        let mut chars = text.to_vec();
        chars.push(0);
        // SAFETY: both buffers hold `text.len()` elements; the DC is live.
        let done = unsafe {
            GetGlyphIndicesW(
                dc,
                PCWSTR(chars.as_ptr()),
                text.len() as i32,
                glyphs.as_mut_ptr(),
                GGI_MARK_NONEXISTING_GLYPHS,
            )
        };
        if done == u32::MAX {
            return text.len();
        }
        glyphs
            .iter()
            .zip(text)
            .filter(|(glyph, ch)| **glyph == 0xffff && **ch >= 0x20)
            .count()
    }

    /// Whether the font selected into `dc` really is `family`: the font mapper
    /// answers an unknown name with another font, without saying so.
    fn is_family(dc: HDC, family: &str) -> bool {
        let mut face = [0u16; 64];
        // SAFETY: `face` is a valid buffer; the DC is live.
        let length = unsafe { GetTextFaceW(dc, Some(&mut face)) }.max(1) as usize - 1;
        String::from_utf16_lossy(&face[..length]).eq_ignore_ascii_case(family)
    }

    /// The face that draws `whole`: the requested one when it has every
    /// character, else the first fallback that has them all, else the one that
    /// has the most.
    fn choose_face(dc: HDC, font: GdiFont<'_>, fallbacks: &[&str], whole: &[u16]) -> HFONT {
        let mut best = None;
        for (rank, family) in std::iter::once(&font.family).chain(fallbacks).enumerate() {
            let handle = font_handle(font, family);
            // SAFETY: the DC and the font are live.
            unsafe { SelectObject(dc, HGDIOBJ(handle.0)) };
            if rank > 0 && !is_family(dc, family) {
                continue;
            }
            let lacking = missing(dc, whole);
            // A requested family the host does not have is only a last resort.
            let lacking = if rank == 0 && !is_family(dc, family) {
                lacking + 1
            } else {
                lacking
            };
            if lacking == 0 {
                return handle;
            }
            if best.is_none_or(|(least, _)| lacking < least) {
                best = Some((lacking, handle));
            }
        }
        best.map_or(font_handle(font, font.family), |(_, handle)| handle)
    }

    /// One line of text as it is: no `&` prefix, no wrapping. `DrawText` and not
    /// `TextOut`, because it kerns, which is what Winamp 2.95's rows show.
    fn text_format() -> DRAW_TEXT_FORMAT {
        DT_LEFT | DT_TOP | DT_SINGLELINE | DT_NOPREFIX | DT_NOCLIP
    }

    /// A memory DC for measuring.
    struct MemoryDc(HDC);

    impl MemoryDc {
        fn new() -> Option<Self> {
            // SAFETY: a DC compatible with the screen.
            let dc = unsafe { CreateCompatibleDC(None) };
            (!dc.is_invalid()).then_some(Self(dc))
        }
    }

    impl Drop for MemoryDc {
        fn drop(&mut self) {
            // SAFETY: the DC is ours and no longer used.
            let _ = unsafe { DeleteDC(self.0) };
        }
    }

    /// The width of `text` in pixels, in the face `whole` is drawn in.
    pub fn text_width(
        font: GdiFont<'_>,
        fallbacks: &[&str],
        whole: &str,
        text: &str,
    ) -> Option<u32> {
        let dc = MemoryDc::new()?;
        let face = choose_face(dc.0, font, fallbacks, &wide(whole));
        // SAFETY: the DC and the font are live.
        unsafe { SelectObject(dc.0, HGDIOBJ(face.0)) };
        let mut rect = RECT::default();
        let mut chars = wide(text);
        if chars.is_empty() {
            return Some(0);
        }
        // SAFETY: `rect` and the text are valid; nothing is drawn.
        unsafe { DrawTextW(dc.0, &mut chars, &mut rect, text_format() | DT_CALCRECT) };
        Some(rect.right.max(0) as u32)
    }

    /// `text` as RGBA rows from the top, in the face `whole` is drawn in.
    /// Pixels that stayed `background` are transparent, so the result can be
    /// laid over what it was blended against.
    #[allow(clippy::too_many_arguments)]
    pub fn render_text(
        font: GdiFont<'_>,
        fallbacks: &[&str],
        whole: &str,
        text: &str,
        width: u32,
        height: u32,
        ink: [u8; 3],
        background: [u8; 3],
    ) -> Option<Vec<u8>> {
        if width == 0 || height == 0 {
            return None;
        }
        let dc = MemoryDc::new()?;
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        // SAFETY: `info` describes the bitmap; `bits` receives its pixels.
        let bitmap: HBITMAP =
            unsafe { CreateDIBSection(Some(dc.0), &info, DIB_RGB_COLORS, &mut bits, None, 0) }
                .ok()?;
        let count = width as usize * height as usize;
        // SAFETY: the section holds `count` 32-bit pixels and stays alive until
        // the `DeleteObject` below.
        let pixels = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u32>(), count) };
        let bgr = u32::from(background[0]) << 16
            | u32::from(background[1]) << 8
            | u32::from(background[2]);
        pixels.fill(bgr);
        let face = choose_face(dc.0, font, fallbacks, &wide(whole));
        // SAFETY: the DC, the bitmap and the font are live; the text is a
        // valid buffer.
        unsafe {
            SelectObject(dc.0, HGDIOBJ(bitmap.0));
            SelectObject(dc.0, HGDIOBJ(face.0));
            SetBkMode(dc.0, TRANSPARENT);
            SetTextColor(
                dc.0,
                COLORREF(u32::from(ink[0]) | u32::from(ink[1]) << 8 | u32::from(ink[2]) << 16),
            );
            let mut chars = wide(text);
            if !chars.is_empty() {
                let mut rect = RECT {
                    left: 0,
                    top: 0,
                    right: width as i32,
                    bottom: height as i32,
                };
                DrawTextW(dc.0, &mut chars, &mut rect, text_format());
            }
        }
        let mut rgba = Vec::with_capacity(count * 4);
        for pixel in pixels.iter() {
            let pixel = *pixel & 0x00ff_ffff;
            let alpha = if pixel == bgr { 0 } else { 255 };
            rgba.extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8, alpha]);
        }
        // SAFETY: nothing reads the section any more; deleting the DC follows.
        let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
        Some(rgba)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const ARIAL: GdiFont<'static> = GdiFont {
            family: "Arial",
            size: 13,
            bold: false,
            italic: false,
        };
        const FALLBACKS: &[&str] = &["Yu Gothic", "Meiryo", "MS Gothic", "Malgun Gothic"];

        #[test]
        fn a_text_is_drawn_and_its_width_is_known() {
            let width = text_width(ARIAL, FALLBACKS, "Alpha", "Alpha").unwrap();
            assert!(width > 10);
            let rgba = render_text(
                ARIAL, FALLBACKS, "Alpha", "Alpha", width, 16, [255; 3], [0; 3],
            )
            .unwrap();
            assert!(rgba.chunks(4).any(|pixel| pixel[3] == 255));
        }

        #[test]
        fn the_extension_changes_its_font_with_a_title_the_font_cannot_draw() {
            let part = |whole: &str| {
                render_text(ARIAL, FALLBACKS, whole, ".mp3", 40, 16, [255; 3], [0; 3]).unwrap()
            };
            assert_ne!(part(".mp3"), part("\u{65E5}\u{672C}.mp3"));
        }

        #[test]
        fn an_untouched_pixel_is_transparent() {
            let rgba = render_text(ARIAL, FALLBACKS, "", "", 4, 4, [255; 3], [9, 9, 9]).unwrap();
            assert!(rgba.chunks(4).all(|pixel| pixel[3] == 0));
        }
    }
}
