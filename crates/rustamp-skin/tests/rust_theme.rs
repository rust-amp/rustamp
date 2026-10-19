// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Verification tests for the Rust-themed built-in skin (task card J2).
//!
//! Validates:
//! 1. All 32 entries + `nums_ex.bmp` load completely with zero missing-file substitutions.
//! 2. Static check: Every sheet's width and height matches the exact spec dimensions.
//! 3. All 11 cursors are valid Windows `.cur` files of 32x32 px with valid hotspots.
//! 4. Text files (`pledit.txt`, `viscolor.txt`, `region.txt`) parse into valid theme data.
//! 5. Fixture comparison: When `RUSTAMP_TESTSKINS` is set, compares widths and heights
//!    against `base-2.91.wsz`, skipping cleanly with a note when unset.

use std::collections::HashSet;
use std::io::Read;

use rustamp_skin::rust_theme;
use rustamp_skin::webamp::{self, Sheet};
use rustamp_skin::{CursorKind, PlaylistStyle, Region};

/// Exact spec dimensions per `specs/S01-window-geometry.md` §5 (lines 680–697).
const SPEC_SHEET_SIZES: &[(Sheet, u32, u32, &str)] = &[
    (Sheet::Main, 275, 116, "S01 §5, line 680"),
    (Sheet::Titlebar, 344, 87, "S01 §5, line 681"),
    (Sheet::Cbuttons, 136, 36, "S01 §5, line 682"),
    (Sheet::Shufrep, 92, 85, "S01 §5, line 683"),
    (Sheet::Posbar, 307, 10, "S01 §5, line 684"),
    (Sheet::Volume, 68, 433, "S01 §5, line 685"),
    (Sheet::Balance, 68, 433, "S01 §5, line 686"),
    (Sheet::Monoster, 58, 24, "S01 §5, line 687"),
    (Sheet::Playpaus, 42, 9, "S01 §5, line 688"),
    (Sheet::Numbers, 99, 13, "S01 §5, line 689"),
    (Sheet::NumsEx, 108, 13, "S01 §2.3"),
    (Sheet::Text, 155, 74, "S01 §5, line 690"),
    (Sheet::Pledit, 280, 186, "S01 §5, line 691"),
    (Sheet::EqMain, 275, 315, "S01 §5, line 692"),
    (Sheet::EqEx, 275, 82, "S01 §5, line 693"),
    (Sheet::Gen, 194, 109, "S01 §5, line 694"),
    (Sheet::GenEx, 130, 75, "S01 §5, line 695"),
    (Sheet::Mb, 234, 119, "S01 §5, line 696"),
    (Sheet::Video, 234, 119, "S01 §5, line 697"),
];

const CURSOR_NAMES_11: &[(&str, u16, u16)] = &[
    ("normal.cur", 0, 0),
    ("pnormal.cur", 0, 0),
    ("close.cur", 4, 4),
    ("titlebar.cur", 8, 8),
    ("posbar.cur", 8, 8),
    ("volbal.cur", 8, 8),
    ("eqslid.cur", 8, 8),
    ("eqtitle.cur", 8, 8),
    ("psize.cur", 8, 8),
    ("ptbar.cur", 8, 8),
    ("pvscroll.cur", 8, 8),
];

#[test]
fn rust_theme_loads_all_32_entries_without_substitutions() {
    let skin = rust_theme::builtin_skin().expect("built-in Rust theme skin must load cleanly");

    // Main window sprites present
    assert!(skin.has_main_window_sprites());

    // Zero substitutions: the built-in skin is complete with all its own files
    assert_eq!(
        skin.substitutions(),
        &[],
        "built-in Rust theme must not require fallback substitutions"
    );

    // All 19 sheets exist
    for &sheet in Sheet::ALL {
        let raster = skin
            .sheet(sheet)
            .unwrap_or_else(|| panic!("missing sheet {:?} in Rust theme", sheet));
        assert!(raster.width() > 0, "sheet {:?} has 0 width", sheet);
        assert!(raster.height() > 0, "sheet {:?} has 0 height", sheet);
    }

    // All 11 cursors are loaded and are .cur files
    assert_eq!(
        skin.cursors().len(),
        11,
        "Rust theme must ship all 11 cursors"
    );
    for (name, _, _) in CURSOR_NAMES_11 {
        let cur = skin
            .cursor(name)
            .unwrap_or_else(|| panic!("missing cursor {name}"));
        assert_eq!(cur.kind, CursorKind::Cur);
        assert!(!cur.bytes.is_empty());
    }

    // Text files loaded and parsed
    assert_eq!(skin.pledit().normal, [0xDF, 0x7B, 0x38]);
    assert_eq!(skin.pledit().current, [0xFF, 0xFF, 0xFF]);
    assert_eq!(skin.pledit().normal_bg, [0x14, 0x15, 0x17]);
    assert_eq!(skin.pledit().selected_bg, [0x00, 0x00, 0xC6]);

    // Viscolors: 24 colors loaded
    let vis = skin.viscolors();
    assert_eq!(vis[0], [12, 13, 14]); // background
    assert_eq!(vis[1], [38, 32, 29]); // grid
    assert_eq!(vis[8], [229, 90, 28]); // Rust orange
    assert_eq!(vis[23], [255, 224, 130]); // peak dots

    // Region: Normal, WindowShade, Equalizer non-empty
    assert!(!skin.region().normal().is_empty());
    assert!(!skin.region().window_shade().is_empty());
    assert!(!skin.region().polygons("equalizer").is_empty());
}

#[test]
fn static_sheet_dimensions_match_spec_exact() {
    for &(sheet, expected_w, expected_h, cite) in SPEC_SHEET_SIZES {
        let image = rust_theme::sheet_image(sheet)
            .unwrap_or_else(|| panic!("failed to decode sheet {:?}", sheet));

        assert_eq!(
            (image.width(), image.height()),
            (expected_w, expected_h),
            "STATIC CHECK FAILURE for sheet {:?} (cited in {}): expected {}x{}, got {}x{}",
            sheet,
            cite,
            expected_w,
            expected_h,
            image.width(),
            image.height()
        );
    }
}

#[test]
fn cursor_dimensions_and_formats_are_valid() {
    for &(name, expected_hx, expected_hy) in CURSOR_NAMES_11 {
        let bytes = rust_theme::cursor_bytes(name)
            .unwrap_or_else(|| panic!("missing cursor bytes for {}", name));

        assert!(
            bytes.len() >= 22 + 40,
            "cursor {} is too small ({} bytes)",
            name,
            bytes.len()
        );

        // Header check: idReserved == 0, idType == 2 (CUR), idCount == 1
        assert_eq!(
            &bytes[0..4],
            &[0, 0, 2, 0],
            "invalid CUR header for {}",
            name
        );
        let id_count = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        assert_eq!(id_count, 1, "expected 1 cursor image for {}", name);

        // Directory entry: width, height, hotspot
        let w = if bytes[6] == 0 { 256 } else { bytes[6] as u32 };
        let h = if bytes[7] == 0 { 256 } else { bytes[7] as u32 };
        assert_eq!((w, h), (32, 32), "cursor {} must be 32x32", name);

        let hx = u16::from_le_bytes(bytes[10..12].try_into().unwrap());
        let hy = u16::from_le_bytes(bytes[12..14].try_into().unwrap());
        assert_eq!(
            (hx, hy),
            (expected_hx, expected_hy),
            "hotspot mismatch for {}",
            name
        );

        // BITMAPINFOHEADER at offset 22
        let dib_w = i32::from_le_bytes(bytes[26..30].try_into().unwrap());
        let dib_h = i32::from_le_bytes(bytes[30..34].try_into().unwrap());
        assert_eq!(dib_w, 32, "DIB width for {} must be 32", name);
        assert_eq!(
            dib_h, 64,
            "DIB height for {} must be 64 (XOR+AND mask)",
            name
        );
    }
}

#[test]
fn text_files_parse_results_match_rust_theme() {
    let pledit = PlaylistStyle::parse(rust_theme::pledit_str());
    assert_eq!(pledit.normal, [0xDF, 0x7B, 0x38]);
    assert_eq!(pledit.current, [0xFF, 0xFF, 0xFF]);
    assert_eq!(pledit.normal_bg, [0x14, 0x15, 0x17]);
    assert_eq!(pledit.selected_bg, [0x00, 0x00, 0xC6]);
    assert_eq!(pledit.font.as_deref(), Some("Arial"));

    let vis = rustamp_skin::viscolor::parse_viscolors(rust_theme::viscolor_str());
    assert_eq!(vis.len(), 24);
    assert_eq!(vis[0], [12, 13, 14]); // background
    assert_eq!(vis[1], [38, 32, 29]); // grid
    assert_eq!(vis[8], [229, 90, 28]); // Rust orange
    assert_eq!(vis[23], [255, 224, 130]); // peak dots

    let region = Region::parse(rust_theme::region_str());
    assert_eq!(region.normal().len(), 1);
    assert_eq!(
        region.normal()[0],
        vec![(0, 0), (275, 0), (275, 116), (0, 116)]
    );
    assert_eq!(region.window_shade().len(), 1);
    assert_eq!(
        region.window_shade()[0],
        vec![(0, 0), (275, 0), (275, 14), (0, 14)]
    );
}

#[test]
fn compare_sheet_dimensions_against_fixture_when_env_set() {
    let Ok(dir) = std::env::var("RUSTAMP_TESTSKINS") else {
        eprintln!("RUSTAMP_TESTSKINS is unset; skipping fixture dimension comparison");
        return;
    };

    let wsz_path = std::path::Path::new(&dir).join("base-2.91.wsz");
    if !wsz_path.exists() {
        eprintln!(
            "Fixture file {} does not exist; skipping fixture dimension comparison",
            wsz_path.display()
        );
        return;
    }

    let file = std::fs::File::open(&wsz_path).expect("failed to open fixture wsz");
    let mut archive = zip::ZipArchive::new(file).expect("failed to parse fixture zip");

    // LEGAL RULE: We compare dimensions only (width and height), NEVER pixels or artwork.
    let mut checked_sheets = HashSet::new();
    let mut checked_cursors = HashSet::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).expect("failed to get zip entry");
        let name = entry.name().expect("valid entry name").to_ascii_lowercase();
        let stem = name.rsplit('/').next().unwrap_or(&name).to_string();

        if stem.ends_with(".bmp") {
            let mut header = [0u8; 26];
            if entry.read_exact(&mut header).is_ok() && header.starts_with(b"BM") {
                let orig_w = i32::from_le_bytes(header[18..22].try_into().unwrap()) as u32;
                let orig_h = i32::from_le_bytes(header[22..26].try_into().unwrap()).unsigned_abs();

                let matching_sheet = Sheet::ALL
                    .iter()
                    .find(|s| s.file_name().eq_ignore_ascii_case(&stem));

                if let Some(&sheet) = matching_sheet {
                    let our_image = rust_theme::sheet_image(sheet)
                        .unwrap_or_else(|| panic!("missing sheet {stem} in shipped Rust theme"));
                    assert_eq!(
                        (our_image.width(), our_image.height()),
                        (orig_w, orig_h),
                        "Dimension mismatch with fixture for sheet {stem}: original is {orig_w}x{orig_h}, ours is {}x{}",
                        our_image.width(),
                        our_image.height()
                    );
                    checked_sheets.insert(stem);
                }
            }
        } else if stem.ends_with(".cur") {
            let mut header = [0u8; 22];
            if entry.read_exact(&mut header).is_ok() && header[0..4] == [0, 0, 2, 0] {
                let orig_w = if header[6] == 0 {
                    256
                } else {
                    header[6] as u32
                };
                let orig_h = if header[7] == 0 {
                    256
                } else {
                    header[7] as u32
                };

                let our_bytes = rust_theme::cursor_bytes(&stem)
                    .unwrap_or_else(|| panic!("missing cursor {stem} in shipped Rust theme"));
                let our_w = if our_bytes[6] == 0 {
                    256
                } else {
                    our_bytes[6] as u32
                };
                let our_h = if our_bytes[7] == 0 {
                    256
                } else {
                    our_bytes[7] as u32
                };

                assert_eq!(
                    (our_w, our_h),
                    (orig_w, orig_h),
                    "Cursor dimension mismatch for {stem}: original is {orig_w}x{orig_h}, ours is {our_w}x{our_h}"
                );
                checked_cursors.insert(stem);
            }
        }
    }

    // Assert that every original sheet from base-2.91.wsz was checked
    assert_eq!(
        checked_sheets.len(),
        18,
        "Expected to verify 18 classic sheets against fixture, verified {}",
        checked_sheets.len()
    );
    assert_eq!(
        checked_cursors.len(),
        11,
        "Expected to verify 11 cursors against fixture, verified {}",
        checked_cursors.len()
    );
}

#[test]
fn every_sprite_exists_with_expected_dimensions_in_rust_theme() {
    let skin = rust_theme::builtin_skin().expect("built-in Rust theme must load");

    for s in webamp::SPRITES {
        let sheet = skin
            .sheet(s.sheet)
            .unwrap_or_else(|| panic!("sheet {:?} missing for sprite {}", s.sheet, s.name));

        assert!(
            s.width > 0 && s.height > 0,
            "sprite {} has zero dimensions: {}x{}",
            s.name,
            s.width,
            s.height
        );

        assert!(
            s.x + s.width <= sheet.width(),
            "sprite {} ({}x{}) at ({}, {}) extends beyond sheet width {} in {:?}",
            s.name,
            s.width,
            s.height,
            s.x,
            s.y,
            sheet.width(),
            s.sheet
        );

        assert!(
            s.y + s.height <= sheet.height(),
            "sprite {} ({}x{}) at ({}, {}) extends beyond sheet height {} in {:?}",
            s.name,
            s.width,
            s.height,
            s.x,
            s.y,
            sheet.height(),
            s.sheet
        );
    }
}
