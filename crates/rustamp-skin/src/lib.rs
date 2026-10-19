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
//!   ([`Skin::viscolors`]) and `region.txt` ([`Region`]), including the
//!   **window-shape mask** ([`RegionMask`]) the region's polygons describe
//!   for a window of a given size;
//! * the cursors the skin ships: the classic cursor **slots**
//!   ([`CursorSlot`]) and the file bytes behind them
//!   ([`CursorFile`], [`Skin::cursor_slot`]). The bytes stay untouched —
//!   this crate never decodes a `.cur` or an `.ani`.
//!
//! Missing files never fail the whole load: each one falls back per file (see
//! [`Substitution`]). A missing sheet falls back to the default skin, a
//! missing cursor falls back to the default skin's own cursor of the same name
//! — or stays absent when the default skin carries no cursor under that name.
//! A slot nobody carries is absent, and the caller falls back to the system
//! cursor.
//!
//! `nums_ex.bmp` and `numbers.bmp` are **not** interchangeable: when a skin
//! carries `nums_ex.bmp`, the time display uses it instead of
//! `numbers.bmp` and the digit layout changes with it (S17 §3). The sheet the
//! time display takes its digits from is [`Skin::time_digit_sheet`].
//!
//! ## Sprite tables
//!
//! The sprite rectangles and default colours in [`webamp`] were ported from
//! webamp (MIT) and are now **reconciled with the audited specs S01 and S17**
//! (`specs/S01-window-geometry.md`, `specs/S17-skin-loading-quirks.md`): every
//! value cites the spec section that measured it, and the rows the specs still
//! call provisional keep a `// provisional: webamp, pending S01/S17` marker.
//! Fallback behaviour follows the audited spec S17.
//!
//! Codes against the frozen core contract in [`rustamp_core`]; see
//! `docs/rfc/0001-core-contracts.md`.

#![forbid(unsafe_code)]

pub mod bmp;
pub mod cursor;
pub mod error;
pub mod ini;
pub mod lookalike;
pub mod pledit;
pub mod raster;
pub mod region;
pub mod region_mask;
pub mod rust_theme;
pub mod skin;
pub mod source;
pub mod viscolor;
pub mod webamp;

pub use cursor::CursorSlot;
pub use error::{Result, SkinError};
pub use pledit::PlaylistStyle;
pub use raster::RasterImage;
pub use region::{Polygon, Region, RegionSection};
pub use region_mask::{MaskSpan, RegionMask, build_mask};
pub use skin::{
    BuiltinDefaultSkin, BuiltinLookalikeSkin, CursorFile, CursorKind, PLACEHOLDER_RGBA, Skin,
    SkinFallback, Substitute, Substitution,
};
pub use source::{ArchiveSource, FolderSource, MemorySource, SkinFile, SkinSource};
pub use webamp::{Sheet, SpriteDef};
