//! A decoded image plus the raw bytes it was decoded from.

/// A decoded 8-bit RGBA image.
///
/// The raw, decompressed source bytes are kept alongside the pixels: some
/// classic sprites are tinted or inspected through their original palette or
/// byte layout, which the RGBA conversion loses.
#[derive(Clone, PartialEq, Eq)]
pub struct RasterImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    raw: Vec<u8>,
}

impl std::fmt::Debug for RasterImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RasterImage({}x{})", self.width, self.height)
    }
}

impl RasterImage {
    /// Creates an image from decoded RGBA pixels and the raw source bytes.
    ///
    /// # Panics
    /// Panics when `rgba.len() != width * height * 4`. This is a programmer
    /// error in the decoder, not untrusted input.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>, raw: Vec<u8>) -> Self {
        assert_eq!(
            rgba.len(),
            width as usize * height as usize * 4,
            "RGBA buffer does not match {width}x{height}"
        );
        Self {
            width,
            height,
            rgba,
            raw,
        }
    }

    /// Creates a solid image with `raw` empty; used for generated defaults.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..width as usize * height as usize {
            data.extend_from_slice(&rgba);
        }
        Self {
            width,
            height,
            rgba: data,
            raw: Vec::new(),
        }
    }

    /// Width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The decoded RGBA pixels.
    pub fn as_rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The decompressed source bytes (for a BMP, the whole file).
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The pixel at `(x, y)`, or `None` when out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        Some([
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ])
    }

    /// Consumes the image and returns `(width, height, rgba)` so a caller can
    /// hand the buffer to the compositor without this crate depending on it.
    pub fn into_parts(self) -> (u32, u32, Vec<u8>) {
        (self.width, self.height, self.rgba)
    }
}
