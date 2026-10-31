// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The skinned title bar of the player's own secondary windows: the media
//! library and the dialogs it draws itself. Windows made of the operating
//! system's controls (the preferences, the file dialogs) keep the native one.
//!
//! The classic windows draw their measured title bars themselves. These
//! windows have no measured one, so theirs is an **own choice** built from the
//! skin's playlist-editor strip (`pledit.bmp`, S01 §4.1): the left corner, the
//! tile, the end of the right corner with its close button, and the title on
//! the plate the editor's title sits on. A native caption would be a second
//! look in a player whose every window wears the skin.

use rustamp_gfx::{Image, Rect};
use rustamp_skin::{Sheet, Skin};

use crate::playlist_font::{self, SystemFont};

/// The title bar's height: the strip's own (S01 §4.1).
pub const TITLE_BAR_HEIGHT: u32 = 20;

/// How much of the right corner sprite is drawn: the part with the close
/// button, without the shade button, which these windows do not have.
const RIGHT_END_WIDTH: u32 = 12;

/// The close button: the 9 x 9 cell the editor's is, from the right edge.
const CLOSE_FROM_RIGHT: i32 = 11;
const CLOSE_TOP: i32 = 3;
const CLOSE_SIZE: i32 = 9;

/// The size of the title's real font (own choice: what the strip holds).
const TITLE_FONT_SIZE: u32 = 12;

/// The padding of the title's plate, on each side of the text.
const PLATE_PAD: u32 = 8;

/// Whether `(x, y)` lies on the close button of a window `width` pixels wide.
/// The button takes a little more than its cell, so it is easy to hit.
pub fn hits_close(width: u32, x: i32, y: i32) -> bool {
    let left = width as i32 - CLOSE_FROM_RIGHT;
    (left - 1..left + CLOSE_SIZE + 1).contains(&x)
        && (CLOSE_TOP - 1..CLOSE_TOP + CLOSE_SIZE + 1).contains(&y)
}

/// Whether `y` lies on the title bar.
pub fn hits_title_bar(y: i32) -> bool {
    (0..TITLE_BAR_HEIGHT as i32).contains(&y)
}

/// Draws the title bar across the top of `canvas`.
pub fn draw_title_bar(canvas: &mut Image, skin: &Skin, title: &str, focused: bool) {
    let width = canvas.width();
    let style = skin.pledit();
    canvas.fill_rect(
        Rect::new(0, 0, width, TITLE_BAR_HEIGHT),
        [
            style.normal_bg[0],
            style.normal_bg[1],
            style.normal_bg[2],
            255,
        ],
    );
    let (corner, plate, tile, right) = if focused {
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
    let font = SystemFont {
        family: playlist_font::DEFAULT_FAMILY,
        size: TITLE_FONT_SIZE,
        bold: false,
        italic: false,
    };
    let text_width = playlist_font::text_width(font, title).unwrap_or(0);
    let plate_width = text_width + 2 * PLATE_PAD;
    let plate_x = (width.saturating_sub(plate_width) / 2) as i32;

    if let Some(sheet) = pledit_sheet(skin) {
        let source = |name: &str| {
            skin.sprite(name)
                .filter(|sprite| sprite.sheet == Sheet::Pledit)
                .map(|sprite| {
                    Rect::new(
                        sprite.x as i32,
                        sprite.y as i32,
                        sprite.width,
                        sprite.height,
                    )
                })
        };
        if let Some(tile) = source(tile) {
            let mut x = 0;
            while x < width as i32 {
                canvas.blit_rect(&sheet, tile, x, 0);
                x += tile.width.max(1) as i32;
            }
        }
        if let Some(corner) = source(corner) {
            canvas.blit_rect(&sheet, corner, 0, 0);
        }
        if let Some(right) = source(right) {
            let end = Rect::new(
                right.x + (right.width - RIGHT_END_WIDTH) as i32,
                right.y,
                RIGHT_END_WIDTH,
                right.height,
            );
            canvas.blit_rect(&sheet, end, (width - RIGHT_END_WIDTH) as i32, 0);
        }
        // The plate: one column of the editor's own plate, left of its baked-in
        // caption, repeated under the title.
        if let Some(plate) = source(plate) {
            let column = Rect::new(plate.x + 3, plate.y, 1, plate.height);
            for x in plate_x..plate_x + plate_width as i32 {
                canvas.blit_rect(&sheet, column, x, 0);
            }
        }
    }
    let colour = if focused { style.current } else { style.normal };
    if let Some(text) = playlist_font::render(font, title, text_width.max(1), 14, colour) {
        canvas.blit_alpha(&text, text.bounds(), plate_x + PLATE_PAD as i32, 3);
    }
}

fn pledit_sheet(skin: &Skin) -> Option<Image> {
    let raster = skin.sheet(Sheet::Pledit)?;
    Image::from_rgba(raster.width(), raster.height(), raster.as_rgba().to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_close_button_sits_at_the_right_end_of_the_bar() {
        assert!(hits_close(400, 393, 7));
        assert!(!hits_close(400, 300, 7));
        assert!(!hits_close(400, 393, 15));
    }

    #[test]
    fn the_title_bar_is_the_strip_at_the_top() {
        assert!(hits_title_bar(0));
        assert!(hits_title_bar(19));
        assert!(!hits_title_bar(20));
        assert!(!hits_title_bar(-1));
    }
}
