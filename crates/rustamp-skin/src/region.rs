//! `region.txt` window-region parsing.
//!
//! A region file has one section per window state. S17 §7 names the three
//! classic sections `[Normal]`, `[WindowShade]` and `[Equalizer]`. Each
//! section carries `NumPoints` (a comma-separated list of polygon vertex
//! counts) and `PointList` (the vertices, alternating `x` and `y`, separated by
//! commas or spaces). The parser builds one polygon per count and drops counts
//! below three.
//!
//! S17 §7 leaves the handling of a malformed `NumPoints`/`PointList` (mismatched
//! counts) open. The parser's own rule: a count below three is dropped, a
//! polygon with fewer than three points left is dropped, and a polygon whose
//! point list ends early keeps the points it has. The other polygons of the
//! section are kept.
//!
//! A count that is **not a number at all** (a negative count, a word, a
//! number too large to count points) is the one case the rule cannot cover:
//! the point list can no longer be aligned with the counts, so the parser
//! stops at that count and keeps the polygons it has already read. Guessing
//! the missing count would read the *next* polygon's points as this one's —
//! a silently wrong shape, which is worse than no shape at all.
//!
//! The polygons are the window's *shape*; [`crate::region_mask`] turns them
//! into a per-pixel coverage mask for a window of a given size.

use std::collections::HashMap;

use crate::ini::parse_ini;
use crate::region_mask::{RegionMask, build_mask};

/// A polygon as a list of `(x, y)` vertices.
pub type Polygon = Vec<(i32, i32)>;

/// The window states a `region.txt` can describe (S17 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegionSection {
    /// `[Normal]` — the windows in their normal state.
    Normal,
    /// `[WindowShade]` — the windows in window-shade mode.
    WindowShade,
    /// `[Equalizer]` — the equalizer window.
    Equalizer,
}

impl RegionSection {
    /// The section name as it is written in `region.txt`.
    pub const fn name(self) -> &'static str {
        match self {
            RegionSection::Normal => "Normal",
            RegionSection::WindowShade => "WindowShade",
            RegionSection::Equalizer => "Equalizer",
        }
    }
}

/// The polygons of a `region.txt`, grouped by section.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Region {
    sections: HashMap<String, Vec<Polygon>>,
}

impl Region {
    /// Parses `region.txt` text.
    pub fn parse(text: &str) -> Region {
        let ini = parse_ini(text);
        let mut sections = HashMap::new();
        for (name, properties) in ini {
            let (Some(numpoints), Some(pointlist)) =
                (properties.get("numpoints"), properties.get("pointlist"))
            else {
                continue;
            };
            let polygons = build_polygons(numpoints, pointlist);
            if !polygons.is_empty() {
                sections.insert(name, polygons);
            }
        }
        Region { sections }
    }

    /// The polygons of a section (case-insensitive name).
    pub fn polygons(&self, section: &str) -> &[Polygon] {
        self.sections
            .get(&section.to_ascii_lowercase())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The `Normal` section, used for the unshaded windows.
    pub fn normal(&self) -> &[Polygon] {
        self.polygons("normal")
    }

    /// The `WindowShade` section.
    pub fn window_shade(&self) -> &[Polygon] {
        self.polygons("windowshade")
    }

    /// The `Equalizer` section.
    pub fn equalizer(&self) -> &[Polygon] {
        self.polygons("equalizer")
    }

    /// True when no section produced a usable polygon.
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    /// The section names present.
    pub fn section_names(&self) -> impl Iterator<Item = &str> {
        self.sections.keys().map(String::as_str)
    }

    /// The coverage mask of one section for a window of `width` x `height`.
    ///
    /// `None` means **no mask**: the section has no polygon, so the window
    /// keeps its rectangle. That is the base skin's case — S17 §7 records that
    /// its `region.txt` has every polygon commented out.
    ///
    /// A mask may still cover no pixel at all, for example when every polygon
    /// of the section lies outside the window; what to do with such a mask is
    /// the application's decision.
    ///
    /// The mask is pure data; the application applies it to the window.
    /// See [`crate::region_mask`] for the fill rule.
    pub fn mask(&self, section: RegionSection, width: u32, height: u32) -> Option<RegionMask> {
        let polygons = self.polygons(section.name());
        if polygons.is_empty() {
            return None;
        }
        Some(build_mask(polygons, width, height))
    }
}

fn build_polygons(numpoints: &str, pointlist: &str) -> Vec<Polygon> {
    let tokens: Vec<i32> = pointlist
        .split([',', ' ', '\t', '\r', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<i32>().unwrap_or(0))
        .collect();

    let mut polygons = Vec::new();
    // `index` counts the points the polygons have read so far; the token
    // list stores two numbers per point.
    let mut index = 0usize;
    for raw in numpoints.split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            // An empty entry is no polygon at all, so it consumes no points.
            continue;
        }
        let Ok(count) = raw.parse::<usize>() else {
            // A count that is not a number cannot be aligned with the point
            // list: the rest of the section is dropped rather than read at
            // the wrong offset.
            break;
        };
        if count < 3 {
            // A polygon with fewer than three points is dropped, and the
            // points it would have used are skipped.
            index = index.saturating_add(count);
            continue;
        }
        // `index` counts points; the token list stores two numbers per point.
        // The arithmetic is saturating, so an absurd count cannot overflow.
        let start = index.saturating_mul(2);
        let end = index
            .saturating_add(count)
            .saturating_mul(2)
            .min(tokens.len());
        let slice = tokens.get(start..end).unwrap_or(&[]);
        if slice.len() < 6 {
            index = index.saturating_add(count);
            continue;
        }
        let polygon: Polygon = slice
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| (pair[0], pair[1]))
            .collect();
        polygons.push(polygon);
        index = index.saturating_add(count);
    }
    polygons
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_normal_and_window_shade() {
        let text = "\
[Normal]\r\n\
NumPoints=4\r\n\
PointList=0, 0, 275, 0, 275, 116, 0, 116\r\n\
[WindowShade]\r\n\
NumPoints=4\r\n\
PointList=0,0,275,0,275,14,0,14\r\n";
        let region = Region::parse(text);
        assert_eq!(
            region.normal(),
            &[vec![(0, 0), (275, 0), (275, 116), (0, 116)]]
        );
        assert_eq!(
            region.window_shade(),
            &[vec![(0, 0), (275, 0), (275, 14), (0, 14)]]
        );
        assert!(region.polygons("Normal").len() == 1);
    }

    #[test]
    fn multiple_polygons_and_small_counts_dropped() {
        let text = "[normal]\nNumPoints=4,2\nPointList=0,0,10,0,10,10,0,10,1,1\n";
        let region = Region::parse(text);
        assert_eq!(region.normal().len(), 1);
        assert_eq!(region.normal()[0].len(), 4);
    }

    #[test]
    fn missing_pointlist_is_skipped() {
        let region = Region::parse("[normal]\nNumPoints=4\n");
        assert!(region.is_empty());
    }

    #[test]
    fn empty_text_is_empty() {
        assert!(Region::parse("").is_empty());
    }

    /// S17 §7: the three classic sections are `[Normal]`, `[WindowShade]` and
    /// `[Equalizer]`.
    #[test]
    fn parses_the_three_classic_sections() {
        let text = "\
[Normal]\r\n\
NumPoints=4\r\n\
PointList=0, 0, 275, 0, 275, 116, 0, 116\r\n\
[WindowShade]\r\n\
NumPoints=4\r\n\
PointList=0,0,275,0,275,14,0,14\r\n\
[Equalizer]\r\n\
NumPoints=4\r\n\
PointList=0,0,275,0,275,116,0,116\r\n";
        let region = Region::parse(text);
        assert_eq!(region.normal().len(), 1);
        assert_eq!(region.window_shade().len(), 1);
        assert_eq!(region.equalizer().len(), 1);
        assert_eq!(
            region.equalizer()[0],
            vec![(0, 0), (275, 0), (275, 116), (0, 116)]
        );
    }

    /// S17 §7: the base skin's `region.txt` has every polygon commented out,
    /// so the windows are plain rectangles and there is no masking.
    #[test]
    fn a_fully_commented_region_masks_nothing() {
        let text = "\
; a comment\r\n\
;NumPoints=4\r\n\
;PointList=0, 0, 275, 0, 275, 116, 0, 116\r\n\
[Normal]\r\n\
;NumPoints=4\r\n\
;PointList=0, 0, 275, 0, 275, 116, 0, 116\r\n";
        let region = Region::parse(text);
        assert!(region.is_empty());
        assert!(region.normal().is_empty());
    }

    /// A section without a polygon means **no mask**: the window keeps its
    /// rectangle.
    #[test]
    fn a_region_without_polygons_has_no_mask() {
        let region = Region::parse("");
        assert!(region.mask(RegionSection::Normal, 275, 116).is_none());
        assert!(region.mask(RegionSection::WindowShade, 275, 14).is_none());
        assert!(region.mask(RegionSection::Equalizer, 275, 116).is_none());
    }

    /// Every section gets the mask of its own polygons.
    #[test]
    fn every_section_gets_its_own_mask() {
        let text = "\
[Normal]\r\n\
NumPoints=4\r\n\
PointList=0, 0, 275, 0, 275, 116, 0, 116\r\n\
[WindowShade]\r\n\
NumPoints=4\r\n\
PointList=0, 0, 275, 0, 275, 14, 0, 14\r\n";
        let region = Region::parse(text);
        let normal = region
            .mask(RegionSection::Normal, 275, 116)
            .expect("normal mask");
        assert_eq!(normal.covered_pixels(), 275 * 116);
        let shade = region
            .mask(RegionSection::WindowShade, 275, 116)
            .expect("window shade mask");
        assert_eq!(shade.covered_pixels(), 275 * 14);
        // The equalizer section is missing: no mask.
        assert!(region.mask(RegionSection::Equalizer, 275, 116).is_none());
    }
}
