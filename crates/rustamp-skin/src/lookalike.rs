// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Shipped lookalike default skin assets and raster decoder.
//!
//! Procedurally generated in `crates/rustamp-skin/generator/generate_skin.py`.
//! No original Winamp assets are copied or referenced.

use crate::raster::RasterImage;
use crate::webamp::Sheet;

const MAIN_BMP: &[u8] = include_bytes!("../assets/lookalike/main.bmp");
const TITLEBAR_BMP: &[u8] = include_bytes!("../assets/lookalike/titlebar.bmp");
const CBUTTONS_BMP: &[u8] = include_bytes!("../assets/lookalike/cbuttons.bmp");
const SHUFREP_BMP: &[u8] = include_bytes!("../assets/lookalike/shufrep.bmp");
const POSBAR_BMP: &[u8] = include_bytes!("../assets/lookalike/posbar.bmp");
const VOLUME_BMP: &[u8] = include_bytes!("../assets/lookalike/volume.bmp");
const BALANCE_BMP: &[u8] = include_bytes!("../assets/lookalike/balance.bmp");
const MONOSTER_BMP: &[u8] = include_bytes!("../assets/lookalike/monoster.bmp");
const PLAYPAUS_BMP: &[u8] = include_bytes!("../assets/lookalike/playpaus.bmp");
const NUMBERS_BMP: &[u8] = include_bytes!("../assets/lookalike/numbers.bmp");
const NUMS_EX_BMP: &[u8] = include_bytes!("../assets/lookalike/nums_ex.bmp");
const TEXT_BMP: &[u8] = include_bytes!("../assets/lookalike/text.bmp");
const PLEDIT_BMP: &[u8] = include_bytes!("../assets/lookalike/pledit.bmp");
const EQMAIN_BMP: &[u8] = include_bytes!("../assets/lookalike/eqmain.bmp");
const EQ_EX_BMP: &[u8] = include_bytes!("../assets/lookalike/eq_ex.bmp");
const GEN_BMP: &[u8] = include_bytes!("../assets/lookalike/gen.bmp");

/// Returns the embedded raw BMP file bytes for `sheet`.
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
        _ => None,
    }
}

/// Decodes and returns the raster image for `sheet` from the lookalike skin.
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
                "error decoding built-in skin sheet {}: {err}",
                sheet.file_name()
            );
            None
        }
    }
}

/// Builds a [`crate::source::MemorySource`] holding the look-alike skin sheets.
pub fn source() -> crate::source::MemorySource {
    let mut source = crate::source::MemorySource::new("built-in lookalike skin");
    for &sheet in &[
        Sheet::Main,
        Sheet::Titlebar,
        Sheet::Cbuttons,
        Sheet::Shufrep,
        Sheet::Posbar,
        Sheet::Volume,
        Sheet::Balance,
        Sheet::Monoster,
        Sheet::Playpaus,
        Sheet::Numbers,
        Sheet::NumsEx,
        Sheet::Text,
        Sheet::Pledit,
        Sheet::EqMain,
        Sheet::EqEx,
        Sheet::Gen,
    ] {
        if let Some(bytes) = sheet_bytes(sheet) {
            source.add(sheet.file_name(), bytes.to_vec());
        }
    }
    source
}
