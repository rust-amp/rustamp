// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The skin's window shape: the part of `region.txt` a window uses and the
//! rectangles its mask becomes.
//!
//! S17 §7: a skin's `region.txt` describes the **visible** area of a window.
//! Its polygons are drawn in the skin's own pixels — the 1x layout — while
//! the player's windows are drawn at that size or at exactly twice it (the
//! double-size mode). The mask therefore belongs to the **layout** size and
//! is scaled to the window's physical pixels, the same way the skin's
//! bitmaps are scaled.
//!
//! The rules this module follows:
//!
//! * **No mask, no shape.** A skin without a `region.txt` — or with a
//!   section that has no polygon, which is the base skin's case (S17 §7) —
//!   leaves the window a plain rectangle.
//! * **A mask that covers no pixel is not a shape.** The window stays a
//!   rectangle: a window with an empty region would be invisible.
//! * **The playlist editor has no shape.** `region.txt` has no section for
//!   it, so it keeps its rectangle. That is an own choice: the section
//!   names S17 §7 fixes are the main window's, the shade band's and the
//!   equalizer's.
//!
//! The module is pure: it turns a skin's region into rectangles and is
//! tested without a window. Handing them to the operating system's window
//! lives in [`rustamp_platform::window_region`].

use rustamp_platform::window_region::RegionRect;
use rustamp_skin::{Region, RegionSection};
use rustamp_ui::docking::WindowKind;

/// The section of `region.txt` the main window uses.
///
/// The main window has two shapes: the normal one and the shade band
/// (S01 §1). `region.txt` has a section for each of them (S17 §7).
pub const fn main_section(shaded: bool) -> RegionSection {
    if shaded {
        RegionSection::WindowShade
    } else {
        RegionSection::Normal
    }
}

/// The section of `region.txt` the equalizer window uses.
///
/// The equalizer has a section of its own (S17 §7). Its shade band is drawn
/// from the same band as the main window's, but the section stays the
/// equalizer's: `region.txt` describes the equalizer, not its shade.
pub const fn equalizer_section() -> RegionSection {
    RegionSection::Equalizer
}

/// The rectangles the main window's shape covers, in physical pixels.
///
/// An empty list means **no shape**: the window stays a plain rectangle.
pub fn main_rects(region: &Region, shaded: bool, double_size: bool) -> Vec<RegionRect> {
    rects_for(
        region,
        main_section(shaded),
        WindowKind::Main,
        shaded,
        double_size,
    )
}

/// The rectangles the equalizer window's shape covers, in physical pixels.
///
/// An empty list means **no shape**: the window stays a plain rectangle.
pub fn equalizer_rects(region: &Region, shaded: bool, double_size: bool) -> Vec<RegionRect> {
    rects_for(
        region,
        equalizer_section(),
        WindowKind::Equalizer,
        shaded,
        double_size,
    )
}

/// The rectangles of one window's shape.
fn rects_for(
    region: &Region,
    section: RegionSection,
    kind: WindowKind,
    shaded: bool,
    double_size: bool,
) -> Vec<RegionRect> {
    // The polygons are drawn in the skin's own pixels: the layout size, not
    // the physical one.
    let (width, height) = rustamp_ui::docking::window_size(kind, shaded, false);
    mask_rects(region, section, (width, height), scale(double_size))
}

/// The number of physical pixels per skin pixel.
fn scale(double_size: bool) -> u32 {
    if double_size { 2 } else { 1 }
}

/// The rectangles the mask of `section` covers in a window of `size` skin
/// pixels, scaled to `scale` physical pixels per skin pixel.
///
/// An empty list means **no shape**: either the section has no polygon, or
/// the mask covers no pixel of the window. The caller then leaves the
/// window a plain rectangle.
pub fn mask_rects(
    region: &Region,
    section: RegionSection,
    size: (u32, u32),
    scale: u32,
) -> Vec<RegionRect> {
    let Some(mask) = region.mask(section, size.0, size.1) else {
        return Vec::new();
    };
    mask.spans()
        .iter()
        .map(|span| {
            let run = RegionRect::run(span.x_start, span.x_end, span.y);
            RegionRect {
                left: run.left * scale as i32,
                top: run.top * scale as i32,
                right: run.right * scale as i32,
                bottom: run.bottom * scale as i32,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `region.txt` with a rectangle of `width` x `height` in `[Normal]`.
    fn normal_region(width: i32, height: i32) -> Region {
        Region::parse(&format!(
            "[Normal]\r\nNumPoints=4\r\nPointList=0,0,{width},0,{width},{height},0,{height}\r\n"
        ))
    }

    #[test]
    fn every_window_uses_the_section_of_its_state() {
        assert_eq!(main_section(false), RegionSection::Normal);
        assert_eq!(main_section(true), RegionSection::WindowShade);
        assert_eq!(equalizer_section(), RegionSection::Equalizer);
    }

    #[test]
    fn a_skin_without_a_region_leaves_the_window_a_rectangle() {
        let region = Region::parse("");
        assert!(main_rects(&region, false, false).is_empty());
        assert!(main_rects(&region, true, false).is_empty());
        assert!(equalizer_rects(&region, false, false).is_empty());
    }

    #[test]
    fn the_shape_covers_the_window_the_skin_asks_for() {
        let region = normal_region(275, 116);
        let rects = main_rects(&region, false, false);
        // One run per row: the whole window is covered.
        assert_eq!(rects.len(), 116);
        assert_eq!(
            rects[0],
            RegionRect {
                left: 0,
                top: 0,
                right: 275,
                bottom: 1
            }
        );
        assert_eq!(
            rects[115],
            RegionRect {
                left: 0,
                top: 115,
                right: 275,
                bottom: 116
            }
        );
    }

    #[test]
    fn every_run_of_pixels_becomes_one_rectangle() {
        // Two rectangles with a gap between them: two runs per row.
        let region = Region::parse(
            "[Normal]\r\nNumPoints=4,4\r\n\
             PointList=0,0,10,0,10,10,0,10,20,0,30,0,30,10,20,10\r\n",
        );
        let rects = mask_rects(&region, RegionSection::Normal, (275, 116), 1);
        assert_eq!(rects.len(), 20);
        assert_eq!(
            rects[0],
            RegionRect {
                left: 0,
                top: 0,
                right: 10,
                bottom: 1
            }
        );
        assert_eq!(
            rects[1],
            RegionRect {
                left: 20,
                top: 0,
                right: 30,
                bottom: 1
            }
        );
        assert_eq!(
            rects[2],
            RegionRect {
                left: 0,
                top: 1,
                right: 10,
                bottom: 2
            }
        );
    }

    #[test]
    fn the_double_size_scales_the_shape_with_the_skin() {
        let region = normal_region(275, 116);
        let rects = main_rects(&region, false, true);
        // The rows are twice as tall and twice as wide: the skin's shape is
        // drawn at the same scale as the skin's bitmaps.
        assert_eq!(rects.len(), 116);
        assert_eq!(
            rects[0],
            RegionRect {
                left: 0,
                top: 0,
                right: 550,
                bottom: 2
            }
        );
        assert_eq!(
            rects[115],
            RegionRect {
                left: 0,
                top: 230,
                right: 550,
                bottom: 232
            }
        );
    }

    #[test]
    fn a_mask_that_covers_no_pixel_leaves_the_window_a_rectangle() {
        // A polygon that lies outside the window: the mask is empty.
        let region = Region::parse(
            "[Normal]\r\nNumPoints=4\r\nPointList=1000,1000,1010,1000,1010,1010,1000,1010\r\n",
        );
        assert!(main_rects(&region, false, false).is_empty());
    }

    #[test]
    fn the_shade_band_uses_its_own_section() {
        let region =
            Region::parse("[WindowShade]\r\nNumPoints=4\r\nPointList=0,0,275,0,275,14,0,14\r\n");
        let rects = main_rects(&region, true, false);
        assert_eq!(rects.len(), 14);
        assert_eq!(
            rects[13],
            RegionRect {
                left: 0,
                top: 13,
                right: 275,
                bottom: 14
            }
        );
        // The normal section is missing: the unshaded window is a rectangle.
        assert!(main_rects(&region, false, false).is_empty());
    }

    #[test]
    fn the_equalizer_uses_its_own_section() {
        let region =
            Region::parse("[Equalizer]\r\nNumPoints=4\r\nPointList=0,0,275,0,275,116,0,116\r\n");
        let rects = equalizer_rects(&region, false, false);
        assert_eq!(rects.len(), 116);
        // The main window's section does not shape the equalizer.
        assert!(main_rects(&region, false, false).is_empty());
    }
}
