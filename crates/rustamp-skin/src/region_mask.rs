//! Window-shape masks built from a skin's `region.txt`.
//!
//! S17 §7: the polygons of a `region.txt` section define the **visible** area
//! of a window — everything outside them is transparent — while the window
//! rectangle itself does not change. The base skin's file has every polygon
//! commented out, so with the base skin the windows are plain rectangles and
//! there is **no mask** at all.
//!
//! This module turns the parsed polygons into a **coverage mask** for a
//! window of a given size: a bitmap that says for every pixel of the window
//! whether it lies inside the skin's shape. It is pure data — no Win32 call,
//! no window, no dependency — so it can be built and tested headless. The
//! application applies the mask later, with `SetWindowRgn`.
//!
//! ## The fill rule
//!
//! S17 §7 does not settle how a self-intersecting polygon is filled, so the
//! rule used here is **this module's own choice**, stated in full:
//!
//! * **Non-zero winding rule, sampled at the pixel centre.** A pixel
//!   `(x, y)` is covered when the point `(x + 0.5, y + 0.5)` is inside the
//!   polygon, "inside" meaning that the winding number of the polygon around
//!   the point is not zero. The rule fills a concave polygon the only
//!   sensible way, and it is the one a Win32 application reproduces when it
//!   hands the same polygons to the system and combines the results.
//! * **The polygons of one section are additive** (S17 §7): the mask is the
//!   union of their areas, so a pixel covered by any polygon of the section is
//!   covered. A polygon wound the other way inside another one therefore does
//!   **not** cut a hole.
//! * **Coordinates are pixel corners** (S17 §7, provisional): the polygon
//!   `(0,0)-(10,0)-(10,10)-(0,10)` covers the pixels `x = 0..9`, `y = 0..9`,
//!   that is exactly the 10 x 10 pixels whose outer corners it names.
//! * **A pixel whose centre lies exactly on an edge** belongs to the span
//!   that starts at that edge: the covered interval of a scanline is
//!   half-open, `[left, right)`. A diagonal edge that runs through pixel
//!   corners therefore splits those pixels cleanly instead of covering them
//!   twice or not at all.
//!
//! The rasteriser works in exact integer arithmetic — every edge crossing is
//! a rational number — so a diagonal edge never misplaces a pixel because of
//! a rounding error.
//!
//! ## Applying the mask
//!
//! The application applies the mask to the client area of the window:
//!
//! 1. build the mask for the window's current size and the section that
//!    belongs to the window's state ([`crate::Region::mask`]);
//! 2. hand the mask's runs to the window as one rectangle per run, so that
//!    the window keeps its size and only the pixels outside the shape become
//!    transparent — that is what S17 §7 measured;
//! 3. do nothing when there is no mask: the window stays a rectangle.
//!
//! The mask belongs to one window size. A window that changes its size needs
//! a new mask.

use crate::region::Polygon;

/// A half-open horizontal run of covered pixels: the columns `x_start..x_end`
/// of the row `y`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaskSpan {
    /// The row the run is on.
    pub y: u32,
    /// The first covered column (inclusive).
    pub x_start: u32,
    /// The column after the last covered one (exclusive).
    pub x_end: u32,
}

/// The per-pixel coverage of a window by a skin's region.
///
/// The mask is a bitmap with one bit per pixel of the window, in row-major
/// order: a set bit means "this pixel is inside the skin's shape", a clear
/// bit means "this pixel is outside it". The window rectangle itself is not
/// changed by the mask — S17 §7 records that a window keeps its size and only
/// its transparent parts change.
#[derive(Clone, PartialEq, Eq)]
pub struct RegionMask {
    width: u32,
    height: u32,
    bits: Vec<u64>,
}

impl std::fmt::Debug for RegionMask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RegionMask({}x{}, {} pixels covered)",
            self.width,
            self.height,
            self.covered_pixels()
        )
    }
}

impl RegionMask {
    /// The width of the window the mask was built for, in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// The height of the window the mask was built for, in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Whether the pixel at `(x, y)` is covered.
    ///
    /// A pixel outside the window is never covered.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.is_covered(x, y)
    }

    /// How many pixels the mask covers.
    pub fn covered_pixels(&self) -> u64 {
        self.bits.iter().map(|word| word.count_ones() as u64).sum()
    }

    /// The covered pixels as horizontal runs, row by row.
    ///
    /// This is the run-length spelling of the mask: every run is a half-open
    /// column range on one row, the runs of a row do not overlap, and the
    /// whole list is ordered by row and then by column. It is what an
    /// application needs to hand a window region to the system, one rectangle
    /// per run.
    pub fn spans(&self) -> Vec<MaskSpan> {
        let mut spans: Vec<MaskSpan> = Vec::new();
        for y in 0..self.height {
            let mut start: Option<u32> = None;
            for x in 0..self.width {
                if self.is_covered(x, y) {
                    if start.is_none() {
                        start = Some(x);
                    }
                } else if let Some(start_x) = start.take() {
                    spans.push(MaskSpan {
                        y,
                        x_start: start_x,
                        x_end: x,
                    });
                }
            }
            if let Some(start_x) = start {
                spans.push(MaskSpan {
                    y,
                    x_start: start_x,
                    x_end: self.width,
                });
            }
        }
        spans
    }

    /// Whether the bit of the pixel at `(x, y)` is set; the caller has
    /// checked the bounds.
    fn is_covered(&self, x: u32, y: u32) -> bool {
        let index = y as usize * self.width as usize + x as usize;
        self.bits[index / 64] & (1u64 << (index % 64)) != 0
    }
}

/// Builds the coverage mask of `polygons` for a window of `width` x `height`.
///
/// The polygons are **additive** (S17 §7): the mask is the union of their
/// areas. A polygon that reaches outside the window is clipped to it — the
/// pixels outside the window are simply not part of the mask. The result
/// covers no pixel at all when every polygon lies outside the window.
///
/// The window size is the size of the *client area* the mask is applied to:
/// the mask's `(0, 0)` is the window's top-left pixel.
pub fn build_mask(polygons: &[Polygon], width: u32, height: u32) -> RegionMask {
    let mut spans: Vec<MaskSpan> = Vec::new();
    for polygon in polygons {
        spans.extend(polygon_spans(polygon, width, height));
    }
    merge_spans(&mut spans);
    mask_from_spans(width, height, &spans)
}

/// One edge of a polygon.
#[derive(Debug, Clone, Copy)]
struct Edge {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl Edge {
    /// The crossing of the edge with the sample line of row `y`.
    ///
    /// The sample line of a row is `y + 0.5`. The crossing is
    ///
    /// ```text
    /// x = x0 + (x1 - x0) * (y + 0.5 - y0) / (y1 - y0)
    /// ```
    ///
    /// written in half-pixel units (`2 * x`) as the exact rational
    /// `(2 * x0 * d + n) / d` with `d = y1 - y0` and
    /// `n = (x1 - x0) * (2y + 1 - 2y0)`.
    fn crossing(&self, y: i32) -> Crossing {
        let d = self.y1 as i64 - self.y0 as i64;
        let n = (self.x1 as i64 - self.x0 as i64) * (2 * y as i64 + 1 - 2 * self.y0 as i64);
        let mut numerator = 2 * self.x0 as i128 * d as i128 + n as i128;
        let mut denominator = d as i128;
        if denominator < 0 {
            denominator = -denominator;
            numerator = -numerator;
        }
        Crossing {
            numerator,
            denominator,
            direction: if self.y1 > self.y0 { 1 } else { -1 },
        }
    }
}

/// A scanline crossing of one edge, kept as an exact rational.
#[derive(Debug, Clone, Copy)]
struct Crossing {
    /// The numerator of the crossing's x coordinate, in half-pixel units.
    numerator: i128,
    /// The denominator of that coordinate; always positive.
    denominator: i128,
    /// The winding change at the crossing: `+1` for an edge going down, `-1`
    /// for one going up.
    direction: i32,
}

impl Crossing {
    /// The first pixel whose centre is at or right of the crossing.
    ///
    /// The centre of pixel `i` is at `i + 0.5`, that is `2i + 1` in
    /// half-pixel units, so the first pixel at or right of the crossing `X`
    /// is `ceil((X - 1) / 2)`.
    fn first_pixel(&self) -> i128 {
        ceil_div(self.numerator - self.denominator, 2 * self.denominator)
    }
}

/// The covered pixels of one polygon, as runs.
fn polygon_spans(polygon: &Polygon, width: u32, height: u32) -> Vec<MaskSpan> {
    if polygon.len() < 3 || width == 0 || height == 0 {
        return Vec::new();
    }
    // The horizontal edges never cross a sample line: the sample line sits
    // half a pixel below a row, and the vertices are on whole pixels.
    let edges: Vec<Edge> = polygon
        .iter()
        .enumerate()
        .map(|(index, &(x0, y0))| {
            let (x1, y1) = polygon[(index + 1) % polygon.len()];
            Edge { x0, y0, x1, y1 }
        })
        .filter(|edge| edge.y0 != edge.y1)
        .collect();
    if edges.is_empty() {
        return Vec::new();
    }

    let min_y = edges
        .iter()
        .map(|edge| edge.y0.min(edge.y1))
        .min()
        .unwrap_or(0);
    let max_y = edges
        .iter()
        .map(|edge| edge.y0.max(edge.y1))
        .max()
        .unwrap_or(0);
    let first_row = min_y.max(0);
    let last_row = max_y.min(height as i32 - 1);

    let mut spans: Vec<MaskSpan> = Vec::new();
    for y in first_row..=last_row {
        // The half-open rule on the row: an edge counts when exactly one of
        // its ends is on or above the row. A vertex is therefore never
        // counted twice.
        let mut crossings: Vec<Crossing> = edges
            .iter()
            .filter(|edge| (edge.y0 <= y) != (edge.y1 <= y))
            .map(|edge| edge.crossing(y))
            .collect();
        if crossings.len() < 2 {
            continue;
        }
        crossings.sort_by(|a, b| (a.numerator * b.denominator).cmp(&(b.numerator * a.denominator)));

        // The non-zero winding rule: a pixel is covered while the winding
        // number around it is not zero.
        let mut winding = 0i32;
        let mut start: Option<usize> = None;
        for (index, crossing) in crossings.iter().enumerate() {
            winding += crossing.direction;
            if winding != 0 {
                if start.is_none() {
                    start = Some(index);
                }
            } else if let Some(start_index) = start.take() {
                push_span(
                    &mut spans,
                    &crossings[start_index],
                    &crossings[index],
                    y,
                    width,
                );
            }
        }
        if let Some(start_index) = start {
            // A closed polygon always returns to a winding number of zero
            // (the number is zero far to the left and to the right of it), so
            // this only happens for a degenerate input. The run then reaches
            // to the right edge of the window.
            push_span_to_edge(&mut spans, &crossings[start_index], y, width);
        }
    }
    spans
}

/// Adds the run between two crossings, clipped to the window.
fn push_span(spans: &mut Vec<MaskSpan>, left: &Crossing, right: &Crossing, y: i32, width: u32) {
    // The right end of the run is exclusive, so the last covered pixel is
    // the one before the first pixel at or right of the right crossing.
    push_pixel_range(spans, left.first_pixel(), right.first_pixel() - 1, y, width);
}

/// Adds the run from a crossing to the right edge of the window.
fn push_span_to_edge(spans: &mut Vec<MaskSpan>, left: &Crossing, y: i32, width: u32) {
    push_pixel_range(spans, left.first_pixel(), width as i128 - 1, y, width);
}

/// Adds a run of pixels, clipped to the window.
fn push_pixel_range(spans: &mut Vec<MaskSpan>, first: i128, last: i128, y: i32, width: u32) {
    let first = first.max(0);
    let last = last.min(width as i128 - 1);
    if last < first {
        return;
    }
    spans.push(MaskSpan {
        y: y as u32,
        x_start: first as u32,
        x_end: last as u32 + 1,
    });
}

/// Sorts the runs and merges the ones that overlap or touch.
fn merge_spans(spans: &mut Vec<MaskSpan>) {
    spans.sort_by_key(|span| (span.y, span.x_start));
    let mut merged: Vec<MaskSpan> = Vec::with_capacity(spans.len());
    for span in spans.drain(..) {
        match merged.last_mut() {
            Some(last) if last.y == span.y && span.x_start <= last.x_end => {
                last.x_end = last.x_end.max(span.x_end);
            }
            _ => merged.push(span),
        }
    }
    *spans = merged;
}

/// Turns runs into a mask.
fn mask_from_spans(width: u32, height: u32, spans: &[MaskSpan]) -> RegionMask {
    let pixels = width as usize * height as usize;
    let mut bits = vec![0u64; pixels.div_ceil(64)];
    for span in spans {
        let end = span.x_end.min(width);
        for x in span.x_start..end {
            let index = span.y as usize * width as usize + x as usize;
            bits[index / 64] |= 1u64 << (index % 64);
        }
    }
    RegionMask {
        width,
        height,
        bits,
    }
}

/// The smallest integer that is not smaller than `a / b`; `b` is positive.
fn ceil_div(a: i128, b: i128) -> i128 {
    let quotient = a.div_euclid(b);
    if a.rem_euclid(b) == 0 {
        quotient
    } else {
        quotient + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rectangle polygon with the given corners.
    fn rectangle(x0: i32, y0: i32, x1: i32, y1: i32) -> Polygon {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    #[test]
    fn a_rectangle_polygon_covers_every_pixel() {
        let mask = build_mask(&[rectangle(0, 0, 10, 10)], 10, 10);
        assert_eq!(mask.width(), 10);
        assert_eq!(mask.height(), 10);
        assert_eq!(mask.covered_pixels(), 100);
        for y in 0..10u32 {
            for x in 0..10u32 {
                assert!(mask.contains(x, y), "pixel ({x}, {y}) is not covered");
            }
        }
    }

    #[test]
    fn a_triangle_covers_the_pixels_below_its_diagonal() {
        // The right angle is at the top left, the hypotenuse runs from the
        // top right to the bottom left.
        let triangle: Polygon = vec![(0, 0), (10, 0), (0, 10)];
        let mask = build_mask(&[triangle], 10, 10);
        // The covered interval of a row is half-open, so the diagonal takes
        // the pixel centres strictly below it: row `y` covers `x = 0..8 - y`.
        for y in 0..10u32 {
            for x in 0..10u32 {
                assert_eq!(
                    mask.contains(x, y),
                    x + y <= 8,
                    "pixel ({x}, {y}) is covered wrongly"
                );
            }
        }
        assert_eq!(mask.covered_pixels(), 45);
    }

    #[test]
    fn a_concave_polygon_covers_its_notch() {
        // An L shape: the full width of the top four rows, the left four
        // columns of the rows below.
        let l_shape: Polygon = vec![(0, 0), (10, 0), (10, 4), (4, 4), (4, 10), (0, 10)];
        let mask = build_mask(&[l_shape], 10, 10);
        for y in 0..10u32 {
            for x in 0..10u32 {
                assert_eq!(
                    mask.contains(x, y),
                    y < 4 || x < 4,
                    "pixel ({x}, {y}) is covered wrongly"
                );
            }
        }
    }

    #[test]
    fn several_polygons_are_additive() {
        let polygons = vec![rectangle(0, 0, 5, 5), rectangle(10, 0, 15, 5)];
        let mask = build_mask(&polygons, 20, 10);
        assert_eq!(mask.covered_pixels(), 50);
        // Five rows per rectangle, two rectangles.
        assert_eq!(mask.spans().len(), 10);
        assert!(mask.contains(0, 0));
        assert!(mask.contains(14, 4));
        assert!(!mask.contains(5, 0));
        assert!(!mask.contains(9, 0));
    }

    #[test]
    fn a_polygon_outside_the_window_is_clipped() {
        // Bigger than the window on every side.
        let mask = build_mask(&[rectangle(-10, -10, 40, 40)], 20, 20);
        assert_eq!(mask.covered_pixels(), 400);

        // Half of it outside on the left and on the top.
        let mask = build_mask(&[rectangle(-10, -10, 10, 10)], 20, 20);
        assert_eq!(mask.covered_pixels(), 100);
        assert!(!mask.contains(10, 10));
        assert!(mask.contains(9, 9));

        // Entirely outside: nothing is covered.
        let mask = build_mask(&[rectangle(100, 100, 110, 110)], 20, 20);
        assert_eq!(mask.covered_pixels(), 0);
        assert!(mask.spans().is_empty());
    }

    #[test]
    fn the_fill_rule_is_the_non_zero_winding_rule() {
        // A square wound twice: the winding number inside it is two, so the
        // non-zero rule fills it. The even-odd rule would leave it empty.
        let wound_twice: Polygon = vec![
            (0, 0),
            (10, 0),
            (10, 10),
            (0, 10),
            (0, 0),
            (10, 0),
            (10, 10),
            (0, 10),
        ];
        let mask = build_mask(&[wound_twice], 10, 10);
        assert_eq!(mask.covered_pixels(), 100);
    }

    #[test]
    fn the_run_length_view_round_trips() {
        let polygons = vec![rectangle(0, 0, 5, 5), rectangle(3, 3, 9, 9)];
        let mask = build_mask(&polygons, 12, 12);
        let spans = mask.spans();

        // The two rectangles overlap in a 2 x 2 area, so the runs of the
        // overlapping rows are merged into one.
        assert_eq!(spans.len(), 9);
        assert_eq!(
            spans[0],
            MaskSpan {
                y: 0,
                x_start: 0,
                x_end: 5
            }
        );
        assert_eq!(
            spans[3],
            MaskSpan {
                y: 3,
                x_start: 0,
                x_end: 9
            }
        );
        assert_eq!(
            spans[8],
            MaskSpan {
                y: 8,
                x_start: 3,
                x_end: 9
            }
        );

        // The runs are ordered, do not overlap and stay inside the window.
        let mut covered = 0u64;
        for (index, span) in spans.iter().enumerate() {
            assert!(span.x_start < span.x_end);
            assert!(span.x_end <= mask.width());
            assert!(span.y < mask.height());
            covered += (span.x_end - span.x_start) as u64;
            if index > 0 {
                let previous = spans[index - 1];
                assert!(
                    previous.y < span.y || (previous.y == span.y && previous.x_end < span.x_start)
                );
            }
        }
        assert_eq!(covered, mask.covered_pixels());

        // The runs describe the same pixels as the mask.
        let rebuilt = mask_from_spans(mask.width(), mask.height(), &spans);
        assert_eq!(rebuilt, mask);
    }

    #[test]
    fn a_degenerate_polygon_covers_nothing() {
        // Fewer than three points.
        assert_eq!(
            build_mask(&[vec![(0, 0), (10, 10)]], 10, 10).covered_pixels(),
            0
        );
        // All points on one line.
        assert_eq!(
            build_mask(&[vec![(0, 0), (5, 0), (10, 0)]], 10, 10).covered_pixels(),
            0
        );
        // All points the same.
        assert_eq!(
            build_mask(&[vec![(3, 3), (3, 3), (3, 3)]], 10, 10).covered_pixels(),
            0
        );
        // No polygon at all.
        assert_eq!(build_mask(&[], 10, 10).covered_pixels(), 0);
        // A window without pixels.
        assert_eq!(
            build_mask(&[rectangle(0, 0, 10, 10)], 0, 10).covered_pixels(),
            0
        );
    }

    #[test]
    fn a_pixel_outside_the_window_is_not_covered() {
        let mask = build_mask(&[rectangle(0, 0, 10, 10)], 10, 10);
        assert!(!mask.contains(10, 0));
        assert!(!mask.contains(0, 10));
    }

    /// The winding number of `polygon` around the point `(px, py)`, computed
    /// the slow way: every edge that crosses the horizontal line through the
    /// point and lies to its left adds its direction.
    fn winding_number(polygon: &Polygon, px: f64, py: f64) -> i32 {
        let mut winding = 0;
        for index in 0..polygon.len() {
            let (x0, y0) = polygon[index];
            let (x1, y1) = polygon[(index + 1) % polygon.len()];
            if (y0 as f64 <= py) != (y1 as f64 <= py) {
                let t = (py - y0 as f64) / (y1 - y0) as f64;
                let x = x0 as f64 + t * (x1 - x0) as f64;
                if x <= px {
                    winding += if y1 > y0 { 1 } else { -1 };
                }
            }
        }
        winding
    }

    /// The mask must agree with a winding number computed pixel by pixel:
    /// the scanline rasteriser must not lose or gain a pixel.
    #[test]
    fn the_mask_agrees_with_a_per_pixel_winding_number() {
        let polygons: Vec<Polygon> = vec![
            vec![(0, 0), (20, 0), (20, 20), (0, 20)],
            vec![(0, 0), (20, 0), (0, 20)],
            vec![(0, 0), (20, 4), (6, 20), (0, 12)],
            vec![(2, 2), (18, 5), (12, 18), (3, 15)],
            vec![(0, 0), (20, 0), (20, 10), (10, 20), (0, 10)],
        ];
        for polygon in polygons {
            let mask = build_mask(std::slice::from_ref(&polygon), 20, 20);
            for y in 0..20u32 {
                for x in 0..20u32 {
                    // The sample point is nudged to the right, so a pixel
                    // whose centre lies exactly on an edge counts as being
                    // just inside it — the rule the rasteriser uses.
                    let winding = winding_number(&polygon, x as f64 + 0.5 + 1e-9, y as f64 + 0.5);
                    assert_eq!(
                        mask.contains(x, y),
                        winding != 0,
                        "pixel ({x}, {y}) of {polygon:?}"
                    );
                }
            }
        }
    }
}
