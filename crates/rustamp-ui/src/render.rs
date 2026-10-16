//! Renders one main-window frame from a [`MainWindowState`] and a loaded
//! [`Skin`], using `rustamp-gfx` for every blit.
//!
//! The renderer is pure: it never opens a window, never reads the clock and
//! never changes the state. A missing sprite is skipped (the loader has already
//! substituted the built-in default per S17), so a skin without a sheet still
//! produces a frame instead of panicking.
//!
//! Provisional choices that S01 does not settle are called out in comments and
//! in the module doc: the marquee/kbps/kHz field widths, the volume/balance
//! frame-to-value mapping, the leading "no digit" rule, the decoder-active
//! condition and the window-button window positions (S02 §7 gives the pressed
//! sheet cells but not where the buttons sit).

use rustamp_gfx::{BitmapFont, GlyphMap, Image, Rect};
use rustamp_skin::{Sheet, Skin, webamp};

use crate::docking::{WindowKind, window_size};
use crate::layout::{MainWindowLayout, SHADE_HEIGHT, WINDOW_HEIGHT, WINDOW_WIDTH};
use crate::state::{MainWindowState, MonoStereo, TransportButton, TransportState, WindowButton};

/// The digit sprite names for `numbers.bmp`, indexed by digit value.
const DIGIT_NAMES: [&str; 10] = [
    "DIGIT_0", "DIGIT_1", "DIGIT_2", "DIGIT_3", "DIGIT_4", "DIGIT_5", "DIGIT_6", "DIGIT_7",
    "DIGIT_8", "DIGIT_9",
];

/// The digit sprite names for `nums_ex.bmp`, indexed by digit value.
const DIGIT_NAMES_EX: [&str; 10] = [
    "DIGIT_0_EX",
    "DIGIT_1_EX",
    "DIGIT_2_EX",
    "DIGIT_3_EX",
    "DIGIT_4_EX",
    "DIGIT_5_EX",
    "DIGIT_6_EX",
    "DIGIT_7_EX",
    "DIGIT_8_EX",
    "DIGIT_9_EX",
];

/// The frame size a state renders to, including the double-size scale.
///
/// Delegates to [`docking::window_size`] (single source of truth, S05 §1.1 and
/// §2.1). The shaded-plus-double-size combination (550 × 28, the 275 × 14 shade
/// frame scaled 2x) is open in S05, which never measures it; it is preserved
/// here so the size matches the painted frame (see [`docking::window_size`]).
pub fn frame_size(state: &MainWindowState) -> (u32, u32) {
    window_size(WindowKind::Main, state.windowshade, state.double_size)
}

/// Renders one RGBA frame for `state`.
///
/// Double-size is the exact integer 2x scale of the 1x frame.
pub fn render_main_window(
    state: &MainWindowState,
    skin: &Skin,
    layout: &MainWindowLayout,
) -> Image {
    let frame = render_1x(state, skin, layout);
    if state.double_size {
        frame.scaled(2)
    } else {
        frame
    }
}

/// The decoded sheets the main window reads, converted once per frame.
struct Sheets {
    main: Option<Image>,
    titlebar: Option<Image>,
    cbuttons: Option<Image>,
    shufrep: Option<Image>,
    posbar: Option<Image>,
    volume: Option<Image>,
    balance: Option<Image>,
    monoster: Option<Image>,
    playpaus: Option<Image>,
    numbers: Option<Image>,
    nums_ex: Option<Image>,
    text: Option<Image>,
}

impl Sheets {
    fn load(skin: &Skin) -> Self {
        let get = |sheet: Sheet| -> Option<Image> {
            let raster = skin.sheet(sheet)?;
            Image::from_rgba(raster.width(), raster.height(), raster.as_rgba().to_vec()).ok()
        };
        Self {
            main: get(Sheet::Main),
            titlebar: get(Sheet::Titlebar),
            cbuttons: get(Sheet::Cbuttons),
            shufrep: get(Sheet::Shufrep),
            posbar: get(Sheet::Posbar),
            volume: get(Sheet::Volume),
            balance: get(Sheet::Balance),
            monoster: get(Sheet::Monoster),
            playpaus: get(Sheet::Playpaus),
            numbers: get(Sheet::Numbers),
            nums_ex: get(Sheet::NumsEx),
            text: get(Sheet::Text),
        }
    }

    fn get(&self, sheet: Sheet) -> Option<&Image> {
        match sheet {
            Sheet::Main => self.main.as_ref(),
            Sheet::Titlebar => self.titlebar.as_ref(),
            Sheet::Cbuttons => self.cbuttons.as_ref(),
            Sheet::Shufrep => self.shufrep.as_ref(),
            Sheet::Posbar => self.posbar.as_ref(),
            Sheet::Volume => self.volume.as_ref(),
            Sheet::Balance => self.balance.as_ref(),
            Sheet::Monoster => self.monoster.as_ref(),
            Sheet::Playpaus => self.playpaus.as_ref(),
            Sheet::Numbers => self.numbers.as_ref(),
            Sheet::NumsEx => self.nums_ex.as_ref(),
            Sheet::Text => self.text.as_ref(),
            _ => None,
        }
    }
}

/// Blits the named sprite's rectangle at `(dx, dy)`, skipping it when the
/// sprite or its sheet is missing.
fn blit_named(canvas: &mut Image, sheets: &Sheets, skin: &Skin, name: &str, dx: i32, dy: i32) {
    let Some(def) = skin.sprite(name) else {
        return;
    };
    let Some(src) = sheets.get(def.sheet) else {
        return;
    };
    canvas.blit_rect(
        src,
        Rect::new(def.x as i32, def.y as i32, def.width, def.height),
        dx,
        dy,
    );
}

/// Blits an arbitrary region of a sheet at `(dx, dy)`.
fn blit_region(canvas: &mut Image, sheets: &Sheets, sheet: Sheet, src: Rect, dx: i32, dy: i32) {
    if let Some(image) = sheets.get(sheet) {
        canvas.blit_rect(image, src, dx, dy);
    }
}

fn render_1x(state: &MainWindowState, skin: &Skin, layout: &MainWindowLayout) -> Image {
    let sheets = Sheets::load(skin);

    if state.windowshade {
        let mut canvas = Image::filled(WINDOW_WIDTH, SHADE_HEIGHT, [0, 0, 0, 255]);
        draw_shade(&mut canvas, &sheets, skin, layout, state);
        return canvas;
    }

    let mut canvas = Image::filled(WINDOW_WIDTH, WINDOW_HEIGHT, [0, 0, 0, 255]);

    // Background (S01 §2.1, verified).
    blit_named(
        &mut canvas,
        &sheets,
        skin,
        "MAIN_WINDOW_BACKGROUND",
        layout.background.x,
        layout.background.y,
    );

    // Title bar (S01 §2.1, verified): focused vs unfocused.
    let title = if state.title_bar_focused {
        "MAIN_TITLE_BAR_SELECTED"
    } else {
        "MAIN_TITLE_BAR"
    };
    blit_named(
        &mut canvas,
        &sheets,
        skin,
        title,
        layout.title_bar.x,
        layout.title_bar.y,
    );

    draw_clutter_bar(&mut canvas, &sheets, skin, layout, state);
    draw_window_buttons(&mut canvas, &sheets, skin, layout, state);
    draw_transport(&mut canvas, &sheets, skin, layout, state);
    draw_toggles(&mut canvas, &sheets, skin, layout, state);
    draw_position(&mut canvas, &sheets, skin, layout, state);
    draw_volume(&mut canvas, &sheets, skin, layout, state);
    draw_balance(&mut canvas, &sheets, skin, layout, state);
    draw_time(&mut canvas, &sheets, skin, layout, state);
    draw_indicators(&mut canvas, &sheets, skin, layout, state);
    draw_marquee(&mut canvas, &sheets, layout, state);
    draw_text_field(&mut canvas, &sheets, layout.kbps, state.kbps);
    draw_text_field(&mut canvas, &sheets, layout.khz, state.khz);

    canvas
}

fn draw_shade(
    canvas: &mut Image,
    sheets: &Sheets,
    skin: &Skin,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    // S01 §2.1 lists the shade title bars (provisional); S01 does not cover the
    // shade position slider, so that part stays webamp-provisional.
    let title = if state.title_bar_focused {
        "MAIN_SHADE_BACKGROUND_SELECTED"
    } else {
        "MAIN_SHADE_BACKGROUND"
    };
    blit_named(
        canvas,
        sheets,
        skin,
        title,
        layout.shade_title_bar.x,
        layout.shade_title_bar.y,
    );
    blit_named(
        canvas,
        sheets,
        skin,
        "MAIN_SHADE_POSITION_BACKGROUND",
        layout.shade_position.x,
        layout.shade_position.y,
    );
    let travel = layout.shade_position.width.saturating_sub(3);
    let offset = (u32::from(state.position.min(100)) * travel + 50) / 100;
    blit_named(
        canvas,
        sheets,
        skin,
        "MAIN_SHADE_POSITION_THUMB",
        layout.shade_position.x + offset as i32,
        layout.shade_position.y,
    );
}

fn draw_clutter_bar(
    canvas: &mut Image,
    sheets: &Sheets,
    skin: &Skin,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    blit_named(
        canvas,
        sheets,
        skin,
        "MAIN_CLUTTER_BAR_BACKGROUND",
        layout.clutter_bar.x,
        layout.clutter_bar.y,
    );
    for (flag, name, rect) in [
        (
            state.clutter_o,
            "MAIN_CLUTTER_BAR_BUTTON_O_SELECTED",
            layout.clutter_o,
        ),
        (
            state.clutter_a,
            "MAIN_CLUTTER_BAR_BUTTON_A_SELECTED",
            layout.clutter_a,
        ),
        (
            state.clutter_i,
            "MAIN_CLUTTER_BAR_BUTTON_I_SELECTED",
            layout.clutter_i,
        ),
        (
            state.clutter_d,
            "MAIN_CLUTTER_BAR_BUTTON_D_SELECTED",
            layout.clutter_d,
        ),
        (
            state.clutter_v,
            "MAIN_CLUTTER_BAR_BUTTON_V_SELECTED",
            layout.clutter_v,
        ),
    ] {
        if flag {
            blit_named(canvas, sheets, skin, name, rect.x, rect.y);
        }
    }
}

/// The pressed window-button sprite names (S02 §7).
fn window_button_sprite(button: WindowButton) -> &'static str {
    match button {
        WindowButton::Menu => "MAIN_OPTIONS_BUTTON_DEPRESSED",
        WindowButton::Minimize => "MAIN_MINIMIZE_BUTTON_DEPRESSED",
        WindowButton::Shade => "MAIN_SHADE_BUTTON_DEPRESSED",
        WindowButton::Close => "MAIN_CLOSE_BUTTON_DEPRESSED",
    }
}

/// Draws the held window button from its 9 x 9 `titlebar.bmp` cell (S02 §7).
///
/// In the normal state the four buttons come from the title-bar body sprite and
/// nothing is drawn here (S01 §2.1/S02 §7); only the held state is overlaid.
fn draw_window_buttons(
    canvas: &mut Image,
    sheets: &Sheets,
    skin: &Skin,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    let Some(button) = state.pressed_window_button else {
        return;
    };
    let rect = layout.window_button(button);
    blit_named(
        canvas,
        sheets,
        skin,
        window_button_sprite(button),
        rect.x,
        rect.y,
    );
}

fn button_sprites(button: TransportButton) -> (&'static str, &'static str) {
    match button {
        TransportButton::Previous => ("MAIN_PREVIOUS_BUTTON", "MAIN_PREVIOUS_BUTTON_ACTIVE"),
        TransportButton::Play => ("MAIN_PLAY_BUTTON", "MAIN_PLAY_BUTTON_ACTIVE"),
        TransportButton::Pause => ("MAIN_PAUSE_BUTTON", "MAIN_PAUSE_BUTTON_ACTIVE"),
        TransportButton::Stop => ("MAIN_STOP_BUTTON", "MAIN_STOP_BUTTON_ACTIVE"),
        TransportButton::Next => ("MAIN_NEXT_BUTTON", "MAIN_NEXT_BUTTON_ACTIVE"),
        TransportButton::Eject => ("MAIN_EJECT_BUTTON", "MAIN_EJECT_BUTTON_ACTIVE"),
    }
}

fn draw_transport(
    canvas: &mut Image,
    sheets: &Sheets,
    skin: &Skin,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    for (button, rect) in [
        (TransportButton::Previous, layout.previous),
        (TransportButton::Play, layout.play),
        (TransportButton::Pause, layout.pause),
        (TransportButton::Stop, layout.stop),
        (TransportButton::Next, layout.next),
        (TransportButton::Eject, layout.eject),
    ] {
        let (normal, pressed) = button_sprites(button);
        let name = if state.is_pressed(button) {
            pressed
        } else {
            normal
        };
        blit_named(canvas, sheets, skin, name, rect.x, rect.y);
    }
}

fn draw_toggles(
    canvas: &mut Image,
    sheets: &Sheets,
    skin: &Skin,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    let shuffle = if state.shuffle {
        "MAIN_SHUFFLE_BUTTON_SELECTED"
    } else {
        "MAIN_SHUFFLE_BUTTON"
    };
    blit_named(
        canvas,
        sheets,
        skin,
        shuffle,
        layout.shuffle.x,
        layout.shuffle.y,
    );
    let repeat = if state.repeat {
        "MAIN_REPEAT_BUTTON_SELECTED"
    } else {
        "MAIN_REPEAT_BUTTON"
    };
    blit_named(
        canvas,
        sheets,
        skin,
        repeat,
        layout.repeat.x,
        layout.repeat.y,
    );
    let eq = if state.eq_open {
        "MAIN_EQ_BUTTON_SELECTED"
    } else {
        "MAIN_EQ_BUTTON"
    };
    blit_named(
        canvas,
        sheets,
        skin,
        eq,
        layout.eq_toggle.x,
        layout.eq_toggle.y,
    );
    let playlist = if state.playlist_open {
        "MAIN_PLAYLIST_BUTTON_SELECTED"
    } else {
        "MAIN_PLAYLIST_BUTTON"
    };
    blit_named(
        canvas,
        sheets,
        skin,
        playlist,
        layout.playlist_toggle.x,
        layout.playlist_toggle.y,
    );
}

fn draw_position(
    canvas: &mut Image,
    sheets: &Sheets,
    skin: &Skin,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    blit_named(
        canvas,
        sheets,
        skin,
        "MAIN_POSITION_SLIDER_BACKGROUND",
        layout.position_track.x,
        layout.position_track.y,
    );
    let thumb = layout.position_thumb_at(state.position);
    let thumb_sprite = if state.position_thumb_pressed {
        "MAIN_POSITION_SLIDER_THUMB_SELECTED"
    } else {
        "MAIN_POSITION_SLIDER_THUMB"
    };
    blit_named(canvas, sheets, skin, thumb_sprite, thumb.x, thumb.y);
}

/// The volume frame index for a 0..=100 value.
///
/// **Provisional** (S01 open question 4): the sheet has 28 frames of 15 rows but
/// S01 does not say which frame maps to which value. This uses webamp's
/// `round(value/100 * 28) - 1`, clamped to the 28 frames.
fn volume_frame(value: u8) -> u32 {
    let value = value.min(100) as u32;
    let frame = (value * 28 + 50) / 100;
    frame.saturating_sub(1).min(27)
}

/// The balance frame index for a 0..=100 value (50 is centred).
///
/// **Provisional** (S01 open question 4): the balance fill grows outward from
/// the centre, so the frame follows the distance from 50.
fn balance_frame(value: u8) -> u32 {
    let value = value.min(100) as u32;
    let distance = value.abs_diff(50);
    (distance * 27) / 50
}

fn draw_volume(
    canvas: &mut Image,
    sheets: &Sheets,
    skin: &Skin,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    let frame = volume_frame(state.volume);
    blit_region(
        canvas,
        sheets,
        Sheet::Volume,
        Rect::new(
            0,
            (frame * 15) as i32,
            layout.volume_track.width,
            layout.volume_track.height,
        ),
        layout.volume_track.x,
        layout.volume_track.y,
    );
    let thumb = layout.volume_thumb_at(state.volume);
    blit_named(canvas, sheets, skin, "MAIN_VOLUME_THUMB", thumb.x, thumb.y);
}

fn draw_balance(
    canvas: &mut Image,
    sheets: &Sheets,
    skin: &Skin,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    let frame = balance_frame(state.balance);
    blit_region(
        canvas,
        sheets,
        Sheet::Balance,
        Rect::new(
            0,
            (frame * 15) as i32,
            layout.balance_track.width,
            layout.balance_track.height,
        ),
        layout.balance_track.x,
        layout.balance_track.y,
    );
    let thumb = layout.balance_thumb_at(state.balance);
    blit_named(canvas, sheets, skin, "MAIN_BALANCE_THUMB", thumb.x, thumb.y);
}

/// True when the skin ships `nums_ex.bmp` itself.
///
/// The loader records a substitution when `nums_ex.bmp` was absent and filled
/// from `numbers.bmp` (S17 §3), so the absence of that record means the skin
/// provided the variant.
fn uses_nums_ex(skin: &Skin) -> bool {
    skin.sheet(Sheet::NumsEx).is_some()
        && !skin
            .substitutions()
            .iter()
            .any(|s| s.requested.eq_ignore_ascii_case("nums_ex.bmp"))
}

fn draw_time(
    canvas: &mut Image,
    sheets: &Sheets,
    skin: &Skin,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    // S02 §4: stopped and the blink-off phase draw a blank display.
    if !state.time_visible {
        return;
    }
    let nums_ex = uses_nums_ex(skin);
    let names = if nums_ex { DIGIT_NAMES_EX } else { DIGIT_NAMES };
    let blank = if nums_ex {
        "NO_MINUS_SIGN_EX"
    } else {
        "NO_DIGIT"
    };
    for (cell, rect) in state.time_cells().iter().zip(layout.time_digits.iter()) {
        match cell {
            Some(digit) => blit_named(canvas, sheets, skin, names[*digit as usize], rect.x, rect.y),
            None => blit_named(canvas, sheets, skin, blank, rect.x, rect.y),
        }
    }
    if state.time_remaining {
        if nums_ex {
            // The nums_ex minus is a full 9x13 cell; its layout is unresolved
            // (S01 open question 6), so it is drawn in the leading digit cell.
            let rect = layout.time_digits[0];
            blit_named(canvas, sheets, skin, "MINUS_SIGN_EX", rect.x, rect.y);
        } else {
            blit_named(
                canvas,
                sheets,
                skin,
                "MINUS_SIGN",
                layout.minus_sign.x,
                layout.minus_sign.y,
            );
        }
    }
}

fn draw_indicators(
    canvas: &mut Image,
    sheets: &Sheets,
    skin: &Skin,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    let transport = match state.transport {
        TransportState::Playing => "MAIN_PLAYING_INDICATOR",
        TransportState::Paused => "MAIN_PAUSED_INDICATOR",
        TransportState::Stopped => "MAIN_STOPPED_INDICATOR",
    };
    blit_named(
        canvas,
        sheets,
        skin,
        transport,
        layout.play_indicator.x,
        layout.play_indicator.y,
    );
    let decoder = if state.decoder_active() {
        "MAIN_WORKING_INDICATOR"
    } else {
        "MAIN_NOT_WORKING_INDICATOR"
    };
    blit_named(
        canvas,
        sheets,
        skin,
        decoder,
        layout.decoder_indicator.x,
        layout.decoder_indicator.y,
    );
    // S02 §6: the lit label follows the channel mode, but stopped (or nothing
    // loaded) dims both labels.
    let lit = state.transport != TransportState::Stopped;
    let mono = if lit && state.channels == MonoStereo::Mono {
        "MAIN_MONO_SELECTED"
    } else {
        "MAIN_MONO"
    };
    blit_named(canvas, sheets, skin, mono, layout.mono.x, layout.mono.y);
    let stereo = if lit && state.channels == MonoStereo::Stereo {
        "MAIN_STEREO_SELECTED"
    } else {
        "MAIN_STEREO"
    };
    blit_named(
        canvas,
        sheets,
        skin,
        stereo,
        layout.stereo.x,
        layout.stereo.y,
    );
}

/// The `text.bmp` bitmap font for a skin.
///
/// Glyphs are blitted **opaquely**, matching S01 §6 ("the classic sheets are
/// copied opaquely") and webamp's opaque glyph extraction. S01 §2.2's remark
/// that a glyph from any of the four font copies "matches the render exactly"
/// could instead imply a colour key on the non-ink pixels; the two cannot be
/// told apart from the base skin, because its marquee and kbps/kHz areas are
/// black and so is the first font copy's background. This is a documented
/// ambiguity, not a verified fact.
fn text_font(sheets: &Sheets) -> Option<BitmapFont> {
    let image = sheets.get(Sheet::Text)?.clone();
    let mut glyphs = GlyphMap::new(webamp::TEXT_CELL.0, webamp::TEXT_CELL.1);
    for (ch, col, row) in webamp::TEXT_GLYPHS {
        glyphs.insert(*ch, *col, *row);
    }
    Some(BitmapFont::new(image, glyphs, None, 0))
}

/// Renders `text` into a `field`-sized scratch buffer and composites it, so the
/// text cannot spill outside the field.
fn draw_text_field(canvas: &mut Image, sheets: &Sheets, field: Rect, text: Option<u32>) {
    let Some(text) = text else {
        return;
    };
    let Some(font) = text_font(sheets) else {
        return;
    };
    let mut scratch = Image::new(field.width, field.height);
    font.render_text(&mut scratch, 0, 0, &text.to_string());
    canvas.blit_alpha(&scratch, scratch.bounds(), field.x, field.y);
}

fn draw_marquee(
    canvas: &mut Image,
    sheets: &Sheets,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    let Some(font) = text_font(sheets) else {
        return;
    };
    let mut scratch = Image::new(layout.marquee.width, layout.marquee.height);
    // The classic font has no lowercase; lowercase letters select the uppercase
    // glyphs (S01 §2.2). The offset is the marquee scroll (S02 §3).
    font.render_text(
        &mut scratch,
        -state.marquee_offset,
        0,
        &state.title.to_ascii_lowercase(),
    );
    canvas.blit_alpha(
        &scratch,
        scratch.bounds(),
        layout.marquee.x,
        layout.marquee.y,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_frames_cover_the_sheet() {
        assert_eq!(volume_frame(0), 0);
        assert_eq!(volume_frame(100), 27);
        for v in 0..=100u8 {
            assert!(volume_frame(v) <= 27);
        }
    }

    #[test]
    fn balance_frames_grow_from_the_centre() {
        assert_eq!(balance_frame(50), 0);
        assert_eq!(balance_frame(0), 27);
        assert_eq!(balance_frame(100), 27);
        for v in 0..=100u8 {
            assert!(balance_frame(v) <= 27);
        }
    }

    #[test]
    fn frame_size_tracks_shade_and_double_size() {
        let mut state = MainWindowState::default();
        assert_eq!(frame_size(&state), (275, 116));
        state.windowshade = true;
        assert_eq!(frame_size(&state), (275, 14));
        state.double_size = true;
        assert_eq!(frame_size(&state), (550, 28));
        state.windowshade = false;
        assert_eq!(frame_size(&state), (550, 232));
    }

    #[test]
    fn shaded_double_size_matches_the_painted_frame_s05_silent() {
        // S05 never measures shade plus double-size together (§1.1 is 1x, §2.1
        // is unshaded): this pins the preserved scaled-shade behaviour (550 x
        // 28) and the delegation, so the next lane cannot regress it silently.
        // Agreement with the painted frame itself is covered by the corpus
        // test (`frame_size == frame dimensions` for all four combinations).
        use crate::docking::{WindowKind, window_size};
        let state = MainWindowState {
            windowshade: true,
            double_size: true,
            ..MainWindowState::default()
        };
        assert_eq!(frame_size(&state), (550, 28));
        assert_eq!(
            frame_size(&state),
            window_size(WindowKind::Main, true, true)
        );
    }
}
