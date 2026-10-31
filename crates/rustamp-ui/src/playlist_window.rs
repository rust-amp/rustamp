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
//! * Scrollbar handle `PLEDIT.BMP` (52,53,8,18) at (260,y) with y = 20 at the
//!   scroll top, 40 in the middle and 60 at the bottom (the three y values:
//!   S01 §4.2, round 2, verified). The sprite's own cell is 8 x 18: the
//!   base291 `playlist-active` capture shows the handle's right border column
//!   (x = 267) and its bottom border row (y = 37), and that 8 x 18 rectangle
//!   at (260,20) matches the capture pixel for pixel. S01 §4.2's (52,53,7,17)
//!   is the subset it verified by mean difference, which a smaller rect is;
//!   the drawn cell is the larger one. The arrow pair `PLEDIT.BMP`
//!   (261,72,8,10) is fixed at (260,78) (S01 §4.2, verified). Arrow click
//!   points (263,81) up / (263,87) down and the one-page step (S06 §3,
//!   settled).
//! * Windowshade band: 275 x 14 (S01 §1, verified), assembled from three
//!   cells of `PLEDIT.BMP` (S01 §4 calls the shade rects provisional): the
//!   25 x 14 left corner at sheet (72,42), the borderless 25 x 14 body tile
//!   at sheet (72,57) tiled from x = 25, and the 29 x 14 right corner at
//!   sheet (120,y) drawn at x = 246. The base291 `playlist-shade` capture
//!   pins all three: the window's four-pixel left border is drawn at x = 0 in
//!   both focus states, the body has **no** seam at the 25 px steps (so the
//!   border-carrying cell must not be tiled), and the right corner comes from
//!   row 42 while focused and row 57 while unfocused (lit vs dim window
//!   buttons). Tiling the corner cell instead, as an earlier revision did,
//!   drew a four-pixel seam every 25 px.
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
//! * Entry text uses the installed family from `pledit.txt` (S17 §5), at the
//!   size and with the style the same line asks for. Our rasterization
//!   size/hinting are not a claim of GDI-exact reference pixels. If no system
//!   font is available, the bitmap font is an explicit fallback.
//! * The up/down split of the arrow pair (at y = 83) is an own choice; only
//!   the two click points are settled.
//! * The handle-y interpolation between the three settled points is an own
//!   linear choice.
//! * The drag insertion line is an own-choice one-pixel rule in the skin's
//!   `Normal` colour. S06 §8 Q6 establishes neither its shape nor its colour.
//! * MISC popup contents and message exposure stay open (S06 §8).

use rustamp_gfx::{BitmapFont, GlyphMap, Image, Rect};
use rustamp_skin::{Sheet, Skin, webamp};

use crate::playlist::{PlaylistButton, PlaylistWindow, ScrollDirection, VISIBLE_ROWS};

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

// ---------------------------------------------------------------------------
// Windowshade band (S01 §1 for the size, S01 §4 for the sheet cells)
// ---------------------------------------------------------------------------

/// Height of the shaded playlist window: one 14 px title row (S01 §1).
pub const SHADE_TITLE_HEIGHT: i32 = 14;

/// The shade band's left corner: `PLEDIT.BMP` (72,42,25,14).
///
/// This cell carries the window's own four-pixel left border (a dark, a
/// light and two mid columns at x = 0-3), so it is drawn once at x = 0 in
/// **both** focus states - the cell below it, (72,57), is borderless.
pub const SHADE_CORNER_SRC: Rect = Rect::new(72, 42, 25, 14);

/// The shade band's repeating body tile: `PLEDIT.BMP` (72,57,25,14).
///
/// Borderless on purpose: the base291 `playlist-shade` capture has no seam at
/// the 25 px steps, so the border-carrying corner cell above must not be
/// tiled. Its 25 px width is also the tile pitch.
pub const SHADE_BODY_SRC: Rect = Rect::new(72, 57, 25, 14);

/// x of the shade band's right corner: 275 - its 29 px width.
pub const SHADE_RIGHT_X: i32 = 246;

/// The right corner of a **focused** shade band: `PLEDIT.BMP` (120,42,29,14),
/// the cell with the lit window buttons.
pub const SHADE_RIGHT_FOCUSED_SRC: Rect = Rect::new(120, 42, 29, 14);

/// The right corner of an **unfocused** shade band: `PLEDIT.BMP`
/// (120,57,29,14), the same cell with the dim window buttons.
pub const SHADE_RIGHT_SRC: Rect = Rect::new(120, 57, 29, 14);

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
///
/// This is the pitch of the **default** font size: the pitch follows the
/// font (see [`row_pitch`]), so a skin that names a size or a user who
/// changes the size draws taller or shorter rows.
pub const ROW_PITCH: i32 = 13;

/// The height of the list page in pixels: the settled four rows of 13 px
/// (S01 §4.2, S06 §3).
///
/// The page is the room the rows have; the number of rows in it follows the
/// row pitch (see [`visible_rows`]).
pub const LIST_PAGE_HEIGHT: i32 = ROW_PITCH * VISIBLE_ROWS as i32;

/// The row height (the row pitch) for a font size, in pixels.
///
/// The row is as tall as the text it carries (own choice): at the default
/// 13 px font this is the settled 13 px pitch of S01 §4.2.
pub fn row_pitch(size: u32) -> i32 {
    i32::try_from(size).unwrap_or(i32::MAX).max(1)
}

/// How many whole rows fit in the list page at a row pitch.
///
/// The page is the settled four rows of 13 px (S01 §4.2, S06 §3). The pitch
/// follows the font, so the number of rows in the page is **recomputed**: a
/// bigger font shows fewer rows, a smaller one more. The page never reaches
/// past the list area (S06 §2), so a row is never clipped.
pub fn visible_rows(pitch: i32) -> usize {
    let pitch = pitch.max(1);
    (LIST_PAGE_HEIGHT / pitch).max(1) as usize
}

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
/// Returns `None` past the page: the rows below it are list background (own
/// reading; the spec fixes the band and the pitch only).
pub fn row_rect(row: usize, pitch: i32) -> Option<Rect> {
    if row >= visible_rows(pitch) {
        return None;
    }
    // The rows' pitch starts one pixel above the list band, and the band cuts
    // the first row: Winamp 2.95 selects y 23..=34, 35..=47, 48..=60 (measured
    // 2026-10-31; S06 §2's band starts at 23).
    let nominal = row_top(row, pitch);
    let top = nominal.max(LIST_TOP);
    Some(Rect::new(
        LIST_LEFT,
        top,
        (LIST_RIGHT - LIST_LEFT) as u32,
        (nominal + pitch - top) as u32,
    ))
}

/// Where row `row`'s text starts, above the band's cut for the first row.
fn row_top(row: usize, pitch: i32) -> i32 {
    LIST_TOP + ROW_DRAW_OFFSET + row as i32 * pitch
}

/// Whether `(x, y)` is inside the settled list band (S06 §2).
pub fn in_list_area(x: i32, y: i32) -> bool {
    in_rect(x, y, list_area())
}

/// Which visible row (if any) a click at `(x, y)` lands on.
///
/// The rows are the bands of `pitch` pixels from [`row_rect`]; the band below
/// the page is background and hits nothing. S06 §2 settles the first-row
/// click only; selecting the actual row elsewhere is an own choice.
pub fn hit_test_list_row(x: i32, y: i32, pitch: i32) -> Option<usize> {
    if !(LIST_LEFT..LIST_RIGHT).contains(&x) || y < LIST_TOP {
        return None;
    }
    let pitch = pitch.max(1);
    let row = ((y - LIST_TOP) / pitch) as usize;
    if row < visible_rows(pitch) {
        Some(row)
    } else {
        None
    }
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

/// Source rect of the handle in `PLEDIT.BMP`: 8 x 18 at (52,53).
///
/// S01 §4.2 records (52,53,7,17) - the rectangle whose mean difference it
/// verified, which a smaller sub-rect is. The base291 `playlist-active`
/// capture shows the sprite's own right border column (window x = 267) and
/// bottom border row (window y = 37), and the full 8 x 18 cell at (260,20)
/// matches that capture pixel for pixel (measured 2026-10-21).
pub const SCROLL_HANDLE_SRC: Rect = Rect::new(52, 53, 8, 18);

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
/// renders under the cursor). A list that fits the page shows the handle at
/// the top. The page holds as many rows as the row pitch allows (see
/// [`visible_rows`]).
pub fn scroll_handle_y(first_visible: usize, len: usize, pitch: i32) -> i32 {
    let rows = visible_rows(pitch);
    if len <= rows {
        return SCROLL_TOP_Y;
    }
    let max_first = len.saturating_sub(rows);
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
/// to hold the 8 px handle and the 8 px arrows.
pub const SCROLL_TRACK_RECT: Rect = Rect::new(260, 20, 8, 58);

/// The handle rectangle for a scroll offset of `first_visible` in a list of
/// `len` (S01 §4.2: `PLEDIT.BMP` (52,53,8,18) drawn at (260,y)).
pub fn scroll_handle_rect(first_visible: usize, len: usize, pitch: i32) -> Rect {
    Rect::new(
        SCROLL_HANDLE_X,
        scroll_handle_y(first_visible, len, pitch),
        SCROLL_HANDLE_SRC.width,
        SCROLL_HANDLE_SRC.height,
    )
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
pub fn scroll_offset_for_handle_y(y: i32, len: usize, pitch: i32) -> usize {
    let max_first = len.saturating_sub(visible_rows(pitch));
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
    pitch: i32,
) -> Option<ScrollbarHit> {
    if in_rect(x, y, SCROLL_ARROWS_RECT) {
        return None;
    }
    if !in_rect(x, y, SCROLL_TRACK_RECT) {
        return None;
    }
    let handle = scroll_handle_rect(first_visible, len, pitch);
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
    if window.windowshade {
        let width = window.size().0;
        let right_x = width as i32 - 29;
        let mut canvas = Image::filled(width, SHADE_TITLE_HEIGHT as u32, [0, 0, 0, 255]);
        let pledit = load_sheet(skin, Sheet::Pledit);
        // Playlist ignores 2x. Corner once (it carries the window's left
        // border), then the borderless body tile, then the right corner taken
        // from the focused or the unfocused row (S01 §4, measured).
        blit_region(&mut canvas, pledit.as_ref(), SHADE_CORNER_SRC, 0, 0);
        for x in (SHADE_CORNER_SRC.width as i32..right_x).step_by(SHADE_BODY_SRC.width as usize) {
            blit_region(&mut canvas, pledit.as_ref(), SHADE_BODY_SRC, x, 0);
        }
        blit_region(
            &mut canvas,
            pledit.as_ref(),
            if focused {
                SHADE_RIGHT_FOCUSED_SRC
            } else {
                SHADE_RIGHT_SRC
            },
            right_x,
            0,
        );
        draw_title_buttons(&mut canvas, pledit.as_ref(), skin, window);
        return canvas;
    }
    let (width, height) = window.size();
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
    draw_status_fields(&mut canvas, skin, window);
    draw_drag_insertion(&mut canvas, skin, window);
    draw_scrollbar(&mut canvas, pledit.as_ref(), window);
    draw_title_buttons(&mut canvas, pledit.as_ref(), skin, window);
    canvas
}

fn draw_title_buttons(
    canvas: &mut Image,
    pledit: Option<&Image>,
    skin: &Skin,
    window: &PlaylistWindow,
) {
    if let Some(button) = window.pressed_title_button {
        let (name, rect) = match button {
            PlaylistTitleButton::Close => ("PLAYLIST_CLOSE_SELECTED", CLOSE_BUTTON_RECT),
            PlaylistTitleButton::Shade => (
                if window.windowshade {
                    "PLAYLIST_EXPAND_SELECTED"
                } else {
                    "PLAYLIST_COLLAPSE_SELECTED"
                },
                SHADE_BUTTON_RECT,
            ),
        };
        blit_sprite(
            canvas,
            pledit,
            skin,
            name,
            rect.x + canvas.width() as i32 - 275,
            rect.y,
        );
    }
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
    let width = canvas.width() as i32;
    let title_x = (width - TITLE_STRIP_WIDTH as i32) / 2;
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
    // S01 §4.1: fresh partial tiles at each band's left edge, then full
    // tiles anchored to the fixed piece on its right (not to the window).
    if let Some(def) = skin.sprite(tile) {
        for (left, right) in [(25, title_x), (title_x + 100, width - 25)] {
            let partial = (right - left) % TITLE_TILE_WIDTH as i32;
            let mut dx = left;
            while dx < right {
                let width = if dx == left && partial != 0 {
                    partial
                } else {
                    TITLE_TILE_WIDTH as i32
                };
                blit_region(
                    canvas,
                    pledit,
                    Rect::new(def.x as i32, def.y as i32, width as u32, 20),
                    dx,
                    0,
                );
                dx += width;
            }
        }
    }
    blit_sprite(canvas, pledit, skin, strip, title_x, 0);
    blit_sprite(canvas, pledit, skin, right, width - 25, 0);
    // Resting window buttons are already in the title body; only a held
    // button overlays the separate selected cell (see draw_title_buttons).
}

fn draw_side_rails(canvas: &mut Image, pledit: Option<&Image>, skin: &Skin) {
    // Left rail tiles from y = 20 (S01 §4.1, verified); the right rail stays
    // provisional there but is drawn the same way. Tiles stop at the bottom
    // bar (y = 78).
    let mut dy = LIST_TOP - 3;
    let bottom = canvas.height() as i32 - 38;
    let right_x = canvas.width() as i32 - 20;
    while dy < bottom {
        let tile_height = (bottom - dy).min(29) as u32;
        for (name, x) in [("PLAYLIST_LEFT_TILE", 0), ("PLAYLIST_RIGHT_TILE", right_x)] {
            if let Some(def) = skin.sprite(name) {
                blit_region(
                    canvas,
                    pledit,
                    Rect::new(def.x as i32, def.y as i32, def.width, tile_height),
                    x,
                    dy,
                );
            }
        }
        dy += 29;
    }
}

fn draw_bottom_bar(canvas: &mut Image, pledit: Option<&Image>, skin: &Skin) {
    let width = canvas.width() as i32;
    let bottom = canvas.height() as i32 - 38;
    blit_sprite(
        canvas,
        pledit,
        skin,
        "PLAYLIST_BOTTOM_LEFT_CORNER",
        0,
        bottom,
    );
    if let Some(def) = skin.sprite("PLAYLIST_BOTTOM_TILE") {
        let mut x = 125;
        while x < width - 150 {
            let tile_width = (width - 150 - x).min(25) as u32;
            blit_region(
                canvas,
                pledit,
                Rect::new(def.x as i32, def.y as i32, tile_width, 38),
                x,
                bottom,
            );
            x += tile_width as i32;
        }
    }
    blit_sprite(
        canvas,
        pledit,
        skin,
        "PLAYLIST_BOTTOM_RIGHT_CORNER",
        width - 150,
        bottom,
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
            dst.x
                + if *button == PlaylistButton::ListOpts {
                    width - 275
                } else {
                    0
                },
            dst.y + bottom - 78,
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
        if pixel[..3] == background[..3] || pixel[3] < 128 {
            *pixel = [0; 4];
        } else {
            // The bitmap fallback obeys the same 1-bit contract as the system
            // font: fractional source alpha must not create off-palette ink.
            *pixel = [255; 4];
        }
    }
    Some(BitmapFont::new(image, glyphs, None, 0))
}

/// The two text fields of the bottom bar, in the skin's `text.bmp`. Their places
/// are measured against Winamp 2.95 (2026-10-31), from the bottom right corner.
fn draw_status_fields(canvas: &mut Image, skin: &Skin, window: &PlaylistWindow) {
    let Some(raster) = skin.sheet(Sheet::Text) else {
        return;
    };
    let Ok(image) = Image::from_rgba(raster.width(), raster.height(), raster.as_rgba().to_vec())
    else {
        return;
    };
    let mut glyphs = GlyphMap::new(webamp::TEXT_CELL.0, webamp::TEXT_CELL.1);
    for (ch, col, row) in webamp::TEXT_GLYPHS {
        glyphs.insert(*ch, *col, *row);
    }
    let font = BitmapFont::new(image, glyphs, None, 0);
    let (width, height) = (canvas.width() as i32, canvas.height() as i32);
    font.render_text(canvas, width - 143, height - 28, &window.info_text);
    if let Some(digits) = window.time_digits {
        for (digit, x) in digits.iter().zip([77, 72, 64, 59]) {
            font.render_text(canvas, width - x, height - 15, &digit.to_string());
        }
    }
}

/// Where the rows are drawn relative to the list band: Winamp 2.95 draws them
/// one pixel above it (measured side by side, 2026-10-31).
const ROW_DRAW_OFFSET: i32 = -1;

/// The room kept between a row's text and its length (own choice).
const LENGTH_GAP: u32 = 4;

/// The room right of a row's length (measured against Winamp 2.95, 2026-10-31).
const LENGTH_RIGHT: u32 = 2;

/// Paints the visible rows: the selection band first, then the labels.
///
/// The selected row's colour is the **skin's** `SelectedBG` from
/// `pledit.txt` ([`rustamp_skin::PlaylistStyle::selected_bg`]), never a
/// colour of ours - the skin decides what a selected row looks like. The
/// band is one row of the settled list area ([`row_rect`], S06 §2: x 12-254,
/// the row pitch of the current font), so it cannot reach the side rails or
/// the scrollbar.
///
/// `Normal` and `Current` tint the glyph ink independently of selection;
/// `Current` applies only to the app-supplied current entry, not the clicked
/// or selected one (S17 §5). The **font** is the skin's too: the `Font=`
/// line of `pledit.txt` names the family and the style (S17 §5), and the
/// **size** is the one the user can change at runtime (S03 §2 `40335` /
/// `40336`), so the model owns it.
fn draw_rows(canvas: &mut Image, skin: &Skin, window: &PlaylistWindow, labels: &[String]) {
    let style = skin.pledit();
    let selected = [
        style.selected_bg[0],
        style.selected_bg[1],
        style.selected_bg[2],
        255,
    ];
    let font = text_font(skin);
    let pitch = window.row_pitch();
    let request = crate::playlist_font::SystemFont::from_playlist_font(style.font.as_ref())
        .with_size(window.font_size().size());
    let visible = window.visible_ids();
    let selected_ids = window.selected_ids();
    for (i, id) in visible.iter().enumerate() {
        let nominal = row_top(i, pitch);
        let band_top = nominal.max(LIST_TOP);
        let clipped = (band_top - nominal) as u32;
        let rect = Rect::new(LIST_LEFT, nominal, window.size().0 - 33, pitch as u32);
        let band = Rect::new(rect.x, band_top, rect.width, rect.height - clipped);
        if selected_ids.contains(id) {
            canvas.fill_rect(band, selected);
        }
        let label = labels.get(i).map(String::as_str).unwrap_or("");
        if label.is_empty() {
            continue;
        }
        // What follows a tab is the entry's length, at the row's right edge.
        let (text, length) = match label.split_once('\t') {
            Some((text, length)) => (text, Some(length)),
            None => (label, None),
        };
        let length_width = length
            .and_then(|length| crate::playlist_font::text_width_as_part_of(request, label, length))
            .unwrap_or(0);
        let text_width = rect
            .width
            .saturating_sub(length_width + LENGTH_GAP * u32::from(length_width > 0));
        let colour = if window.current() == Some(*id) {
            style.current
        } else {
            style.normal
        };
        // The name and the length are one row and one font, on the row's own
        // background (the skin's, or its selection colour).
        let under = crate::render::under_text(canvas, band.x, band.y);
        if let Some(scratch) = crate::playlist_font::render_as_part_of(
            request,
            label,
            text,
            text_width,
            rect.height,
            colour,
            under,
        ) {
            let visible = Rect::new(0, clipped as i32, scratch.width(), rect.height - clipped);
            canvas.blit_alpha(&scratch, visible, rect.x, band.y);
            if let Some(length) = length
                && let Some(scratch) = crate::playlist_font::render_as_part_of(
                    request,
                    label,
                    length,
                    length_width,
                    rect.height,
                    colour,
                    under,
                )
            {
                let x = rect.x + rect.width as i32 - (length_width + LENGTH_RIGHT) as i32;
                let visible = Rect::new(0, clipped as i32, scratch.width(), rect.height - clipped);
                canvas.blit_alpha(&scratch, visible, x, band.y);
            }
        } else if let Some(font) = font.as_ref() {
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
        window.size().0 as i32 - 15,
        window.scrollbar_handle_y(),
    );
    blit_region(
        canvas,
        pledit,
        SCROLL_ARROWS_SRC,
        window.size().0 as i32 - 15,
        window.size().1 as i32 - 38,
    );
}

fn draw_drag_insertion(canvas: &mut Image, skin: &Skin, window: &PlaylistWindow) {
    let Some(gap) = window.drag_insertion() else {
        return;
    };
    let Some(row) = gap.checked_sub(window.first_visible()) else {
        return;
    };
    let pitch = window.row_pitch();
    if row > window.visible_rows() {
        return;
    }
    // Own-choice geometry, not reference pixels: integer, opaque, skin-coloured
    // and clipped to the list. No extra asset or antialiased overlay is used.
    let y = (LIST_TOP + row as i32 * pitch).min(window.size().1 as i32 - 24);
    let colour = skin.pledit().normal;
    canvas.fill_rect(
        Rect::new(LIST_LEFT, y, window.size().0 - 33, 1),
        [colour[0], colour[1], colour[2], 255],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playlist::PLAYLIST_WINDOW_SIZE;

    #[test]
    fn resized_playlist_draws_the_skin_and_uses_the_larger_viewport() {
        use rustamp_core::playlist::EntryId;
        use rustamp_skin::BuiltinDefaultSkin;

        let skin = Skin::from_source(&mut BuiltinDefaultSkin::source(), &BuiltinDefaultSkin)
            .expect("skin");
        let mut window = PlaylistWindow::new();
        window.set_entries((0..30).map(EntryId).collect());
        window.set_size(550, 348);
        let frame = render_playlist_window(&skin, &window, &[], true);
        assert_eq!((frame.width(), frame.height()), (550, 348));
        assert_eq!(
            frame.pixel(300, 100),
            Some([
                skin.pledit().normal_bg[0],
                skin.pledit().normal_bg[1],
                skin.pledit().normal_bg[2],
                255
            ])
        );
        assert_ne!(frame.pixel(549, 347), Some([0, 0, 0, 255]));
        assert!(window.visible_rows() > 4);
        assert_eq!(window.row_at(400, 23 + 13 * 8), Some(8));
        window.scroll_to(usize::MAX);
        assert_eq!(window.scrollbar_handle_y(), 20 + 348 - 76);
        assert!(window.press_scrollbar(535, 310));
    }

    #[test]
    fn drag_indicator_is_one_opaque_skin_coloured_line_and_disappears_on_cancel() {
        use rustamp_core::playlist::EntryId;
        use rustamp_skin::{BuiltinDefaultSkin, BuiltinLookalikeSkin};

        for mut source in [BuiltinDefaultSkin::source(), BuiltinLookalikeSkin::source()] {
            source.add(
                "pledit.txt",
                b"[Text]\nNormal=#13579B\nNormalBG=#192A3B\nSelectedBG=#4C5D6E\n".to_vec(),
            );
            let skin = Skin::from_source(&mut source, &BuiltinDefaultSkin).unwrap();
            let mut window = PlaylistWindow::new();
            window.set_entries((0..4).map(EntryId).collect());
            window.mouse_down(60, 28, crate::shortcuts::Modifiers::NONE);
            let before = render_playlist_window(&skin, &window, &[], true);
            for (pointer_y, line_y) in [
                (23, LIST_TOP),
                (50, LIST_TOP + 2 * ROW_PITCH),
                (85, LIST_TOP + 4 * ROW_PITCH),
            ] {
                window.mouse_move(64, pointer_y);
                let frame = render_playlist_window(&skin, &window, &[], true);
                for y in 0..frame.height() as i32 {
                    for x in 0..frame.width() as i32 {
                        if y == line_y && (LIST_LEFT..LIST_RIGHT).contains(&x) {
                            assert_eq!(frame.pixel(x, y), Some([0x13, 0x57, 0x9b, 255]));
                        } else {
                            assert_eq!(frame.pixel(x, y), before.pixel(x, y), "{x},{y}");
                        }
                    }
                }
            }
            window.cancel_drag();
            assert_eq!(
                render_playlist_window(&skin, &window, &[], true).as_rgba(),
                before.as_rgba()
            );
        }
    }

    #[test]
    fn playlist_mask_removes_nonblack_background_and_quantizes_ink_alpha() {
        let background = [0x51, 0x62, 0x73, 255];
        let mut sheet = Image::filled(155, 18, background);
        // Black ink must not be mistaken for background; transparent ink
        // must not become visible merely because its RGB differs from the key.
        sheet.set_pixel(0, 0, [0, 0, 0, 255]);
        sheet.set_pixel(1, 0, [0, 0, 0, 128]);
        sheet.set_pixel(2, 0, [9, 8, 7, 0]);
        sheet.set_pixel(3, 0, [9, 8, 7, 127]);
        let font = playlist_font_mask(sheet).expect("the space cell exists");
        let mut scratch = Image::new(10, 6);
        font.render_text(&mut scratch, 0, 0, "a ");
        assert_eq!(scratch.pixel(0, 0), Some([255; 4]));
        assert_eq!(scratch.pixel(1, 0), Some([255; 4]));
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
                        let row = row_rect(i, ROW_PITCH).unwrap();
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
                        let mut background_count = 0;
                        for y in row.y..row.bottom() {
                            for x in row.x..row.right() {
                                let pixel = frame.pixel(x, y).unwrap();
                                ink_count += usize::from(
                                    (0..3)
                                        .map(|c| (i32::from(pixel[c]) - i32::from(ink[c])).pow(2))
                                        .sum::<i32>()
                                        < (0..3)
                                            .map(|c| {
                                                (i32::from(pixel[c]) - i32::from(bg[c])).pow(2)
                                            })
                                            .sum::<i32>(),
                                );
                                background_count += usize::from(pixel == bg);
                            }
                        }
                        assert!(ink_count > 0, "a nonempty label must have visible ink");
                        assert!(background_count > 0, "glyphs must not erase the band");
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

        let row = row_rect(0, ROW_PITCH).expect("the first visible row");
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

    /// The classic invariant at every size: a row holds the skin's ink and
    /// its background, never a blended shade (no antialiasing).
    #[test]
    fn the_rows_show_ink_on_their_background_at_every_font_size() {
        use rustamp_core::playlist::EntryId;
        use rustamp_skin::{BuiltinDefaultSkin, BuiltinLookalikeSkin};

        for mut source in [BuiltinDefaultSkin::source(), BuiltinLookalikeSkin::source()] {
            source.add(
                "pledit.txt",
                b"[Text]\nNormal=#13579B\nCurrent=#ECA864\nNormalBG=#192A3B\nSelectedBG=#4C5D6E\n"
                    .to_vec(),
            );
            let skin = Skin::from_source(&mut source, &BuiltinDefaultSkin).unwrap();
            let mut window = PlaylistWindow::new();
            window.set_entries((0..8).map(EntryId).collect());
            let labels = vec!["Mixed Case AV ffi 0123456789".to_string(); 8];
            for size in [8u32, 10, 13, 16, 20, 26, 32] {
                window.set_font_size_override(Some(size));
                let frame = render_playlist_window(&skin, &window, &labels, true);
                let style = skin.pledit();
                let ink = [style.normal[0], style.normal[1], style.normal[2], 255];
                let bg = [
                    style.normal_bg[0],
                    style.normal_bg[1],
                    style.normal_bg[2],
                    255,
                ];
                let pitch = window.row_pitch();
                for row in 0..window.visible_rows() {
                    let rect = row_rect(row, pitch).expect("the visible row");
                    let mut ink_count = 0;
                    for y in rect.y..rect.bottom() {
                        for x in rect.x..rect.right() {
                            let pixel = frame.pixel(x, y).expect("a pixel");
                            ink_count += usize::from(
                                (0..3)
                                    .map(|c| (i32::from(pixel[c]) - i32::from(ink[c])).pow(2))
                                    .sum::<i32>()
                                    < (0..3)
                                        .map(|c| (i32::from(pixel[c]) - i32::from(bg[c])).pow(2))
                                        .sum::<i32>(),
                            );
                        }
                    }
                    assert!(ink_count > 0, "size {size}, row {row}: no ink");
                }
            }
        }
    }

    /// The runtime size replaces the **size** only: the family and the style
    /// stay the skin's (S17 §5).
    #[test]
    fn the_runtime_size_keeps_the_skins_family_and_style() {
        use rustamp_core::playlist::EntryId;
        use rustamp_skin::{BuiltinDefaultSkin, Skin};

        fn skin_with_font(font: &str) -> Skin {
            let mut source = BuiltinDefaultSkin::source();
            source.add(
                "pledit.txt",
                format!("[Text]\nNormal=#00FF00\nCurrent=#FFFFFF\nNormalBG=#000000\nSelectedBG=#0000C6\nFont={font}\n").into_bytes(),
            );
            Skin::from_source(&mut source, &BuiltinDefaultSkin).expect("the skin loads")
        }

        let mut window = PlaylistWindow::new();
        window.set_entries(vec![EntryId(1)]);
        let labels = vec!["Mixed Case AV ffi".to_string()];
        // The user picks a size; the family and the style stay the skin's.
        window.set_font_size_override(Some(18));
        let courier =
            render_playlist_window(&skin_with_font("Courier New,13"), &window, &labels, true);
        let arial = render_playlist_window(&skin_with_font("Arial,13"), &window, &labels, true);
        let bold = render_playlist_window(
            &skin_with_font("Courier New,13,bold"),
            &window,
            &labels,
            true,
        );
        assert_ne!(courier, arial, "the family is still the skin's");
        assert_ne!(courier, bold, "the style is still the skin's");
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
        // The band cuts the first row's top pixel (measured 2026-10-31).
        assert_eq!(row_rect(0, ROW_PITCH), Some(Rect::new(12, 23, 242, 12)));
        assert_eq!(row_rect(1, ROW_PITCH), Some(Rect::new(12, 35, 242, 13)));
        assert_eq!(row_rect(3, ROW_PITCH), Some(Rect::new(12, 61, 242, 13)));
        assert_eq!(row_rect(4, ROW_PITCH), None);
    }

    /// The row height follows the font, so the page holds more rows with a
    /// smaller font and fewer with a bigger one - and never clips a row.
    #[test]
    fn the_row_height_follows_the_font_and_the_page_holds_whole_rows() {
        assert_eq!(row_pitch(13), 13);
        assert_eq!(row_pitch(8), 8);
        assert_eq!(row_pitch(32), 32);
        // The settled page is four rows of 13 px (S01 §4.2).
        assert_eq!(visible_rows(ROW_PITCH), 4);
        assert_eq!(LIST_PAGE_HEIGHT, 52);
        // A smaller font fits more rows, a bigger one fewer.
        assert_eq!(visible_rows(10), 5);
        assert_eq!(visible_rows(17), 3);
        assert_eq!(visible_rows(26), 2);
        assert_eq!(visible_rows(32), 1);
        // The rows never leave the list area (S06 §2): the page is 52 px and
        // the band is 70 px tall.
        for size in 8..=32 {
            let pitch = row_pitch(size);
            let rows = visible_rows(pitch);
            assert!(rows * pitch as usize <= LIST_PAGE_HEIGHT as usize, "{size}");
            assert!(
                LIST_TOP + (rows as i32) * pitch <= LIST_BOTTOM,
                "the last row must stay inside the list area ({size})"
            );
            assert!(row_rect(rows, pitch).is_none(), "{size}");
        }
    }

    #[test]
    fn list_hit_testing_maps_rows_and_background() {
        assert_eq!(hit_test_list_row(60, 28, ROW_PITCH), Some(0));
        assert_eq!(hit_test_list_row(12, 23, ROW_PITCH), Some(0));
        assert_eq!(hit_test_list_row(253, 74, ROW_PITCH), Some(3));
        // Below the fourth row is background (own reading).
        assert_eq!(hit_test_list_row(60, 75, ROW_PITCH), None);
        assert_eq!(hit_test_list_row(60, 92, ROW_PITCH), None);
        // Outside the band horizontally.
        assert_eq!(hit_test_list_row(11, 28, ROW_PITCH), None);
        assert_eq!(hit_test_list_row(254, 28, ROW_PITCH), None);
        assert!(in_list_area(60, 92));
        assert!(!in_list_area(60, 93));
    }

    #[test]
    fn scroll_handle_pins_the_three_settled_points() {
        // S01 §4.2: y = 20 top, 40 middle, 60 bottom (60-entry list).
        assert_eq!(scroll_handle_y(0, 60, ROW_PITCH), 20);
        assert_eq!(scroll_handle_y(28, 60, ROW_PITCH), 40);
        assert_eq!(scroll_handle_y(56, 60, ROW_PITCH), 60);
        // A list that fits the page stays at the top.
        assert_eq!(scroll_handle_y(0, 4, ROW_PITCH), 20);
        assert_eq!(scroll_handle_y(0, 0, ROW_PITCH), 20);
        // Clamped past the end.
        assert_eq!(scroll_handle_y(1000, 60, ROW_PITCH), 60);
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
            assert_eq!(
                hit_test_list_row(60, 23 + row as i32 * ROW_PITCH, ROW_PITCH),
                Some(row)
            );
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
            let offset = scroll_offset_for_handle_y(y, 60, ROW_PITCH);
            assert_eq!(scroll_handle_y(offset, 60, ROW_PITCH), y, "y {y}");
        }
        // Offsets that share a pixel row land within one row of themselves.
        for offset in 0..=56usize {
            let back =
                scroll_offset_for_handle_y(scroll_handle_y(offset, 60, ROW_PITCH), 60, ROW_PITCH);
            assert!(back.abs_diff(offset) <= 1, "offset {offset}");
        }
        // Ends and clamps.
        assert_eq!(scroll_offset_for_handle_y(20, 60, ROW_PITCH), 0);
        assert_eq!(scroll_offset_for_handle_y(60, 60, ROW_PITCH), 56);
        assert_eq!(scroll_offset_for_handle_y(0, 60, ROW_PITCH), 0);
        assert_eq!(scroll_offset_for_handle_y(100, 60, ROW_PITCH), 56);
        // The settled middle point (S01 §4.2: y = 40 for a 60-entry list).
        assert_eq!(scroll_offset_for_handle_y(40, 60, ROW_PITCH), 28);
        // A list that fits one page never moves.
        assert_eq!(scroll_offset_for_handle_y(60, 3, ROW_PITCH), 0);
    }

    #[test]
    fn scrollbar_hit_testing_separates_handle_track_and_arrows() {
        // A 60-entry list at the bottom page: handle at y = 60..77.
        assert_eq!(
            scroll_handle_rect(56, 60, ROW_PITCH),
            Rect::new(260, 60, 8, 18)
        );
        assert_eq!(
            hit_test_scrollbar(263, 65, 56, 60, ROW_PITCH),
            Some(ScrollbarHit::Handle)
        );
        // Track above the handle pages up, below it pages down (own choice).
        assert_eq!(
            hit_test_scrollbar(263, 30, 56, 60, ROW_PITCH),
            Some(ScrollbarHit::TrackAbove)
        );
        assert_eq!(
            hit_test_scrollbar(263, 77, 0, 60, ROW_PITCH),
            Some(ScrollbarHit::TrackBelow)
        );
        // The arrow pair keeps its own path and is not a track hit.
        assert_eq!(hit_test_scrollbar(263, 81, 56, 60, ROW_PITCH), None);
        assert_eq!(hit_test_scrollbar(263, 87, 56, 60, ROW_PITCH), None);
        // Outside the column nothing hits.
        assert_eq!(hit_test_scrollbar(259, 30, 56, 60, ROW_PITCH), None);
        assert_eq!(hit_test_scrollbar(268, 30, 56, 60, ROW_PITCH), None);
        assert_eq!(hit_test_scrollbar(263, 19, 56, 60, ROW_PITCH), None);
    }
}
