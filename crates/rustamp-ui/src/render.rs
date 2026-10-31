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
use crate::visualization::Geometry;
use crate::widget::Widget;

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
    draw_text_field(&mut canvas, &sheets, layout.kbps, state.kbps.map(kbps_text));
    draw_text_field(
        &mut canvas,
        &sheets,
        layout.khz,
        state.khz.map(|khz| khz.to_string()),
    );

    crate::visualization::draw_visualization(
        &mut canvas,
        layout.visualization,
        Geometry::Normal,
        &state.visualization,
        skin.viscolors(),
        state.transport != TransportState::Stopped,
    );

    canvas
}

/// Where the blank cell left of the elapsed time sits.
const TIME_SIGN_CELL: (i32, i32) = (36, 26);

/// The four digit cells of the shaded window's time, in `text.bmp`'s font, and
/// their row (measured against Winamp 2.95, 2026-10-31).
const SHADE_TIME_CELLS: [i32; 4] = [134, 139, 147, 152];
const SHADE_TIME_Y: i32 = 4;

/// The shaded window's two narrow text cells left of the time, three pixels of
/// a `text.bmp` cell each: the first stays blank, the second is the sign of a
/// remaining time (measured against Winamp 2.95, 2026-10-31).
const SHADE_BLANK_CELL_X: i32 = 126;
const SHADE_SIGN_CELL_X: i32 = 130;
const SHADE_SIGN_CELL_WIDTH: u32 = 3;

/// The shaded position thumb's first column inside the groove, and its travel:
/// the groove's first column never carries it (same measurement).
const SHADE_THUMB_START: i32 = 1;
const SHADE_THUMB_TRAVEL: i32 = 12;

/// The shaded position thumb at `percent`: where it sits inside the groove, and
/// its cell. The skin draws three thumbs, leaning towards the groove's ends
/// (measured over a 300 s track against Winamp 2.95, 2026-10-31).
fn shade_thumb(percent: u8) -> (i32, &'static str) {
    let step = i32::from(percent.min(100)) * SHADE_THUMB_TRAVEL / 100;
    let cell = match step {
        ..=4 => "MAIN_SHADE_POSITION_THUMB_LEFT",
        5..=7 => "MAIN_SHADE_POSITION_THUMB",
        _ => "MAIN_SHADE_POSITION_THUMB_RIGHT",
    };
    (SHADE_THUMB_START + step, cell)
}

fn draw_shade(
    canvas: &mut Image,
    sheets: &Sheets,
    skin: &Skin,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    // S01 §2.1 lists the shade title bars (provisional).
    //
    // The strip is the skin's own shade title bar and nothing else: the
    // measured shaded main window of Winamp 2.95 (base-2.91, client capture
    // of 2026-10-21) is pixel for pixel `titlebar.bmp` (27,29,275,14), so
    // 2.95 paints no slider of its own over the strip - the mini position
    // groove it shows there is part of that cell. The webamp-provisional
    // `MAIN_SHADE_POSITION_*` cells live in the 9 x 9 window-button area
    // that 2.95 never reads (S01 §2.4, open question 1), so blitting them
    // would only cover the skin's own art with the leftovers of a button
    // cell. That capture was of a stopped player. While a track plays or is
    // paused 2.95 does draw a thumb on the groove (measured 2026-10-31), the
    // cell the sheet keeps for it; the groove is the strip's seek target (see
    // `MainWindowLayout::shade_position`).
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
    // S08 §1: the shaded main window carries its own analyser, 38 x 5 px with
    // five bands.
    crate::visualization::draw_visualization(
        canvas,
        layout.shade_visualization,
        Geometry::Shaded,
        &state.visualization,
        skin.viscolors(),
        // A paused track has no floor of peak marks (measured 2026-10-31).
        state.transport == TransportState::Playing,
    );
    if state.track_loaded && state.transport != TransportState::Stopped {
        let track = layout.shade_position;
        blit_named(
            canvas,
            sheets,
            skin,
            "MAIN_SHADE_POSITION_BACKGROUND",
            track.x,
            track.y,
        );
        let (offset, cell) = shade_thumb(state.position);
        blit_named(canvas, sheets, skin, cell, track.x + offset, track.y);
    }
    if state.transport != TransportState::Stopped {
        let minus = state.time_remaining && state.time_visible;
        blit_shade_sign_cell(canvas, sheets, ' ', SHADE_BLANK_CELL_X);
        let sign = if minus { '-' } else { ' ' };
        blit_shade_sign_cell(canvas, sheets, sign, SHADE_SIGN_CELL_X);
    }
    if state.transport != TransportState::Stopped
        && let Some(font) = text_font(sheets)
    {
        // The pause blink's dark phase is blank cells, not the strip's art
        // (measured against Winamp 2.95, 2026-10-31).
        for (cell, x) in state.time_cells().iter().zip(SHADE_TIME_CELLS) {
            let shown = cell.filter(|_| state.time_visible);
            let text = shown.map_or_else(|| " ".to_owned(), |digit| digit.to_string());
            font.render_text(canvas, x, SHADE_TIME_Y, &text);
        }
    }
    if state.pressed_window_button == Some(WindowButton::Shade) {
        let rect = layout.window_button(WindowButton::Shade);
        blit_named(
            canvas,
            sheets,
            skin,
            "MAIN_SHADE_BUTTON_SELECTED_DEPRESSED",
            rect.x,
            rect.y,
        );
    } else {
        draw_window_buttons(canvas, sheets, skin, layout, state);
    }
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
    // The sheet's transport row is five 22-px buttons at a 23-px pitch
    // (S01 §2.4), so a one-pixel separator column stands between them.
    // Measured 2026-10-21 (base-2.91 client capture of Winamp 2.95): the
    // reference draws that column too - the column left of the play button is
    // `cbuttons.bmp`'s column 22, pixel for pixel. The first button has no
    // column to its left, and the eject stands six pixels away from the next
    // button, so it has none either.
    let row = [
        (TransportButton::Previous, layout.previous),
        (TransportButton::Play, layout.play),
        (TransportButton::Pause, layout.pause),
        (TransportButton::Stop, layout.stop),
        (TransportButton::Next, layout.next),
    ];
    for (index, (button, rect)) in row.into_iter().enumerate() {
        let (normal, pressed) = button_sprites(button);
        let name = if state.is_pressed(button) {
            pressed
        } else {
            normal
        };
        if index > 0 {
            draw_transport_separator(canvas, sheets, skin, name, rect);
        }
        blit_named(canvas, sheets, skin, name, rect.x, rect.y);
    }
    let (normal, pressed) = button_sprites(TransportButton::Eject);
    let name = if state.is_pressed(TransportButton::Eject) {
        pressed
    } else {
        normal
    };
    blit_named(canvas, sheets, skin, name, layout.eject.x, layout.eject.y);
}

/// Draws the one-pixel separator column in front of a transport button.
///
/// The column belongs to the same sheet row as the button it stands in front
/// of, so the resting row brings the skin's own separator and the pressed row
/// its dark one. The source column is the button's own column minus one, so
/// the sheet stays the only place that knows the cells.
fn draw_transport_separator(
    canvas: &mut Image,
    sheets: &Sheets,
    skin: &Skin,
    name: &str,
    at: Rect,
) {
    let Some(def) = skin.sprite(name) else {
        return;
    };
    if def.x == 0 {
        return;
    }
    let Some(src) = sheets.get(def.sheet) else {
        return;
    };
    canvas.blit_rect(
        src,
        Rect::new(def.x as i32 - 1, def.y as i32, 1, def.height),
        at.x - 1,
        at.y,
    );
}

fn draw_toggles(
    canvas: &mut Image,
    sheets: &Sheets,
    skin: &Skin,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    let shuffle = match (state.shuffle, state.pressed_widget == Some(Widget::Shuffle)) {
        (false, false) => "MAIN_SHUFFLE_BUTTON",
        (false, true) => "MAIN_SHUFFLE_BUTTON_DEPRESSED",
        (true, false) => "MAIN_SHUFFLE_BUTTON_SELECTED",
        (true, true) => "MAIN_SHUFFLE_BUTTON_SELECTED_DEPRESSED",
    };
    blit_named(
        canvas,
        sheets,
        skin,
        shuffle,
        layout.shuffle.x,
        layout.shuffle.y,
    );
    let repeat = match (state.repeat, state.pressed_widget == Some(Widget::Repeat)) {
        (false, false) => "MAIN_REPEAT_BUTTON",
        (false, true) => "MAIN_REPEAT_BUTTON_DEPRESSED",
        (true, false) => "MAIN_REPEAT_BUTTON_SELECTED",
        (true, true) => "MAIN_REPEAT_BUTTON_SELECTED_DEPRESSED",
    };
    blit_named(
        canvas,
        sheets,
        skin,
        repeat,
        layout.repeat.x,
        layout.repeat.y,
    );
    let eq = match (
        state.eq_open,
        state.pressed_widget == Some(Widget::EqToggle),
    ) {
        (false, false) => "MAIN_EQ_BUTTON",
        (false, true) => "MAIN_EQ_BUTTON_DEPRESSED",
        (true, false) => "MAIN_EQ_BUTTON_SELECTED",
        (true, true) => "MAIN_EQ_BUTTON_DEPRESSED_SELECTED",
    };
    blit_named(
        canvas,
        sheets,
        skin,
        eq,
        layout.eq_toggle.x,
        layout.eq_toggle.y,
    );
    let playlist = match (
        state.playlist_open,
        state.pressed_widget == Some(Widget::PlaylistToggle),
    ) {
        (false, false) => "MAIN_PLAYLIST_BUTTON",
        (false, true) => "MAIN_PLAYLIST_BUTTON_DEPRESSED",
        (true, false) => "MAIN_PLAYLIST_BUTTON_SELECTED",
        (true, true) => "MAIN_PLAYLIST_BUTTON_DEPRESSED_SELECTED",
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
    // Measured 2026-10-21 (base-2.91 client capture of Winamp 2.95): while
    // nothing is loaded, the reference draws **no** position bar - the skin's
    // own `main.bmp` shows through the whole track band (16,72)-(263,81).
    // Only a loaded track gets the track and the thumb; a track that cannot
    // be seeked keeps the disabled look below.
    // The same while stopped with a track loaded (measured 2026-10-31).
    if !state.track_loaded || state.transport == TransportState::Stopped {
        return;
    }
    blit_named(
        canvas,
        sheets,
        skin,
        "MAIN_POSITION_SLIDER_BACKGROUND",
        layout.position_track.x,
        layout.position_track.y,
    );
    let thumb = match state.position_px {
        Some(offset) => layout.position_thumb_at_px(offset),
        None => layout.position_thumb_at(state.position),
    };
    if state.seekable {
        let thumb_sprite = if state.position_thumb_pressed {
            "MAIN_POSITION_SLIDER_THUMB_SELECTED"
        } else {
            "MAIN_POSITION_SLIDER_THUMB"
        };
        blit_named(canvas, sheets, skin, thumb_sprite, thumb.x, thumb.y);
    } else {
        // The disabled look of a control that cannot be used.
        draw_disabled(canvas, sheets, skin, "MAIN_POSITION_SLIDER_THUMB", thumb);
    }
}

/// The opacity a disabled control's sprite is drawn with.
///
/// **Rustamp's own look.** The classic skin carries no "disabled" sprite for
/// the position slider, and S19 leaves the reference's own display open (its
/// open question 7 asks what the seek bar shows without a length), so the
/// disabled look is ours: the skin's own sprite, dimmed over the track. The
/// control keeps its shape and reads as inactive.
const DISABLED_ALPHA: u8 = 96;

/// Draws the named sprite dimmed over the canvas.
///
/// The sprite is the skin's own art; only its opacity is lowered, so the
/// result is the skin's picture blended with what is behind it.
fn draw_disabled(canvas: &mut Image, sheets: &Sheets, skin: &Skin, name: &str, at: Rect) {
    let Some(def) = skin.sprite(name) else {
        return;
    };
    let Some(src) = sheets.get(def.sheet) else {
        return;
    };
    let mut scratch = Image::new(def.width, def.height);
    scratch.blit_rect(
        src,
        Rect::new(def.x as i32, def.y as i32, def.width, def.height),
        0,
        0,
    );
    for pixel in scratch.as_rgba_mut().as_chunks_mut::<4>().0 {
        pixel[3] = DISABLED_ALPHA;
    }
    canvas.blit_alpha(&scratch, scratch.bounds(), at.x, at.y);
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
    let thumb = layout.volume_thumb_for_core(state.volume_core, state.volume);
    let name = if state.pressed_widget == Some(Widget::VolumeSlider) {
        "MAIN_VOLUME_THUMB_SELECTED"
    } else {
        "MAIN_VOLUME_THUMB"
    };
    blit_named(canvas, sheets, skin, name, thumb.x, thumb.y);
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
            9,
            (frame * 15) as i32,
            layout.balance_track.width,
            layout.balance_track.height,
        ),
        layout.balance_track.x,
        layout.balance_track.y,
    );
    let thumb = layout.balance_thumb_at(state.balance);
    let name = if state.pressed_widget == Some(Widget::BalanceSlider) {
        "MAIN_BALANCE_THUMB_ACTIVE"
    } else {
        "MAIN_BALANCE_THUMB"
    };
    blit_named(canvas, sheets, skin, name, thumb.x, thumb.y);
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
    // The loader distinguishes shipped digit art from per-file fallback.
    // Sheets are independent: a fallback NumsEx must not shadow real Numbers.
    let nums_ex = skin.time_digit_sheet() == Sheet::NumsEx;
    let names = if nums_ex { DIGIT_NAMES_EX } else { DIGIT_NAMES };
    let blank = if nums_ex {
        "NO_MINUS_SIGN_EX"
    } else {
        "NO_DIGIT"
    };
    // A track whose length is unknown has no remaining time: the display
    // carries the placeholder alone (the dash below) and no cells at all.
    // Four blank cells would still read as a time, which is the thing the
    // placeholder is there to avoid.
    if !state.shows_time_placeholder() {
        for (cell, rect) in state.time_cells().iter().zip(layout.time_digits.iter()) {
            match cell {
                Some(digit) => {
                    blit_named(canvas, sheets, skin, names[*digit as usize], rect.x, rect.y)
                }
                None => blit_named(canvas, sheets, skin, blank, rect.x, rect.y),
            }
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
    } else if !state.shows_time_placeholder() {
        // Elapsed time: the sign's place holds the blank digit cell, whose
        // background differs from `main.bmp`'s (measured 2026-10-31).
        blit_named(
            canvas,
            sheets,
            skin,
            blank,
            TIME_SIGN_CELL.0,
            TIME_SIGN_CELL.1,
        );
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
    // Winamp 2.95 lights the decoder strip only while playing. Paused and
    // stopped it holds the blank cell of the same sheet, not the "not working"
    // sprite (measured 2026-10-31).
    if state.decoder_active() {
        blit_named(
            canvas,
            sheets,
            skin,
            "MAIN_WORKING_INDICATOR",
            layout.decoder_indicator.x,
            layout.decoder_indicator.y,
        );
    } else {
        blit_region(
            canvas,
            sheets,
            Sheet::Playpaus,
            Rect::new(27, 0, 3, 9),
            layout.decoder_indicator.x,
            layout.decoder_indicator.y,
        );
    }
    // S01's decoder and transport footprints share x26. The complete 9x9
    // transport cell must win that column (S02 §1), not be overwritten by
    // the decoder strip's rightmost pixel.
    blit_named(
        canvas,
        sheets,
        skin,
        transport,
        layout.play_indicator.x,
        layout.play_indicator.y,
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
    Some(BitmapFont::new(image, text_glyphs(), None, 0))
}

fn text_glyphs() -> GlyphMap {
    let mut glyphs = GlyphMap::new(webamp::TEXT_CELL.0, webamp::TEXT_CELL.1);
    for (ch, col, row) in webamp::TEXT_GLYPHS {
        glyphs.insert(*ch, *col, *row);
    }
    glyphs
}

/// Draws the left part of `ch`'s `text.bmp` cell at `x` on the shaded time's row.
fn blit_shade_sign_cell(canvas: &mut Image, sheets: &Sheets, ch: char, x: i32) {
    let (Some(text), Some((col, row))) = (sheets.get(Sheet::Text), text_glyphs().cell(ch)) else {
        return;
    };
    let (width, height) = webamp::TEXT_CELL;
    let cell = Rect::new(
        (col * width) as i32,
        (row * height) as i32,
        SHADE_SIGN_CELL_WIDTH,
        height,
    );
    canvas.blit_rect(text, cell, x, SHADE_TIME_Y);
}

/// The skin's colour at `(x, y)`: what a text drawn there is blended against.
pub(crate) fn under_text(canvas: &Image, x: i32, y: i32) -> [u8; 3] {
    canvas
        .pixel(x, y)
        .map_or([0; 3], |pixel| [pixel[0], pixel[1], pixel[2]])
}

/// Whether the `text.bmp` font can draw `text` as it reads.
///
/// The classic font has no lowercase: lowercase letters select the uppercase
/// glyphs (S01 §2.2), and a capital selects the same one. The Å, Ö and Ä cells
/// are not relied on, because a skin may leave them empty. Text with any other
/// character goes to a real font (see [`draw_system_text`]).
pub(crate) fn bitmap_can_draw(text: &str) -> bool {
    text.chars().all(|ch| {
        ch.is_ascii()
            && webamp::TEXT_GLYPHS
                .iter()
                .any(|(glyph, _, _)| *glyph == ch.to_ascii_lowercase())
    })
}

/// The size of the real font in the bitmap font's places (own choice): the
/// largest one the marquee's band and a dialog's line hold.
const SYSTEM_TEXT_SIZE: u32 = 13;

/// The real font in the bitmap font's places.
fn system_text_font() -> crate::playlist_font::SystemFont<'static> {
    crate::playlist_font::SystemFont {
        family: crate::playlist_font::DEFAULT_FAMILY,
        size: SYSTEM_TEXT_SIZE,
        bold: false,
        italic: false,
    }
}

/// Draws `text` in a real font inside `band`, starting `x` pixels into it
/// (negative while the text scrolls), in `colour`. `None` when the host has no
/// font at all.
pub(crate) fn draw_system_text(
    canvas: &mut Image,
    band: Rect,
    x: i32,
    text: &str,
    colour: [u8; 3],
) -> Option<()> {
    let font = system_text_font();
    let width = crate::playlist_font::text_width(font, text)?;
    let under = under_text(canvas, band.x, band.y + band.height as i32 / 2);
    let scratch =
        crate::playlist_font::render(font, text, width.max(1), band.height, colour, under)?;
    let skipped = (-x).max(0);
    let shown = (width as i32 - skipped).clamp(0, band.width as i32 - x.max(0));
    if shown > 0 {
        let visible = Rect::new(skipped, 0, shown as u32, band.height);
        canvas.blit_alpha(&scratch, visible, band.x + x.max(0), band.y);
    }
    Some(())
}

/// The pixel width `text` takes in the marquee: 5 px a cell in the bitmap font
/// (S02 §3), its real width in the real font.
pub(crate) fn marquee_text_width(text: &str) -> u32 {
    let cells = text.chars().count() as u32 * 5;
    if bitmap_can_draw(text) {
        return cells;
    }
    crate::playlist_font::text_width(system_text_font(), text).unwrap_or(cells)
}

/// The bitrate as the three cells of the numeral field (S02 §5) hold it. A rate
/// of four digits shows its hundreds and an `H`: 1411 kbps is `14H`, as Winamp
/// 2.95 shows it (measured side by side, 2026-10-31).
fn kbps_text(kbps: u32) -> String {
    if kbps < 1000 {
        kbps.to_string()
    } else {
        format!("{}h", (kbps / 100).min(99))
    }
}

/// The ink colour of the skin's `text.bmp`: the first pixel of the `a` cell
/// that differs from the blank cell. Text in a real font wears it, so it looks
/// like the skin's own text.
fn text_ink(sheets: &Sheets) -> [u8; 3] {
    let fallback = [0xff; 3];
    let Some(sheet) = sheets.get(Sheet::Text) else {
        return fallback;
    };
    let (cell_w, cell_h) = webamp::TEXT_CELL;
    let blank = webamp::TEXT_GLYPHS
        .iter()
        .find(|(glyph, _, _)| *glyph == ' ')
        .and_then(|(_, col, row)| sheet.pixel((col * cell_w) as i32, (row * cell_h) as i32));
    for y in 0..cell_h as i32 {
        for x in 0..cell_w as i32 {
            if let Some(pixel) = sheet.pixel(x, y)
                && Some(pixel) != blank
            {
                return [pixel[0], pixel[1], pixel[2]];
            }
        }
    }
    fallback
}

/// Renders `text` into a `field`-sized scratch buffer and composites it, so the
/// text cannot spill outside the field.
fn draw_text_field(canvas: &mut Image, sheets: &Sheets, field: Rect, text: Option<String>) {
    let Some(text) = text else {
        return;
    };
    let Some(font) = text_font(sheets) else {
        return;
    };
    let mut scratch = Image::new(field.width, field.height);
    font.render_text(&mut scratch, 0, 0, &text);
    canvas.blit_alpha(&scratch, scratch.bounds(), field.x, field.y);
}

fn draw_marquee(
    canvas: &mut Image,
    sheets: &Sheets,
    layout: &MainWindowLayout,
    state: &MainWindowState,
) {
    // Characters the bitmap font does not have: the whole title goes to a real
    // font, in a band taller than the 6 px row, because no character of that
    // kind can be read at 6 px. The marquee's black panel has the room.
    if !bitmap_can_draw(&state.title) {
        let band = Rect::new(
            layout.marquee.x,
            layout.marquee.y - 4,
            layout.marquee.width,
            layout.marquee.height + 8,
        );
        let colour = text_ink(sheets);
        if draw_system_text(canvas, band, -state.marquee_offset, &state.title, colour).is_some() {
            return;
        }
    }
    let Some(font) = text_font(sheets) else {
        return;
    };
    let mut scratch = Image::new(layout.marquee.width, layout.marquee.height);
    // The offset is the marquee scroll (S02 §3).
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
    #[test]
    fn the_shaded_thumb_follows_winamps_measured_steps() {
        use super::shade_thumb;
        const LEFT: &str = "MAIN_SHADE_POSITION_THUMB_LEFT";
        const CENTRE: &str = "MAIN_SHADE_POSITION_THUMB";
        const RIGHT: &str = "MAIN_SHADE_POSITION_THUMB_RIGHT";
        for (percent, offset, cell) in [
            (0, 1, LEFT),
            (8, 1, LEFT),
            (10, 2, LEFT),
            (41, 5, LEFT),
            (43, 6, CENTRE),
            (66, 8, CENTRE),
            (68, 9, RIGHT),
            (100, 13, RIGHT),
        ] {
            assert_eq!(shade_thumb(percent), (offset, cell), "{percent}%");
        }
    }

    #[test]
    fn only_what_the_bitmap_font_has_stays_with_the_bitmap_font() {
        assert!(bitmap_can_draw("Jump To File"));
        assert!(bitmap_can_draw(r"a:\b-c.wav (1:00)"));
        assert!(!bitmap_can_draw("Ünïcödé"));
        assert!(!bitmap_can_draw("日本語"));
        assert!(!bitmap_can_draw("Åäö"));
    }

    #[test]
    fn a_bitrate_of_four_digits_fits_the_three_cells() {
        assert_eq!(kbps_text(192), "192");
        assert_eq!(kbps_text(999), "999");
        assert_eq!(kbps_text(1411), "14h");
    }

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
