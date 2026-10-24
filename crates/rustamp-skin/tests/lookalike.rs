// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Tests for the built-in clean-room lookalike skin (task card J1).
//!
//! The lookalike is no longer the default — `BuiltinDefaultSkin` is the Rust
//! theme (task card J2) — so every test here names `BuiltinLookalikeSkin`
//! explicitly. A previous revision of this file imported `BuiltinDefaultSkin`
//! and silently retargeted to the Rust theme when the default moved; the
//! `builtin_default_skin_is_still_the_rust_theme` test below pins that routing
//! so the two skins cannot be confused again.

use rustamp_skin::lookalike;
use rustamp_skin::webamp::{self, Sheet};
use rustamp_skin::{
    BuiltinDefaultSkin, BuiltinLookalikeSkin, PLACEHOLDER_RGBA, Skin, SkinFallback,
};

/// The lookalike's own sheet dimensions, decoded from the committed assets.
///
/// Five of these are deliberately non-spec (a fragile coupling the W10 review
/// flagged, now pinned): `titlebar.bmp` is 350 wide (spec 344),
/// `balance.bmp` is 47 wide (spec 68), `monoster.bmp` is 56 wide (spec 58),
/// `eq_ex.bmp` is 275 x 56 (spec 275 x 82) and `gen.bmp` is 178 x 86
/// (spec 194 x 109). Every sprite still fits inside them — see
/// `every_sprite_fits_within_the_lookalike_sheets` — but any sprite-table
/// change must re-check these five first.
const LOOKALIKE_SHEET_SIZES: &[(Sheet, u32, u32)] = &[
    (Sheet::Main, 275, 116),
    (Sheet::Titlebar, 350, 87),
    (Sheet::Cbuttons, 136, 36),
    (Sheet::Shufrep, 92, 85),
    (Sheet::Posbar, 307, 10),
    (Sheet::Volume, 68, 433),
    (Sheet::Balance, 47, 433),
    (Sheet::Monoster, 56, 24),
    (Sheet::Playpaus, 42, 9),
    (Sheet::Numbers, 99, 13),
    (Sheet::NumsEx, 108, 13),
    (Sheet::Text, 155, 74),
    (Sheet::Pledit, 280, 186),
    (Sheet::EqMain, 275, 315),
    (Sheet::EqEx, 275, 56),
    (Sheet::Gen, 178, 86),
];

#[test]
fn lookalike_sheets_decode_at_their_own_dimensions() {
    for &(sheet, expected_w, expected_h) in LOOKALIKE_SHEET_SIZES {
        // The lookalike carries its own art for exactly these 16 sheets.
        assert!(
            lookalike::sheet_bytes(sheet).is_some(),
            "the lookalike must carry its own {:?}",
            sheet
        );
        let image = BuiltinLookalikeSkin
            .sheet_image(sheet)
            .unwrap_or_else(|| panic!("failed to decode lookalike sheet {:?}", sheet));
        assert_eq!(
            (image.width(), image.height()),
            (expected_w, expected_h),
            "lookalike sheet {:?}: expected {}x{}, got {}x{}",
            sheet,
            expected_w,
            expected_h,
            image.width(),
            image.height()
        );
    }

    // The lookalike carries no art of its own for the three newer sheets;
    // those come from the fallback fill at the spec extents.
    for sheet in [Sheet::GenEx, Sheet::Mb, Sheet::Video] {
        assert!(
            lookalike::sheet_bytes(sheet).is_none(),
            "the lookalike must not carry its own {:?}",
            sheet
        );
    }
}

#[test]
fn lookalike_source_loads_with_its_own_bytes() {
    let mut source = BuiltinLookalikeSkin::source();
    let skin = Skin::from_source(&mut source, &BuiltinLookalikeSkin)
        .expect("the built-in lookalike skin must load without error");

    assert!(skin.has_main_window_sprites());

    // The 16 lookalike sheets keep their embedded bytes; the three newer
    // sheets are generated fallback fills, which carry no raw bytes.
    for &(sheet, _, _) in LOOKALIKE_SHEET_SIZES {
        assert!(
            skin.sheet(sheet).is_some(),
            "missing sheet {:?} in lookalike skin",
            sheet
        );
        assert!(
            !skin.sheet_raw(sheet).unwrap().is_empty(),
            "sheet {:?} must keep its own embedded bytes, not a fill",
            sheet
        );
    }
    for sheet in [Sheet::GenEx, Sheet::Mb, Sheet::Video] {
        let raster = skin
            .sheet(sheet)
            .unwrap_or_else(|| panic!("missing fallback sheet {:?}", sheet));
        assert_eq!(
            (raster.width(), raster.height()),
            webamp::sheet_extent(sheet),
            "fallback sheet {:?} must sit at its spec extent",
            sheet
        );
        assert!(
            skin.sheet_raw(sheet).unwrap().is_empty(),
            "fallback sheet {:?} must be a generated fill",
            sheet
        );
    }
}

#[test]
fn every_sprite_fits_within_the_lookalike_sheets() {
    let mut source = BuiltinLookalikeSkin::source();
    let skin = Skin::from_source(&mut source, &BuiltinLookalikeSkin)
        .expect("the built-in lookalike skin must load");

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

#[test]
fn no_sprite_is_placeholder_magenta_colour() {
    let mut source = BuiltinLookalikeSkin::source();
    let skin = Skin::from_source(&mut source, &BuiltinLookalikeSkin)
        .expect("the built-in lookalike skin must load");

    for s in webamp::SPRITES {
        let sheet = skin.sheet(s.sheet).unwrap();

        let mut magenta_count = 0;
        let total_pixels = (s.width * s.height) as usize;

        for py in s.y..(s.y + s.height) {
            for px in s.x..(s.x + s.width) {
                if sheet.pixel(px, py) == Some(PLACEHOLDER_RGBA) {
                    magenta_count += 1;
                }
            }
        }

        // Assert no sprite is entirely composed of the magenta placeholder
        assert!(
            magenta_count < total_pixels,
            "sprite {} in {:?} is entirely the magenta placeholder colour",
            s.name,
            s.sheet
        );
    }
}

#[test]
fn text_font_renders_legible_glyphs() {
    let mut source = BuiltinLookalikeSkin::source();
    let skin = Skin::from_source(&mut source, &BuiltinLookalikeSkin)
        .expect("the built-in lookalike skin must load");

    let text_sheet = skin
        .sheet(Sheet::Text)
        .expect("Sheet::Text must exist in lookalike skin");

    let cell_w = webamp::TEXT_CELL.0;
    let cell_h = webamp::TEXT_CELL.1;
    assert_eq!((cell_w, cell_h), (5, 6));

    // Sample background colour from cell (30, 0) which is space
    let space_x = 30 * cell_w;
    let space_y = 0;
    let bg_color = text_sheet
        .pixel(space_x, space_y)
        .expect("space pixel must exist");

    for &(ch, col, row) in webamp::TEXT_GLYPHS {
        let gx = col * cell_w;
        let gy = row * cell_h;

        let mut ink_pixels = 0;
        let total_cell_pixels = (cell_w * cell_h) as usize;

        for y in gy..(gy + cell_h) {
            for x in gx..(gx + cell_w) {
                if let Some(pixel) = text_sheet.pixel(x, y)
                    && pixel != bg_color
                {
                    ink_pixels += 1;
                }
            }
        }

        if ch == ' ' {
            assert_eq!(
                ink_pixels, 0,
                "space glyph at col {col}, row {row} must have 0 ink pixels"
            );
        } else {
            // Every printable glyph must have coverage: at least 1 ink pixel,
            // but not completely solid (ink < total_cell_pixels), ensuring distinct readable shapes.
            assert!(
                ink_pixels > 0,
                "glyph {ch:?} at col {col}, row {row} has 0 ink pixels (blank)"
            );
            assert!(
                ink_pixels < total_cell_pixels,
                "glyph {ch:?} at col {col}, row {row} is completely solid (not a legible character)"
            );
        }
    }
}

/// Guards the F6 confusion directly: the built-in *default* skin is the Rust
/// theme — complete with all 19 sheets and 11 cursors — and must stay that
/// way. The lookalike is only reachable through `BuiltinLookalikeSkin`.
#[test]
fn builtin_default_skin_is_still_the_rust_theme() {
    let mut source = BuiltinDefaultSkin::source();
    let skin = Skin::from_source(&mut source, &BuiltinDefaultSkin)
        .expect("the built-in default skin must load");

    for &sheet in Sheet::ALL {
        skin.sheet(sheet)
            .unwrap_or_else(|| panic!("missing sheet {sheet:?} in default skin"));
    }
    assert_eq!(
        skin.sheet(Sheet::Main).unwrap().width(),
        275,
        "the default skin's main.bmp must be the 275-wide Rust theme art"
    );
    assert_eq!(
        skin.cursors().len(),
        11,
        "the default skin must ship the Rust theme's 11 cursors"
    );
    assert!(
        skin.substitutions().is_empty(),
        "the built-in default skin must load with no substitutions, got {:?}",
        skin.substitutions()
    );
}
