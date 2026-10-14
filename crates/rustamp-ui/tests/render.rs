//! Structural and pixel-level tests for the classic main-window renderer.
//!
//! The pixel checks build a small **synthetic** skin in memory (uncompressed
//! 24-bit BMPs, one flat colour per sprite region) so the expected sprite pixel
//! is unambiguous. No real skin or reference capture is used here; the corpus
//! test at the bottom only asserts that real skins render without panicking.

use std::path::PathBuf;

use rustamp_gfx::{Image, Rect};
use rustamp_skin::{BuiltinDefaultSkin, MemorySource, PLACEHOLDER_RGBA, Skin};
use rustamp_ui::{
    MainWindowLayout, MainWindowState, MonoStereo, TransportButton, TransportState, Widget,
    WindowButton, frame_size, hit_test, render_main_window,
};

// Distinctive colours painted into the synthetic sheets.
const MAIN_BG: [u8; 3] = [20, 20, 31];
const PLAY_BUTTON: [u8; 3] = [10, 20, 30];
const PLAY_BUTTON_ACTIVE: [u8; 3] = [11, 21, 31];
const DIGIT_1: [u8; 3] = [40, 50, 60];
const DIGIT_2: [u8; 3] = [70, 80, 90];
const DIGIT_3: [u8; 3] = [100, 110, 120];
const DIGIT_4: [u8; 3] = [130, 140, 150];
const NO_DIGIT: [u8; 3] = [1, 2, 3];
const MINUS: [u8; 3] = [4, 5, 6];
const CLUTTER_BG: [u8; 3] = [8, 9, 10];
const CLUTTER_A: [u8; 3] = [160, 170, 180];
const POS_THUMB: [u8; 3] = [190, 200, 210];
const GLYPH_A: [u8; 3] = [220, 230, 240];
const GLYPH_B: [u8; 3] = [46, 47, 48];
const SHADE: [u8; 3] = [5, 6, 7];
// The window-button body colour and the four pressed cells (S02 §7).
const TITLE_BAR_BODY: [u8; 3] = [25, 26, 27];
const OPTIONS_DEPRESSED: [u8; 3] = [31, 32, 33];
const MINIMIZE_DEPRESSED: [u8; 3] = [34, 35, 36];
const SHADE_DEPRESSED: [u8; 3] = [37, 38, 39];
const CLOSE_DEPRESSED: [u8; 3] = [40, 41, 42];
const POS_THUMB_PRESSED: [u8; 3] = [43, 44, 45];
// The lit and dim mono/stereo labels (S02 §6).
const STEREO_LIT: [u8; 3] = [50, 51, 52];
const STEREO_DIM: [u8; 3] = [53, 54, 55];
const MONO_LIT: [u8; 3] = [56, 57, 58];
const MONO_DIM: [u8; 3] = [59, 60, 61];

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
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved
    out.extend_from_slice(&54u32.to_le_bytes()); // pixel data offset
    out.extend_from_slice(&40u32.to_le_bytes()); // DIB header size
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&24u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes()); // x pixels/metre
    out.extend_from_slice(&2835u32.to_le_bytes()); // y pixels/metre
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

/// Paints `rgb` into a `(x, y, w, h)` region of a top-down pixel buffer.
fn paint(pixels: &mut [[u8; 3]], width: u32, rect: (u32, u32, u32, u32), rgb: [u8; 3]) {
    let (rx, ry, rw, rh) = rect;
    for y in ry..ry + rh {
        for x in rx..rx + rw {
            pixels[(y * width + x) as usize] = rgb;
        }
    }
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
        paint(&mut pixels, width, (x, y, w, h), rgb);
    }
    bmp_24(width, height, &pixels)
}

/// Builds the synthetic main-window skin.
fn synthetic_skin() -> Skin {
    let mut source = MemorySource::new("synthetic");
    source.add("main.bmp", sheet(275, 116, MAIN_BG, &[]));
    source.add(
        "cbuttons.bmp",
        sheet(
            136,
            36,
            [0, 0, 0],
            &[
                (23, 0, 22, 18, PLAY_BUTTON),
                (23, 18, 22, 18, PLAY_BUTTON_ACTIVE),
            ],
        ),
    );
    source.add(
        "numbers.bmp",
        sheet(
            99,
            13,
            [0, 0, 0],
            &[
                (9, 0, 9, 13, DIGIT_1),
                (18, 0, 9, 13, DIGIT_2),
                (27, 0, 9, 13, DIGIT_3),
                (36, 0, 9, 13, DIGIT_4),
                (90, 0, 9, 13, NO_DIGIT),
                (20, 6, 5, 1, MINUS),
            ],
        ),
    );
    source.add(
        "titlebar.bmp",
        sheet(
            344,
            87,
            [0, 0, 0],
            &[
                (27, 0, 275, 14, TITLE_BAR_BODY),
                (304, 0, 8, 43, CLUTTER_BG),
                (316, 55, 4, 6, CLUTTER_A),
                (27, 29, 275, 14, SHADE),
                // The four pressed window-button cells (S02 §7).
                (0, 9, 9, 9, OPTIONS_DEPRESSED),
                (9, 9, 9, 9, MINIMIZE_DEPRESSED),
                (18, 9, 9, 9, CLOSE_DEPRESSED),
                (9, 18, 9, 9, SHADE_DEPRESSED),
            ],
        ),
    );
    source.add(
        "posbar.bmp",
        sheet(
            307,
            10,
            [0, 0, 0],
            &[
                (246, 0, 29, 10, POS_THUMB),
                (278, 0, 29, 10, POS_THUMB_PRESSED),
            ],
        ),
    );
    source.add(
        "text.bmp",
        sheet(
            155,
            18,
            [0, 0, 0],
            &[(0, 0, 5, 6, GLYPH_A), (5, 0, 5, 6, GLYPH_B)],
        ),
    );
    source.add("shufrep.bmp", sheet(92, 85, [0, 0, 0], &[]));
    source.add("volume.bmp", sheet(68, 433, [0, 0, 0], &[]));
    source.add("balance.bmp", sheet(68, 433, [0, 0, 0], &[]));
    source.add(
        "monoster.bmp",
        sheet(
            58,
            24,
            [0, 0, 0],
            &[
                (0, 0, 29, 12, STEREO_LIT),
                (0, 12, 29, 12, STEREO_DIM),
                (29, 0, 27, 12, MONO_LIT),
                (29, 12, 27, 12, MONO_DIM),
            ],
        ),
    );
    source.add("playpaus.bmp", sheet(42, 9, [0, 0, 0], &[]));
    Skin::from_source(&mut source, &BuiltinDefaultSkin).expect("synthetic skin loads")
}

fn rgb(image: &Image, x: i32, y: i32) -> [u8; 3] {
    let [r, g, b, _] = image.pixel(x, y).expect("pixel in bounds");
    [r, g, b]
}

fn playing_state() -> MainWindowState {
    MainWindowState {
        transport: TransportState::Playing,
        clutter_a: true,
        time_seconds: 12 * 60 + 34,
        title: "A".to_string(),
        position: 0,
        ..MainWindowState::default()
    }
}

#[test]
fn synthetic_skin_puts_sprites_at_their_s01_positions() {
    let skin = synthetic_skin();
    let layout = MainWindowLayout::CLASSIC;
    let frame = render_main_window(&playing_state(), &skin, &layout);

    assert_eq!((frame.width(), frame.height()), (275, 116));
    // Transport button (S01 §2.1: play at 39,88).
    assert_eq!(rgb(&frame, 39, 88), PLAY_BUTTON);
    // Time digits (S01 §2.1/§2.3: cells at 48,60,78,90 y=26).
    assert_eq!(rgb(&frame, 48, 26), DIGIT_1);
    assert_eq!(rgb(&frame, 60, 26), DIGIT_2);
    assert_eq!(rgb(&frame, 78, 26), DIGIT_3);
    assert_eq!(rgb(&frame, 90, 26), DIGIT_4);
    // Clutter-bar letter A (S01 §2.1: at 14,33) over the clutter background.
    assert_eq!(rgb(&frame, 14, 33), CLUTTER_A);
    assert_eq!(rgb(&frame, 10, 22), CLUTTER_BG);
    // Position thumb (S01 §2.1: at 16,72 at position 0).
    assert_eq!(rgb(&frame, 16, 72), POS_THUMB);
    // Marquee glyph (S01 §2.2: first cell at 111,27).
    assert_eq!(rgb(&frame, 111, 27), GLYPH_A);
    // The background shows where nothing else is drawn.
    assert_eq!(rgb(&frame, 270, 110), MAIN_BG);
}

#[test]
fn no_digit_cell_and_minus_sign_follow_the_time_state() {
    let skin = synthetic_skin();
    let layout = MainWindowLayout::CLASSIC;

    // Under ten minutes the tens-of-minutes cell is the "no digit" filler.
    let short = MainWindowState {
        time_seconds: 83,
        ..MainWindowState::default()
    };
    let frame = render_main_window(&short, &skin, &layout);
    assert_eq!(rgb(&frame, 48, 26), NO_DIGIT);
    assert_eq!(rgb(&frame, 60, 26), DIGIT_1);

    // Remaining time draws the minus sign at (38,32).
    let remaining = MainWindowState {
        time_seconds: 83,
        time_remaining: true,
        ..MainWindowState::default()
    };
    let frame = render_main_window(&remaining, &skin, &layout);
    assert_eq!(rgb(&frame, 38, 32), MINUS);
}

#[test]
fn pressed_transport_button_uses_the_active_sprite() {
    let skin = synthetic_skin();
    let layout = MainWindowLayout::CLASSIC;
    let mut state = playing_state();
    let frame = render_main_window(&state, &skin, &layout);
    assert_eq!(rgb(&frame, 39, 88), PLAY_BUTTON);

    state.pressed = Some(TransportButton::Play);
    let frame = render_main_window(&state, &skin, &layout);
    assert_eq!(rgb(&frame, 39, 88), PLAY_BUTTON_ACTIVE);
}

#[test]
fn pressed_window_buttons_draw_from_the_small_cells() {
    let skin = synthetic_skin();
    let layout = MainWindowLayout::CLASSIC;
    let mut state = playing_state();

    // Normal state: the four buttons are part of the title-bar body sprite
    // (S01 §2.1/S02 §7), so the pressed cells must not appear.
    let normal = render_main_window(&state, &skin, &layout);
    for (x, y) in [(6, 3), (244, 3), (254, 3), (264, 3)] {
        assert_eq!(
            rgb(&normal, x, y),
            TITLE_BAR_BODY,
            "normal button at ({x},{y})"
        );
    }

    for (button, (x, y), colour) in [
        (WindowButton::Menu, (6, 3), OPTIONS_DEPRESSED),
        (WindowButton::Minimize, (244, 3), MINIMIZE_DEPRESSED),
        (WindowButton::Shade, (254, 3), SHADE_DEPRESSED),
        (WindowButton::Close, (264, 3), CLOSE_DEPRESSED),
    ] {
        state.pressed_window_button = Some(button);
        let frame = render_main_window(&state, &skin, &layout);
        assert_eq!(rgb(&frame, x, y), colour, "{button:?} pressed cell");
        state.pressed_window_button = None;
    }

    // The normal state is unchanged.
    let again = render_main_window(&playing_state(), &skin, &layout);
    assert_eq!(normal, again);
}

#[test]
fn position_thumb_uses_the_pressed_sprite_while_held() {
    let skin = synthetic_skin();
    let layout = MainWindowLayout::CLASSIC;
    let mut state = playing_state();
    let normal = render_main_window(&state, &skin, &layout);
    assert_eq!(rgb(&normal, 16, 72), POS_THUMB);

    state.position_thumb_pressed = true;
    let pressed = render_main_window(&state, &skin, &layout);
    assert_eq!(rgb(&pressed, 16, 72), POS_THUMB_PRESSED);
}

#[test]
fn mono_stereo_labels_follow_transport_and_channels() {
    let skin = synthetic_skin();
    let layout = MainWindowLayout::CLASSIC;
    let mut state = playing_state();

    state.channels = MonoStereo::Stereo;
    let frame = render_main_window(&state, &skin, &layout);
    assert_eq!(rgb(&frame, 239, 41), STEREO_LIT);
    assert_eq!(rgb(&frame, 212, 41), MONO_DIM);

    state.channels = MonoStereo::Mono;
    let frame = render_main_window(&state, &skin, &layout);
    assert_eq!(rgb(&frame, 212, 41), MONO_LIT);
    assert_eq!(rgb(&frame, 239, 41), STEREO_DIM);

    // Stopped dims both labels (S02 §6).
    state.channels = MonoStereo::Stereo;
    state.transport = TransportState::Stopped;
    let frame = render_main_window(&state, &skin, &layout);
    assert_eq!(rgb(&frame, 239, 41), STEREO_DIM);
    assert_eq!(rgb(&frame, 212, 41), MONO_DIM);

    // No stream info dims both too.
    state.transport = TransportState::Playing;
    state.channels = MonoStereo::Unknown;
    let frame = render_main_window(&state, &skin, &layout);
    assert_eq!(rgb(&frame, 239, 41), STEREO_DIM);
    assert_eq!(rgb(&frame, 212, 41), MONO_DIM);
}

#[test]
fn marquee_offset_shifts_the_text() {
    let skin = synthetic_skin();
    let layout = MainWindowLayout::CLASSIC;
    let mut state = playing_state();
    state.title = "ab".to_string();

    let frame = render_main_window(&state, &skin, &layout);
    assert_eq!(rgb(&frame, 111, 27), GLYPH_A);
    assert_eq!(rgb(&frame, 116, 27), GLYPH_B);

    state.marquee_offset = 5;
    let frame = render_main_window(&state, &skin, &layout);
    assert_eq!(rgb(&frame, 111, 27), GLYPH_B);
}

#[test]
fn a_blank_time_display_draws_nothing() {
    let skin = synthetic_skin();
    let layout = MainWindowLayout::CLASSIC;
    let mut state = MainWindowState {
        time_seconds: 83,
        ..MainWindowState::default()
    };
    let frame = render_main_window(&state, &skin, &layout);
    assert_eq!(rgb(&frame, 48, 26), NO_DIGIT);

    // The blink-off phase and stopped are blank (S02 §4).
    state.time_visible = false;
    let frame = render_main_window(&state, &skin, &layout);
    assert_eq!(rgb(&frame, 48, 26), MAIN_BG);

    // The remaining-time minus sign is blank too.
    state.time_remaining = true;
    let frame = render_main_window(&state, &skin, &layout);
    assert_eq!(rgb(&frame, 38, 32), MAIN_BG);
}

#[test]
fn double_size_is_exactly_twice_in_both_axes() {
    let skin = synthetic_skin();
    let layout = MainWindowLayout::CLASSIC;
    let single = render_main_window(&playing_state(), &skin, &layout);

    let mut doubled_state = playing_state();
    doubled_state.double_size = true;
    let doubled = render_main_window(&doubled_state, &skin, &layout);

    assert_eq!(frame_size(&doubled_state), (550, 232));
    assert_eq!((doubled.width(), doubled.height()), (550, 232));
    assert_eq!(doubled, single.scaled(2));
}

#[test]
fn windowshade_matches_s01_geometry() {
    let skin = synthetic_skin();
    let layout = MainWindowLayout::CLASSIC;
    let mut state = playing_state();
    state.windowshade = true;
    let frame = render_main_window(&state, &skin, &layout);

    assert_eq!(frame_size(&state), (275, 14));
    assert_eq!((frame.width(), frame.height()), (275, 14));
    // The shade title bar band (S01 §2.1, provisional) is drawn at (0,0).
    assert_eq!(rgb(&frame, 0, 0), SHADE);
    assert_eq!(rgb(&frame, 274, 13), SHADE);
}

#[test]
fn a_skin_missing_sheets_still_renders() {
    // Only main.bmp is provided; every other main-window sheet comes from the
    // built-in placeholder (S17 §2), so the renderer must not panic.
    let mut source = MemorySource::new("sparse");
    source.add("main.bmp", sheet(275, 116, MAIN_BG, &[]));
    let skin = Skin::from_source(&mut source, &BuiltinDefaultSkin).expect("sparse skin loads");

    let frame = render_main_window(
        &MainWindowState::default(),
        &skin,
        &MainWindowLayout::CLASSIC,
    );
    assert_eq!((frame.width(), frame.height()), (275, 116));
    // The title bar is the magenta placeholder.
    assert_eq!(
        rgb(&frame, 5, 5),
        [
            PLACEHOLDER_RGBA[0],
            PLACEHOLDER_RGBA[1],
            PLACEHOLDER_RGBA[2]
        ]
    );
}

#[test]
fn hit_test_is_consistent_with_the_layout() {
    let layout = MainWindowLayout::CLASSIC;
    for &widget in Widget::ALL {
        let r = widget.rect(&layout);
        assert_eq!(hit_test(&layout, r.x, r.y), Some(widget));
    }
    assert_eq!(hit_test(&layout, -1, 0), None);
    assert_eq!(hit_test(&layout, 275, 0), None);
}

fn corpus_dir() -> PathBuf {
    std::env::var_os("RUSTAMP_SKIN_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"D:\code\rustamp-free-sources\skins-corpus"))
}

#[test]
fn corpus_skins_render_a_frame_without_panicking() {
    let dir = corpus_dir();
    if !dir.is_dir() {
        println!(
            "SKIP corpus render: {} does not exist (set RUSTAMP_SKIN_CORPUS)",
            dir.display()
        );
        return;
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                            e.eq_ignore_ascii_case("wsz") || e.eq_ignore_ascii_case("zip")
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    assert!(!files.is_empty(), "corpus directory has no .wsz/.zip files");

    let layout = MainWindowLayout::CLASSIC;
    let stopped = MainWindowState {
        title: "Rustamp test tone".to_string(),
        ..MainWindowState::default()
    };
    let mut playing = playing_state();
    playing.kbps = Some(320);
    playing.khz = Some(48);
    playing.volume = 100;
    playing.balance = 0;
    playing.time_remaining = true;

    // A third state that exercises the D2 render fields: a scrolling marquee,
    // a held window button, the pressed position thumb and a blank readout.
    let mut extras = playing_state();
    extras.title = "A much longer title that will scroll across the marquee".to_string();
    extras.marquee_offset = 10;
    extras.position_thumb_pressed = true;
    extras.channels = MonoStereo::Unknown;
    extras.time_visible = false;

    let states = [&stopped, &playing, &extras];
    let window_buttons = [
        None,
        Some(WindowButton::Menu),
        Some(WindowButton::Minimize),
        Some(WindowButton::Shade),
        Some(WindowButton::Close),
    ];

    let mut rendered = 0usize;
    for path in &files {
        let skin = Skin::load(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for state in states {
            for windowshade in [false, true] {
                for double_size in [false, true] {
                    for pressed_window_button in window_buttons {
                        let mut state = state.clone();
                        state.windowshade = windowshade;
                        state.double_size = double_size;
                        state.pressed_window_button = pressed_window_button;
                        let frame = render_main_window(&state, &skin, &layout);
                        assert_eq!(frame_size(&state), (frame.width(), frame.height()));
                    }
                }
            }
        }
        rendered += 1;
    }
    println!("corpus render: {rendered} skins rendered without panicking");
}

/// A tiny check that the layout table really is the renderer's source of truth:
/// moving a rectangle moves the drawn sprite.
#[test]
fn renderer_follows_the_layout() {
    let skin = synthetic_skin();
    let mut layout = MainWindowLayout::CLASSIC;
    layout.play = Rect::new(200, 88, 22, 18);
    let frame = render_main_window(&playing_state(), &skin, &layout);
    assert_eq!(rgb(&frame, 200, 88), PLAY_BUTTON);
    assert_ne!(rgb(&frame, 39, 88), PLAY_BUTTON);
}
