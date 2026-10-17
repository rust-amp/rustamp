// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Shipped complete Rust-themed default skin assets, cursors, text files and raster decoder.
//!
//! Procedurally generated in `crates/rustamp-skin/generator/generate_skin.py`.
//! All 32 classic skin entries (18 BMPs, 11 CURs, 3 TXTs) plus `nums_ex.bmp` are
//! mathematically and procedurally generated clean-room artwork at pixel-exact
//! spec dimensions. No original Winamp assets are copied or referenced.

use crate::raster::RasterImage;
use crate::source::MemorySource;
use crate::webamp::Sheet;

// 18 classic sheets + 1 extended sheet
const MAIN_BMP: &[u8] = include_bytes!("../assets/rust-theme/main.bmp");
const TITLEBAR_BMP: &[u8] = include_bytes!("../assets/rust-theme/titlebar.bmp");
const CBUTTONS_BMP: &[u8] = include_bytes!("../assets/rust-theme/cbuttons.bmp");
const SHUFREP_BMP: &[u8] = include_bytes!("../assets/rust-theme/shufrep.bmp");
const POSBAR_BMP: &[u8] = include_bytes!("../assets/rust-theme/posbar.bmp");
const VOLUME_BMP: &[u8] = include_bytes!("../assets/rust-theme/volume.bmp");
const BALANCE_BMP: &[u8] = include_bytes!("../assets/rust-theme/balance.bmp");
const MONOSTER_BMP: &[u8] = include_bytes!("../assets/rust-theme/monoster.bmp");
const PLAYPAUS_BMP: &[u8] = include_bytes!("../assets/rust-theme/playpaus.bmp");
const NUMBERS_BMP: &[u8] = include_bytes!("../assets/rust-theme/numbers.bmp");
const NUMS_EX_BMP: &[u8] = include_bytes!("../assets/rust-theme/nums_ex.bmp");
const TEXT_BMP: &[u8] = include_bytes!("../assets/rust-theme/text.bmp");
const PLEDIT_BMP: &[u8] = include_bytes!("../assets/rust-theme/pledit.bmp");
const EQMAIN_BMP: &[u8] = include_bytes!("../assets/rust-theme/eqmain.bmp");
const EQ_EX_BMP: &[u8] = include_bytes!("../assets/rust-theme/eq_ex.bmp");
const GEN_BMP: &[u8] = include_bytes!("../assets/rust-theme/gen.bmp");
const GENEX_BMP: &[u8] = include_bytes!("../assets/rust-theme/genex.bmp");
const MB_BMP: &[u8] = include_bytes!("../assets/rust-theme/mb.bmp");
const VIDEO_BMP: &[u8] = include_bytes!("../assets/rust-theme/video.bmp");

// 11 cursors
const NORMAL_CUR: &[u8] = include_bytes!("../assets/rust-theme/normal.cur");
const PNORMAL_CUR: &[u8] = include_bytes!("../assets/rust-theme/pnormal.cur");
const CLOSE_CUR: &[u8] = include_bytes!("../assets/rust-theme/close.cur");
const TITLEBAR_CUR: &[u8] = include_bytes!("../assets/rust-theme/titlebar.cur");
const POSBAR_CUR: &[u8] = include_bytes!("../assets/rust-theme/posbar.cur");
const VOLBAL_CUR: &[u8] = include_bytes!("../assets/rust-theme/volbal.cur");
const EQSLID_CUR: &[u8] = include_bytes!("../assets/rust-theme/eqslid.cur");
const EQTITLE_CUR: &[u8] = include_bytes!("../assets/rust-theme/eqtitle.cur");
const PSIZE_CUR: &[u8] = include_bytes!("../assets/rust-theme/psize.cur");
const PTBAR_CUR: &[u8] = include_bytes!("../assets/rust-theme/ptbar.cur");
const PVSCROLL_CUR: &[u8] = include_bytes!("../assets/rust-theme/pvscroll.cur");

// 3 text files
const PLEDIT_TXT: &str = include_str!("../assets/rust-theme/pledit.txt");
const REGION_TXT: &str = include_str!("../assets/rust-theme/region.txt");
const VISCOLOR_TXT: &str = include_str!("../assets/rust-theme/viscolor.txt");

/// Returns the embedded raw BMP file bytes for `sheet` from the Rust theme.
pub const fn sheet_bytes(sheet: Sheet) -> Option<&'static [u8]> {
    match sheet {
        Sheet::Main => Some(MAIN_BMP),
        Sheet::Titlebar => Some(TITLEBAR_BMP),
        Sheet::Cbuttons => Some(CBUTTONS_BMP),
        Sheet::Shufrep => Some(SHUFREP_BMP),
        Sheet::Posbar => Some(POSBAR_BMP),
        Sheet::Volume => Some(VOLUME_BMP),
        Sheet::Balance => Some(BALANCE_BMP),
        Sheet::Monoster => Some(MONOSTER_BMP),
        Sheet::Playpaus => Some(PLAYPAUS_BMP),
        Sheet::Numbers => Some(NUMBERS_BMP),
        Sheet::NumsEx => Some(NUMS_EX_BMP),
        Sheet::Text => Some(TEXT_BMP),
        Sheet::Pledit => Some(PLEDIT_BMP),
        Sheet::EqMain => Some(EQMAIN_BMP),
        Sheet::EqEx => Some(EQ_EX_BMP),
        Sheet::Gen => Some(GEN_BMP),
        Sheet::GenEx => Some(GENEX_BMP),
        Sheet::Mb => Some(MB_BMP),
        Sheet::Video => Some(VIDEO_BMP),
    }
}

/// Decodes and returns the raster image for `sheet` from the Rust theme skin.
pub fn sheet_image(sheet: Sheet) -> Option<RasterImage> {
    let bytes = sheet_bytes(sheet)?;
    match crate::bmp::decode(bytes) {
        Ok(decoded) => Some(RasterImage::new(
            decoded.width,
            decoded.height,
            decoded.rgba,
            bytes.to_vec(),
        )),
        Err(err) => {
            eprintln!(
                "error decoding Rust-themed skin sheet {}: {err}",
                sheet.file_name()
            );
            None
        }
    }
}

/// Returns the raw bytes of a cursor by its file name (case-insensitive).
pub fn cursor_bytes(name: &str) -> Option<&'static [u8]> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "normal.cur" => Some(NORMAL_CUR),
        "pnormal.cur" => Some(PNORMAL_CUR),
        "close.cur" => Some(CLOSE_CUR),
        "titlebar.cur" => Some(TITLEBAR_CUR),
        "posbar.cur" => Some(POSBAR_CUR),
        "volbal.cur" => Some(VOLBAL_CUR),
        "eqslid.cur" => Some(EQSLID_CUR),
        "eqtitle.cur" => Some(EQTITLE_CUR),
        "psize.cur" => Some(PSIZE_CUR),
        "ptbar.cur" => Some(PTBAR_CUR),
        "pvscroll.cur" => Some(PVSCROLL_CUR),
        _ => None,
    }
}

/// Returns the text content of `pledit.txt`.
pub const fn pledit_str() -> &'static str {
    PLEDIT_TXT
}

/// Returns the text content of `region.txt`.
pub const fn region_str() -> &'static str {
    REGION_TXT
}

/// Returns the text content of `viscolor.txt`.
pub const fn viscolor_str() -> &'static str {
    VISCOLOR_TXT
}

/// Builds a [`MemorySource`] holding all 32 classic entries plus `nums_ex.bmp`
/// of the Rust-themed skin.
pub fn source() -> MemorySource {
    let mut source = MemorySource::new("built-in default (Rust theme) skin");

    // All sheets
    for &sheet in Sheet::ALL {
        if let Some(bytes) = sheet_bytes(sheet) {
            source.add(sheet.file_name(), bytes.to_vec());
        }
    }

    // All cursors
    for (name, bytes) in [
        ("normal.cur", NORMAL_CUR),
        ("pnormal.cur", PNORMAL_CUR),
        ("close.cur", CLOSE_CUR),
        ("titlebar.cur", TITLEBAR_CUR),
        ("posbar.cur", POSBAR_CUR),
        ("volbal.cur", VOLBAL_CUR),
        ("eqslid.cur", EQSLID_CUR),
        ("eqtitle.cur", EQTITLE_CUR),
        ("psize.cur", PSIZE_CUR),
        ("ptbar.cur", PTBAR_CUR),
        ("pvscroll.cur", PVSCROLL_CUR),
    ] {
        source.add(name, bytes.to_vec());
    }

    // All text files
    source.add("pledit.txt", PLEDIT_TXT.as_bytes().to_vec());
    source.add("region.txt", REGION_TXT.as_bytes().to_vec());
    source.add("viscolor.txt", VISCOLOR_TXT.as_bytes().to_vec());

    source
}

/// Loads the complete built-in Rust-themed skin.
pub fn builtin_skin() -> crate::error::Result<crate::skin::Skin> {
    let mut s = source();
    crate::skin::Skin::from_source(&mut s, &crate::skin::BuiltinDefaultSkin)
}
