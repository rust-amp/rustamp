//! Load-time statistics of a decoded sheet.
//!
//! A sheet that decodes to a flat, near-white image paints a blank window and
//! leaves no other trace: nothing fails, the window is simply empty. [`stats`]
//! summarises a decoded sheet once, while the skin is loading, so the loader
//! can report such a sheet. Nothing here runs per frame and nothing allocates
//! beyond the returned [`SheetStats`].

use crate::raster::RasterImage;

/// The smallest value a colour channel may have and still count as near-white.
const NEAR_WHITE: u8 = 0xF0;

/// Summary statistics of one decoded sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SheetStats {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixels whose alpha is 255.
    pub opaque: u64,
    /// Opaque pixels whose R, G and B are all at least `0xF0`.
    pub near_white_opaque: u64,
    /// The mean of each colour channel over every pixel, transparent or not.
    pub mean_rgb: [u8; 3],
}

impl SheetStats {
    /// The number of pixels in the sheet.
    pub const fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// True when more than 90% of the sheet's pixels are opaque near-white.
    ///
    /// That is what a sheet looks like when the window would paint as an empty
    /// white box: a palettised sheet a decoder mis-read to white, or artwork
    /// that never got its background drawn.
    pub fn is_mostly_near_white_opaque(&self) -> bool {
        let pixels = self.pixels();
        pixels > 0 && self.near_white_opaque * 10 > pixels * 9
    }
}

/// Measures `image` in a single pass.
pub fn stats(image: &RasterImage) -> SheetStats {
    let mut opaque = 0u64;
    let mut near_white_opaque = 0u64;
    let mut sum = [0u64; 3];
    let mut pixels = 0u64;
    for pixel in image.as_rgba().as_chunks::<4>().0 {
        pixels += 1;
        for (total, channel) in sum.iter_mut().zip(&pixel[..3]) {
            *total += u64::from(*channel);
        }
        if pixel[3] == 255 {
            opaque += 1;
            if pixel[..3].iter().all(|channel| *channel >= NEAR_WHITE) {
                near_white_opaque += 1;
            }
        }
    }
    let mean_rgb = if pixels == 0 {
        [0, 0, 0]
    } else {
        sum.map(|total| (total / pixels) as u8)
    };
    SheetStats {
        width: image.width(),
        height: image.height(),
        opaque,
        near_white_opaque,
        mean_rgb,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, pixels: &[[u8; 4]]) -> RasterImage {
        let mut rgba = Vec::with_capacity(pixels.len() * 4);
        for pixel in pixels {
            rgba.extend_from_slice(pixel);
        }
        RasterImage::new(width, height, rgba, Vec::new())
    }

    #[test]
    fn counts_opaque_and_near_white_pixels() {
        let image = image(
            2,
            2,
            &[
                [255, 255, 255, 255],
                [0xF0, 0xF5, 0xFF, 255],
                [0xEF, 0xFF, 0xFF, 255],
                [255, 255, 255, 0],
            ],
        );
        let stats = stats(&image);
        assert_eq!(stats.width, 2);
        assert_eq!(stats.height, 2);
        assert_eq!(stats.pixels(), 4);
        // Three pixels are opaque; the transparent white one is not.
        assert_eq!(stats.opaque, 3);
        // Only the two whose every channel is at least 0xF0 count as near
        // white; the 0xEF red channel is just below the threshold.
        assert_eq!(stats.near_white_opaque, 2);
    }

    #[test]
    fn mean_is_taken_over_every_pixel() {
        let image = image(2, 1, &[[10, 20, 30, 255], [40, 60, 80, 0]]);
        assert_eq!(stats(&image).mean_rgb, [25, 40, 55]);
    }

    #[test]
    fn a_blank_sheet_is_mostly_near_white_opaque() {
        let white = [[255, 255, 255, 255]; 100];
        assert!(stats(&image(10, 10, &white)).is_mostly_near_white_opaque());

        // Exactly 90 near-white pixels out of 100 is not "more than 90%".
        let mut pixels = vec![[0, 0, 0, 255]; 10];
        pixels.extend_from_slice(&white[..90]);
        assert!(!stats(&image(10, 10, &pixels)).is_mostly_near_white_opaque());

        // One pixel more tips it over.
        let mut pixels = vec![[0, 0, 0, 255]; 9];
        pixels.extend_from_slice(&white[..91]);
        assert!(stats(&image(10, 10, &pixels)).is_mostly_near_white_opaque());
    }

    #[test]
    fn a_colourful_sheet_is_not_blank() {
        let pixels: Vec<[u8; 4]> = (0..64)
            .map(|i| [(i * 4) as u8, (i * 3) as u8, (i * 2) as u8, 255])
            .collect();
        let stats = stats(&image(8, 8, &pixels));
        assert_eq!(stats.opaque, 64);
        assert!(!stats.is_mostly_near_white_opaque());
    }

    #[test]
    fn an_empty_sheet_is_not_blank() {
        let empty = RasterImage::filled(0, 0, [0, 0, 0, 255]);
        let stats = stats(&empty);
        assert_eq!(stats.pixels(), 0);
        assert!(!stats.is_mostly_near_white_opaque());
        assert_eq!(stats.mean_rgb, [0, 0, 0]);
    }
}
