// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless render tests for the main-menu popup.
//!
//! The pixel checks build a small **synthetic** skin in memory (the same
//! one-colour-sheet pattern as `tests/render.rs`), so the expected pixels are
//! unambiguous. No real skin, reference capture or window is used here. What
//! the tests pin:
//!
//! * the frame is exactly the measured client size (323 x 569, S04 §1/§8);
//! * labels draw through the skin's `text.bmp` font;
//! * separators etch their band;
//! * flipping a check changes pixels only inside the check cell (x = 16-29,
//!   S04 §3.1) of that row's band;
//! * flipping the time flag changes no main-menu pixel: the elapsed/remaining
//!   checks live in the Options popup (S04 §4.1), not on the main menu
//!   (S04 §2).

use rustamp_skin::{BuiltinDefaultSkin, MemorySource, Skin};
use rustamp_ui::menu::{
    MAIN_MENU_CLIENT_SIZE, MainMenuId, MenuFlags, SubmenuId, check_state, render_main_menu,
    render_submenu, row_bands, submenu_client_size, submenu_entries, top_level,
};
use rustamp_ui::{MainWindowState, menu};

const FACE_SAMPLE: (i32, i32) = (5, 5);
const GLYPH_A: [u8; 3] = [220, 230, 240];

/// Encodes an uncompressed 24-bit BMP (bottom-up rows, BGR, 4-byte padded).
fn bmp_24(width: u32, height: u32, pixels: &[[u8; 3]]) -> Vec<u8> {
    assert_eq!(pixels.len(), (width * height) as usize);
    let row_bytes = width * 3;
    let padding = (4 - (row_bytes % 4)) % 4;
    let stride = row_bytes + padding;
    let image_size = stride * height;
    let file_size = 54 + image_size;

    let mut out = Vec::with_capacity(file_size as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&file_size.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    for y in (0..height).rev() {
        for x in 0..width {
            let p = pixels[(y * width + x) as usize];
            out.push(p[2]);
            out.push(p[1]);
            out.push(p[0]);
        }
        out.extend(std::iter::repeat_n(0u8, padding as usize));
    }
    out
}

/// A one-colour sheet with the given painted regions.
fn sheet(
    width: u32,
    height: u32,
    base: [u8; 3],
    regions: &[(u32, u32, u32, u32, [u8; 3])],
) -> Vec<u8> {
    let mut pixels = vec![base; (width * height) as usize];
    for &(x, y, w, h, rgb) in regions {
        for dy in y..y + h {
            for dx in x..x + w {
                pixels[(dy * width + dx) as usize] = rgb;
            }
        }
    }
    bmp_24(width, height, &pixels)
}

/// A skin whose `text.bmp` carries two known glyph cells (`a` and `b`); every
/// other sheet falls back to the built-in default.
fn synthetic_skin() -> Skin {
    let mut source = MemorySource::new("synthetic-menu");
    source.add(
        "text.bmp",
        sheet(
            155,
            18,
            [0, 0, 0],
            &[(0, 0, 5, 6, GLYPH_A), (5, 0, 5, 6, [46, 47, 48])],
        ),
    );
    Skin::from_source(&mut source, &BuiltinDefaultSkin).expect("synthetic skin loads")
}

fn face(skin: &Skin) -> [u8; 4] {
    let frame = render_main_menu(skin, &MainWindowState::default(), &MenuFlags::default());
    frame.pixel(FACE_SAMPLE.0, FACE_SAMPLE.1).unwrap()
}

/// Pixel-wise difference of two frames: the differing pixel coordinates.
fn diff(a: &rustamp_gfx::Image, b: &rustamp_gfx::Image) -> Vec<(u32, u32)> {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    let mut out = Vec::new();
    for y in 0..a.height() {
        for x in 0..a.width() {
            if a.pixel(x as i32, y as i32) != b.pixel(x as i32, y as i32) {
                out.push((x, y));
            }
        }
    }
    out
}

#[test]
fn the_menu_frame_is_the_measured_client_size() {
    let skin = synthetic_skin();
    let frame = render_main_menu(&skin, &MainWindowState::default(), &MenuFlags::default());
    assert_eq!((frame.width(), frame.height()), MAIN_MENU_CLIENT_SIZE);
    assert_eq!((frame.width(), frame.height()), (323, 569));
}

#[test]
fn labels_draw_through_the_skin_font() {
    let skin = synthetic_skin();
    let frame = render_main_menu(&skin, &MainWindowState::default(), &MenuFlags::default());
    let background = face(&skin);
    // Row 0 (the `Nullsoft Winamp...` title, S04 §2 row 1, band 0..35,
    // labels at x = 45 per the S04 §3.5 text zone) holds `a` cells.
    let mut glyph_pixels = 0;
    for y in 0..35 {
        for x in 45..200 {
            let pixel = frame.pixel(x, y).unwrap();
            if [pixel[0], pixel[1], pixel[2]] == GLYPH_A {
                glyph_pixels += 1;
            }
        }
    }
    assert!(glyph_pixels > 0, "no label glyph reached the frame");
    // Away from labels, checks and arrows the face shows through.
    assert_eq!(
        frame.pixel(FACE_SAMPLE.0, FACE_SAMPLE.1).unwrap(),
        background
    );
}

#[test]
fn separators_etch_their_band() {
    let skin = synthetic_skin();
    let frame = render_main_menu(&skin, &MainWindowState::default(), &MenuFlags::default());
    let background = face(&skin);
    // First separator band is 35..46 (the measured y = 40 rule, S04 §2);
    // the etched pair sits mid-band.
    assert_eq!(frame.pixel(100, 35).unwrap(), background);
    assert_ne!(frame.pixel(100, 40).unwrap(), background);
    assert_ne!(frame.pixel(100, 41).unwrap(), background);
    assert_eq!(frame.pixel(100, 45).unwrap(), background);
}

#[test]
fn a_check_flip_only_touches_the_check_cell_of_its_row() {
    let skin = synthetic_skin();
    let flags = MenuFlags::default();
    let plain = render_main_menu(&skin, &MainWindowState::default(), &flags);
    let checked = render_main_menu(
        &skin,
        &MainWindowState {
            eq_open: true,
            ..MainWindowState::default()
        },
        &flags,
    );
    let changed = diff(&plain, &checked);
    assert!(!changed.is_empty(), "the Equalizer check drew nothing");
    // The Equalizer row is index 8 (S04 §2 row 7); the check cell is the
    // measured x = 16-29 (S04 §3.1).
    let bands = row_bands();
    let (top, bottom) = bands[8];
    assert_eq!(
        top_level(MainMenuId::Equalizer).unwrap().check,
        menu::MenuCheck::Equalizer
    );
    for (x, y) in &changed {
        assert!(
            (16..30).contains(x),
            "check pixel outside the check cell: ({x}, {y})"
        );
        assert!(
            (top..bottom).contains(y),
            "check pixel outside the Equalizer band: ({x}, {y})"
        );
    }
    assert!(
        changed.iter().all(|(x, _)| (16..30).contains(x)),
        "the check difference escapes x = 16-29"
    );
}

#[test]
fn the_time_flip_touches_no_main_menu_pixel() {
    // The elapsed/remaining checks live in the Options popup (S04 §4.1),
    // not on the main menu (S04 §2: no time rows), so flipping the flag
    // must not change the rendered main menu — while the check pair itself
    // still resolves oppositely from live state.
    let skin = synthetic_skin();
    let flags = MenuFlags::default();
    let elapsed = render_main_menu(&skin, &MainWindowState::default(), &flags);
    let remaining = render_main_menu(
        &skin,
        &MainWindowState {
            time_remaining: true,
            ..MainWindowState::default()
        },
        &flags,
    );
    assert!(
        diff(&elapsed, &remaining).is_empty(),
        "the time flip must not redraw the main menu"
    );
    assert!(check_state(
        menu::MenuCheck::TimeElapsed,
        &MainWindowState::default(),
        &flags
    ));
    assert!(check_state(
        menu::MenuCheck::TimeRemaining,
        &MainWindowState {
            time_remaining: true,
            ..MainWindowState::default()
        },
        &flags
    ));
}

#[test]
fn submenu_frames_hold_one_band_per_entry() {
    let skin = synthetic_skin();
    for id in [
        SubmenuId::Play,
        SubmenuId::Playback,
        SubmenuId::Bookmarks,
        SubmenuId::Options,
        SubmenuId::Visualization,
        SubmenuId::Skins,
    ] {
        let entries = submenu_entries(id);
        assert!(!entries.is_empty());
        let (width, height) = submenu_client_size(id, &skin);
        assert_eq!(height, entries.len() as u32 * 35, "{id:?}");
        let frame = render_submenu(
            &skin,
            id,
            &MainWindowState::default(),
            &MenuFlags::default(),
        );
        assert_eq!((frame.width(), frame.height()), (width, height));
    }
}
