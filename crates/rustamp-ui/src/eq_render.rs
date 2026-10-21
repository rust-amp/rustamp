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
//!   (39,18,33,12)); engagement adds only the 6 px indicator cell.
//! * Presets button (S01 §3, held-states run): normal (224,164,44,12),
//!   held (224,176,44,12), drawn at (217,18).
//! * Graph background (S01 §3, round 2): (0,294,113,19) at (86,17).
//! * Slider tracks and thumbs (S01 §3.1, measured 2026-10-21 against the
//!   reference capture): the track is the sheet's own 14 x 63 column, drawn
//!   one pixel left of the thumb; the thumb is (0,164,11,11) resting and
//!   (0,176,11,11) dragged, at the mapped thumb-y.
//!
//! Provisional drawing rules (own choices, **not** a measured DSP response):
//!
//! * The ten bands form a one-pixel, unsmoothed polyline across the graph,
//!   with the control points spread evenly over the graph's inner width
//!   (two pixels in from each edge). Saved values 0/31/63 map to graph rows
//!   0/9/18; the two intervals are linear and rounded to nearest. Colours
//!   are sampled from the sheet's 19-row ramp at the plotted y (S01 §3.1).
//! * The eleventh value, preamp, moves a separate full-width reference strip,
//!   not the band curve: `18 - round(value * 18 / 63)`. The two endpoints
//!   agree with S07 §8.2 (saved 0 -> window y35, 63 -> y17); the strip's
//!   identity and intermediate mapping remain provisional (S07 §7.3 O8).
//!   It is painted first, so the curve wins intersections. Enabled/Auto do not
//!   hide the configured curve (own choice).
//! * Which of the sheet's 26 track sprites a saved value picks is an own
//!   choice: the sheet's own colour order, the most cut value first. The two
//!   anchors S01 §3.1 gives hold (value 0 is the red body, the ramp's row 0,
//!   value 63 the green one, row 18), and the flat value 31 lands on the
//!   sprite the measured reference capture draws. S07 §12 O5 leaves the
//!   intermediate mapping open.
//!
//! The track atlas (13,164,209,129) is **not** blitted as a whole (S01 refutes
//! that); each slider takes one 14 x 63 column of it. Shade drawing remains
//! provisional (S01 §3, S05 §1.5). See `docs/tasks/S07r-eq-renderer.md` for
//! settling measurements.
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
    EqSlider, EqWidget, EqualizerLayout, EqualizerState, FLAT_VALUE, MAX_VALUE, TRACK_HEIGHT,
    TRACK_WIDTH,
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
///
/// The sheet keeps the Auto button in four columns 59 px apart, each 33 px
/// wide: the normal one at x = 35, the engaged at 94, the held at 153 and the
/// held-engaged at 212. The measured reference capture draws the resting
/// button in the same 33-px footprint as the held one, at window x = 39.
fn auto_sheet_rect(state: &EqualizerState) -> Rect {
    let held = state.held == Some(EqWidget::Auto);
    let column = match (state.auto, held) {
        (false, false) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (true, true) => 3,
    };
    Rect::new(35 + column * 59, 119, 33, 12)
}

/// The number of track sprites the sheet's atlas holds (13 per row band).
const TRACK_SPRITE_COUNT: i32 = 26;
/// The atlas' first track column (S01 §3.1: the atlas starts at x = 13).
const TRACK_ATLAS_X: i32 = 13;
/// The distance between two track columns in the sheet.
const TRACK_CELL_PITCH: i32 = 15;
/// The row band of the most cut half of the value range.
const TRACK_CUT_ROW: i32 = 164;
/// The row band of the most boost half of the value range.
const TRACK_BOOST_ROW: i32 = 229;
/// The curve's row at the flat value: the graph's centre row.
const CURVE_FLAT_ROW: i32 = 9;
/// The curve's row at the ends of the value range (the graph has 19 rows).
const CURVE_MAX_ROW: i32 = 18;
/// The graph's margin, in pixels, that the curve keeps on each side.
///
/// The measured reference capture draws the flat curve from the graph's
/// second to its second-to-last column.
const GRAPH_MARGIN: i32 = 2;

/// The track sprite's index in the sheet's own colour order, 0 = the most cut
/// value and 25 the most boost one.
///
/// **Own choice:** the sheet's 26 sprites in their own order, the highest
/// value first. The two anchors S01 §3.1 gives hold: value 0 is the red body
/// (the ramp's row 0) and value 63 the green one (row 18). The flat value 31
/// lands on the first sprite of the boost band, which is the one the
/// measured reference capture draws.
fn track_sprite_index(value: u8) -> i32 {
    let value = i32::from(value.min(MAX_VALUE));
    ((i32::from(MAX_VALUE) - value) * (TRACK_SPRITE_COUNT - 1) + 31) / i32::from(MAX_VALUE)
}

/// The `eqmain.bmp` track sprite for a saved value.
///
/// The sheet's slider atlas (S01 §3.1: (13,164,209,129), never blitted as a
/// whole) holds 26 complete track sprites: 13 columns in the row band at
/// y = 164 and 13 in the band at y = 229, the columns 15 px apart. Each
/// sprite is 14 x 63 px and carries the groove, the coloured body, its bright
/// core and the end caps.
fn track_sheet_rect(value: u8) -> Rect {
    let index = track_sprite_index(value);
    let (cell, row) = if index < TRACK_SPRITE_COUNT / 2 {
        (index, TRACK_CUT_ROW)
    } else {
        (index - TRACK_SPRITE_COUNT / 2, TRACK_BOOST_ROW)
    };
    Rect::new(
        TRACK_ATLAS_X + cell * TRACK_CELL_PITCH,
        row,
        TRACK_WIDTH,
        TRACK_HEIGHT,
    )
}

/// The row of the sheet's 19-row colour scale (115,294) for a saved value.
///
/// S01 §3.1 fixes the two ends of the scale: value 0 is row 0 (red) and
/// value 63 row 18 (green). The intermediate rows are a nearest-row
/// rounding, shared by the preamp strip and the curve.
fn colour_scale_row(value: u8) -> i32 {
    (i32::from(value.min(MAX_VALUE)) * 18 + 31) / 63
}

/// The curve's row for a saved value.
///
/// The flat anchor lies on the graph's centre row: the measured reference
/// capture draws the flat curve there, and S01 §3.1's measured range is
/// rows 7..=9.
fn curve_row(value: u8) -> i32 {
    let value = i32::from(value.min(MAX_VALUE));
    let flat = i32::from(FLAT_VALUE);
    if value <= flat {
        (value * CURVE_FLAT_ROW + flat / 2) / flat
    } else {
        CURVE_FLAT_ROW + ((value - flat) * (CURVE_MAX_ROW - CURVE_FLAT_ROW) + 16) / 32
    }
}

/// The graph's control point x for band `band`.
///
/// The ten control points are spread evenly over the graph's inner width.
fn curve_x(band: usize, graph: Rect) -> i32 {
    let last = graph.width as i32 - GRAPH_MARGIN - 1;
    GRAPH_MARGIN + (band as i32 * (last - GRAPH_MARGIN)) / 9
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
        graph.y + CURVE_MAX_ROW - colour_scale_row(state.preamp),
    );

    // Own choice: the ten control points spread evenly over the graph's
    // inner width, not by log-frequency or a DSP filter response; no preamp
    // addition.
    for band in 0..9 {
        draw_graph_segment(
            canvas,
            sheets,
            graph,
            (curve_x(band, graph), curve_row(state.bands[band])),
            (curve_x(band + 1, graph), curve_row(state.bands[band + 1])),
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

/// Draws the slider's track: the sheet's own column with the groove, the
/// coloured body and the end caps (measured 2026-10-21 against the reference
/// capture). The thumb is drawn on top of it.
fn draw_slider_track(canvas: &mut Image, sheets: &EqSheets, slider: EqSlider, value: u8) {
    let track = EqualizerLayout::track_rect(slider);
    blit_eqmain(canvas, sheets, track_sheet_rect(value), track.x, track.y);
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
    blit_eqmain(
        &mut canvas,
        &sheets,
        auto_sheet_rect(state),
        layout.auto.x,
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

    // Draw each track, then the measured thumb sprite on top of it.
    for slider in EqSlider::ALL_BANDS {
        let value = state.slider_value(slider);
        draw_slider_track(&mut canvas, &sheets, slider, value);
        let thumb = EqualizerLayout::thumb_rect(slider, value);
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
        assert_eq!([curve_row(0), curve_row(31), curve_row(63)], [0, 9, 18]);
        assert_eq!([colour_scale_row(0), colour_scale_row(63)], [0, 18]);
        for value in 0..=255 {
            assert!((0..=18).contains(&curve_row(value)));
            assert!((0..=18).contains(&colour_scale_row(value)));
            if value > 0 {
                assert!(curve_row(value) >= curve_row(value - 1));
                assert!(colour_scale_row(value) >= colour_scale_row(value - 1));
            }
        }
    }

    #[test]
    fn the_track_sprites_cover_the_sheets_own_colour_order() {
        // The most cut value is the green body (the ramp's row 18), the most
        // boost one the red body (row 0) and the flat value the sprite the
        // measured reference capture draws.
        assert_eq!(track_sheet_rect(63), Rect::new(13, 164, 14, 63));
        assert_eq!(track_sheet_rect(31), Rect::new(13, 229, 14, 63));
        assert_eq!(track_sheet_rect(0), Rect::new(193, 229, 14, 63));
        // Every sprite is inside the atlas and the value order is monotone.
        let mut previous = i32::MAX;
        for value in 0..=MAX_VALUE {
            let rect = track_sheet_rect(value);
            assert!(rect.x >= TRACK_ATLAS_X);
            assert!(rect.x + rect.width as i32 <= 275);
            assert_eq!(rect.height, TRACK_HEIGHT);
            assert_eq!(rect.width, TRACK_WIDTH);
            let index = track_sprite_index(value);
            assert!(index <= previous, "the track order must not go backwards");
            previous = index;
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
                let x = curve_x(band, graph);
                let y = curve_row(state.bands[band]);
                assert_eq!(
                    canvas.pixel(graph.x + x, graph.y + y),
                    sheet.pixel(115, 294 + y)
                );
            }
            // The left endpoint is never at reference row9, or at the
            // opposite extreme: this checks preamp mapping independently.
            let reference = CURVE_MAX_ROW - colour_scale_row(preamp);
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
    fn the_track_is_one_sheet_column_left_of_the_thumb() {
        let sheets = marked_sheets();
        let sheet = sheets.eqmain.as_ref().unwrap();
        for value in [0, 15, 31, 46, 63, 255] {
            let track = EqualizerLayout::track_rect(EqSlider::Preamp);
            let untouched = [1, 2, 3, 255];
            let mut canvas = Image::filled(275, 116, untouched);
            draw_slider_track(&mut canvas, &sheets, EqSlider::Preamp, value);
            let src = track_sheet_rect(value);
            for y in 0..TRACK_HEIGHT as i32 {
                for x in 0..TRACK_WIDTH as i32 {
                    assert_eq!(
                        canvas.pixel(track.x + x, track.y + y),
                        sheet.pixel(src.x + x, src.y + y)
                    );
                }
            }
            // The column stops at the track's own rectangle.
            for (x, y) in [
                (track.x - 1, 50),
                (track.right(), 50),
                (track.x, 37),
                (track.x, track.bottom()),
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
            draw_slider_track(&mut canvas, &sheets, EqSlider::Preamp, 31);
            assert_eq!(canvas, before);
        }
    }
}
