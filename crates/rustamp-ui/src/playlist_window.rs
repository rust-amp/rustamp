//! The playlist editor window: layout, hit-testing and rendering.
//!
//! [`PlaylistWindow`](crate::playlist::PlaylistWindow) (spec **S06**) is the
//! headless behaviour model; this module is the window around it. Everything
//! here is headless: layout is arithmetic on the S01 rectangles, hit-testing
//! reuses the settled S01/S06 points, and rendering paints one RGBA frame from
//! a [`Skin`] through `rustamp-gfx`. No `winit`, no clock, no window.
//!
//! Provenance per element (see `specs/S01-window-geometry.md` §4 and
//! `specs/S06-playlist.md`):
//!
//! * Client size 275 x 116: S06 scope / S01 §1, verified. The playlist editor
//!   is not affected by double-size mode (S05 §2.1, settled).
//! * Title bar: the 25 px corners, the 100 px strip blitted once at x = 87 and
//!   the 25 px tile pitch (S01 §4.1, round 2, verified). Focused rows are sheet
//!   y 0-19, unfocused rows y 21-40 (S01 §4.1, round 2, verified).
//! * **No window-menu button** (S06 §1, settled with qualification): only Close
//!   at (264,3) and Shade at (254,3), 9 x 9 each (S01 §4, provisional).
//! * List band bbox (12,23)-(254,93) (S06 §2, settled: the single-click change).
//!   Rows at 13 px pitch (S01 §4.2: the captured list "shows four rows at 13 px
//!   pitch"). The band below the fourth row is list background (own reading).
//! * Selection fill: the `SelectedBG` colour from `pledit.txt` over the list
//!   area (S01 §4, verified by diff); entry text in the `Normal` colour
//!   (`#00FF00` in the base skin), or `Current` for the current playback entry.
//! * Scrollbar handle `PLEDIT.BMP` (52,53,7,17) at (260,y) with y = 20 at the
//!   scroll top, 40 in the middle and 60 at the bottom (S01 §4.2, round 2,
//!   verified); the arrow pair `PLEDIT.BMP` (261,72,8,10) fixed at (260,78)
//!   (S01 §4.2, verified). Arrow click points (263,81) up / (263,87) down and
//!   the one-page step (S06 §3, settled).
//! * Bottom bar blocks (0,72,125,38) at (0,78) and (126,72,150,38) at (125,78)
//!   (S01 §4, verified with a small residual). Buttons from sheet y = 86 drawn
//!   at window y = 92-109, x ≈ 11/43/72/101/228, widths ≈ 22-25 px (S01 §4.1,
//!   verified; x origins ambiguous by ±3 px, so every button rect here is
//!   **provisional**). Behavioural hit-testing uses the settled
//!   [`crate::playlist::hit_test_button`] ranges, not these rects.
//!
//! Deliberate simplifications (the spec leaves these open, so they are kept
//! simple and named):
//!
//! * The tiled title band clips whole 25 px tiles at the band edges instead of
//!   the measured partial anchor pieces (S01 §4.1); the pixels differ where a
//!   partial piece would start.
//! * Entry text is drawn with the `text.bmp` bitmap font as a stand-in. S01 §4
//!   settles that 2.95 uses a real font here, whose identity is open, so no
//!   stand-in can match it; the colour follows `pledit.txt`.
//! * The up/down split of the arrow pair (at y = 83) is an own choice; only
//!   the two click points are settled.
//! * The handle-y interpolation between the three settled points is an own
//!   linear choice.
//! * MISC popup contents, sort/randomise, double-click, drag, keyboard and
//!   message exposure stay open (S06 §8) and are not rendered or wired here.

use rustamp_gfx::{BitmapFont, GlyphMap, Image, Rect};
use rustamp_skin::{Sheet, Skin, webamp};

use crate::playlist::{
    PLAYLIST_WINDOW_SIZE, PlaylistButton, PlaylistWindow, ScrollDirection, VISIBLE_ROWS,
};

// ---------------------------------------------------------------------------
// Title bar (S01 §4.1; S06 §1: no window-menu button)
// ---------------------------------------------------------------------------

/// Height of the playlist title band (S01 §4.1: sheet rows 0-19).
pub const TITLE_BAR_HEIGHT: i32 = 20;

/// Left edge of the 100 px title strip, blitted exactly once (S01 §4.1).
pub const TITLE_STRIP_X: i32 = 87;

/// Width of the title strip (S01 §4.1).
pub const TITLE_STRIP_WIDTH: u32 = 100;

/// Width of the title corners and the tile pitch (S01 §4.1, 25 px).
pub const TITLE_TILE_WIDTH: u32 = 25;

/// Close button rect (S01 §4, provisional: 9 x 9 at (264,3)).
pub const CLOSE_BUTTON_RECT: Rect = Rect::new(264, 3, 9, 9);

/// Shade button rect (S01 §4, provisional: 9 x 9 at (254,3)).
pub const SHADE_BUTTON_RECT: Rect = Rect::new(254, 3, 9, 9);

/// The playlist title bar's buttons.
///
/// There is deliberately no menu variant: S06 §1 settles that the playlist
/// title bar carries no window-menu button, so nothing may offer one here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistTitleButton {
    /// Close (S01 §4, provisional rect).
    Close,
    /// Shade (S01 §4, provisional rect).
    Shade,
}

/// Which title-bar button (if any) a click at window-relative `(x, y)` hits.
///
/// Close and shade rects are S01 §4 (provisional); (6,3) — where a
/// window-menu button would sit — hits nothing (S06 §1, settled).
pub fn hit_test_title_bar(x: i32, y: i32) -> Option<PlaylistTitleButton> {
    if !(0..TITLE_BAR_HEIGHT).contains(&y) {
        return None;
    }
    if in_rect(x, y, CLOSE_BUTTON_RECT) {
        return Some(PlaylistTitleButton::Close);
    }
    if in_rect(x, y, SHADE_BUTTON_RECT) {
        return Some(PlaylistTitleButton::Shade);
    }
    None
}

fn in_rect(x: i32, y: i32, rect: Rect) -> bool {
    x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()
}

// ---------------------------------------------------------------------------
// List area (S06 §2; S01 §4.2)
// ---------------------------------------------------------------------------

/// Left edge of the list band (S06 §2, settled bbox (12,23)-(254,93)).
pub const LIST_LEFT: i32 = 12;

/// Top edge of the list band (S06 §2, settled).
pub const LIST_TOP: i32 = 23;

/// Right edge (exclusive) of the list band (S06 §2, settled).
pub const LIST_RIGHT: i32 = 254;

/// Bottom edge (exclusive) of the list band (S06 §2, settled).
pub const LIST_BOTTOM: i32 = 93;

/// Row pitch in pixels (S01 §4.2: "four rows at 13 px pitch").
pub const ROW_PITCH: i32 = 13;

/// The settled list band as a rectangle (S06 §2).
pub fn list_area() -> Rect {
    Rect::new(
        LIST_LEFT,
        LIST_TOP,
        (LIST_RIGHT - LIST_LEFT) as u32,
        (LIST_BOTTOM - LIST_TOP) as u32,
    )
}

/// The rectangle of visible row `row` (0-based).
///
/// Returns `None` past [`VISIBLE_ROWS`]: rows below the fourth band are list
/// background (own reading; the spec fixes the band and the pitch only).
pub fn row_rect(row: usize) -> Option<Rect> {
    if row >= VISIBLE_ROWS {
        return None;
    }
    Some(Rect::new(
        LIST_LEFT,
        LIST_TOP + row as i32 * ROW_PITCH,
        (LIST_RIGHT - LIST_LEFT) as u32,
        ROW_PITCH as u32,
    ))
}

/// Whether `(x, y)` is inside the settled list band (S06 §2).
pub fn in_list_area(x: i32, y: i32) -> bool {
    in_rect(x, y, list_area())
}

/// Which visible row (if any) a click at `(x, y)` lands on.
///
/// Rows are the 13 px bands from [`row_rect`]; the band below the fourth row
/// is background and hits nothing. Callers that implement the S06 §2 click
/// map any hit to the model's first-visible-row selection (a single click
/// selects the first visible row, whatever row was clicked).
pub fn hit_test_list_row(x: i32, y: i32) -> Option<usize> {
    if !(LIST_LEFT..LIST_RIGHT).contains(&x) || y < LIST_TOP {
        return None;
    }
    let row = ((y - LIST_TOP) / ROW_PITCH) as usize;
    if row < VISIBLE_ROWS { Some(row) } else { None }
}

// ---------------------------------------------------------------------------
// Scrollbar (S01 §4.2; S06 §3)
// ---------------------------------------------------------------------------

/// Scrollbar handle x (S01 §4.2, verified: drawn at (260,y)).
pub const SCROLL_HANDLE_X: i32 = 260;

/// Handle y at the scroll top (S01 §4.2, verified).
pub const SCROLL_TOP_Y: i32 = 20;

/// Handle travel in pixels: 20 at the top, 40 in the middle, 60 at the bottom
/// (S01 §4.2, verified).
pub const SCROLL_TRAVEL_PX: i32 = 40;

/// The arrow pair rect: `PLEDIT.BMP` (261,72,8,10) at (260,78) (S01 §4.2,
/// verified; no pressed sprite).
pub const SCROLL_ARROWS_RECT: Rect = Rect::new(260, 78, 8, 10);

/// Source rect of the handle in `PLEDIT.BMP` (S01 §4.2, verified: 7 x 17).
pub const SCROLL_HANDLE_SRC: Rect = Rect::new(52, 53, 7, 17);

/// Source rect of the arrow pair in `PLEDIT.BMP` (S01 §4.2, verified).
pub const SCROLL_ARROWS_SRC: Rect = Rect::new(261, 72, 8, 10);

/// Up/down split inside the arrow pair.
///
/// Own choice: only the (263,81) up and (263,87) down points are settled
/// (S06 §3); the split between them is not.
pub const SCROLL_SPLIT_Y: i32 = 83;

/// Handle y for a scroll offset of `first_visible` in a list of `len`.
///
/// The three S01 §4.2 points (20 top, 40 middle, 60 bottom) are pinned; the
/// linear interpolation between them is an own choice, rounded to the
/// nearest row so that it is the exact inverse of
/// [`scroll_offset_for_handle_y`] on the pixel grid (a dragged handle
/// renders under the cursor). A list that fits the four-row page shows the
/// handle at the top.
pub fn scroll_handle_y(first_visible: usize, len: usize) -> i32 {
    if len <= VISIBLE_ROWS {
        return SCROLL_TOP_Y;
    }
    let max_first = len.saturating_sub(VISIBLE_ROWS);
    let clamped = first_visible.min(max_first);
    // Round to the nearest row, the same way the inverse rounds; the
    // settled points are exact multiples of the row count, so they are
    // unchanged by the rounding.
    let rows = max_first.max(1);
    SCROLL_TOP_Y + ((SCROLL_TRAVEL_PX as usize * clamped + rows / 2) / rows) as i32
}

/// Which scroll direction (if any) a click at `(x, y)` requests.
///
/// The settled points are (263,81) up and (263,87) down (S06 §3); the split
/// at [`SCROLL_SPLIT_Y`] is an own choice.
pub fn hit_test_scroll(x: i32, y: i32) -> Option<ScrollDirection> {
    if !in_rect(x, y, SCROLL_ARROWS_RECT) {
        return None;
    }
    if y < SCROLL_SPLIT_Y {
        Some(ScrollDirection::Up)
    } else {
        Some(ScrollDirection::Down)
    }
}

/// The scrollbar track: the column the handle travels in, above the arrow
/// pair (S01 §4.2: handle drawn at (260,y) with y = 20/40/60; arrows fixed at
/// (260,78)).
///
/// Own choice, stated plainly: no spec fixes the track rectangle — only the
/// three handle points and the arrow rect are settled — so the track is the
/// 8 px column from the top settled point down to the arrow pair, wide enough
/// to hold the 7 px handle and the 8 px arrows.
pub const SCROLL_TRACK_RECT: Rect = Rect::new(260, 20, 8, 58);

/// The handle rectangle for a scroll offset of `first_visible` in a list of
/// `len` (S01 §4.2: `PLEDIT.BMP` (52,53,7,17) drawn at (260,y)).
pub fn scroll_handle_rect(first_visible: usize, len: usize) -> Rect {
    Rect::new(SCROLL_HANDLE_X, scroll_handle_y(first_visible, len), 7, 17)
}

/// The scroll offset for a handle-top position `y` in a list of `len`.
///
/// Own choice, stated plainly: S06 §3 settles only that a dragged handle
/// "moves the view" (a visual reading toward the top of the list); no spec
/// fixes the pixel-to-row mapping. This is the linear inverse of
/// [`scroll_handle_y`] (which is itself an own-choice interpolation between
/// the three settled S01 §4.2 points), clamped to the track ends and rounded
/// to the nearest row, so the two mappings are exact inverses on the pixel
/// grid: every handle position maps to an offset and back to the same
/// position. An exact round trip in the other direction — every offset
/// through its position and back — is impossible, because the 57 offsets of
/// a 60-entry list share 41 integer pixel rows; offsets that share a row
/// land within one row of themselves.
pub fn scroll_offset_for_handle_y(y: i32, len: usize) -> usize {
    let max_first = len.saturating_sub(VISIBLE_ROWS);
    if max_first == 0 {
        return 0;
    }
    let clamped = y.clamp(SCROLL_TOP_Y, SCROLL_TOP_Y + SCROLL_TRAVEL_PX);
    ((clamped - SCROLL_TOP_Y) as usize * max_first + (SCROLL_TRAVEL_PX as usize / 2))
        / SCROLL_TRAVEL_PX as usize
}

/// Where on the scrollbar (if anywhere) a click at `(x, y)` lands.
///
/// Own choice, stated plainly: the spec settles the two arrow points only.
/// A click on the handle is for dragging ([`scroll_offset_for_handle_y`]); a
/// click on the track above the handle pages up and below it pages down
/// (one page step each, via the model's `scroll_page`), the
/// usual track behaviour. The arrow pair keeps its own path
/// ([`hit_test_scroll`]) and returns `None` here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollbarHit {
    /// On the handle: drag it ([`scroll_offset_for_handle_y`]).
    Handle,
    /// On the track above the handle: page up.
    TrackAbove,
    /// On the track below the handle: page down.
    TrackBelow,
}

/// Which scrollbar part (if any) a click at `(x, y)` hits, for a view showing
/// `first_visible` in a list of `len`.
pub fn hit_test_scrollbar(
    x: i32,
    y: i32,
    first_visible: usize,
    len: usize,
) -> Option<ScrollbarHit> {
    if in_rect(x, y, SCROLL_ARROWS_RECT) {
        return None;
    }
    if !in_rect(x, y, SCROLL_TRACK_RECT) {
        return None;
    }
    let handle = scroll_handle_rect(first_visible, len);
    if in_rect(x, y, handle) {
        Some(ScrollbarHit::Handle)
    } else if y < handle.y {
        Some(ScrollbarHit::TrackAbove)
    } else {
        Some(ScrollbarHit::TrackBelow)
    }
}

// ---------------------------------------------------------------------------
// Bottom bar (S01 §4, §4.1; S06 §4)
// ---------------------------------------------------------------------------

/// The bottom bar background: two blocks at (0,78) and (125,78), 38 px tall
/// (S01 §4, verified with a small residual).
pub const BOTTOM_BAR_RECT: Rect = Rect::new(0, 78, 275, 38);

/// Source sheet row of the bottom-bar buttons (S01 §4.1, verified: y = 86).
pub const BUTTON_SRC_Y: i32 = 86;

/// Source x of the LIST OPTS button (S01 §4.1: sheet x = 229, one pixel right
/// of the drawn x = 228).
pub const LIST_OPTS_SRC_X: i32 = 229;

/// The drawn rectangle of `button`.
///
/// Provisional throughout (S01 §4.1: x origins ≈ 11/43/72/101/228 ambiguous
/// by ±3 px, widths ≈ 22-25 px). Behavioural hit-testing must use the settled
/// [`hit_test_button`] ranges, never these rects.
pub fn button_rect(button: PlaylistButton) -> Rect {
    match button {
        PlaylistButton::Add => Rect::new(11, 92, 23, 18),
        PlaylistButton::Rem => Rect::new(43, 92, 24, 18),
        PlaylistButton::Sel => Rect::new(72, 92, 24, 18),
        PlaylistButton::Misc => Rect::new(101, 92, 24, 18),
        PlaylistButton::ListOpts => Rect::new(228, 92, 24, 18),
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Renders one 275 x 116 playlist frame.
///
/// `labels[i]` is the text of `window.visible_ids()[i]`; a missing label
/// renders as an empty row. `focused` selects the title band variant. A
/// missing sprite or sheet is skipped (the loader has already substituted the
/// built-in default per S17), so a skin without a sheet still produces a
/// frame instead of panicking.
pub fn render_playlist_window(
    skin: &Skin,
    window: &PlaylistWindow,
    labels: &[String],
    focused: bool,
) -> Image {
    let (width, height) = PLAYLIST_WINDOW_SIZE;
    let style = skin.pledit();
    let mut canvas = Image::filled(
        width,
        height,
        [
            style.normal_bg[0],
            style.normal_bg[1],
            style.normal_bg[2],
            255,
        ],
    );
    let pledit = load_sheet(skin, Sheet::Pledit);
    draw_title_bar(&mut canvas, pledit.as_ref(), skin, focused);
    draw_side_rails(&mut canvas, pledit.as_ref(), skin);
    draw_bottom_bar(&mut canvas, pledit.as_ref(), skin);
    draw_rows(&mut canvas, skin, window, labels);
    draw_scrollbar(&mut canvas, pledit.as_ref(), window);
    canvas
}

fn load_sheet(skin: &Skin, sheet: Sheet) -> Option<Image> {
    let raster = skin.sheet(sheet)?;
    Image::from_rgba(raster.width(), raster.height(), raster.as_rgba().to_vec()).ok()
}

fn blit_sprite(
    canvas: &mut Image,
    pledit: Option<&Image>,
    skin: &Skin,
    name: &str,
    dx: i32,
    dy: i32,
) {
    let Some(def) = skin.sprite(name) else {
        return;
    };
    if def.sheet != Sheet::Pledit {
        return;
    }
    let Some(src) = pledit else {
        return;
    };
    canvas.blit_rect(
        src,
        Rect::new(def.x as i32, def.y as i32, def.width, def.height),
        dx,
        dy,
    );
}

fn blit_region(canvas: &mut Image, pledit: Option<&Image>, src: Rect, dx: i32, dy: i32) {
    if let Some(image) = pledit {
        canvas.blit_rect(image, src, dx, dy);
    }
}

fn draw_title_bar(canvas: &mut Image, pledit: Option<&Image>, skin: &Skin, focused: bool) {
    // Focused vs unfocused rows (S01 §4.1, verified); the SELECTED-means-focused
    // reading follows the main-window convention in `render::render_1x`.
    let (corner, strip, tile, right) = if focused {
        (
            "PLAYLIST_TOP_LEFT_SELECTED",
            "PLAYLIST_TITLE_BAR_SELECTED",
            "PLAYLIST_TOP_TILE_SELECTED",
            "PLAYLIST_TOP_RIGHT_CORNER_SELECTED",
        )
    } else {
        (
            "PLAYLIST_TOP_LEFT_CORNER",
            "PLAYLIST_TITLE_BAR",
            "PLAYLIST_TOP_TILE",
            "PLAYLIST_TOP_RIGHT_CORNER",
        )
    };
    blit_sprite(canvas, pledit, skin, corner, 0, 0);
    // Tiled bands at 25 px pitch, clipped at the band edges (whole tiles; the
    // measured partial anchor pieces of S01 §4.1 are an owned simplification).
    // The tiles go down first so the strip and the corners cover the overflow.
    let mut dx = TITLE_TILE_WIDTH as i32;
    while dx < TITLE_STRIP_X {
        blit_sprite(canvas, pledit, skin, tile, dx, 0);
        dx += TITLE_TILE_WIDTH as i32;
    }
    let mut dx = TITLE_STRIP_X + TITLE_STRIP_WIDTH as i32;
    while dx < 250 {
        blit_sprite(canvas, pledit, skin, tile, dx, 0);
        dx += TITLE_TILE_WIDTH as i32;
    }
    blit_sprite(canvas, pledit, skin, strip, TITLE_STRIP_X, 0);
    blit_sprite(canvas, pledit, skin, right, 250, 0);
    // Close/shade from the raw sheet rects (S01 §4, provisional); the webamp
    // table only names the pressed cells, so no sprite name is used.
    blit_region(canvas, pledit, Rect::new(52, 42, 9, 9), 264, 3);
    blit_region(canvas, pledit, Rect::new(62, 42, 9, 9), 254, 3);
}

fn draw_side_rails(canvas: &mut Image, pledit: Option<&Image>, skin: &Skin) {
    // Left rail tiles from y = 20 (S01 §4.1, verified); the right rail stays
    // provisional there but is drawn the same way. Tiles stop at the bottom
    // bar (y = 78).
    let mut dy = LIST_TOP - 3;
    while dy + 29 <= BOTTOM_BAR_RECT.y {
        blit_sprite(canvas, pledit, skin, "PLAYLIST_LEFT_TILE", 0, dy);
        blit_sprite(canvas, pledit, skin, "PLAYLIST_RIGHT_TILE", 255, dy);
        dy += 29;
    }
}

fn draw_bottom_bar(canvas: &mut Image, pledit: Option<&Image>, skin: &Skin) {
    blit_sprite(canvas, pledit, skin, "PLAYLIST_BOTTOM_LEFT_CORNER", 0, 78);
    blit_sprite(
        canvas,
        pledit,
        skin,
        "PLAYLIST_BOTTOM_RIGHT_CORNER",
        125,
        78,
    );
    // Button faces from sheet y = 86 at the drawn destinations (S01 §4.1).
    // No bottom-bar button draws a pressed sprite (S01 §4.1, round 2).
    for button in PlaylistButton::ALL {
        let dst = button_rect(*button);
        let src_x = match button {
            PlaylistButton::ListOpts => LIST_OPTS_SRC_X,
            _ => dst.x,
        };
        blit_region(
            canvas,
            pledit,
            Rect::new(src_x, BUTTON_SRC_Y, dst.width, dst.height),
            dst.x,
            dst.y,
        );
    }
}

/// The `text.bmp` bitmap font, as a stand-in for the playlist's real font.
///
/// S01 §4 settles that 2.95 draws the list with a real font (identity open),
/// so this stand-in cannot match it; see the module docs. The helper mirrors
/// `render::text_font`'s glyph map, but not its opaque cell backgrounds.
/// The space cell's top-left pixel is the background key (own-choice mask
/// extraction, not a measured font rule). Only glyph ink survives, so it can
/// be tinted without painting a sheet-coloured box over `SelectedBG`.
fn text_font(skin: &Skin) -> Option<BitmapFont> {
    let raster = skin.sheet(Sheet::Text)?;
    let image =
        Image::from_rgba(raster.width(), raster.height(), raster.as_rgba().to_vec()).ok()?;
    playlist_font_mask(image)
}

fn playlist_font_mask(mut image: Image) -> Option<BitmapFont> {
    let mut glyphs = GlyphMap::new(webamp::TEXT_CELL.0, webamp::TEXT_CELL.1);
    for (ch, col, row) in webamp::TEXT_GLYPHS {
        glyphs.insert(*ch, *col, *row);
    }
    let (col, row) = glyphs.cell(' ')?;
    let background = image.pixel(
        (col * webamp::TEXT_CELL.0) as i32,
        (row * webamp::TEXT_CELL.1) as i32,
    )?;
    for pixel in image.as_rgba_mut().as_chunks_mut::<4>().0 {
        if pixel[..3] == background[..3] {
            *pixel = [0; 4];
        } else {
            pixel[..3].fill(255);
        }
    }
    Some(BitmapFont::new(image, glyphs, None, 0))
}

/// Paints the visible rows: the selection band first, then the labels.
///
/// The selected row's colour is the **skin's** `SelectedBG` from
/// `pledit.txt` ([`rustamp_skin::PlaylistStyle::selected_bg`]), never a
/// colour of ours - the skin decides what a selected row looks like. The
/// band is one row of the settled list area ([`row_rect`], S06 §2: x 12-254,
/// 13 px pitch), so it cannot reach the side rails or the scrollbar.
///
/// `Normal` and `Current` tint the glyph ink independently of selection;
/// `Current` applies only to the app-supplied current entry, not the clicked
/// or selected one (S17 §5). The bitmap shapes remain a stand-in for `Font`.
fn draw_rows(canvas: &mut Image, skin: &Skin, window: &PlaylistWindow, labels: &[String]) {
    let style = skin.pledit();
    let selected = [
        style.selected_bg[0],
        style.selected_bg[1],
        style.selected_bg[2],
        255,
    ];
    let font = text_font(skin);
    let visible = window.visible_ids();
    let selected_ids = window.selected_ids();
    for (i, id) in visible.iter().enumerate().take(VISIBLE_ROWS) {
        let Some(rect) = row_rect(i) else {
            continue;
        };
        if selected_ids.contains(id) {
            canvas.fill_rect(rect, selected);
        }
        let text = labels.get(i).map(String::as_str).unwrap_or("");
        if text.is_empty() {
            continue;
        }
        if let Some(font) = font.as_ref() {
            // Scratch-clipped like the main-window marquee, so text cannot
            // spill past the row or into the scrollbar.
            let mut scratch = Image::new(rect.width, rect.height);
            font.render_text(&mut scratch, 0, 0, text);
            let colour = if window.current() == Some(*id) {
                style.current
            } else {
                style.normal
            };
            for pixel in scratch.as_rgba_mut().as_chunks_mut::<4>().0 {
                if pixel[3] != 0 {
                    pixel[..3].copy_from_slice(&colour);
                }
            }
            canvas.blit_alpha(&scratch, scratch.bounds(), rect.x, rect.y);
        }
    }
}

fn draw_scrollbar(canvas: &mut Image, pledit: Option<&Image>, window: &PlaylistWindow) {
    // Verified raw rects (S01 §4.2) rather than the webamp 8x18 handle name.
    blit_region(
        canvas,
        pledit,
        SCROLL_HANDLE_SRC,
        SCROLL_HANDLE_X,
        scroll_handle_y(window.first_visible(), window.len()),
    );
    blit_region(
        canvas,
        pledit,
        SCROLL_ARROWS_SRC,
        SCROLL_ARROWS_RECT.x,
        SCROLL_ARROWS_RECT.y,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_mask_removes_nonblack_background_and_preserves_ink_alpha() {
        let background = [0x51, 0x62, 0x73, 255];
        let mut sheet = Image::filled(155, 18, background);
        // Black ink must not be mistaken for background; transparent ink
        // must not become visible merely because its RGB differs from the key.
        sheet.set_pixel(0, 0, [0, 0, 0, 255]);
        sheet.set_pixel(1, 0, [0, 0, 0, 128]);
        sheet.set_pixel(2, 0, [9, 8, 7, 0]);
        let font = playlist_font_mask(sheet).expect("the space cell exists");
        let mut scratch = Image::new(10, 6);
        font.render_text(&mut scratch, 0, 0, "a ");
        assert_eq!(scratch.pixel(0, 0), Some([255; 4]));
        assert_eq!(scratch.pixel(1, 0), Some([255, 255, 255, 128]));
        assert_eq!(scratch.pixel(2, 0).unwrap()[3], 0);
        assert_eq!(scratch.pixel(3, 0), Some([0; 4]));
        assert_eq!(scratch.pixel(5, 0), Some([0; 4]), "space is transparent");
    }

    #[test]
    fn row_ink_uses_normal_and_current_without_erasing_either_background() {
        use rustamp_core::{command::SelectOp, playlist::EntryId};
        use rustamp_skin::{BuiltinDefaultSkin, BuiltinLookalikeSkin};

        for mut source in [BuiltinDefaultSkin::source(), BuiltinLookalikeSkin::source()] {
            // All four keys differ from both embedded palettes and from one
            // another, so baked-in or hardcoded colours cannot pass.
            source.add(
                "pledit.txt",
                b"[Text]\nNormal=#13579B\nCurrent=#ECA864\nNormalBG=#192A3B\nSelectedBG=#4C5D6E\n"
                    .to_vec(),
            );
            let skin = Skin::from_source(&mut source, &BuiltinDefaultSkin).unwrap();
            let mut window = PlaylistWindow::new();
            window.set_entries((0..8).map(EntryId).collect());
            window.scroll_to(4);
            // Long enough to exercise clipping at the list's right edge.
            let labels = vec!["a a ".repeat(30); VISIBLE_ROWS];
            for current in [Some(EntryId(5)), Some(EntryId(6)), None] {
                window.set_current(current);
                for selected in [EntryId(5), EntryId(6)] {
                    window.apply_select(&SelectOp::Set(vec![selected]));
                    let empty = render_playlist_window(&skin, &window, &[], true);
                    let frame = render_playlist_window(&skin, &window, &labels, true);
                    for (i, id) in window.visible_ids().into_iter().enumerate() {
                        let row = row_rect(i).unwrap();
                        let style = skin.pledit();
                        let bg = if id == selected {
                            style.selected_bg
                        } else {
                            style.normal_bg
                        };
                        let ink = if Some(id) == current {
                            style.current
                        } else {
                            style.normal
                        };
                        let bg = [bg[0], bg[1], bg[2], 255];
                        let ink = [ink[0], ink[1], ink[2], 255];
                        let mut ink_count = 0;
                        for y in row.y..row.bottom() {
                            for x in row.x..row.right() {
                                let pixel = frame.pixel(x, y).unwrap();
                                assert!(pixel == bg || pixel == ink, "row {i}: {pixel:?}");
                                ink_count += usize::from(pixel == ink);
                            }
                        }
                        assert!(ink_count > 0, "a nonempty label must have visible ink");
                        assert_eq!(
                            frame.pixel(row.x + 5, row.y),
                            Some(bg),
                            "space keeps the band"
                        );
                        assert_eq!(frame.pixel(row.x, row.bottom() - 1), Some(bg));
                    }
                    for y in 0..frame.height() as i32 {
                        for x in 0..frame.width() as i32 {
                            if !in_list_area(x, y) {
                                assert_eq!(
                                    frame.pixel(x, y),
                                    empty.pixel(x, y),
                                    "clipping at {x},{y}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// The selected row's colour is the skin's, not ours.
    ///
    /// The owner's screenshot showed a brown-and-orange selection band that
    /// fights the skin. This is the check that says where that colour comes
    /// from: a skin whose `pledit.txt` names a selection colour must get
    /// exactly that colour, in exactly the settled list band. It fails if
    /// anyone hardcodes a highlight here, and it fails if the band spills
    /// outside the list area.
    ///
    /// The band is the only thing a headless check can state. Whether the
    /// *bundled* skin's own `SelectedBG` is pleasant is a choice in that
    /// skin's `pledit.txt` (`crates/rustamp-skin/assets/rust-theme`), not in
    /// this renderer - see the card.
    #[test]
    fn the_selected_row_is_filled_with_the_skins_own_selectedbg() {
        use rustamp_core::playlist::EntryId;
        use rustamp_skin::{BuiltinDefaultSkin, Skin};

        // A skin that differs from every default, so a hardcoded colour
        // cannot pass by accident.
        let wanted = [0x12u8, 0x34, 0x56];
        let mut source = rustamp_skin::rust_theme::source();
        source.add(
            "pledit.txt",
            b"[Text]\nNormal=#00FF00\nCurrent=#FFFFFF\nNormalBG=#000000\nSelectedBG=#123456\n"
                .to_vec(),
        );
        let skin = Skin::from_source(&mut source, &BuiltinDefaultSkin).expect("the skin loads");
        assert_eq!(skin.pledit().selected_bg, wanted, "the skin was read");

        let mut window = PlaylistWindow::default();
        window.set_entries(vec![EntryId(1), EntryId(2)]);
        window.apply_select(&rustamp_core::command::SelectOp::All);
        let labels = vec![String::new(), String::new()];
        let frame = render_playlist_window(&skin, &window, &labels, true);

        let row = row_rect(0).expect("the first visible row");
        let pixel = |x: i32, y: i32| {
            let index = ((y as u32 * frame.width() + x as u32) * 4) as usize;
            let rgba = &frame.as_rgba()[index..index + 3];
            [rgba[0], rgba[1], rgba[2]]
        };
        // Inside the band: the skin's colour, at both ends of the row.
        assert_eq!(pixel(row.x, row.y), wanted);
        assert_eq!(pixel(LIST_RIGHT - 1, row.y + ROW_PITCH - 1), wanted);
        // Outside it: the side rails are the skin's art, never the fill.
        assert_ne!(
            pixel(LIST_LEFT - 1, row.y),
            wanted,
            "the band must stop at the list area (S06 section 2: x 12-254)"
        );
        assert_ne!(
            pixel(LIST_RIGHT, row.y),
            wanted,
            "the band must not reach the scrollbar"
        );
    }

    #[test]
    fn client_size_is_275x116_at_rest() {
        // S06 scope / S01 §1, verified; double-size leaves it unchanged.
        assert_eq!(PLAYLIST_WINDOW_SIZE, (275, 116));
    }

    #[test]
    fn title_bar_has_no_window_menu_button() {
        // S06 §1: no `#32768` popup after the (6,3) click, so no button.
        assert_eq!(hit_test_title_bar(6, 3), None);
        assert_eq!(hit_test_title_bar(264, 3), Some(PlaylistTitleButton::Close));
        assert_eq!(hit_test_title_bar(254, 3), Some(PlaylistTitleButton::Shade));
        // Outside the 20 px band nothing hits.
        assert_eq!(hit_test_title_bar(264, 20), None);
        assert_eq!(hit_test_title_bar(0, 21), None);
    }

    #[test]
    fn list_rows_follow_the_settled_band_and_pitch() {
        // S06 §2 bbox (12,23)-(254,93); S01 §4.2 13 px pitch, four rows.
        assert_eq!(list_area(), Rect::new(12, 23, 242, 70));
        assert_eq!(row_rect(0), Some(Rect::new(12, 23, 242, 13)));
        assert_eq!(row_rect(3), Some(Rect::new(12, 62, 242, 13)));
        assert_eq!(row_rect(4), None);
    }

    #[test]
    fn list_hit_testing_maps_rows_and_background() {
        assert_eq!(hit_test_list_row(60, 28), Some(0));
        assert_eq!(hit_test_list_row(12, 23), Some(0));
        assert_eq!(hit_test_list_row(253, 74), Some(3));
        // Below the fourth row is background (own reading).
        assert_eq!(hit_test_list_row(60, 75), None);
        assert_eq!(hit_test_list_row(60, 92), None);
        // Outside the band horizontally.
        assert_eq!(hit_test_list_row(11, 28), None);
        assert_eq!(hit_test_list_row(254, 28), None);
        assert!(in_list_area(60, 92));
        assert!(!in_list_area(60, 93));
    }

    #[test]
    fn scroll_handle_pins_the_three_settled_points() {
        // S01 §4.2: y = 20 top, 40 middle, 60 bottom (60-entry list).
        assert_eq!(scroll_handle_y(0, 60), 20);
        assert_eq!(scroll_handle_y(28, 60), 40);
        assert_eq!(scroll_handle_y(56, 60), 60);
        // A list that fits the page stays at the top.
        assert_eq!(scroll_handle_y(0, 4), 20);
        assert_eq!(scroll_handle_y(0, 0), 20);
        // Clamped past the end.
        assert_eq!(scroll_handle_y(1000, 60), 60);
    }

    #[test]
    fn scroll_arrows_split_at_the_settled_points() {
        // S06 §3: (263,81) up, (263,87) down; the split is an own choice.
        assert_eq!(hit_test_scroll(263, 81), Some(ScrollDirection::Up));
        assert_eq!(hit_test_scroll(263, 87), Some(ScrollDirection::Down));
        assert_eq!(hit_test_scroll(260, 78), Some(ScrollDirection::Up));
        assert_eq!(hit_test_scroll(260, 87), Some(ScrollDirection::Down));
        assert_eq!(hit_test_scroll(259, 81), None);
        assert_eq!(hit_test_scroll(260, 88), None);
    }

    #[test]
    fn button_rects_stay_inside_the_button_band() {
        // S01 §4.1: drawn at window y 92-109. Behavioural hit-testing uses
        // `hit_test_button`, never these provisional rects.
        for button in PlaylistButton::ALL {
            let rect = button_rect(*button);
            assert!(rect.y >= 92 && rect.bottom() <= 110, "{button:?}");
        }
        assert_eq!(button_rect(PlaylistButton::ListOpts).x, 228);
    }

    #[test]
    fn settled_bottom_bar_points_hit_their_buttons() {
        // S01 §4.2 posted positives at y = 92 route through the model ranges.
        use crate::playlist::{PlaylistButton, hit_test_button};
        assert_eq!(hit_test_button(24, 92), Some(PlaylistButton::Add));
        assert_eq!(hit_test_button(46, 92), Some(PlaylistButton::Rem));
        assert_eq!(hit_test_button(74, 92), Some(PlaylistButton::Sel));
        assert_eq!(hit_test_button(112, 92), Some(PlaylistButton::Misc));
        assert_eq!(hit_test_button(242, 92), Some(PlaylistButton::ListOpts));
    }

    #[test]
    fn row_index_maps_to_the_model_view() {
        // The row-to-model mapping: visible row i is `visible_ids()[i]`.
        use crate::playlist::PlaylistWindow;
        use rustamp_core::playlist::EntryId;

        let mut model = PlaylistWindow::new();
        model.set_entries((0..60u64).map(EntryId).collect());
        model.scroll_to(56);
        let visible = model.visible_ids();
        assert_eq!(visible.len(), VISIBLE_ROWS);
        for (row, id) in visible.iter().enumerate() {
            assert_eq!(hit_test_list_row(60, 23 + row as i32 * 13), Some(row));
            assert_eq!(*id, EntryId(56 + row as u64));
        }
    }

    #[test]
    fn handle_drag_mapping_round_trips_every_position() {
        // S06 §3 settles only that a dragged handle "moves the view"; the
        // linear pixel-to-row mapping is an own choice, pinned here by the
        // round-trip in the drag direction: every handle position maps to a
        // scroll offset and back to the same position, so a dragged handle
        // renders under the cursor. The reverse — every offset through its
        // position and back — cannot be exact, because the 57 offsets of a
        // 60-entry list share 41 integer pixel rows; those offsets land
        // within one row of themselves, pinned below.
        for y in SCROLL_TOP_Y..=SCROLL_TOP_Y + SCROLL_TRAVEL_PX {
            let offset = scroll_offset_for_handle_y(y, 60);
            assert_eq!(scroll_handle_y(offset, 60), y, "y {y}");
        }
        // Offsets that share a pixel row land within one row of themselves.
        for offset in 0..=56usize {
            let back = scroll_offset_for_handle_y(scroll_handle_y(offset, 60), 60);
            assert!(back.abs_diff(offset) <= 1, "offset {offset}");
        }
        // Ends and clamps.
        assert_eq!(scroll_offset_for_handle_y(20, 60), 0);
        assert_eq!(scroll_offset_for_handle_y(60, 60), 56);
        assert_eq!(scroll_offset_for_handle_y(0, 60), 0);
        assert_eq!(scroll_offset_for_handle_y(100, 60), 56);
        // The settled middle point (S01 §4.2: y = 40 for a 60-entry list).
        assert_eq!(scroll_offset_for_handle_y(40, 60), 28);
        // A list that fits one page never moves.
        assert_eq!(scroll_offset_for_handle_y(60, 3), 0);
    }

    #[test]
    fn scrollbar_hit_testing_separates_handle_track_and_arrows() {
        // A 60-entry list at the bottom page: handle at y = 60..77.
        assert_eq!(scroll_handle_rect(56, 60), Rect::new(260, 60, 7, 17));
        assert_eq!(
            hit_test_scrollbar(263, 65, 56, 60),
            Some(ScrollbarHit::Handle)
        );
        // Track above the handle pages up, below it pages down (own choice).
        assert_eq!(
            hit_test_scrollbar(263, 30, 56, 60),
            Some(ScrollbarHit::TrackAbove)
        );
        assert_eq!(
            hit_test_scrollbar(263, 77, 0, 60),
            Some(ScrollbarHit::TrackBelow)
        );
        // The arrow pair keeps its own path and is not a track hit.
        assert_eq!(hit_test_scrollbar(263, 81, 56, 60), None);
        assert_eq!(hit_test_scrollbar(263, 87, 56, 60), None);
        // Outside the column nothing hits.
        assert_eq!(hit_test_scrollbar(259, 30, 56, 60), None);
        assert_eq!(hit_test_scrollbar(268, 30, 56, 60), None);
        assert_eq!(hit_test_scrollbar(263, 19, 56, 60), None);
    }
}
