//! Headless monochrome playlist system-font rasterization (S17 §5).
//! No window or DC; each glyph pixel is either opaque ink or transparent.
//!
//! The request comes from the skin's `pledit.txt` (`Font=` line: the family,
//! the size and the style) or from the library window's own constant. A
//! family the host does not have falls back to the base skin's family and
//! then to the generic sans-serif, so the list is never empty.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ab_glyph::{Font, FontArc, FontVec, PxScale, ScaleFont, point};
use rustamp_gfx::Image;
use rustamp_skin::PlaylistFont;

/// The family the playlist falls back to when the skin names none.
///
/// It is the base skin's own family (S17 §5) and the first step of the
/// fallback chain for a family the host does not have.
pub(crate) const DEFAULT_FAMILY: &str = rustamp_skin::webamp::DEFAULT_PLAYLIST_FONT;

/// The size the playlist rows are rasterized at when the skin names none.
///
/// S01 §4.2 measured the classic list at a 13 px row pitch, and the built-in
/// font was rasterized at that size before a skin's `Font=` line was
/// honoured, so a skin that names no size keeps it.
pub(crate) const DEFAULT_SIZE: u32 = 13;

/// A system font request: the family, the size in pixels and the style.
///
/// The request is what the renderer needs; where it comes from is the
/// caller's business - the skin's `pledit.txt` for the playlist, the
/// library window's own constant for the modern library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SystemFont<'a> {
    /// The family name. A family the host does not have falls back to
    /// [`DEFAULT_FAMILY`] and then to the generic sans-serif.
    pub family: &'a str,
    /// The size in pixels.
    pub size: u32,
    /// Whether a bold face is wanted.
    pub bold: bool,
    /// Whether an italic face is wanted.
    pub italic: bool,
}

impl<'a> SystemFont<'a> {
    /// The request a skin's `pledit.txt` makes.
    ///
    /// A skin that names no font gets the built-in default family at the
    /// default size; a skin that names a family but no size gets the
    /// default size. A size larger than a playlist row is honored as asked
    /// for and clipped by the row: the list has a fixed row pitch.
    pub(crate) fn from_playlist_font(font: Option<&'a PlaylistFont>) -> SystemFont<'a> {
        match font {
            Some(font) => SystemFont {
                family: &font.family,
                size: font.size.unwrap_or(DEFAULT_SIZE),
                bold: font.bold,
                italic: font.italic,
            },
            None => SystemFont {
                family: DEFAULT_FAMILY,
                size: DEFAULT_SIZE,
                bold: false,
                italic: false,
            },
        }
    }
}

/// The face a request selects: the family and the style flags. The size is
/// applied when the face is scaled, so it is not part of the key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FaceKey {
    family: String,
    bold: bool,
    italic: bool,
}

struct Fonts {
    database: fontdb::Database,
    loaded: HashMap<FaceKey, Option<FontArc>>,
}

/// The name the host knows the family by, or `None` when it has no such
/// family.
///
/// Windows matches font names without regard to case and a skin may write
/// `Font=arial`, so the name is folded before the query.
fn known_family(database: &fontdb::Database, family: &str) -> Option<String> {
    database
        .faces()
        .filter_map(|face| {
            face.families
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(family))
        })
        .map(|(name, _)| name.clone())
        .next()
}

/// The face for the request, or `None` when the host has no font at all.
///
/// The chain is the requested family, then the built-in default family, then
/// the generic sans-serif. S17 §5 leaves "a `Font` name that is not
/// installed" *not determined*; falling back to a real font instead of
/// drawing nothing is this implementation's own choice.
fn font(request: SystemFont<'_>) -> Option<FontArc> {
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
    let key = FaceKey {
        family: request.family.to_string(),
        bold: request.bold,
        italic: request.italic,
    };
    if let Some(font) = fonts.loaded.get(&key) {
        return font.clone();
    }
    let requested =
        known_family(&fonts.database, request.family).unwrap_or_else(|| request.family.to_string());
    let id = fonts.database.query(&fontdb::Query {
        families: &[
            fontdb::Family::Name(&requested),
            fontdb::Family::Name(DEFAULT_FAMILY),
            fontdb::Family::SansSerif,
        ],
        weight: if request.bold {
            fontdb::Weight::BOLD
        } else {
            fontdb::Weight::NORMAL
        },
        style: if request.italic {
            fontdb::Style::Italic
        } else {
            fontdb::Style::Normal
        },
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
    fonts.loaded.insert(key, font.clone());
    font
}

/// The width of `text` in pixels, in the metrics [`render`] uses.
///
/// The width is the sum of the glyph advances plus the kerning between them,
/// so it is the place where a caret behind the text belongs. It is `None`
/// when the host has no font at all.
pub(crate) fn text_width(request: SystemFont<'_>, text: &str) -> Option<u32> {
    let face = font(request)?;
    let scaled = face.as_scaled(PxScale::from(request.size as f32));
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
/// The size is the skin's when its `Font=` line names one and the measured
/// 13 px row pitch otherwise; the baseline is our choice within that pitch.
/// This is not a claim of GDI-identical hinting or kerning.
pub(crate) fn render(
    request: SystemFont<'_>,
    text: &str,
    width: u32,
    height: u32,
    colour: [u8; 3],
) -> Option<Image> {
    let face = font(request)?;
    let scaled = face.as_scaled(PxScale::from(request.size as f32));
    let mut image = Image::new(width, height);
    let mut x = 0.0;
    let mut previous = None;
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(previous) = previous {
            x += scaled.kern(previous, id);
        }
        let glyph = id.with_scale_and_position(scaled.scale(), point(x, scaled.ascent()));
        if let Some(outline) = face.outline_glyph(glyph) {
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

    const INK: [u8; 3] = [0x13, 0x57, 0x9B];

    fn request(family: &str, size: u32, bold: bool, italic: bool) -> SystemFont<'_> {
        SystemFont {
            family,
            size,
            bold,
            italic,
        }
    }

    #[test]
    fn system_font_pixels_are_exact_ink_or_transparent() {
        let ink = [INK[0], INK[1], INK[2], 255];
        for family in ["Arial", "Courier New", "Rustamp missing family fallback"] {
            for (size, bold, italic) in [
                (13, false, false),
                (9, false, false),
                (20, true, false),
                (13, false, true),
            ] {
                for text in ["Mixed Case AV ffi 0123456789", "jgyW", ""] {
                    let image = render(
                        request(family, size, bold, italic),
                        text,
                        120,
                        size * 2,
                        INK,
                    )
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

    /// A family the host does not have must not change the picture: the
    /// fallback chain ends at the same face the default family names.
    #[test]
    fn an_unknown_family_renders_like_the_fallback_family() {
        let unknown = render(
            request("Rustamp missing family fallback", 13, false, false),
            "Fallback",
            120,
            26,
            INK,
        )
        .expect("the host has a font at all");
        let fallback = render(
            request(DEFAULT_FAMILY, 13, false, false),
            "Fallback",
            120,
            26,
            INK,
        )
        .expect("the host has a font at all");
        assert_eq!(unknown, fallback);
    }

    /// Windows matches font names without regard to case, so a skin may
    /// write `Font=arial`.
    #[test]
    fn a_family_is_matched_without_regard_to_case() {
        let lower = render(request("arial", 13, false, false), "Case", 120, 26, INK)
            .expect("the host has a font at all");
        let exact = render(request("Arial", 13, false, false), "Case", 120, 26, INK)
            .expect("the host has a font at all");
        assert_eq!(lower, exact);
    }

    /// The size is the skin's business: a bigger font takes more room.
    #[test]
    fn a_bigger_size_takes_more_room() {
        let small = text_width(request("Arial", 10, false, false), "Playlist").expect("a font");
        let big = text_width(request("Arial", 20, false, false), "Playlist").expect("a font");
        assert!(big > small, "{big} must be wider than {small}");
    }

    /// A bold request must reach the font query: when the host has a bold
    /// face for the family, the bold glyphs are the ones that are drawn.
    #[test]
    fn a_bold_request_picks_the_bold_face_when_the_host_has_one() {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        let regular = database.query(&fontdb::Query {
            families: &[fontdb::Family::Name(DEFAULT_FAMILY)],
            ..Default::default()
        });
        let bold = database.query(&fontdb::Query {
            families: &[fontdb::Family::Name(DEFAULT_FAMILY)],
            weight: fontdb::Weight::BOLD,
            ..Default::default()
        });
        if regular == bold {
            // The host has no bold face for the family: there is nothing to
            // tell apart.
            return;
        }
        let plain = render(
            request(DEFAULT_FAMILY, 13, false, false),
            "Bold",
            120,
            26,
            INK,
        )
        .expect("the host has a font at all");
        let heavy = render(
            request(DEFAULT_FAMILY, 13, true, false),
            "Bold",
            120,
            26,
            INK,
        )
        .expect("the host has a font at all");
        assert_ne!(plain, heavy, "the bold face must draw different glyphs");
    }
}
