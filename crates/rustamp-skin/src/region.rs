//! `region.txt` window-region parsing.
//!
//! A region file has one section per window state (`Normal`, `WindowShade`, …).
//! Each section carries `NumPoints` (a comma-separated list of polygon vertex
//! counts) and `PointList` (the vertices, alternating `x` and `y`, separated by
//! commas or spaces). The parser builds one polygon per count and drops counts
//! below three, matching the documented behaviour.

use std::collections::HashMap;

use crate::ini::parse_ini;

/// A polygon as a list of `(x, y)` vertices.
pub type Polygon = Vec<(i32, i32)>;

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

    /// True when no section produced a usable polygon.
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    /// The section names present.
    pub fn section_names(&self) -> impl Iterator<Item = &str> {
        self.sections.keys().map(String::as_str)
    }
}

fn build_polygons(numpoints: &str, pointlist: &str) -> Vec<Polygon> {
    let counts: Vec<usize> = numpoints
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<usize>().ok())
        .collect();

    let tokens: Vec<i32> = pointlist
        .split([',', ' ', '\t', '\r', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<i32>().unwrap_or(0))
        .collect();

    let mut polygons = Vec::new();
    let mut index = 0usize;
    for count in counts {
        if count < 3 {
            index += count;
            continue;
        }
        // `index` counts points; the token list stores two numbers per point.
        let start = index * 2;
        let end = ((index + count) * 2).min(tokens.len());
        let slice = tokens.get(start..end).unwrap_or(&[]);
        if slice.len() < 6 {
            index += count;
            continue;
        }
        let polygon: Polygon = slice
            .chunks_exact(2)
            .map(|pair| (pair[0], pair[1]))
            .collect();
        polygons.push(polygon);
        index += count;
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
}
