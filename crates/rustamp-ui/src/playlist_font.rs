//! Headless monochrome playlist system-font rasterization (S17 §5).
//! No window or DC; each glyph pixel is either opaque ink or transparent.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ab_glyph::{Font, FontArc, FontVec, PxScale, ScaleFont, point};
use rustamp_gfx::Image;

struct Fonts {
    database: fontdb::Database,
    loaded: HashMap<String, Option<FontArc>>,
}

fn font(family: &str) -> Option<FontArc> {
    static FONTS: OnceLock<Mutex<Fonts>> = OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        Mutex::new(Fonts {
            database,
            loaded: HashMap::new(),
        })
    });
    let mut fonts = fonts.lock().ok()?;
    if let Some(font) = fonts.loaded.get(family) {
        return font.clone();
    }
    let id = fonts.database.query(&fontdb::Query {
        families: &[
            fontdb::Family::Name(family),
            fontdb::Family::Name("Arial"),
            fontdb::Family::SansSerif,
        ],
        ..Default::default()
    });
    let font = id
        .and_then(|id| {
            fonts.database.with_face_data(id, |bytes, index| {
                FontVec::try_from_vec_and_index(bytes.to_vec(), index)
                    .ok()
                    .map(FontArc::new)
            })
        })
        .flatten();
    fonts.loaded.insert(family.into(), font.clone());
    font
}

/// The width of `text` in pixels, in the metrics [`render`] uses.
///
/// The width is the sum of the glyph advances plus the kerning between them,
/// so it is the place where a caret behind the text belongs. It is `None`
/// when the host has no font at all.
pub(crate) fn text_width(family: &str, text: &str) -> Option<u32> {
    let font = font(family)?;
    let scaled = font.as_scaled(PxScale::from(13.0));
    let mut x = 0.0;
    let mut previous = None;
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(previous) = previous {
            x += scaled.kern(previous, id);
        }
        x += scaled.h_advance(id);
        previous = Some(id);
    }
    Some(x.ceil() as u32)
}

/// Render using the requested installed family, falling back to Arial/sans.
/// Quantize coverage to a 1-bit mask before compositing: playlist rows must
/// contain only the skin's foreground and background, never blended shades.
/// Size and baseline are our choices within S01's measured 13px row pitch;
/// this is not a claim of GDI-identical hinting or kerning.
pub(crate) fn render(
    family: &str,
    text: &str,
    width: u32,
    height: u32,
    colour: [u8; 3],
) -> Option<Image> {
    let font = font(family)?;
    let scaled = font.as_scaled(PxScale::from(13.0));
    let mut image = Image::new(width, height);
    let mut x = 0.0;
    let mut previous = None;
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(previous) = previous {
            x += scaled.kern(previous, id);
        }
        let glyph = id.with_scale_and_position(scaled.scale(), point(x, scaled.ascent()));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                if coverage >= 0.5 {
                    image.set_pixel(
                        bounds.min.x as i32 + gx as i32,
                        bounds.min.y as i32 + gy as i32,
                        [colour[0], colour[1], colour[2], 255],
                    );
                }
            });
        }
        x += scaled.h_advance(id);
        previous = Some(id);
        if x >= width as f32 {
            break;
        }
    }
    Some(image)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn system_font_pixels_are_exact_ink_or_transparent() {
        let ink = [0x13, 0x57, 0x9B, 255];
        for family in ["Arial", "Courier New", "Rustamp missing family fallback"] {
            for text in ["Mixed Case AV ffi 0123456789", "jgyW", ""] {
                let image = render(family, text, 120, 13, [ink[0], ink[1], ink[2]])
                    .expect("Windows has an installed Arial fallback");
                assert!(
                    image
                        .as_rgba()
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .all(|p| *p == ink || *p == [0; 4])
                );
                assert_eq!(
                    image.as_rgba().as_chunks::<4>().0.contains(&ink),
                    !text.is_empty()
                );
            }
        }
    }
}
