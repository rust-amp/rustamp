//! Shared helpers for the skin loader integration tests.
//!
//! Everything here builds synthetic skins in memory or in a temporary folder.
//! No real skin is ever committed to the repository.

#![allow(dead_code)]

use std::io::Write;
use std::path::Path;

/// Builds an uncompressed 24-bit BMP with the given top-down RGB rows.
pub fn bmp24(width: u32, height: u32, rows_top_down: &[u8]) -> Vec<u8> {
    assert_eq!(rows_top_down.len() as u32, width * height * 3);
    let row_bytes = (width as usize * 3).div_ceil(4) * 4;
    let mut pixels = vec![0u8; row_bytes * height as usize];
    for y in 0..height as usize {
        // BMP rows are stored bottom-up.
        let src_row = &rows_top_down[y * width as usize * 3..(y + 1) * width as usize * 3];
        let dst_y = height as usize - 1 - y;
        for x in 0..width as usize {
            let dst = dst_y * row_bytes + x * 3;
            pixels[dst] = src_row[x * 3 + 2]; // B
            pixels[dst + 1] = src_row[x * 3 + 1]; // G
            pixels[dst + 2] = src_row[x * 3]; // R
        }
    }
    wrap_bmp(width, height, 24, &pixels, &[], 0)
}

/// Builds a solid 24-bit BMP.
pub fn bmp24_solid(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut rows = Vec::with_capacity(width as usize * height as usize * 3);
    for _ in 0..width as usize * height as usize {
        rows.extend_from_slice(&rgb);
    }
    bmp24(width, height, &rows)
}

/// Builds a patterned 24-bit BMP: `(x, y, x ^ y)` per pixel.
pub fn bmp24_pattern(width: u32, height: u32) -> Vec<u8> {
    let mut rows = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        for x in 0..width {
            rows.extend_from_slice(&[x as u8, y as u8, (x ^ y) as u8]);
        }
    }
    bmp24(width, height, &rows)
}

/// Builds an 8-bit palettised BMP from top-down palette indices.
pub fn bmp8(width: u32, height: u32, indices: &[u8], palette: &[[u8; 3]]) -> Vec<u8> {
    assert_eq!(indices.len() as u32, width * height);
    let row_bytes = (width as usize).div_ceil(4) * 4;
    let mut pixels = vec![0u8; row_bytes * height as usize];
    for y in 0..height as usize {
        let dst_y = height as usize - 1 - y;
        for x in 0..width as usize {
            pixels[dst_y * row_bytes + x] = indices[y * width as usize + x];
        }
    }
    let palette_bytes: Vec<u8> = palette.iter().flat_map(|c| [c[2], c[1], c[0], 0]).collect();
    wrap_bmp(
        width,
        height,
        8,
        &pixels,
        &palette_bytes,
        palette.len() as u32,
    )
}

fn wrap_bmp(
    width: u32,
    height: u32,
    bit_depth: u16,
    pixels: &[u8],
    palette: &[u8],
    clr_used: u32,
) -> Vec<u8> {
    let dib = 40u32;
    let palette_offset = 14 + dib as usize;
    let pixel_offset = palette_offset + palette.len();
    let file_size = (pixel_offset + pixels.len()) as u32;
    let mut out = vec![0u8; pixel_offset];
    out[0] = b'B';
    out[1] = b'M';
    out[2..6].copy_from_slice(&file_size.to_le_bytes());
    out[10..14].copy_from_slice(&(pixel_offset as u32).to_le_bytes());
    out[14..18].copy_from_slice(&dib.to_le_bytes());
    out[18..22].copy_from_slice(&(width as i32).to_le_bytes());
    out[22..26].copy_from_slice(&(height as i32).to_le_bytes());
    out[26..28].copy_from_slice(&1u16.to_le_bytes());
    out[28..30].copy_from_slice(&bit_depth.to_le_bytes());
    out[46..50].copy_from_slice(&clr_used.to_le_bytes());
    out[pixel_offset - palette.len()..pixel_offset].copy_from_slice(palette);
    out.extend_from_slice(pixels);
    out
}

/// Packs `(name, bytes)` entries into a ZIP archive.
pub fn zip_bytes(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in entries {
        writer.start_file(name.clone(), options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// Writes `(name, bytes)` entries into a folder, creating subfolders as needed.
pub fn write_folder(dir: &Path, entries: &[(String, Vec<u8>)]) {
    for (name, bytes) in entries {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }
}

/// The pledit text used by the synthetic skin.
pub const PLEDIT: &str = "[Text]\nNormal=#112233\nCurrent=#445566\nNormalBG=#778899\nSelectedBG=#AABBCC\nFont=Comic Sans MS\n";
/// The viscolor text used by the synthetic skin.
pub const VISCOLOR: &str = "10,20,30\n40,50,60\n";
/// The region text used by the synthetic skin.
pub const REGION: &str = "[Normal]\nNumPoints=4\nPointList=0, 0, 275, 0, 275, 116, 0, 116\n[WindowShade]\nNumPoints=4\nPointList=0,0,275,0,275,14,0,14\n";

/// Builds the complete synthetic skin as a list of entries.
pub fn synthetic_entries() -> Vec<(String, Vec<u8>)> {
    let mut entries = Vec::new();
    let mut add = |name: &str, bytes: Vec<u8>| entries.push((name.to_string(), bytes));

    add("main.bmp", bmp24_pattern(275, 116));
    add("titlebar.bmp", bmp24_solid(344, 87, [1, 2, 3]));
    add("cbuttons.bmp", bmp24_solid(136, 36, [4, 5, 6]));
    add("shufrep.bmp", bmp24_solid(92, 85, [7, 8, 9]));
    add("posbar.bmp", bmp24_solid(307, 10, [10, 11, 12]));
    add("volume.bmp", bmp24_solid(68, 433, [13, 14, 15]));
    add("balance.bmp", bmp24_solid(38, 433, [16, 17, 18]));
    add("monoster.bmp", bmp24_solid(56, 24, [19, 20, 21]));
    add("playpaus.bmp", bmp24_solid(42, 9, [22, 23, 24]));
    add("text.bmp", bmp24_solid(155, 18, [25, 26, 27]));
    add("numbers.bmp", bmp24_solid(99, 13, [28, 29, 30]));
    add("pledit.txt", PLEDIT.as_bytes().to_vec());
    add("viscolor.txt", VISCOLOR.as_bytes().to_vec());
    add("region.txt", REGION.as_bytes().to_vec());
    entries
}
