// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Tests for the built-in clean-room lookalike default skin (task card J1).

use rustamp_skin::webamp::{self, Sheet};
use rustamp_skin::{BuiltinDefaultSkin, MemorySource, PLACEHOLDER_RGBA, Skin};

#[test]
fn builtin_skin_loads_successfully() {
    let mut source = MemorySource::new("test builtin");
    let skin = Skin::from_source(&mut source, &BuiltinDefaultSkin)
        .expect("the built-in lookalike skin must load without error");

    assert!(skin.has_main_window_sprites());

    // Verify all 16 sheets are present in the loaded skin
    for &sheet in Sheet::ALL {
        let raster = skin
            .sheet(sheet)
            .unwrap_or_else(|| panic!("missing sheet {:?} in built-in skin", sheet));
        assert!(raster.width() > 0, "sheet {:?} has 0 width", sheet);
        assert!(raster.height() > 0, "sheet {:?} has 0 height", sheet);
    }
}

#[test]
fn every_sprite_exists_with_expected_dimensions() {
    let mut source = MemorySource::new("test sprites");
    let skin = Skin::from_source(&mut source, &BuiltinDefaultSkin)
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
    let mut source = MemorySource::new("test placeholder");
    let skin = Skin::from_source(&mut source, &BuiltinDefaultSkin)
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
    let mut source = MemorySource::new("test text font");
    let skin = Skin::from_source(&mut source, &BuiltinDefaultSkin)
        .expect("the built-in lookalike skin must load");

    let text_sheet = skin
        .sheet(Sheet::Text)
        .expect("Sheet::Text must exist in built-in skin");

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
