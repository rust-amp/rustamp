//! Headless monochrome playlist system-font rasterization (S17 §5).
//! No window or DC; each glyph pixel is either opaque ink or transparent.
//!
//! The request comes from the skin's `pledit.txt` (`Font=` line: the family,
//! the size and the style) or from the library window's own constant. A
//! family the host does not have falls back to the base skin's family and
//! then to the generic sans-serif, so the list is never empty.
//!
//! The **size** is the one part of the request the user can change at
//! runtime: `Ctrl+Num+` / `Ctrl+Num-` (S03 §2 `40335` / `40336`) walk it
//! within [`MIN_FONT_SIZE`]..=[`MAX_FONT_SIZE`]. The override replaces the
//! size only — the family and the style stay the skin's (S17 §5).
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

/// The smallest font size the playlist accepts (own choice).
///
/// The row is as tall as its font, so a tiny font would make the list
/// unreadable rather than merely small. Eight pixels is this implementation's
/// lower bound; the spec fixes no size range (S03 §2 marks the size rows
/// **provisional**).
pub const MIN_FONT_SIZE: u32 = 8;

/// The largest font size the playlist accepts (own choice).
///
/// The list page is 52 px tall (four rows of 13 px, S01 §4.2), so a font of
/// 32 px still leaves one whole row on the page. The spec fixes no size
/// range (S03 §2 marks the size rows **provisional**).
pub const MAX_FONT_SIZE: u32 = 32;

/// The size change of one `Ctrl+Num+` / `Ctrl+Num-` press (own choice).
///
/// One pixel per press: the size is a pixel count, so a step of one is the
/// smallest change the user can see. The spec fixes no step (the rows are
/// **provisional**).
pub const FONT_SIZE_STEP: u32 = 1;

/// Clamps a font size into the accepted range.
fn clamp_size(size: u32) -> u32 {
    size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
}

/// The font size a skin asks for (S17 §5).
///
/// The `Font=` line's size when it names one, the built-in default (13 px)
/// otherwise. The app pushes this into the playlist model, which owns the
/// size the editor draws with (S03 §2 `40335` / `40336`), so the skin and
/// the model never disagree about the size.
pub fn skin_font_size(skin: &rustamp_skin::Skin) -> u32 {
    skin.pledit()
        .font
        .as_ref()
        .and_then(|font| font.size)
        .unwrap_or(DEFAULT_SIZE)
}

/// The playlist editor's runtime font size (S03 §2 `40335` / `40336`).
///
/// The size is the **skin's** (`pledit.txt`) until the user changes it with
/// `Ctrl+Num+` / `Ctrl+Num-`; the user's choice replaces the **size** only —
/// the family and the style stay the skin's (S17 §5).
///
/// Both sizes are clamped into [`MIN_FONT_SIZE`]..=[`MAX_FONT_SIZE`], so a
/// skin that asks for an extreme size and a user who walks the size to its
/// end draw the same bounded rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaylistFontSize {
    /// The size the skin asks for.
    skin: u32,
    /// The size the user chose, once they changed it.
    user: Option<u32>,
}

impl Default for PlaylistFontSize {
    fn default() -> Self {
        Self::new(DEFAULT_SIZE)
    }
}

impl PlaylistFontSize {
    /// The size for a skin that asks for `skin_size`.
    pub fn new(skin_size: u32) -> Self {
        Self {
            skin: clamp_size(skin_size),
            user: None,
        }
    }

    /// The size the playlist draws with.
    pub fn size(self) -> u32 {
        clamp_size(self.user.unwrap_or(self.skin))
    }

    /// The user's size, once they have chosen one.
    pub fn user_size(self) -> Option<u32> {
        self.user
    }

    /// Whether the user has changed the size away from the skin's.
    pub fn is_overridden(self) -> bool {
        self.user.is_some()
    }

    /// Sets the skin's size (the app syncs it when the skin changes).
    ///
    /// A size the user already chose wins over it.
    pub fn set_skin_size(&mut self, skin_size: u32) {
        self.skin = clamp_size(skin_size);
    }

    /// Restores the user's chosen size, or clears it with `None`.
    pub fn set_user_size(&mut self, size: Option<u32>) {
        self.user = size.map(clamp_size);
    }

    /// One step larger (S03 §2 `40335`). Returns whether the size changed.
    ///
    /// The step starts from the size in effect, so the first press after a
    /// skin change steps away from the skin's size.
    pub fn increase(&mut self) -> bool {
        let next = clamp_size(self.size().saturating_add(FONT_SIZE_STEP));
        if next == self.size() {
            return false;
        }
        self.user = Some(next);
        true
    }

    /// One step smaller (S03 §2 `40336`). Returns whether the size changed.
    pub fn decrease(&mut self) -> bool {
        let next = clamp_size(self.size().saturating_sub(FONT_SIZE_STEP));
        if next == self.size() {
            return false;
        }
        self.user = Some(next);
        true
    }
}

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

    /// The same request with another size.
    ///
    /// The family and the style stay the skin's; only the size changes. The
    /// runtime size (S03 §2 `40335` / `40336`) replaces the skin's size
    /// this way, so the family and the style of the `Font=` line survive the
    /// user's change.
    pub(crate) fn with_size(self, size: u32) -> SystemFont<'a> {
        SystemFont {
            family: self.family,
            size: clamp_size(size),
            bold: self.bold,
            italic: self.italic,
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
    /// The faces of [`FALLBACK_FAMILIES`] that were needed so far, by position
    /// in that list. A face is only loaded when a character asks for it: the
    /// CJK fonts are tens of megabytes.
    fallbacks: Vec<Option<Option<FontArc>>>,
}

/// The installed families asked for a character the requested font does not
/// have, in this order: Latin and Greek first, then Japanese, Chinese and
/// Korean (own choice; S17 §5 says nothing about missing characters). Windows
/// has the first groups, and the last names are what other systems carry.
const FALLBACK_FAMILIES: &[&str] = &[
    DEFAULT_FAMILY,
    "Segoe UI",
    "Arial Unicode MS",
    "Yu Gothic",
    "Meiryo",
    "MS Gothic",
    "Microsoft YaHei",
    "SimSun",
    "Microsoft JhengHei",
    "Malgun Gothic",
    "Segoe UI Symbol",
    "Noto Sans CJK JP",
    "Noto Sans CJK SC",
    "Source Han Sans",
    "WenQuanYi Micro Hei",
    "DejaVu Sans",
];

/// The coverage from which a pixel of the requested font counts as ink.
const INK_COVERAGE: f32 = 0.5;

/// The same for a fallback font: the thin strokes of a CJK character at list
/// size break apart at [`INK_COVERAGE`].
const FALLBACK_INK_COVERAGE: f32 = 0.3;

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

/// `fontdb` answers the generic sans-serif with "Arial", which Linux does not
/// ship: with no Arial, the playlist's own family and its generic fallback both
/// miss and no text is drawn at all. Off Windows the generic is pointed at the
/// first family of this list the host has (macOS carries Helvetica Neue and
/// Arial; the Linux desktops carry one of the others).
#[cfg(not(windows))]
fn point_generic_sans_at_an_installed_family(database: &mut fontdb::Database) {
    const CANDIDATES: &[&str] = &[
        "Arial",
        "Helvetica Neue",
        "Helvetica",
        "Liberation Sans",
        "Noto Sans",
        "DejaVu Sans",
        "Cantarell",
        "Ubuntu",
        "FreeSans",
        "Inter",
    ];
    let installed = |wanted: &str| {
        database.faces().any(|face| {
            face.families
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case(wanted))
        })
    };
    if let Some(found) = CANDIDATES.iter().copied().find(|name| installed(name)) {
        database.set_sans_serif_family(found);
    }
}

/// The fonts of the host, read once.
fn fonts() -> &'static Mutex<Fonts> {
    static FONTS: OnceLock<Mutex<Fonts>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        #[cfg(not(windows))]
        point_generic_sans_at_an_installed_family(&mut database);
        Mutex::new(Fonts {
            database,
            loaded: HashMap::new(),
            fallbacks: vec![None; FALLBACK_FAMILIES.len()],
        })
    })
}

/// Whether `face` lacks `ch`. A control character is never drawn.
fn lacks(face: &FontArc, ch: char) -> bool {
    !ch.is_control() && face.glyph_id(ch).0 == 0
}

/// The fallback face that has the most of `text`, stopping at the first that
/// has all of it, with the number of characters it lacks.
fn best_fallback(text: &str) -> Option<(FontArc, usize)> {
    if ONLY_FACE.get().is_some() {
        return None;
    }
    let mut fonts = fonts().lock().ok()?;
    let mut best: Option<(FontArc, usize)> = None;
    for (index, family) in FALLBACK_FAMILIES.iter().enumerate() {
        if fonts.fallbacks[index].is_none() {
            let face = fonts
                .database
                .query(&fontdb::Query {
                    families: &[fontdb::Family::Name(family)],
                    ..Default::default()
                })
                .and_then(|id| {
                    fonts.database.with_face_data(id, |bytes, index| {
                        FontVec::try_from_vec_and_index(bytes.to_vec(), index)
                            .ok()
                            .map(FontArc::new)
                    })
                })
                .flatten();
            fonts.fallbacks[index] = Some(face);
        }
        if let Some(Some(face)) = &fonts.fallbacks[index] {
            let missing = text.chars().filter(|ch| lacks(face, *ch)).count();
            if best.as_ref().is_none_or(|(_, least)| missing < *least) {
                best = Some((face.clone(), missing));
            }
            if missing == 0 {
                break;
            }
        }
    }
    best
}

/// The one face that draws all of `text`, and whether it is a fallback.
///
/// A text changes its font as a whole: if the requested font lacks one
/// character, every character comes from the fallback, so a title does not
/// mix two fonts. A text no font covers completely takes the face that covers
/// most of it.
fn face_for_text(primary: &FontArc, text: &str) -> (FontArc, bool) {
    let missing = text.chars().filter(|ch| lacks(primary, *ch)).count();
    if missing == 0 {
        return (primary.clone(), false);
    }
    match best_fallback(text) {
        Some((face, least)) if least < missing => (face, true),
        _ => (primary.clone(), false),
    }
}

/// The face for one character of a text drawn in `chosen`. Only a character
/// `chosen` lacks goes elsewhere, because the alternative is an empty box: this
/// is the last resort for a text no installed font covers completely.
fn face_for_char(chosen: &FontArc, ch: char) -> (FontArc, bool) {
    if !lacks(chosen, ch) {
        return (chosen.clone(), false);
    }
    match best_fallback(&ch.to_string()) {
        Some((face, 0)) => (face, true),
        _ => (chosen.clone(), false),
    }
}

/// The face for the request, or `None` when the host has no font at all.
///
/// The chain is the requested family, then the built-in default family, then
/// the generic sans-serif. S17 §5 leaves "a `Font` name that is not
/// installed" *not determined*; falling back to a real font instead of
/// drawing nothing is this implementation's own choice.
fn font(request: SystemFont<'_>) -> Option<FontArc> {
    if let Some(face) = ONLY_FACE.get() {
        return Some(face.clone());
    }
    let mut fonts = fonts().lock().ok()?;
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

/// The face set by [`draw_all_text_in`].
static ONLY_FACE: OnceLock<FontArc> = OnceLock::new();

/// Draws all text of this process in the font file `bytes`, from its outlines,
/// for the rest of its life. Neither GDI nor an installed font is asked again.
///
/// GDI smooths with the user's own setting and the installed fonts are the
/// host's, so their pixels differ between machines. A picture compared against
/// a committed golden needs neither. The player never calls this.
///
/// # Panics
///
/// When `bytes` is not a font file.
pub fn draw_all_text_in(bytes: &'static [u8]) {
    let face = FontArc::try_from_slice(bytes).expect("a font file");
    let _ = ONLY_FACE.set(face);
}

/// Whether GDI draws the text.
fn uses_gdi() -> bool {
    ONLY_FACE.get().is_none()
}

/// The width of `text` in pixels, in the metrics [`render`] uses.
///
/// The width is the sum of the glyph advances plus the kerning between them,
/// so it is the place where a caret behind the text belongs. It is `None`
/// when the host has no font at all.
pub(crate) fn text_width(request: SystemFont<'_>, text: &str) -> Option<u32> {
    text_width_as_part_of(request, text, text)
}

/// The request as GDI takes it.
fn gdi_font(request: SystemFont<'_>) -> rustamp_platform::gdi_text::GdiFont<'_> {
    rustamp_platform::gdi_text::GdiFont {
        family: request.family,
        size: request.size,
        bold: request.bold,
        italic: request.italic,
    }
}

/// [`text_width`] for a part of `whole`, in the font `whole` is drawn in.
pub(crate) fn text_width_as_part_of(
    request: SystemFont<'_>,
    whole: &str,
    text: &str,
) -> Option<u32> {
    if uses_gdi()
        && let Some(width) = rustamp_platform::gdi_text::text_width(
            gdi_font(request),
            FALLBACK_FAMILIES,
            whole,
            text,
        )
    {
        return Some(width);
    }
    let (chosen, _) = face_for_text(&font(request)?, whole);
    let scale = PxScale::from(request.size as f32);
    let mut x = 0.0;
    let mut previous = None;
    for ch in text.chars() {
        let (face, other) = face_for_char(&chosen, ch);
        let scaled = face.as_scaled(scale);
        let id = scaled.glyph_id(ch);
        // Kerning only holds between two glyphs of one face.
        if let Some(previous) = previous
            && !other
        {
            x += scaled.kern(previous, id);
        }
        x += scaled.h_advance(id);
        previous = (!other).then_some(id);
    }
    Some(x.ceil() as u32)
}

/// Render using the requested installed family, falling back to Arial/sans.
/// `background` is the colour under the text, which GDI blends the glyphs'
/// edges against. Without GDI, coverage is quantized to a 1-bit mask.
/// The size is the skin's when its `Font=` line names one and the measured
/// 13 px row pitch otherwise; the baseline is our choice within that pitch.
/// This is not a claim of GDI-identical hinting or kerning.
pub(crate) fn render(
    request: SystemFont<'_>,
    text: &str,
    width: u32,
    height: u32,
    colour: [u8; 3],
    background: [u8; 3],
) -> Option<Image> {
    render_as_part_of(request, text, text, width, height, colour, background)
}

/// [`render`] for a part of `whole`, in the font `whole` is drawn in, so the
/// parts of one row (its name and its length) never differ.
pub(crate) fn render_as_part_of(
    request: SystemFont<'_>,
    whole: &str,
    text: &str,
    width: u32,
    height: u32,
    colour: [u8; 3],
    background: [u8; 3],
) -> Option<Image> {
    // GDI where there is one (see `rustamp_platform::gdi_text`), the outlines
    // of the installed fonts elsewhere.
    if uses_gdi()
        && let Some(rgba) = rustamp_platform::gdi_text::render_text(
            gdi_font(request),
            FALLBACK_FAMILIES,
            whole,
            text,
            width,
            height,
            colour,
            background,
        )
    {
        return Image::from_rgba(width, height, rgba).ok();
    }
    let primary = font(request)?;
    let (chosen, fallback) = face_for_text(&primary, whole);
    let scale = PxScale::from(request.size as f32);
    // The baseline stays the requested font's, whichever font draws the text,
    // so a row does not jump up or down between its neighbours.
    let baseline = primary.as_scaled(scale).ascent();
    let ink = [colour[0], colour[1], colour[2], 255];
    let mut image = Image::new(width, height);
    let mut x = 0.0;
    let mut previous = None;
    for ch in text.chars() {
        let (face, other) = face_for_char(&chosen, ch);
        let scaled = face.as_scaled(scale);
        let id = scaled.glyph_id(ch);
        if let Some(previous) = previous
            && !other
        {
            x += scaled.kern(previous, id);
        }
        let glyph = id.with_scale_and_position(scale, point(x, baseline));
        if let Some(outline) = face.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            let threshold = if fallback || other {
                FALLBACK_INK_COVERAGE
            } else {
                INK_COVERAGE
            };
            // A dot or a colon can be smaller than one pixel and spread over
            // several without reaching the threshold in any: its strongest
            // pixel is inked, so no glyph vanishes from the row.
            let mut inked = false;
            let mut strongest = (0.0, 0, 0);
            outline.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i32 + gx as i32;
                let py = bounds.min.y as i32 + gy as i32;
                if coverage >= threshold {
                    image.set_pixel(px, py, ink);
                    inked = true;
                } else if coverage > strongest.0 {
                    strongest = (coverage, px, py);
                }
            });
            if !inked && strongest.0 > 0.0 {
                image.set_pixel(strongest.1, strongest.2, ink);
            }
        }
        x += scaled.h_advance(id);
        previous = (!other).then_some(id);
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
    fn a_text_changes_its_font_as_a_whole() {
        let primary = font(request("Arial", 13, false, false)).unwrap();
        let (_, fallback) = face_for_text(&primary, "plain.mp3");
        assert!(!fallback, "a text the font has stays in it");

        let (face, fallback) = face_for_text(&primary, "\u{65E5}\u{672C}.mp3");
        assert!(fallback, "the font has no CJK");
        for ch in "\u{65E5}\u{672C}.mp3".chars() {
            assert!(!lacks(&face, ch), "{ch} comes from the one face");
        }
        // The Latin part really is drawn by that face, not by the request's.
        let part = |whole: &str| {
            render_as_part_of(
                request("Arial", 13, false, false),
                whole,
                ".mp3",
                40,
                16,
                [255; 3],
                [0; 3],
            )
            .unwrap()
        };
        assert_ne!(
            part(".mp3").as_rgba(),
            part("\u{65E5}\u{672C}.mp3").as_rgba(),
            "the extension changes its font with the title"
        );
    }

    #[test]
    fn a_character_the_font_lacks_comes_from_a_fallback_font() {
        // Arial has no kana. Without the fallback the row shows its empty box
        // or nothing; the fallback draws the character with real strokes.
        let ink = |text: &str| {
            let image = render(
                request("Arial", 13, false, false),
                text,
                40,
                16,
                [255; 3],
                [0; 3],
            )
            .unwrap();
            image.as_rgba().chunks(4).filter(|p| p[3] != 0).count()
        };
        let notdef = ink("\u{10FFFF}");
        assert!(
            ink("\u{65E5}") > notdef,
            "a CJK character has strokes of its own"
        );
        assert!(
            ink("\u{D55C}") > notdef,
            "a Korean character has strokes of its own"
        );
    }

    #[test]
    fn a_dot_and_a_colon_never_vanish_from_a_row() {
        let ink = |text: &str| {
            let image = render(
                request("Arial", 13, false, false),
                text,
                40,
                16,
                [255; 3],
                [0; 3],
            )
            .unwrap();
            image.as_rgba().chunks(4).filter(|p| p[3] != 0).count()
        };
        assert!(ink(".") > 0);
        assert!(ink(":") > 0);
    }

    #[test]
    fn system_font_pixels_are_exact_ink_or_transparent() {
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
                        [0; 3],
                    )
                    .expect("Windows has an installed Arial fallback");
                    // GDI smooths with the user's own setting, so the ink has
                    // shades; a pixel is the text or it is transparent.
                    assert!(
                        image
                            .as_rgba()
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .all(|p| p[3] == 255 || *p == [0, 0, 0, 0])
                    );
                    assert_eq!(
                        image
                            .as_rgba()
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .any(|p| p[3] == 255),
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
            [0; 3],
        )
        .expect("the host has a font at all");
        let fallback = render(
            request(DEFAULT_FAMILY, 13, false, false),
            "Fallback",
            120,
            26,
            INK,
            [0; 3],
        )
        .expect("the host has a font at all");
        assert_eq!(unknown, fallback);
    }

    /// Windows matches font names without regard to case, so a skin may
    /// write `Font=arial`.
    #[test]
    fn a_family_is_matched_without_regard_to_case() {
        let lower = render(
            request("arial", 13, false, false),
            "Case",
            120,
            26,
            INK,
            [0; 3],
        )
        .expect("the host has a font at all");
        let exact = render(
            request("Arial", 13, false, false),
            "Case",
            120,
            26,
            INK,
            [0; 3],
        )
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
            [0; 3],
        )
        .expect("the host has a font at all");
        let heavy = render(
            request(DEFAULT_FAMILY, 13, true, false),
            "Bold",
            120,
            26,
            INK,
            [0; 3],
        )
        .expect("the host has a font at all");
        assert_ne!(plain, heavy, "the bold face must draw different glyphs");
    }
}
