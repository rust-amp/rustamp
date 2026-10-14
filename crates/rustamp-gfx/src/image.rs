//! The RGBA canvas and the compositing primitives.
//!
//! [`Image`] is an 8-bit non-premultiplied RGBA buffer, row-major, top row
//! first. Every operation that takes a destination rectangle clips against the
//! canvas instead of panicking; out-of-bounds rectangles simply touch fewer
//! pixels.

/// An axis-aligned rectangle in pixel coordinates.
///
/// The origin is the top-left corner. `width` and `height` are unsigned, so a
/// rectangle can never have negative extent, but `x`/`y` may be negative to
/// describe an off-canvas source or destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    /// Left edge (may be negative).
    pub x: i32,
    /// Top edge (may be negative).
    pub y: i32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Rect {
    /// Creates a rectangle.
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The exclusive right edge.
    pub const fn right(&self) -> i32 {
        self.x.saturating_add(self.width as i32)
    }

    /// The exclusive bottom edge.
    pub const fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height as i32)
    }

    /// True when the rectangle covers no pixels.
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The overlapping rectangle, or `None` when the two do not overlap.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= x || bottom <= y {
            return None;
        }
        Some(Rect::new(x, y, (right - x) as u32, (bottom - y) as u32))
    }

    /// Shifts the rectangle by `(dx, dy)`.
    pub fn translate(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(
            self.x.saturating_add(dx),
            self.y.saturating_add(dy),
            self.width,
            self.height,
        )
    }
}

/// Errors raised while building a canvas from raw bytes.
#[derive(Debug, thiserror::Error)]
pub enum GfxError {
    /// The buffer length did not match `width * height * 4`.
    #[error("bad RGBA buffer length: expected {expected}, got {actual}")]
    BadLength {
        /// Expected byte length.
        expected: usize,
        /// Actual byte length.
        actual: usize,
    },
}

/// An 8-bit non-premultiplied RGBA image.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}x{})", self.width, self.height)
    }
}

impl Image {
    /// Creates a fully transparent image of the given size.
    pub fn new(width: u32, height: u32) -> Self {
        let len = width as usize * height as usize * 4;
        Self {
            width,
            height,
            data: vec![0; len],
        }
    }

    /// Creates an image filled with a single RGBA colour.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let mut image = Self::new(width, height);
        image.clear(rgba);
        image
    }

    /// Wraps a raw RGBA buffer, validating its length.
    pub fn from_rgba(width: u32, height: u32, data: Vec<u8>) -> Result<Self, GfxError> {
        let expected = width as usize * height as usize * 4;
        if data.len() != expected {
            return Err(GfxError::BadLength {
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// Width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The image bounds as a rectangle.
    pub const fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    /// The raw RGBA bytes.
    pub fn as_rgba(&self) -> &[u8] {
        &self.data
    }

    /// The raw RGBA bytes, mutably.
    pub fn as_rgba_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Consumes the image, returning `(width, height, rgba)`.
    pub fn into_raw(self) -> (u32, u32, Vec<u8>) {
        (self.width, self.height, self.data)
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        Some((y as usize * self.width as usize + x as usize) * 4)
    }

    /// Returns the pixel at `(x, y)`, or `None` when out of bounds.
    pub fn pixel(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        let i = self.index(x, y)?;
        Some([
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ])
    }

    /// Sets the pixel at `(x, y)`. Out-of-bounds writes are ignored.
    pub fn set_pixel(&mut self, x: i32, y: i32, rgba: [u8; 4]) {
        if let Some(i) = self.index(x, y) {
            self.data[i..i + 4].copy_from_slice(&rgba);
        }
    }

    /// Fills the whole image with one colour.
    pub fn clear(&mut self, rgba: [u8; 4]) {
        for chunk in self.data.as_chunks_mut::<4>().0 {
            chunk.copy_from_slice(&rgba);
        }
    }

    /// Fills `rect`, clipped to the canvas.
    pub fn fill_rect(&mut self, rect: Rect, rgba: [u8; 4]) {
        let Some(area) = rect.intersect(&self.bounds()) else {
            return;
        };
        for y in area.y..area.bottom() {
            let start = (y as usize * self.width as usize + area.x as usize) * 4;
            let end = start + area.width as usize * 4;
            for chunk in self.data[start..end].as_chunks_mut::<4>().0 {
                chunk.copy_from_slice(&rgba);
            }
        }
    }

    /// Copies `src` (opaque, alpha ignored) with its top-left corner at `(dx, dy)`.
    pub fn blit(&mut self, src: &Image, dx: i32, dy: i32) {
        self.blit_rect(src, src.bounds(), dx, dy);
    }

    /// Copies the `src_rect` region of `src` (opaque, alpha ignored) with its
    /// top-left corner at `(dx, dy)`. Both rectangles are clipped.
    pub fn blit_rect(&mut self, src: &Image, src_rect: Rect, dx: i32, dy: i32) {
        let Some(src_area) = src_rect.intersect(&src.bounds()) else {
            return;
        };
        let dst_rect = Rect::new(dx, dy, src_area.width, src_area.height);
        let Some(dst_area) = dst_rect.intersect(&self.bounds()) else {
            return;
        };
        // Offset from the requested origin to the first copied pixel.
        let offset_x = dst_area.x - dst_rect.x;
        let offset_y = dst_area.y - dst_rect.y;
        for row in 0..dst_area.height {
            let sy = src_area.y + offset_y + row as i32;
            let dy_row = dst_area.y + row as i32;
            let src_start =
                (sy as usize * src.width as usize + (src_area.x + offset_x) as usize) * 4;
            let dst_start = (dy_row as usize * self.width as usize + dst_area.x as usize) * 4;
            let len = dst_area.width as usize * 4;
            self.data[dst_start..dst_start + len]
                .copy_from_slice(&src.data[src_start..src_start + len]);
        }
    }

    /// Composites the `src_rect` region of `src` over the canvas using the
    /// source alpha (straight alpha, `over` operator). Both rectangles clip.
    pub fn blit_alpha(&mut self, src: &Image, src_rect: Rect, dx: i32, dy: i32) {
        let Some(src_area) = src_rect.intersect(&src.bounds()) else {
            return;
        };
        let dst_rect = Rect::new(dx, dy, src_area.width, src_area.height);
        let Some(dst_area) = dst_rect.intersect(&self.bounds()) else {
            return;
        };
        let offset_x = dst_area.x - dst_rect.x;
        let offset_y = dst_area.y - dst_rect.y;
        for row in 0..dst_area.height {
            let sy = src_area.y + offset_y + row as i32;
            let dy_row = dst_area.y + row as i32;
            for col in 0..dst_area.width {
                let sx = src_area.x + offset_x + col as i32;
                let dx_col = dst_area.x + col as i32;
                let src_i = (sy as usize * src.width as usize + sx as usize) * 4;
                let dst_i = (dy_row as usize * self.width as usize + dx_col as usize) * 4;
                let sa = src.data[src_i + 3];
                if sa == 0 {
                    continue;
                }
                if sa == 255 {
                    self.data[dst_i..dst_i + 4].copy_from_slice(&src.data[src_i..src_i + 4]);
                    continue;
                }
                // Straight-alpha "source over" with correct output alpha, so
                // compositing onto a transparent canvas stays meaningful.
                let sa = sa as u32;
                let da = self.data[dst_i + 3] as u32;
                let inv = 255 - sa;
                let out_a = sa + (da * inv + 127) / 255;
                if out_a == 0 {
                    continue;
                }
                for c in 0..3 {
                    let s = src.data[src_i + c] as u32;
                    let d = self.data[dst_i + c] as u32;
                    let blended = (s * sa + d * da * inv / 255 + out_a / 2) / out_a;
                    self.data[dst_i + c] = blended.min(255) as u8;
                }
                self.data[dst_i + 3] = out_a.min(255) as u8;
            }
        }
    }

    /// Composites the `src_rect` region of `src` using a colour key: pixels
    /// equal to `key` are treated as transparent, all others as opaque. This is
    /// how the classic palettised sprite sheets are composed.
    pub fn blit_keyed(&mut self, src: &Image, src_rect: Rect, dx: i32, dy: i32, key: [u8; 3]) {
        let Some(src_area) = src_rect.intersect(&src.bounds()) else {
            return;
        };
        let dst_rect = Rect::new(dx, dy, src_area.width, src_area.height);
        let Some(dst_area) = dst_rect.intersect(&self.bounds()) else {
            return;
        };
        let offset_x = dst_area.x - dst_rect.x;
        let offset_y = dst_area.y - dst_rect.y;
        for row in 0..dst_area.height {
            let sy = src_area.y + offset_y + row as i32;
            let dy_row = dst_area.y + row as i32;
            for col in 0..dst_area.width {
                let sx = src_area.x + offset_x + col as i32;
                let dx_col = dst_area.x + col as i32;
                let src_i = (sy as usize * src.width as usize + sx as usize) * 4;
                if src.data[src_i] == key[0]
                    && src.data[src_i + 1] == key[1]
                    && src.data[src_i + 2] == key[2]
                {
                    continue;
                }
                let dst_i = (dy_row as usize * self.width as usize + dx_col as usize) * 4;
                self.data[dst_i..dst_i + 4].copy_from_slice(&src.data[src_i..src_i + 4]);
            }
        }
    }

    /// Returns a copy scaled by an integer factor using nearest-neighbour
    /// sampling. A factor of zero or one returns a clone; the result is
    /// pixel-exact (`out[fy*factor+dy][fx*factor+dx] == in[fy][fx]`).
    pub fn scaled(&self, factor: u32) -> Image {
        if factor <= 1 {
            return self.clone();
        }
        let width = self.width.saturating_mul(factor);
        let height = self.height.saturating_mul(factor);
        let mut out = Image::new(width, height);
        for y in 0..self.height {
            for x in 0..self.width {
                let src_i = (y as usize * self.width as usize + x as usize) * 4;
                let pixel = &self.data[src_i..src_i + 4];
                for sy in 0..factor {
                    let oy = y * factor + sy;
                    for sx in 0..factor {
                        let ox = x * factor + sx;
                        let dst_i = (oy as usize * width as usize + ox as usize) * 4;
                        out.data[dst_i..dst_i + 4].copy_from_slice(pixel);
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    #[test]
    fn rect_intersection() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(
            a.intersect(&Rect::new(5, 5, 10, 10)),
            Some(Rect::new(5, 5, 5, 5))
        );
        assert_eq!(a.intersect(&Rect::new(10, 0, 1, 1)), None);
        assert!(Rect::new(0, 0, 0, 5).is_empty());
    }

    #[test]
    fn blit_clips_on_every_edge() {
        let mut dst = Image::filled(4, 4, RED);
        let src = Image::filled(4, 4, BLUE);
        // Only the bottom-right 1x1 of the source is visible.
        dst.blit(&src, 3, 3);
        assert_eq!(dst.pixel(3, 3), Some(BLUE));
        assert_eq!(dst.pixel(2, 3), Some(RED));
        assert_eq!(dst.pixel(3, 2), Some(RED));

        let mut dst = Image::filled(4, 4, RED);
        // Entirely off-canvas: nothing changes and nothing panics.
        dst.blit(&src, -10, -10);
        dst.blit(&src, 100, 100);
        assert_eq!(dst.as_rgba(), Image::filled(4, 4, RED).as_rgba());
    }

    #[test]
    fn blit_negative_origin_uses_lower_right_of_source() {
        let mut dst = Image::filled(2, 2, RED);
        let mut src = Image::filled(2, 2, RED);
        src.set_pixel(1, 1, BLUE);
        dst.blit(&src, -1, -1);
        assert_eq!(dst.pixel(0, 0), Some(BLUE));
        assert_eq!(dst.pixel(1, 1), Some(RED));
    }

    #[test]
    fn blit_rect_subregion() {
        let mut src = Image::filled(4, 4, RED);
        src.set_pixel(2, 2, BLUE);
        let mut dst = Image::new(2, 2);
        dst.blit_rect(&src, Rect::new(2, 2, 2, 2), 0, 0);
        assert_eq!(dst.pixel(0, 0), Some(BLUE));
        assert_eq!(dst.pixel(1, 1), Some(RED));
    }

    #[test]
    fn alpha_blit_composites_half_transparent() {
        let mut dst = Image::filled(1, 1, [0, 0, 0, 255]);
        let mut src = Image::new(1, 1);
        src.set_pixel(0, 0, [255, 255, 255, 128]);
        dst.blit_alpha(&src, Rect::new(0, 0, 1, 1), 0, 0);
        // 255*128/255 + 0 = 128 (rounded).
        assert_eq!(dst.pixel(0, 0), Some([128, 128, 128, 255]));
    }

    #[test]
    fn alpha_blit_ignores_zero_alpha() {
        let mut dst = Image::filled(1, 1, RED);
        let mut src = Image::new(1, 1);
        src.set_pixel(0, 0, [0, 255, 0, 0]);
        dst.blit_alpha(&src, Rect::new(0, 0, 1, 1), 0, 0);
        assert_eq!(dst.pixel(0, 0), Some(RED));
    }

    #[test]
    fn alpha_blit_onto_transparent_keeps_coverage() {
        let mut dst = Image::new(1, 1);
        let mut src = Image::new(1, 1);
        src.set_pixel(0, 0, [255, 255, 255, 128]);
        dst.blit_alpha(&src, Rect::new(0, 0, 1, 1), 0, 0);
        assert_eq!(dst.pixel(0, 0), Some([255, 255, 255, 128]));
    }

    #[test]
    fn keyed_blit_skips_background_colour() {
        let mut dst = Image::filled(2, 1, RED);
        let mut src = Image::filled(2, 1, [0, 0, 0, 255]);
        src.set_pixel(1, 0, BLUE);
        dst.blit_keyed(&src, Rect::new(0, 0, 2, 1), 0, 0, [0, 0, 0]);
        assert_eq!(dst.pixel(0, 0), Some(RED));
        assert_eq!(dst.pixel(1, 0), Some(BLUE));
    }

    #[test]
    fn integer_scaling_is_pixel_exact() {
        let mut src = Image::new(2, 2);
        src.set_pixel(0, 0, RED);
        src.set_pixel(1, 0, BLUE);
        src.set_pixel(0, 1, [1, 2, 3, 4]);
        src.set_pixel(1, 1, [5, 6, 7, 8]);
        let big = src.scaled(2);
        assert_eq!(big.width(), 4);
        assert_eq!(big.height(), 4);
        for (x, y, expect) in [
            (0, 0, RED),
            (1, 0, RED),
            (2, 0, BLUE),
            (3, 0, BLUE),
            (0, 1, RED),
            (1, 1, RED),
            (2, 1, BLUE),
            (3, 1, BLUE),
            (0, 2, [1, 2, 3, 4]),
            (3, 3, [5, 6, 7, 8]),
        ] {
            assert_eq!(big.pixel(x, y), Some(expect), "at ({x},{y})");
        }
    }
}
