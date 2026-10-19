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
//! * Presets button (S01 §3, held-states run): normal (224,164,44,12),
//!   held (224,176,44,12), drawn at (217,18).
//! * Graph background (S01 §3, round 2): (0,294,113,19) at (86,17).
//! * Slider thumbs (S01 §3.1): resting (0,164,11,11), dragged (0,176,11,11),
//!   at the mapped thumb-y.
//!
//! Provisional drawing rules (own choices, **not** a measured DSP response):
//!
//! * The ten bands form a one-pixel, unsmoothed polyline across the graph,
//!   equally spaced in x. Saved values 0/31/63 map to graph rows 0/8/18;
//!   the two intervals are linear and rounded to nearest. Colours are sampled
//!   from the sheet's 19-row ramp at the plotted y (S01 §3.1).
//! * The eleventh value, preamp, moves a separate full-width reference strip,
//!   not the band curve: `18 - round(value * 18 / 63)`. The two endpoints
//!   agree with S07 §8.2 (saved 0 -> window y35, 63 -> y17); the strip's
//!   identity and intermediate mapping remain provisional (S07 §7.3 O8).
//!   It is painted first, so the curve wins intersections. Enabled/Auto do not
//!   hide the configured curve (own choice).
//! * Each slider restores an 11-px-wide sheet column at y38..99, then fills
//!   a centred 7-px body below the thumb to y99 with a constant ramp colour,
//!   `round(value * 18 / 63)`. S01 fixes the groove's column/y offset and
//!   fill direction/end colours, **not** this width, extent, recipe or rounding.
//!   Preamp shares the band ramp and fills update during drags (own choices).
//!
//! The track atlas (13,164,209,129) is **not** blitted as a whole (S01 refutes
//! that). Shade drawing remains provisional (S01 §3,
//! S05 §1.5). See `docs/tasks/S07r-eq-renderer.md` for settling measurements.
//!
//! # The windowshade form
//!
//! S05 §1.2: the shade is a single 14-px title-bar row and no body rows are
//! drawn, so the shaded frame is that band alone — 275 x 14, or 550 x 28 when
//! the double-size flag is set (the scaled shade, an own choice for the
//! equalizer, S05 §2.3). Its pixels come from `eq_ex.bmp` (S01 §3, provisional:
//! row 0 focused, row 15 unfocused). S07 has no shade wording, so nothing in
//! the audited specs shows sliders in this form and none are drawn; the body
//! controls are not part of the frame.

use rustamp_gfx::{Image, Rect};
use rustamp_skin::{Sheet, Skin};

use crate::docking::{WindowKind, window_size};
use crate::equalizer::{
    EqSlider, EqWidget, EqualizerLayout, EqualizerState, FLAT_VALUE, MAX_VALUE, THUMB_BOTTOM_Y,
    THUMB_SIZE, THUMB_TOP_Y,
};
use crate::layout::SHADE_HEIGHT;

/// The frame size an equalizer state renders to, including double-size.
///
/// Delegates to [`docking::window_size`] (single source of truth, S05 §2.1).
/// The shaded size is the 14-px band of S01 §1 (275 x 14; open in S05 §1.5,
/// which never shaded the equalizer).
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
/// The regions are the S07 §3.1 / S01 §3 sheet rects. The 1 px column omitted
/// from the older 26-pixel On rect is the
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

/// S01 §3.1 fixes the two ramp endpoints, not intermediate quantization.
/// Nearest-row rounding, shared by preamp and band bodies, is our choice.
fn body_colour_row(value: u8) -> i32 {
    (i32::from(value.min(MAX_VALUE)) * 18 + 31) / 63
}

/// Own-choice band-value law, not a frequency-response calculation. The flat
/// anchor lies in S01's measured row range 7..=9; its exact row is our choice.
fn curve_row(value: u8) -> i32 {
    let value = i32::from(value.min(MAX_VALUE));
    let flat = i32::from(FLAT_VALUE);
    if value <= flat {
        (value * 8 + flat / 2) / flat
    } else {
        8 + ((value - flat) * 10 + 16) / 32
    }
}

/// Sample the colour scale, never paste its vertical strip as graph artwork
/// (S01 §3.1). Missing/truncated sheet pixels simply leave the base layer.
fn ramp_colour(sheets: &EqSheets, row: i32) -> Option<[u8; 4]> {
    sheets.eqmain.as_ref()?.pixel(115, 294 + row)
}

fn draw_graph(canvas: &mut Image, sheets: &EqSheets, state: &EqualizerState, graph: Rect) {
    // Table: EQ_PREAMP_LINE (0,314,113,1). The choice to use it as a
    // preamp-controlled reference is provisional (S07 §7.3), not proof of a
    // classic blit. The reversed endpoints are measured in S07 §8.2.
    blit_eqmain(
        canvas,
        sheets,
        Rect::new(0, 314, 113, 1),
        graph.x,
        graph.y + 18 - body_colour_row(state.preamp),
    );

    // Own choice: distribute ten control points evenly across all 113 pixels,
    // not by log-frequency or a DSP filter response; no preamp addition.
    for band in 0..9 {
        let x0 = (band as i32 * 112 + 4) / 9;
        let x1 = ((band as i32 + 1) * 112 + 4) / 9;
        draw_graph_segment(
            canvas,
            sheets,
            graph,
            (x0, curve_row(state.bands[band])),
            (x1, curve_row(state.bands[band + 1])),
        );
    }
}

/// Integer Bresenham rasterization: one-pixel, 8-connected segments without
/// antialiasing. This rasterization and the ramp-at-y colouring are own choices.
fn draw_graph_segment(
    canvas: &mut Image,
    sheets: &EqSheets,
    graph: Rect,
    (mut x, mut y): (i32, i32),
    (end_x, end_y): (i32, i32),
) {
    let dx = end_x - x;
    let dy = -(end_y - y).abs();
    let step_y = if y < end_y { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        if (0..graph.width as i32).contains(&x)
            && (0..graph.height as i32).contains(&y)
            && let Some(colour) = ramp_colour(sheets, y)
        {
            canvas.set_pixel(graph.x + x, graph.y + y, colour);
        }
        if (x, y) == (end_x, end_y) {
            break;
        }
        let twice_error = error * 2;
        if twice_error >= dy {
            error += dy;
            x += 1;
        }
        if twice_error <= dx {
            error += dx;
            y += step_y;
        }
    }
}

fn draw_slider_body(canvas: &mut Image, sheets: &EqSheets, thumb: Rect, value: u8) {
    let bottom = THUMB_BOTTOM_Y + THUMB_SIZE as i32;
    // Verified groove: sheet col14 (identical to col15), y = window y +191
    // (S01 §3.1). Extending the empty (13,229,11,11) cell vertically along
    // that offset is our provisional recipe; do not paste the 209x129 atlas.
    blit_eqmain(
        canvas,
        sheets,
        Rect::new(
            13,
            THUMB_TOP_Y + 191,
            THUMB_SIZE,
            (bottom - THUMB_TOP_Y) as u32,
        ),
        thumb.x,
        THUMB_TOP_Y,
    );
    if let Some(colour) = ramp_colour(sheets, body_colour_row(value)) {
        // S01 fixes a constant fill from thumb to track bottom. The centred
        // 7-px width and y100 exclusive bottom are own choices, derived from
        // the 11-px thumb and measured travel endpoints; no caps invented.
        canvas.fill_rect(
            Rect::new(
                thumb.x + 2,
                thumb.bottom(),
                7,
                (bottom - thumb.bottom()) as u32,
            ),
            colour,
        );
    }
}

fn render_1x(state: &EqualizerState, skin: &Skin, layout: &EqualizerLayout) -> Image {
    let sheets = EqSheets::load(skin);

    if state.windowshade {
        // S05 §1.2: the shade is a single 14-px title-bar row; no body rows
        // are drawn. S01 §3 (provisional): the band comes from `eq_ex.bmp`,
        // row 0 focused and row 15 unfocused.
        let mut canvas = Image::filled(layout.background.width, SHADE_HEIGHT, [0, 0, 0, 255]);
        if let Some(image) = sheets.eq_ex.as_ref() {
            let y = if state.focused { 0 } else { 15 };
            canvas.blit_rect(image, Rect::new(0, y, 275, SHADE_HEIGHT), 0, 0);
        } else {
            // A skin without the shade sheet keeps a title strip instead of a
            // blank window (S17: the loader substitutes the built-in default;
            // this is the last resort for a skin that has no sheet at all).
            blit_eqmain(
                &mut canvas,
                &sheets,
                Rect::new(0, if state.focused { 134 } else { 149 }, 275, SHADE_HEIGHT),
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

    // Graph background (S01 §3, round 2), then our provisional response drawing.
    blit_eqmain(
        &mut canvas,
        &sheets,
        Rect::new(0, 294, 113, 19),
        layout.graph.x,
        layout.graph.y,
    );
    draw_graph(&mut canvas, &sheets, state, layout.graph);

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

    // S01 §3 (held-states run): the 44x12 held sprite comes from row176.
    blit_eqmain(
        &mut canvas,
        &sheets,
        Rect::new(
            224,
            if state.held == Some(EqWidget::Presets) {
                176
            } else {
                164
            },
            44,
            12,
        ),
        layout.presets.x,
        layout.presets.y,
    );

    // Restore each track and colour its body before the measured thumb sprite.
    for slider in EqSlider::ALL_BANDS {
        let value = state.slider_value(slider);
        let thumb = EqualizerLayout::thumb_rect(slider, value);
        draw_slider_body(&mut canvas, &sheets, thumb, value);
        let dragged = state.thumb_dragged && state.held == Some(EqWidget::Slider(slider));
        // A lone down already starts the dragged state (S07 §8.2: 73 px
        // without movement); EqualizerWindow sets thumb_dragged on that down.
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

#[cfg(test)]
mod tests {
    use super::*;

    // Our own coordinate-marked pixels, not a reference skin. Distinct source
    // rows/columns catch wrong strip sampling even when the built-in artwork
    // happens to use a constant colour across a region.
    fn marked_sheets() -> EqSheets {
        let mut image = Image::filled(275, 315, [0, 0, 0, 255]);
        for y in 0..315 {
            for x in 0..275 {
                image.set_pixel(x, y, [x as u8, y as u8, (y / 256) as u8, 255]);
            }
        }
        EqSheets {
            eqmain: Some(image),
            eq_ex: None,
        }
    }

    #[test]
    fn provisional_row_laws_clamp_and_preserve_their_declared_anchors() {
        assert_eq!([curve_row(0), curve_row(31), curve_row(63)], [0, 8, 18]);
        assert_eq!([body_colour_row(0), body_colour_row(63)], [0, 18]);
        for value in 0..=255 {
            assert!((0..=18).contains(&curve_row(value)));
            assert!((0..=18).contains(&body_colour_row(value)));
            if value > 0 {
                assert!(curve_row(value) >= curve_row(value - 1));
                assert!(body_colour_row(value) >= body_colour_row(value - 1));
            }
        }
    }

    #[test]
    fn graph_samples_the_full_reference_strip_and_plots_every_control_point() {
        let sheets = marked_sheets();
        let sheet = sheets.eqmain.as_ref().unwrap();
        let graph = EqualizerLayout::CLASSIC.graph;
        for preamp in [0, 31, 63, 255] {
            let state = EqualizerState {
                bands: [0, 63, 0, 63, 0, 63, 0, 63, 0, 63],
                preamp,
                ..EqualizerState::default()
            };
            let untouched = [1, 2, 3, 255];
            let mut canvas = Image::filled(275, 116, untouched);
            draw_graph(&mut canvas, &sheets, &state, graph);
            for band in 0..10 {
                let x = (band as i32 * 112 + 4) / 9;
                let y = curve_row(state.bands[band]);
                assert_eq!(
                    canvas.pixel(graph.x + x, graph.y + y),
                    sheet.pixel(115, 294 + y)
                );
            }
            // The left endpoint is never at reference row9, or at the
            // opposite extreme: this checks preamp mapping independently.
            let reference = 18 - body_colour_row(preamp);
            let x = if reference == 0 { 112 } else { 0 };
            assert_eq!(
                canvas.pixel(graph.x + x, graph.y + reference),
                sheet.pixel(x, 314)
            );
            for y in 0..116 {
                for x in 0..275 {
                    if x < graph.x || x >= graph.right() || y < graph.y || y >= graph.bottom() {
                        assert_eq!(canvas.pixel(x, y), Some(untouched));
                    }
                }
            }
        }
    }

    #[test]
    fn body_restores_sheet_track_and_fills_only_below_the_thumb() {
        let sheets = marked_sheets();
        let sheet = sheets.eqmain.as_ref().unwrap();
        for value in [0, 15, 31, 46, 63, 255] {
            let thumb = EqualizerLayout::thumb_rect(EqSlider::Preamp, value);
            let untouched = [1, 2, 3, 255];
            let mut canvas = Image::filled(275, 116, untouched);
            draw_slider_body(&mut canvas, &sheets, thumb, value);
            for y in 38..100 {
                assert_eq!(canvas.pixel(thumb.x, y), sheet.pixel(13, y + 191));
                let expected = if y >= thumb.bottom() {
                    sheet.pixel(115, 294 + body_colour_row(value))
                } else {
                    sheet.pixel(15, y + 191)
                };
                assert_eq!(canvas.pixel(thumb.x + 2, y), expected);
            }
            for (x, y) in [
                (thumb.x - 1, 50),
                (thumb.x + 11, 50),
                (thumb.x, 37),
                (thumb.x, 100),
            ] {
                assert_eq!(canvas.pixel(x, y), Some(untouched));
            }
        }
    }

    #[test]
    fn missing_or_truncated_sheet_leaves_new_drawing_untouched() {
        for eqmain in [None, Some(Image::filled(2, 2, [9, 9, 9, 255]))] {
            let sheets = EqSheets {
                eqmain,
                eq_ex: None,
            };
            let mut canvas = Image::filled(275, 116, [1, 2, 3, 255]);
            let before = canvas.clone();
            draw_graph(
                &mut canvas,
                &sheets,
                &EqualizerState::default(),
                EqualizerLayout::CLASSIC.graph,
            );
            draw_slider_body(
                &mut canvas,
                &sheets,
                EqualizerLayout::thumb_rect(EqSlider::Preamp, 31),
                31,
            );
            assert_eq!(canvas, before);
        }
    }
}
