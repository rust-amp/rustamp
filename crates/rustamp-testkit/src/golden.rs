use std::path::{Path, PathBuf};

use crate::image::{ImageError, RgbaImage};

/// Skin namespace for the shipped look-alike default skin. Goldens are
/// committed to the repository under `tests/golden/lookalike/`.
pub const SKIN_LOOKALIKE: &str = "lookalike";
/// Skin namespace for the original base skin. References live in the external
/// `rustamp-testdata` repository (`RUSTAMP_TESTDATA`) and are never committed.
pub const SKIN_CLASSIC_BASE: &str = "classic-base";

/// Something that can be rendered headlessly to an RGBA pixel buffer.
pub trait Snapshot {
    /// Returns `(width, height, rgba)` with `rgba.len() == width * height * 4`.
    fn render_rgba(&self) -> (u32, u32, Vec<u8>);

    /// Convenience: renders into an [`RgbaImage`]. Panics on a bad buffer length.
    fn render_image(&self) -> RgbaImage {
        let (w, h, d) = self.render_rgba();
        RgbaImage::from_raw(w, h, d).expect("Snapshot::render_rgba returned a bad buffer")
    }
}

/// Pixel rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// Result of comparing two images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    /// `Some((expected, actual))` dimensions when they differ.
    pub size_mismatch: Option<((u32, u32), (u32, u32))>,
    /// Number of pixels whose max channel delta exceeds the tolerance. When
    /// sizes differ, every pixel of the union area counts.
    pub mismatched_pixels: u64,
    /// Total pixels compared (area of the union rectangle).
    pub total_pixels: u64,
    /// Bounding box of all mismatches.
    pub bbox: Option<Rect>,
    /// Largest per-channel absolute difference seen (over all pixels,
    /// including those within tolerance). `255` on size mismatch.
    pub max_delta: u8,
}

impl Diff {
    /// True when images have the same size and no pixel exceeds the tolerance.
    pub fn is_match(&self) -> bool {
        self.size_mismatch.is_none() && self.mismatched_pixels == 0
    }

    /// One-line human summary.
    pub fn summary(&self) -> String {
        if let Some((e, a)) = self.size_mismatch {
            return format!(
                "size mismatch: expected {}x{}, actual {}x{}",
                e.0, e.1, a.0, a.1
            );
        }
        match self.bbox {
            None => format!(
                "match ({} px, max delta {})",
                self.total_pixels, self.max_delta
            ),
            Some(b) => format!(
                "{} of {} px differ, max channel delta {}, bbox x={} y={} w={} h={}",
                self.mismatched_pixels,
                self.total_pixels,
                self.max_delta,
                b.x,
                b.y,
                b.width,
                b.height
            ),
        }
    }
}

fn pixel_delta(a: [u8; 4], b: [u8; 4]) -> u8 {
    a.iter()
        .zip(&b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

/// Compares two images. A pixel mismatches when any channel differs by more
/// than `tolerance` (0 = exact).
pub fn compare(expected: &RgbaImage, actual: &RgbaImage, tolerance: u8) -> Diff {
    let (ew, eh) = (expected.width(), expected.height());
    let (aw, ah) = (actual.width(), actual.height());
    if (ew, eh) != (aw, ah) {
        let (w, h) = (ew.max(aw), eh.max(ah));
        return Diff {
            size_mismatch: Some(((ew, eh), (aw, ah))),
            mismatched_pixels: u64::from(w) * u64::from(h),
            total_pixels: u64::from(w) * u64::from(h),
            bbox: Some(Rect {
                x: 0,
                y: 0,
                width: w,
                height: h,
            }),
            max_delta: 255,
        };
    }
    let (mut count, mut max_delta) = (0u64, 0u8);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in 0..eh {
        for x in 0..ew {
            let d = pixel_delta(expected.pixel(x, y), actual.pixel(x, y));
            max_delta = max_delta.max(d);
            if d > tolerance {
                count += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    Diff {
        size_mismatch: None,
        mismatched_pixels: count,
        total_pixels: u64::from(ew) * u64::from(eh),
        bbox: (count > 0).then(|| Rect {
            x: x0,
            y: y0,
            width: x1 - x0 + 1,
            height: y1 - y0 + 1,
        }),
        max_delta,
    }
}

/// Writes a diff visualisation: a dimmed copy of `expected` with differing
/// pixels painted red. On size mismatch the canvas is the union size and
/// everything outside the overlap is red.
pub fn write_diff_image(
    expected: &RgbaImage,
    actual: &RgbaImage,
    tolerance: u8,
    path: impl AsRef<Path>,
) -> Result<(), ImageError> {
    let w = expected.width().max(actual.width());
    let h = expected.height().max(actual.height());
    let mut out = RgbaImage::filled(w, h, [255, 0, 0, 255]);
    let same_size = expected.width() == actual.width() && expected.height() == actual.height();
    for y in 0..expected.height() {
        for x in 0..expected.width() {
            let e = expected.pixel(x, y);
            let differs = !same_size || pixel_delta(e, actual.pixel(x, y)) > tolerance;
            if !differs {
                let dim = |c: u8| (u16::from(c) * 2 / 5) as u8;
                out.set_pixel(x, y, [dim(e[0]), dim(e[1]), dim(e[2]), 255]);
            }
        }
    }
    out.save_png(path)
}

/// Finds the workspace root by walking up from this crate's manifest dir to the
/// first `Cargo.toml` containing `[workspace]`.
pub fn workspace_root() -> PathBuf {
    let start = Path::new(env!("CARGO_MANIFEST_DIR"));
    start
        .ancestors()
        .find(|d| {
            std::fs::read_to_string(d.join("Cargo.toml"))
                .map(|s| s.contains("[workspace]"))
                .unwrap_or(false)
        })
        .expect("workspace root not found")
        .to_path_buf()
}

/// Directory from `RUSTAMP_TESTDATA`, if set and non-empty.
pub fn testdata_dir() -> Option<PathBuf> {
    std::env::var_os("RUSTAMP_TESTDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Directory holding goldens for `skin`. `classic-base` resolves into
/// `$RUSTAMP_TESTDATA/golden/classic-base` (returns `None` when unset); every
/// other skin lives in `<workspace>/tests/golden/<skin>`.
pub fn golden_dir_for(skin: &str) -> Option<PathBuf> {
    if skin == SKIN_CLASSIC_BASE {
        testdata_dir().map(|d| d.join("golden").join(skin))
    } else {
        Some(workspace_root().join("tests").join("golden").join(skin))
    }
}

/// Outcome of [`check_golden`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoldenOutcome {
    /// Image equals the golden.
    Matched,
    /// Golden was written (bless mode).
    Blessed(PathBuf),
    /// No golden exists and blessing is off.
    Missing(PathBuf),
    /// Image differs; artefact paths are given.
    Mismatch {
        /// Comparison result.
        diff: Diff,
        /// Written `.actual.png`.
        actual_path: PathBuf,
        /// Written `.diff.png`.
        diff_path: PathBuf,
    },
}

/// Golden check with explicit directories, used by [`assert_golden`] and by
/// unit tests. `out_dir` receives `<skin>/<name>.actual.png` and
/// `<skin>/<name>.diff.png` on mismatch.
pub fn check_golden(
    golden_dir: &Path,
    out_dir: &Path,
    skin: &str,
    name: &str,
    img: &RgbaImage,
    bless: bool,
) -> Result<GoldenOutcome, ImageError> {
    let golden = golden_dir.join(format!("{name}.png"));
    if bless {
        img.save_png(&golden)?;
        return Ok(GoldenOutcome::Blessed(golden));
    }
    if !golden.exists() {
        return Ok(GoldenOutcome::Missing(golden));
    }
    let expected = RgbaImage::load_png(&golden)?;
    let diff = compare(&expected, img, 0);
    if diff.is_match() {
        return Ok(GoldenOutcome::Matched);
    }
    let base = out_dir.join(skin);
    let actual_path = base.join(format!("{name}.actual.png"));
    let diff_path = base.join(format!("{name}.diff.png"));
    img.save_png(&actual_path)?;
    write_diff_image(&expected, img, 0, &diff_path)?;
    Ok(GoldenOutcome::Mismatch {
        diff,
        actual_path,
        diff_path,
    })
}

/// Asserts `img` equals `tests/golden/<skin>/<name>.png`.
///
/// * `RUSTAMP_BLESS=1` writes/overwrites the golden instead.
/// * On mismatch writes `target/golden-out/<skin>/<name>.{actual,diff}.png`
///   and panics with a summary.
/// * For `classic-base` with `RUSTAMP_TESTDATA` unset, prints a skip note and
///   returns without failing.
pub fn assert_golden(name: &str, skin: &str, img: &RgbaImage) {
    let Some(dir) = golden_dir_for(skin) else {
        eprintln!(
            "SKIP golden {skin}/{name}: RUSTAMP_TESTDATA is not set (classic-base references live in rustamp-testdata)"
        );
        return;
    };
    let bless = std::env::var("RUSTAMP_BLESS").is_ok_and(|v| v == "1");
    let out_dir = workspace_root().join("target").join("golden-out");
    match check_golden(&dir, &out_dir, skin, name, img, bless) {
        Ok(GoldenOutcome::Matched) => {}
        Ok(GoldenOutcome::Blessed(p)) => eprintln!("BLESSED {}", p.display()),
        Ok(GoldenOutcome::Missing(p)) => panic!(
            "golden missing: {}\nhint: run `cargo xtask golden --bless` (or set RUSTAMP_BLESS=1) to create it",
            p.display()
        ),
        Ok(GoldenOutcome::Mismatch {
            diff,
            actual_path,
            diff_path,
        }) => panic!(
            "golden mismatch for {skin}/{name}: {}\n  actual: {}\n  diff:   {}\nhint: if intended, re-run with RUSTAMP_BLESS=1",
            diff.summary(),
            actual_path.display(),
            diff_path.display()
        ),
        Err(e) => panic!("golden {skin}/{name}: {e}"),
    }
}

/// Renders a [`Snapshot`] and runs [`assert_golden`] on it.
pub fn assert_golden_snapshot(name: &str, skin: &str, s: &dyn Snapshot) {
    assert_golden(name, skin, &s.render_image());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::filled(w, h, c)
    }

    #[test]
    fn identical_images_match() {
        let a = img(4, 3, [10, 20, 30, 255]);
        let d = compare(&a, &a.clone(), 0);
        assert!(d.is_match());
        assert_eq!(d.bbox, None);
        assert_eq!(d.max_delta, 0);
    }

    #[test]
    fn counts_bbox_and_max_delta() {
        let a = img(8, 8, [0, 0, 0, 255]);
        let mut b = a.clone();
        b.set_pixel(2, 3, [5, 0, 0, 255]);
        b.set_pixel(5, 6, [0, 0, 40, 255]);
        let d = compare(&a, &b, 0);
        assert_eq!(d.mismatched_pixels, 2);
        assert_eq!(d.max_delta, 40);
        assert_eq!(
            d.bbox,
            Some(Rect {
                x: 2,
                y: 3,
                width: 4,
                height: 4
            })
        );
    }

    #[test]
    fn tolerance_ignores_small_deltas() {
        let a = img(2, 2, [100, 100, 100, 255]);
        let mut b = a.clone();
        b.set_pixel(0, 0, [103, 100, 100, 255]);
        b.set_pixel(1, 1, [100, 100, 110, 255]);
        let d = compare(&a, &b, 5);
        assert_eq!(d.mismatched_pixels, 1);
        assert_eq!(d.max_delta, 10);
        assert_eq!(
            d.bbox,
            Some(Rect {
                x: 1,
                y: 1,
                width: 1,
                height: 1
            })
        );
    }

    #[test]
    fn size_mismatch_reported() {
        let d = compare(&img(2, 2, [0; 4]), &img(3, 2, [0; 4]), 0);
        assert!(!d.is_match());
        assert_eq!(d.size_mismatch, Some(((2, 2), (3, 2))));
        assert_eq!(d.mismatched_pixels, 6);
    }

    #[test]
    fn png_roundtrip_and_bad_length() {
        let mut a = img(3, 2, [1, 2, 3, 4]);
        a.set_pixel(2, 1, [9, 8, 7, 6]);
        let b = RgbaImage::from_png_bytes(&a.to_png_bytes().unwrap()).unwrap();
        assert_eq!(a, b);
        assert!(RgbaImage::from_raw(2, 2, vec![0; 15]).is_err());
    }

    #[test]
    fn diff_image_marks_red() {
        let dir = tempfile::tempdir().unwrap();
        let a = img(3, 1, [200, 200, 200, 255]);
        let mut b = a.clone();
        b.set_pixel(1, 0, [0, 0, 0, 255]);
        let p = dir.path().join("d.png");
        write_diff_image(&a, &b, 0, &p).unwrap();
        let d = RgbaImage::load_png(&p).unwrap();
        assert_eq!(d.pixel(1, 0), [255, 0, 0, 255]);
        assert_eq!(d.pixel(0, 0), [80, 80, 80, 255]);
    }

    #[test]
    fn golden_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let (g, o) = (tmp.path().join("g"), tmp.path().join("o"));
        let a = img(4, 4, [1, 2, 3, 255]);

        let r = check_golden(&g, &o, "lookalike", "x", &a, false).unwrap();
        assert!(matches!(r, GoldenOutcome::Missing(_)));

        let r = check_golden(&g, &o, "lookalike", "x", &a, true).unwrap();
        assert!(matches!(r, GoldenOutcome::Blessed(_)));
        assert_eq!(
            check_golden(&g, &o, "lookalike", "x", &a, false).unwrap(),
            GoldenOutcome::Matched
        );

        let mut b = a.clone();
        b.set_pixel(1, 1, [9, 9, 9, 255]);
        match check_golden(&g, &o, "lookalike", "x", &b, false).unwrap() {
            GoldenOutcome::Mismatch {
                diff,
                actual_path,
                diff_path,
            } => {
                assert_eq!(diff.mismatched_pixels, 1);
                assert!(actual_path.ends_with("lookalike/x.actual.png") && actual_path.exists());
                assert!(diff_path.ends_with("lookalike/x.diff.png") && diff_path.exists());
            }
            other => panic!("unexpected {other:?}"),
        }

        // Re-bless overwrites.
        check_golden(&g, &o, "lookalike", "x", &b, true).unwrap();
        assert_eq!(
            check_golden(&g, &o, "lookalike", "x", &b, false).unwrap(),
            GoldenOutcome::Matched
        );
    }

    #[test]
    fn workspace_root_found() {
        assert!(workspace_root().join("Cargo.toml").exists());
    }

    #[test]
    fn classic_base_skips_without_testdata() {
        if testdata_dir().is_none() {
            // Must not panic even though no golden exists anywhere.
            assert_golden("nonexistent", SKIN_CLASSIC_BASE, &img(1, 1, [0; 4]));
        }
    }
}
