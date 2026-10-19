//! Pixel tests for the playlist editor window renderer.
//!
//! A synthetic `pledit.bmp` (one flat colour per settled sprite region) plus
//! a `pledit.txt` with distinctive colours pins each drawn element to its
//! S01/S06 rectangle. No window is opened; the corpus-style fallback (missing
//! sheets) is covered by rendering with the bare placeholder skin.

use rustamp_core::playlist::EntryId;
use rustamp_gfx::Image;
use rustamp_skin::{BuiltinDefaultSkin, MemorySource, Skin};
use rustamp_ui::{PLAYLIST_WINDOW_SIZE, PlaylistWindow, render_playlist_window};

// Distinctive colours painted into the synthetic sheet.
const CORNER: [u8; 3] = [10, 20, 30];
const STRIP: [u8; 3] = [40, 50, 60];
const TILE: [u8; 3] = [70, 80, 90];
const RCORNER: [u8; 3] = [100, 110, 120];
const CORNER_U: [u8; 3] = [11, 21, 31];
const STRIP_U: [u8; 3] = [41, 51, 61];
const BB_LEFT: [u8; 3] = [130, 140, 150];
const BB_RIGHT: [u8; 3] = [160, 170, 180];
const BTN_ADD: [u8; 3] = [190, 200, 210];
const BTN_REM: [u8; 3] = [191, 201, 211];
const BTN_SEL: [u8; 3] = [192, 202, 212];
const BTN_MISC: [u8; 3] = [193, 203, 213];
const BTN_LIST: [u8; 3] = [194, 204, 214];
const HANDLE: [u8; 3] = [220, 230, 240];
const ARROWS: [u8; 3] = [221, 231, 241];
const CLOSEC: [u8; 3] = [222, 232, 242];
const SHADEC: [u8; 3] = [223, 233, 243];
const RAIL: [u8; 3] = [224, 234, 244];
// `pledit.txt` colours under test.
const NORMAL_BG: [u8; 3] = [0x0A, 0x0A, 0x0B];
const SELECTED_BG: [u8; 3] = [0x14, 0x14, 0xC8];

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

fn paint(pixels: &mut [[u8; 3]], width: u32, rect: (u32, u32, u32, u32), rgb: [u8; 3]) {
    let (rx, ry, rw, rh) = rect;
    for y in ry..ry + rh {
        for x in rx..rx + rw {
            pixels[(y * width + x) as usize] = rgb;
        }
    }
}

fn sheet(
    width: u32,
    height: u32,
    base: [u8; 3],
    regions: &[(u32, u32, u32, u32, [u8; 3])],
) -> Vec<u8> {
    let mut pixels = vec![base; (width * height) as usize];
    for &(x, y, w, h, rgb) in regions {
        paint(&mut pixels, width, (x, y, w, h), rgb);
    }
    bmp_24(width, height, &pixels)
}

/// Builds the synthetic playlist skin: settled `pledit.bmp` regions plus a
/// `pledit.txt` with distinctive list colours.
fn synthetic_playlist_skin() -> Skin {
    let mut source = MemorySource::new("synthetic-playlist");
    source.add(
        "pledit.bmp",
        sheet(
            280,
            186,
            [0, 0, 0],
            &[
                // Focused title band (sheet y 0-19; S01 §4.1).
                (0, 0, 25, 20, CORNER),
                (26, 0, 100, 20, STRIP),
                (127, 0, 25, 20, TILE),
                (153, 0, 25, 20, RCORNER),
                // Unfocused title band (sheet y 21-40).
                (0, 21, 25, 20, CORNER_U),
                (26, 21, 100, 20, STRIP_U),
                // Side rails (S01 §4.1; the right rail stays provisional).
                (0, 42, 12, 29, RAIL),
                (31, 42, 20, 29, RAIL),
                // Bottom bar blocks (S01 §4).
                (0, 72, 125, 38, BB_LEFT),
                (126, 72, 150, 38, BB_RIGHT),
                // Button faces (sheet y 86; S01 §4.1).
                (11, 86, 23, 18, BTN_ADD),
                (43, 86, 24, 18, BTN_REM),
                (72, 86, 24, 18, BTN_SEL),
                (101, 86, 24, 18, BTN_MISC),
                (229, 86, 24, 18, BTN_LIST),
                // Scrollbar handle and arrows (S01 §4.2).
                (52, 53, 7, 17, HANDLE),
                (261, 72, 8, 10, ARROWS),
                // Close/shade cells (S01 §4, provisional).
                (52, 42, 9, 9, CLOSEC),
                (62, 42, 9, 9, SHADEC),
            ],
        ),
    );
    source.add(
        "pledit.txt",
        "[Text]\nNormal=#00FF00\nNormalBG=#0A0A0B\nSelectedBG=#1414C8\n"
            .as_bytes()
            .to_vec(),
    );
    Skin::from_source(&mut source, &BuiltinDefaultSkin).expect("synthetic playlist skin loads")
}

fn rgb(image: &Image, x: i32, y: i32) -> [u8; 3] {
    let [r, g, b, _] = image.pixel(x, y).expect("pixel in bounds");
    [r, g, b]
}

fn window_with(n: u64) -> PlaylistWindow {
    let mut window = PlaylistWindow::new();
    window.set_entries((0..n).map(EntryId).collect());
    window
}

#[test]
fn frame_is_275x116_with_the_pledit_background() {
    let skin = synthetic_playlist_skin();
    let frame = render_playlist_window(&skin, &PlaylistWindow::new(), &[], true);
    assert_eq!((frame.width(), frame.height()), PLAYLIST_WINDOW_SIZE);
    assert_eq!(PLAYLIST_WINDOW_SIZE, (275, 116));
    // An empty row area shows the `NormalBG` colour (S01 §4).
    assert_eq!(rgb(&frame, 200, 30), NORMAL_BG);
}

#[test]
fn title_band_buttons_and_bottom_bar_come_from_their_sprites() {
    let skin = synthetic_playlist_skin();
    let frame = render_playlist_window(&skin, &PlaylistWindow::new(), &[], true);
    // Title band (S01 §4.1): corner, strip at 87, tile fill, right corner.
    assert_eq!(rgb(&frame, 5, 5), CORNER);
    assert_eq!(rgb(&frame, 90, 5), STRIP);
    assert_eq!(rgb(&frame, 50, 5), TILE);
    // The right corner is 25 px wide, so it starts at 275 - 25 = 250 (S01 §4.1
    // puts it at 250,0); 260 is the scroll handle's x, not the corner's.
    assert_eq!(rgb(&frame, 250, 5), RCORNER);
    // Resting buttons are part of the title body; selected cells overlay only
    // while held, rather than making every frame look pressed.
    assert_eq!(rgb(&frame, 264, 3), RCORNER);
    assert_eq!(rgb(&frame, 254, 3), RCORNER);
    for (button, x, colour) in [
        (rustamp_ui::PlaylistTitleButton::Close, 264, CLOSEC),
        (rustamp_ui::PlaylistTitleButton::Shade, 254, SHADEC),
    ] {
        let mut window = PlaylistWindow::new();
        window.pressed_title_button = Some(button);
        let held = render_playlist_window(&skin, &window, &[], true);
        assert_eq!(rgb(&held, x, 3), colour);
    }
    // Bottom bar blocks (S01 §4) and the five button faces (S01 §4.1).
    assert_eq!(rgb(&frame, 5, 100), BB_LEFT);
    assert_eq!(rgb(&frame, 200, 100), BB_RIGHT);
    assert_eq!(rgb(&frame, 15, 95), BTN_ADD);
    assert_eq!(rgb(&frame, 45, 95), BTN_REM);
    assert_eq!(rgb(&frame, 74, 95), BTN_SEL);
    assert_eq!(rgb(&frame, 103, 95), BTN_MISC);
    assert_eq!(rgb(&frame, 230, 95), BTN_LIST);
    // Scrollbar arrows (S01 §4.2).
    assert_eq!(rgb(&frame, 261, 79), ARROWS);
}

#[test]
fn unfocused_frame_uses_the_unfocused_title_band() {
    let skin = synthetic_playlist_skin();
    let frame = render_playlist_window(&skin, &PlaylistWindow::new(), &[], false);
    assert_eq!(rgb(&frame, 5, 5), CORNER_U);
    assert_eq!(rgb(&frame, 90, 5), STRIP_U);
}

#[test]
fn selection_highlight_covers_the_selected_row_band() {
    // S06 §2 + S01 §4: the selected row's background is `SelectedBG`.
    let skin = synthetic_playlist_skin();
    let mut window = window_with(6);
    window.mouse_down(60, 28);
    let labels = vec!["a".to_string(); 4];
    let frame = render_playlist_window(&skin, &window, &labels, true);
    // Row 0 (y 23-36) selected; x = 200 is past the one-glyph label.
    assert_eq!(rgb(&frame, 200, 29), SELECTED_BG);
    // Row 1 (y 36-49) is plain background.
    assert_eq!(rgb(&frame, 200, 42), NORMAL_BG);
}

#[test]
fn scrollbar_handle_tracks_the_scroll_offset() {
    // S01 §4.2: y = 20 top, 60 bottom for the 60-entry list.
    let skin = synthetic_playlist_skin();
    let mut window = window_with(60);
    let frame = render_playlist_window(&skin, &window, &[], true);
    assert_eq!(rgb(&frame, 261, 21), HANDLE);
    window.scroll_to(56);
    let frame = render_playlist_window(&skin, &window, &[], true);
    assert_eq!(rgb(&frame, 261, 61), HANDLE);
    // The top slot is the side rail again, not the handle.
    assert_eq!(rgb(&frame, 261, 21), RAIL);
}

#[test]
fn missing_sheets_still_produce_a_frame() {
    // The bare placeholder skin has no real pledit art; rendering must skip
    // the missing sprites (S17-style) instead of panicking.
    let mut source = MemorySource::new("empty");
    source.add("pledit.txt", "[Text]\n".as_bytes().to_vec());
    let skin = Skin::from_source(&mut source, &BuiltinDefaultSkin).expect("empty skin loads");
    let frame = render_playlist_window(&skin, &PlaylistWindow::new(), &[], true);
    assert_eq!((frame.width(), frame.height()), (275, 116));
}
