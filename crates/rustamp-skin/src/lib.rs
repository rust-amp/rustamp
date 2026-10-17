//! # rustamp-skin
//!
//! Loads a classic Winamp skin from a `.wsz`/`.zip` archive or a folder and
//! exposes everything a window needs to paint itself:
//!
//! * the sprite sheets, decoded to RGBA at 1/4/8/16/24/32 bits per pixel, with
//!   the raw decompressed bytes kept alongside ([`RasterImage`]);
//! * the named sprite rectangles and a lookup that returns the source sheet
//!   ([`Skin::sprite`], [`Skin::sprite_image`], [`webamp::SPRITES`]);
//! * `pledit.txt` ([`PlaylistStyle`]), `viscolor.txt`
//!   ([`Skin::viscolors`]) and `region.txt` ([`Region`]);
//! * the cursors the skin ships ([`CursorFile`]).
//!
//! Missing files never fail the whole load: each one falls back per file (see
//! [`Substitution`]). `nums_ex.bmp` falls back to `numbers.bmp`, and a missing
//! main-window sheet falls back to the default skin.
//!
//! ## Sprite tables
//!
//! The sprite rectangles and default colours in [`webamp`] were ported from
//! webamp (MIT) and are now **reconciled with the audited spec S01**
//! (`specs/S01-window-geometry.md`): every value cites the S01 section that
//! measured it, and the rows S01 still calls provisional keep a
//! `// provisional: webamp, pending S01/S17` marker. Fallback behaviour follows
//! the audited spec S17.
//!
//! Codes against the frozen core contract in [`rustamp_core`]; see
//! `docs/rfc/0001-core-contracts.md`.

#![forbid(unsafe_code)]

pub mod bmp;
pub mod error;
pub mod ini;
pub mod lookalike;
pub mod pledit;
pub mod raster;
pub mod region;
pub mod skin;
pub mod source;
pub mod viscolor;
pub mod webamp;

pub use error::{Result, SkinError};
pub use pledit::PlaylistStyle;
pub use raster::RasterImage;
pub use region::{Polygon, Region};
pub use skin::{
    BuiltinDefaultSkin, CursorFile, CursorKind, PLACEHOLDER_RGBA, Skin, SkinFallback, Substitute,
    Substitution,
};
pub use source::{ArchiveSource, FolderSource, MemorySource, SkinFile, SkinSource};
pub use webamp::{Sheet, SpriteDef};
