//! The loaded skin: sheets, colours, region, cursors and substitutions.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use crate::bmp;
use crate::error::{Result, SkinError};
use crate::pledit::PlaylistStyle;
use crate::raster::RasterImage;
use crate::region::Region;
use crate::source::{ArchiveSource, FolderSource, SkinSource};
use crate::viscolor::parse_viscolors;
use crate::webamp::{self, Sheet, SpriteDef};

/// A cursor shipped with a skin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorFile {
    /// The entry name.
    pub name: String,
    /// The raw `.cur` or `.ani` bytes.
    pub bytes: Vec<u8>,
    /// The container format.
    pub kind: CursorKind,
}

/// The two cursor container formats classic skins use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorKind {
    /// A `.cur` file (`RIFF`-less cursor resource).
    Cur,
    /// An animated `.ani` file (`RIFF` header).
    Ani,
}

/// What a missing file was replaced with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Substitute {
    /// The bundled default skin.
    DefaultSkin,
    /// The documented default values (playlist colours, viscolours, region).
    Defaults,
}

/// A record of one file that was missing and what replaced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substitution {
    /// The file that was requested, e.g. `nums_ex.bmp`.
    pub requested: String,
    /// What was used instead.
    pub with: Substitute,
}

impl fmt::Display for Substitution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.with {
            Substitute::DefaultSkin => {
                write!(f, "{} missing, using the default skin", self.requested)
            }
            Substitute::Defaults => {
                write!(
                    f,
                    "{} missing, using the documented defaults",
                    self.requested
                )
            }
        }
    }
}

/// The legacy colour of the placeholder art.
///
/// Preserved as a constant for backwards compatibility; the built-in default
/// skin now ships newly-drawn lookalike art for all sheets.
pub const PLACEHOLDER_RGBA: [u8; 4] = [0xFF, 0x00, 0xFF, 0xFF];

/// A source of fallback sprite sheets, used when a skin is missing a file.
pub trait SkinFallback {
    /// Returns a stand-in image for `sheet`, or `None` when the sheet has no
    /// known size.
    fn sheet_image(&self, sheet: Sheet) -> Option<RasterImage>;

    /// Returns a stand-in cursor for `name` (e.g. `"close.cur"`), or `None`
    /// when the fallback carries no cursor under that name.
    ///
    /// The default is `None`, so a fallback that only supplies sheets leaves
    /// a missing cursor absent — the same rule sheets follow when the
    /// fallback has no size for them.
    fn cursor(&self, name: &str) -> Option<CursorFile> {
        let _ = name;
        None
    }
}

/// The built-in default skin.
///
/// Supplies newly-drawn clean-room Rust-themed skin sheets for all classic
/// sheets, used when running with the default skin or falling back on missing
/// files.
#[derive(Debug, Default, Clone, Copy)]
pub struct BuiltinDefaultSkin;

impl BuiltinDefaultSkin {
    /// Creates an in-memory [`crate::source::MemorySource`] containing all 32+1
    /// files of the complete Rust-themed skin.
    pub fn source() -> crate::source::MemorySource {
        crate::rust_theme::source()
    }
}

impl SkinFallback for BuiltinDefaultSkin {
    fn sheet_image(&self, sheet: Sheet) -> Option<RasterImage> {
        if let Some(image) = crate::rust_theme::sheet_image(sheet) {
            return Some(image);
        }
        let (width, height) = webamp::sheet_extent(sheet);
        if width == 0 || height == 0 {
            return None;
        }
        Some(RasterImage::filled(width, height, [0x17, 0x18, 0x1C, 0xFF]))
    }

    fn cursor(&self, name: &str) -> Option<CursorFile> {
        let bytes = crate::rust_theme::cursor_bytes(name)?;
        let kind = if bytes.starts_with(b"RIFF") {
            CursorKind::Ani
        } else {
            CursorKind::Cur
        };
        Some(CursorFile {
            name: name.to_string(),
            bytes: bytes.to_vec(),
            kind,
        })
    }
}

/// The classic lookalike fallback skin (task card J1).
///
/// Supplies classic lookalike dark-metallic sheets when explicitly selected.
/// It ships no cursors, so a missing cursor stays absent under this fallback
/// (the [`SkinFallback::cursor`] default).
#[derive(Debug, Default, Clone, Copy)]
pub struct BuiltinLookalikeSkin;

impl BuiltinLookalikeSkin {
    /// Creates an in-memory [`crate::source::MemorySource`] containing the
    /// available sheets of the classic lookalike skin.
    pub fn source() -> crate::source::MemorySource {
        crate::lookalike::source()
    }
}

impl SkinFallback for BuiltinLookalikeSkin {
    fn sheet_image(&self, sheet: Sheet) -> Option<RasterImage> {
        if let Some(image) = crate::lookalike::sheet_image(sheet) {
            return Some(image);
        }
        let (width, height) = webamp::sheet_extent(sheet);
        if width == 0 || height == 0 {
            return None;
        }
        Some(RasterImage::filled(width, height, [0x28, 0x2A, 0x30, 0xFF]))
    }
}

/// A loaded skin.
pub struct Skin {
    source: String,
    sheets: HashMap<Sheet, RasterImage>,
    /// The sheet the main window's time display takes its digits from
    /// (S17 §3).
    time_digit_sheet: Sheet,
    pledit: PlaylistStyle,
    viscolors: [[u8; 3]; 24],
    region: Region,
    cursors: Vec<CursorFile>,
    substitutions: Vec<Substitution>,
    warnings: Vec<String>,
}

impl fmt::Debug for Skin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Skin")
            .field("source", &self.source)
            .field("sheets", &self.sheets.keys().collect::<Vec<_>>())
            .field("substitutions", &self.substitutions)
            .finish()
    }
}

impl Skin {
    /// Loads a skin from a folder, `.wsz` or `.zip` path, using the built-in
    /// placeholder for missing files.
    pub fn load(path: impl AsRef<Path>) -> Result<Skin> {
        Self::load_with_fallback(path, &BuiltinDefaultSkin)
    }

    /// Loads a skin with a caller-supplied fallback for missing files.
    pub fn load_with_fallback(path: impl AsRef<Path>, fallback: &dyn SkinFallback) -> Result<Skin> {
        let path = path.as_ref();
        if path.is_dir() {
            let mut source = FolderSource::open(path)?;
            Self::from_source(&mut source, fallback)
        } else {
            let mut source = ArchiveSource::open(path)?;
            Self::from_source(&mut source, fallback)
        }
    }

    /// Loads a skin from an already-opened container.
    pub fn from_source(source: &mut dyn SkinSource, fallback: &dyn SkinFallback) -> Result<Skin> {
        let mut sheets: HashMap<Sheet, RasterImage> = HashMap::new();
        let mut substitutions: Vec<Substitution> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        // S17 §3: the time display uses `nums_ex.bmp` **instead of**
        // `numbers.bmp` when the skin carries it, and the digit layout changes
        // with it. Which of the two the *skin* provides decides the layout, so
        // the choice is taken from the container and never from the fallback.
        let mut skin_has_nums_ex = false;

        for sheet in Sheet::ALL {
            let Some(file) = source.read_any(sheet.stem(), &["bmp"]) else {
                continue;
            };
            match bmp::decode(&file.bytes) {
                Ok(decoded) => {
                    let image =
                        RasterImage::new(decoded.width, decoded.height, decoded.rgba, file.bytes);
                    if *sheet == Sheet::NumsEx {
                        skin_has_nums_ex = true;
                    }
                    sheets.insert(*sheet, image);
                }
                Err(reason) => {
                    // A corrupt sheet is treated as missing and falls back.
                    warnings.push(format!("{}: {reason}", sheet.file_name()));
                }
            }
        }

        resolve_default_fallbacks(&mut sheets, fallback, &mut substitutions);

        let pledit = match source.read_any("pledit", &["txt"]) {
            Some(file) if !file.bytes.is_empty() => {
                PlaylistStyle::parse(&String::from_utf8_lossy(&file.bytes))
            }
            _ => {
                substitutions.push(Substitution {
                    requested: "pledit.txt".to_string(),
                    with: Substitute::Defaults,
                });
                PlaylistStyle::default()
            }
        };

        let viscolors = match source.read_any("viscolor", &["txt"]) {
            Some(file) if !file.bytes.is_empty() => {
                parse_viscolors(&String::from_utf8_lossy(&file.bytes))
            }
            _ => {
                substitutions.push(Substitution {
                    requested: "viscolor.txt".to_string(),
                    with: Substitute::Defaults,
                });
                webamp::DEFAULT_VISCOLORS
            }
        };

        let region = match source.read_any("region", &["txt"]) {
            Some(file) if !file.bytes.is_empty() => {
                Region::parse(&String::from_utf8_lossy(&file.bytes))
            }
            _ => {
                substitutions.push(Substitution {
                    requested: "region.txt".to_string(),
                    with: Substitute::Defaults,
                });
                Region::default()
            }
        };

        let mut cursors = Vec::new();
        for name in webamp::CURSOR_NAMES {
            let stem = name.strip_suffix(".cur").unwrap_or(name);
            if let Some(file) = source.read_any(stem, &["cur", "ani"]) {
                let kind = if file.bytes.starts_with(b"RIFF") {
                    CursorKind::Ani
                } else {
                    CursorKind::Cur
                };
                cursors.push(CursorFile {
                    name: file.name,
                    bytes: file.bytes,
                    kind,
                });
            }
        }
        resolve_cursor_fallbacks(&mut cursors, fallback, &mut substitutions);

        Ok(Skin {
            source: source.describe(),
            sheets,
            time_digit_sheet: if skin_has_nums_ex {
                Sheet::NumsEx
            } else {
                Sheet::Numbers
            },
            pledit,
            viscolors,
            region,
            cursors,
            substitutions,
            warnings,
        })
    }

    /// Where the skin was loaded from.
    pub fn source_description(&self) -> &str {
        &self.source
    }

    /// The decoded sheet, if the skin provides it (directly or through a
    /// fallback).
    pub fn sheet(&self, sheet: Sheet) -> Option<&RasterImage> {
        self.sheets.get(&sheet)
    }

    /// The raw decompressed bytes of a sheet, as stored in the container.
    ///
    /// Empty for a generated fallback sheet.
    pub fn sheet_raw(&self, sheet: Sheet) -> Option<&[u8]> {
        self.sheets.get(&sheet).map(RasterImage::raw)
    }

    /// Looks up a sprite rectangle by name.
    pub fn sprite(&self, name: &str) -> Option<&'static SpriteDef> {
        webamp::sprite(name)
    }

    /// Looks up a sprite together with the sheet image it should be blitted
    /// from. Returns `None` for an unknown name or a sheet that is missing
    /// entirely.
    pub fn sprite_image(&self, name: &str) -> Option<(&RasterImage, &'static SpriteDef)> {
        let def = webamp::sprite(name)?;
        let image = self.sheets.get(&def.sheet)?;
        Some((image, def))
    }

    /// The sheet the main window's time display takes its digits from.
    ///
    /// S17 §3: when the skin carries `nums_ex.bmp`, the time display uses it
    /// **instead of** `numbers.bmp` — and the digit layout changes with it,
    /// so the two sheets are never substituted for one another. A skin that
    /// carries neither (or carries a `nums_ex.bmp` that does not decode) uses
    /// the classic `numbers.bmp` layout.
    pub fn time_digit_sheet(&self) -> Sheet {
        self.time_digit_sheet
    }

    /// The playlist colours and font.
    pub fn pledit(&self) -> &PlaylistStyle {
        &self.pledit
    }

    /// The 24 analyser colours.
    pub fn viscolors(&self) -> &[[u8; 3]; 24] {
        &self.viscolors
    }

    /// The window regions from `region.txt`.
    pub fn region(&self) -> &Region {
        &self.region
    }

    /// The cursors the skin ships, plus any the fallback supplied for names
    /// the skin does not carry (see [`SkinFallback::cursor`]). A name the
    /// fallback does not carry either is simply absent.
    pub fn cursors(&self) -> &[CursorFile] {
        &self.cursors
    }

    /// Looks up a cursor by its file name (case-insensitive).
    pub fn cursor(&self, name: &str) -> Option<&CursorFile> {
        self.cursors
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Every file that was missing and what replaced it.
    pub fn substitutions(&self) -> &[Substitution] {
        &self.substitutions
    }

    /// Non-fatal problems found while loading (for example a sheet that failed
    /// to decode and was therefore treated as missing).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// True when every sheet the main window needs is present.
    pub fn has_main_window_sprites(&self) -> bool {
        Sheet::required_main_window()
            .iter()
            .all(|sheet| self.sheets.contains_key(sheet))
    }

    /// The sheets that are missing entirely (no file, no fallback).
    pub fn missing_sheets(&self) -> Vec<Sheet> {
        Sheet::ALL
            .iter()
            .copied()
            .filter(|sheet| !self.sheets.contains_key(sheet))
            .collect()
    }
}

/// Fills any missing sheet from the fallback skin.
///
/// S17 §2: the fallback is per element, not all-or-nothing — a skin that is
/// missing one sheet keeps all its other sheets, and the missing element comes
/// from the built-in default skin.
fn resolve_default_fallbacks(
    sheets: &mut HashMap<Sheet, RasterImage>,
    fallback: &dyn SkinFallback,
    substitutions: &mut Vec<Substitution>,
) {
    // Every sheet the fallback can supply, not only the main window's: a skin
    // that is missing, say, `nums_ex.bmp` should still get a legible one from the
    // built-in skin rather than nothing at all. Sheets the fallback has no size
    // for stay absent, exactly as the trait documents.
    //
    // S17 §3: the fallback's `nums_ex.bmp` never takes the time display away
    // from a skin's own `numbers.bmp` — the digit layout follows the file the
    // *skin* carries, see [`Skin::time_digit_sheet`].
    for sheet in Sheet::ALL {
        if sheets.contains_key(sheet) {
            continue;
        }
        if let Some(image) = fallback.sheet_image(*sheet) {
            sheets.insert(*sheet, image);
            substitutions.push(Substitution {
                requested: sheet.file_name().to_string(),
                with: Substitute::DefaultSkin,
            });
        }
    }
}

/// The cursor file-name stem, ignoring a `.cur`/`.ani` suffix of either case.
///
/// A skin that ships `posbar.ani` carries the `posbar.cur` slot, so the
/// fallback must never shadow it with a cursor of its own.
fn cursor_stem(name: &str) -> &str {
    for suffix in [".cur", ".ani"] {
        if name.len() >= suffix.len()
            && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
        {
            return &name[..name.len() - suffix.len()];
        }
    }
    name
}

/// Fills any missing cursor from the fallback skin.
///
/// This is the same rule sheets follow ([`resolve_default_fallbacks`]): a
/// cursor the skin does not carry is taken from the fallback's own cursor of
/// the same name and reported as a [`Substitute::DefaultSkin`] substitution.
/// A name the fallback does not carry either stays absent with no
/// substitution, exactly like a sheet the fallback has no size for.
fn resolve_cursor_fallbacks(
    cursors: &mut Vec<CursorFile>,
    fallback: &dyn SkinFallback,
    substitutions: &mut Vec<Substitution>,
) {
    for name in webamp::CURSOR_NAMES {
        let wanted = cursor_stem(name);
        if cursors
            .iter()
            .any(|c| cursor_stem(&c.name).eq_ignore_ascii_case(wanted))
        {
            continue;
        }
        if let Some(cursor) = fallback.cursor(name) {
            substitutions.push(Substitution {
                requested: (*name).to_string(),
                with: Substitute::DefaultSkin,
            });
            cursors.push(cursor);
        }
    }
}

/// Opens a path as a skin container without loading it.
pub fn open_source(path: impl AsRef<Path>) -> Result<Box<dyn SkinSource>> {
    let path = path.as_ref();
    if path.is_dir() {
        Ok(Box::new(FolderSource::open(path)?))
    } else if path.is_file() {
        Ok(Box::new(ArchiveSource::open(path)?))
    } else {
        Err(SkinError::NotASkin(path.display().to_string()))
    }
}
