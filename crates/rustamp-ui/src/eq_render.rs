// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Renders one equalizer-window frame from an [`EqualizerState`] and a loaded
//! [`Skin`], using `rustamp-gfx` for every blit.
//!
//! The renderer is pure: it never opens a window, never reads the clock and
//! never changes the state. A missing sprite is skipped (the loader has
//! already substituted the built-in default per S17), so a skin without a
//! sheet still produces a frame instead of panicking.
//!
//! What each part comes from:
//!
//! * Window and title bar (S01 §3): the background is `eqmain.bmp` (0,0,275,116)
//!   at (0,0); the title bar is (0,134,275,14) focused / (0,149,275,14)
//!   unfocused at (0,0) — both round-2 verified.
//! * On/Auto buttons (S07 §3.1/§3.2): normal, engaged, held and held-engaged
//!   sheet rects at the measured held footprints (On (14,18,25,12), Auto
//!   held (39,18,33,12)); engagement adds only the 6 px indicator cell.
//! * Presets button (S07 §4.1): normal at (217,18,44,12). Whether a distinct
//!   *sheet* rect backs the held sprite is not separable from captures, so no
//!   held sheet rect is invented: the held state renders the normal sprite and
//!   the 528 px held difference is reported, not drawn.
//! * Graph background (S01 §3, round 2): (0,294,113,19) at (86,17).
//! * Slider thumbs (S01 §3.1): resting (0,164,11,11), dragged (0,176,11,11),
//!   at the mapped thumb-y.
//!
//! Left open exactly as the specs leave them (drawn as the plain background,
//! never invented): the interior tiling (S07 §7.1), the per-value body
//! colouring (S07 §7.2), the graph curve and its mapping (S07 §7.3/§7.4), the
//! band value law beyond the thumb-y interpolation (S07 §7.2), and the shade
//! drawing (S05 §1.5 — the shade *size* follows the existing
//! [`crate::docking::window_size`], the pixels stay the title band).

use rustamp_gfx::{Image, Rect};
use rustamp_skin::{Sheet, Skin};

use crate::docking::{WindowKind, window_size};
use crate::equalizer::{EqSlider, EqWidget, EqualizerLayout, EqualizerState};

/// The frame size an equalizer state renders to, including double-size.
///
/// Delegates to [`docking::window_size`] (single source of truth, S05 §2.1).
/// The equalizer's shade drawing is open (S05 §1.5); the shaded size is
/// whatever the current docking geometry says (275 x 14, per S01 §1) and is
/// deliberately not re-derived here.
pub fn eq_frame_size(state: &EqualizerState) -> (u32, u32) {
    window_size(WindowKind::Equalizer, state.windowshade, state.double_size)
}

/// Renders one RGBA frame for `state`.
///
/// Double-size is the exact integer 2x scale of the 1x frame (S05 §2.1).
pub fn render_equalizer_window(
    state: &EqualizerState,
    skin: &Skin,
    layout: &EqualizerLayout,
) -> Image {
    let frame = render_1x(state, skin, layout);
    if state.double_size {
        frame.scaled(2)
    } else {
        frame
    }
}

/// The decoded equalizer sheets, converted once per frame.
struct EqSheets {
    eqmain: Option<Image>,
    eq_ex: Option<Image>,
}

impl EqSheets {
    fn load(skin: &Skin) -> Self {
        let get = |sheet: Sheet| -> Option<Image> {
            let raster = skin.sheet(sheet)?;
            Image::from_rgba(raster.width(), raster.height(), raster.as_rgba().to_vec()).ok()
        };
        Self {
            eqmain: get(Sheet::EqMain),
            eq_ex: get(Sheet::EqEx),
        }
    }
}

/// Blits an explicit `eqmain.bmp` region at `(dx, dy)`, skipping it when the
/// sheet is missing.
///
/// The regions are the S07 §3.1 sheet rects, spelled out instead of going
/// through the skin's named sprites: the skin table still carries the pre-S07
/// widths (On 26 px instead of the measured 25, Auto held at x = 154/213
/// instead of 153/212 at 33 px wide), which is reported as a finding
/// (rustamp-skin is not this lane's path). The 1 px column this drops is the
/// separator before the Auto body, which S01 §3 says is not drawn at the On
/// button's window position.
fn blit_eqmain(canvas: &mut Image, sheets: &EqSheets, src: Rect, dx: i32, dy: i32) {
    if let Some(image) = sheets.eqmain.as_ref() {
        canvas.blit_rect(image, src, dx, dy);
    }
}

/// The `eqmain.bmp` sheet rect for the On button in its current state
/// (S07 §3.1).
fn on_sheet_rect(state: &EqualizerState) -> Rect {
    let held = state.held == Some(EqWidget::On);
    match (state.enabled, held) {
        (false, false) => Rect::new(10, 119, 25, 12),
        (true, false) => Rect::new(69, 119, 25, 12),
        (false, true) => Rect::new(128, 119, 25, 12),
        (true, true) => Rect::new(187, 119, 25, 12),
    }
}

/// The `eqmain.bmp` sheet rect for the Auto button in its current state
/// (S07 §3.1).
fn auto_sheet_rect(state: &EqualizerState) -> Rect {
    let held = state.held == Some(EqWidget::Auto);
    match (state.auto, held) {
        (false, false) => Rect::new(36, 119, 32, 12),
        (true, false) => Rect::new(95, 119, 32, 12),
        (false, true) => Rect::new(153, 119, 33, 12),
        (true, true) => Rect::new(212, 119, 33, 12),
    }
}

fn render_1x(state: &EqualizerState, skin: &Skin, layout: &EqualizerLayout) -> Image {
    let sheets = EqSheets::load(skin);

    if state.windowshade {
        // S05 §1.5: the equalizer's shade drawing is open. The size is the
        // existing docking geometry; the pixels stay the shade sheet's title
        // band (S01 §3 provisional) with the title bar as fallback.
        let mut canvas = Image::filled(layout.background.width, 14, [0, 0, 0, 255]);
        if let Some(image) = sheets.eq_ex.as_ref() {
            let y = if state.focused { 0 } else { 15 };
            canvas.blit_rect(image, Rect::new(0, y, 275, 14), 0, 0);
        } else {
            blit_eqmain(
                &mut canvas,
                &sheets,
                Rect::new(0, if state.focused { 134 } else { 149 }, 275, 14),
                0,
                0,
            );
        }
        return canvas;
    }

    let mut canvas = Image::filled(
        layout.background.width,
        layout.background.height,
        [0, 0, 0, 255],
    );

    // Background (S01 §3, verified as the base layer).
    blit_eqmain(
        &mut canvas,
        &sheets,
        Rect::new(0, 0, 275, 116),
        layout.background.x,
        layout.background.y,
    );

    // Title bar (S01 §3, round 2: focused (0,134), unfocused (0,149)).
    blit_eqmain(
        &mut canvas,
        &sheets,
        Rect::new(0, if state.focused { 134 } else { 149 }, 275, 14),
        layout.title_bar.x,
        layout.title_bar.y,
    );

    // Graph background (S01 §3, round 2). The curve itself is open (S07
    // §7.3/§7.4), so only the background is drawn.
    blit_eqmain(
        &mut canvas,
        &sheets,
        Rect::new(0, 294, 113, 19),
        layout.graph.x,
        layout.graph.y,
    );

    // On / Auto with the measured held footprints (S07 §3.2). The held Auto
    // rect starts one column left of the resting rect; the resting rect is
    // the blit origin and the held sprite covers the extra column.
    blit_eqmain(
        &mut canvas,
        &sheets,
        on_sheet_rect(state),
        layout.on.x,
        layout.on.y,
    );
    let auto_x = if state.held == Some(EqWidget::Auto) {
        layout.auto_held.x
    } else {
        layout.auto.x
    };
    blit_eqmain(
        &mut canvas,
        &sheets,
        auto_sheet_rect(state),
        auto_x,
        layout.auto.y,
    );

    // Presets (S07 §4.1): no held sheet rect is invented, so the normal
    // sprite draws in every state.
    blit_eqmain(
        &mut canvas,
        &sheets,
        Rect::new(224, 164, 44, 12),
        layout.presets.x,
        layout.presets.y,
    );

    // Slider thumbs (S01 §3.1). Groove, body fill and colouring are open
    // (S07 §7.1/§7.2), so the thumbs draw over the background only.
    for slider in EqSlider::ALL_BANDS {
        let value = state.slider_value(slider);
        let thumb = EqualizerLayout::thumb_rect(slider, value);
        let dragged = state.thumb_dragged && state.held == Some(EqWidget::Slider(slider));
        // Held-but-not-dragged (a press that has not moved) keeps the resting
        // thumb: only an actual drag takes the dragged sprite (S01 §3.1).
        let sheet_y = if dragged { 176 } else { 164 };
        blit_eqmain(
            &mut canvas,
            &sheets,
            Rect::new(0, sheet_y, 11, 11),
            thumb.x,
            thumb.y,
        );
    }

    canvas
}
