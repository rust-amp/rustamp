//! BMP decoding for classic skin sprite sheets.
//!
//! Supported: Windows `BITMAPINFOHEADER` (and later) and OS/2
//! `BITMAPCOREHEADER`, at 1, 4, 8, 24 and 32 bits per pixel, plus the 16-bit
//! `BI_RGB` (5-5-5) images that some real skins ship, uncompressed and 8-bit
//! RLE. Bitfield-compressed images and 4-bit RLE are rejected with a clear
//! reason rather than guessed at.
//!
//! The decoder works from the palette in the file; it never invents colours.
//! For 32-bit images it uses the stored alpha channel, except when the whole
//! alpha channel is zero, which classic tools write for "opaque" (a quirk that
//! spec S17 must confirm).

/// A decoded BMP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedBmp {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA8 pixels, row-major, top row first.
    pub rgba: Vec<u8>,
}

/// The largest image the decoder will allocate, in pixels. Corrupt or hostile
/// headers cannot make us allocate more than this.
const MAX_PIXELS: u64 = 16 * 1024 * 1024;

/// Decodes a BMP file. The error string explains what was wrong; it never
/// panics on malformed input.
pub fn decode(bytes: &[u8]) -> Result<DecodedBmp, String> {
    if bytes.len() < 18 {
        return Err("file shorter than a BMP header".into());
    }
    if &bytes[0..2] != b"BM" {
        return Err("missing BM signature".into());
    }

    let pixel_offset = read_u32(bytes, 10)? as usize;
    let dib_size = read_u32(bytes, 14)?;

    let header = match dib_size {
        12 => Header::parse_core(bytes)?,
        size if size >= 40 => Header::parse_info(bytes)?,
        other => return Err(format!("unsupported DIB header size {other}")),
    };

    if header.width == 0 || header.height == 0 {
        return Err("zero-sized image".into());
    }
    if header.width > u16::MAX as u32 * 4 || header.height > u16::MAX as u32 * 4 {
        return Err("implausibly large image".into());
    }
    let pixel_count = header.width as u64 * header.height as u64;
    if pixel_count > MAX_PIXELS {
        return Err("image too large".into());
    }

    let palette = read_palette(bytes, &header, pixel_offset)?;
    if pixel_offset > bytes.len() {
        return Err("pixel data offset past end of file".into());
    }
    let data = &bytes[pixel_offset..];

    let mut rgba = vec![0u8; pixel_count as usize * 4];
    let top_down = header.top_down;

    match header.compression {
        0 => decode_uncompressed(&header, data, &palette, top_down, &mut rgba)?,
        1 => decode_rle8(&header, data, &palette, top_down, &mut rgba)?,
        2 => return Err("4-bit RLE (BI_RLE4) is not supported".into()),
        3 => return Err("bitfield compression (BI_BITFIELDS) is not supported".into()),
        other => return Err(format!("unsupported compression {other}")),
    }

    Ok(DecodedBmp {
        width: header.width,
        height: header.height,
        rgba,
    })
}

#[derive(Debug)]
struct Header {
    width: u32,
    height: u32,
    bpp: u16,
    compression: u32,
    clr_used: u32,
    palette_offset: usize,
    palette_entry_size: usize,
    /// True when the rows are stored top-down (negative height).
    top_down: bool,
}

impl Header {
    fn parse_core(bytes: &[u8]) -> Result<Header, String> {
        let width = read_u16(bytes, 18)? as i32;
        let height = read_u16(bytes, 20)? as i32;
        let bpp = read_u16(bytes, 24)?;
        Ok(Header {
            width: u32::try_from(width).map_err(|_| "negative core width")?,
            height: u32::try_from(height).map_err(|_| "negative core height")?,
            bpp,
            compression: 0,
            clr_used: 0,
            palette_offset: 14 + 12,
            palette_entry_size: 3,
            top_down: false,
        })
    }

    fn parse_info(bytes: &[u8]) -> Result<Header, String> {
        let dib_size = read_u32(bytes, 14)? as usize;
        let width = read_i32(bytes, 18)?;
        let height = read_i32(bytes, 22)?;
        let bpp = read_u16(bytes, 28)?;
        let compression = read_u32(bytes, 30)?;
        let clr_used = read_u32(bytes, 46)?;
        Ok(Header {
            width: u32::try_from(width).map_err(|_| "negative width")?,
            height: height.unsigned_abs(),
            bpp,
            compression,
            clr_used,
            palette_offset: 14 + dib_size,
            palette_entry_size: 4,
            top_down: height < 0,
        })
    }
}

/// Reads the palette as RGB triples.
///
/// The count is clamped to what actually fits before the pixel data, so a file
/// that declares a 256-entry palette but stores fewer entries still decodes
/// (missing entries read as black) instead of failing.
fn read_palette(
    bytes: &[u8],
    header: &Header,
    pixel_offset: usize,
) -> Result<Vec<[u8; 3]>, String> {
    let count = match header.bpp {
        1 => 2,
        4 => {
            let n = if header.clr_used == 0 {
                16
            } else {
                header.clr_used
            };
            n.min(16) as usize
        }
        8 => {
            let n = if header.clr_used == 0 {
                256
            } else {
                header.clr_used
            };
            n.min(256) as usize
        }
        _ => 0,
    };

    let available_end = pixel_offset.min(bytes.len());
    let max_entries =
        available_end.saturating_sub(header.palette_offset) / header.palette_entry_size;
    let count = count.min(max_entries);

    let mut palette = Vec::with_capacity(count);
    for i in 0..count {
        let base = header.palette_offset + i * header.palette_entry_size;
        if base + header.palette_entry_size > bytes.len() {
            break;
        }
        palette.push([bytes[base + 2], bytes[base + 1], bytes[base]]);
    }
    Ok(palette)
}

fn decode_uncompressed(
    header: &Header,
    data: &[u8],
    palette: &[[u8; 3]],
    top_down: bool,
    out: &mut [u8],
) -> Result<(), String> {
    let width = header.width as usize;
    let height = header.height as usize;
    let bpp = header.bpp as usize;

    if !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) {
        return Err(format!("unsupported bit depth {bpp}"));
    }

    let row_bytes = (bpp * width).div_ceil(32) * 4;
    let needed = row_bytes.checked_mul(height).ok_or("row size overflow")?;
    if data.len() < needed {
        return Err("truncated pixel data".into());
    }

    for row in 0..height {
        let src_row = &data[row * row_bytes..(row + 1) * row_bytes];
        let dst_y = if top_down { row } else { height - 1 - row };
        let dst_row = &mut out[dst_y * width * 4..(dst_y + 1) * width * 4];
        match bpp {
            1 | 4 | 8 => decode_indexed_row(bpp, src_row, palette, dst_row),
            16 => decode_rgb555(width, src_row, dst_row),
            24 => decode_bgr24(width, src_row, dst_row),
            32 => decode_bgra32(width, src_row, dst_row)?,
            _ => unreachable!(),
        }
    }
    Ok(())
}

fn decode_indexed_row(bpp: usize, src: &[u8], palette: &[[u8; 3]], dst: &mut [u8]) {
    for (x, pixel) in dst.chunks_exact_mut(4).enumerate() {
        let index = match bpp {
            1 => (src[x / 8] >> (7 - (x % 8))) & 1,
            4 => (src[x / 2] >> (if x % 2 == 0 { 4 } else { 0 })) & 0xF,
            8 => src[x],
            _ => 0,
        } as usize;
        let rgb = palette.get(index).copied().unwrap_or([0, 0, 0]);
        pixel.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
}

fn decode_rgb555(width: usize, src: &[u8], dst: &mut [u8]) {
    // BI_RGB at 16 bpp is the documented 5-5-5 layout (X1R5G5B5).
    let expand = |v: u8| (v << 3) | (v >> 2);
    for (x, pixel) in dst.chunks_exact_mut(4).take(width).enumerate() {
        let value = u16::from_le_bytes([src[x * 2], src[x * 2 + 1]]);
        let r = expand(((value >> 10) & 0x1F) as u8);
        let g = expand(((value >> 5) & 0x1F) as u8);
        let b = expand((value & 0x1F) as u8);
        pixel.copy_from_slice(&[r, g, b, 255]);
    }
}

fn decode_bgr24(width: usize, src: &[u8], dst: &mut [u8]) {
    for (x, pixel) in dst.chunks_exact_mut(4).take(width).enumerate() {
        let b = src[x * 3];
        let g = src[x * 3 + 1];
        let r = src[x * 3 + 2];
        pixel.copy_from_slice(&[r, g, b, 255]);
    }
}

fn decode_bgra32(width: usize, src: &[u8], dst: &mut [u8]) -> Result<(), String> {
    // First pass: does the image use its alpha channel at all?
    let any_alpha = (0..width).any(|x| src[x * 4 + 3] != 0);
    for (x, pixel) in dst.chunks_exact_mut(4).take(width).enumerate() {
        let b = src[x * 4];
        let g = src[x * 4 + 1];
        let r = src[x * 4 + 2];
        let a = if any_alpha { src[x * 4 + 3] } else { 255 };
        pixel.copy_from_slice(&[r, g, b, a]);
    }
    Ok(())
}

fn decode_rle8(
    header: &Header,
    data: &[u8],
    palette: &[[u8; 3]],
    top_down: bool,
    out: &mut [u8],
) -> Result<(), String> {
    if header.bpp != 8 {
        return Err("BI_RLE8 requires 8 bits per pixel".into());
    }
    let width = header.width as i32;
    let height = header.height as i32;

    let mut x = 0i32;
    let mut y = if top_down { 0 } else { height - 1 };
    let mut p = 0usize;

    let set = |x: i32, y: i32, index: u8, out: &mut [u8]| {
        if x < 0 || y < 0 || x >= width || y >= height {
            return;
        }
        let rgb = palette.get(index as usize).copied().unwrap_or([0, 0, 0]);
        let i = ((y as usize * width as usize) + x as usize) * 4;
        out[i..i + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    };

    while p + 1 < data.len() {
        let count = data[p];
        let value = data[p + 1];
        p += 2;
        if count > 0 {
            for _ in 0..count {
                set(x, y, value, out);
                x += 1;
            }
            continue;
        }
        match value {
            0 => {
                // End of line.
                x = 0;
                y += if top_down { 1 } else { -1 };
            }
            1 => break, // End of bitmap.
            2 => {
                if p + 1 >= data.len() {
                    return Err("truncated RLE delta".into());
                }
                let dx = data[p] as i32;
                let dy = data[p + 1] as i32;
                p += 2;
                x += dx;
                y += if top_down { dy } else { -dy };
            }
            n => {
                // Absolute run of `n` palette indices, padded to a word.
                let n_usize = n as usize;
                if p + n_usize > data.len() {
                    return Err("truncated RLE absolute run".into());
                }
                for i in 0..n_usize {
                    set(x, y, data[p + i], out);
                    x += 1;
                }
                p += n_usize;
                if n_usize % 2 == 1 {
                    p += 1;
                }
            }
        }
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let slice = bytes.get(offset..offset + 2).ok_or("truncated u16")?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes.get(offset..offset + 4).ok_or("truncated u32")?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, String> {
    read_u32(bytes, offset).map(|v| v as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an uncompressed BMP with the given bit depth and pixel data.
    fn build(bit_depth: u16, width: u32, height: u32, payload: &[u8], palette: &[u8]) -> Vec<u8> {
        let dib = 40u32;
        let palette_offset = 14 + dib as usize;
        let pixel_offset = palette_offset + palette.len();
        let mut out = vec![0u8; pixel_offset];
        out[0] = b'B';
        out[1] = b'M';
        let file_size = (pixel_offset + payload.len()) as u32;
        out[2..6].copy_from_slice(&file_size.to_le_bytes());
        out[10..14].copy_from_slice(&(pixel_offset as u32).to_le_bytes());
        out[14..18].copy_from_slice(&dib.to_le_bytes());
        out[18..22].copy_from_slice(&(width as i32).to_le_bytes());
        out[22..26].copy_from_slice(&(height as i32).to_le_bytes());
        out[26..28].copy_from_slice(&1u16.to_le_bytes()); // planes
        out[28..30].copy_from_slice(&bit_depth.to_le_bytes());
        out[46..50].copy_from_slice(&((palette.len() / 4) as u32).to_le_bytes());
        out = out[..pixel_offset].to_vec();
        out[pixel_offset - palette.len()..pixel_offset].copy_from_slice(palette);
        out.extend_from_slice(payload);
        out
    }

    fn palette_bgra(colors: &[[u8; 3]]) -> Vec<u8> {
        let mut v = Vec::new();
        for c in colors {
            v.extend_from_slice(&[c[2], c[1], c[0], 0]);
        }
        v
    }

    #[test]
    fn decode_24bpp() {
        // Two pixels, bottom-up: row is BGR.
        let payload = [0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00];
        let bmp = build(24, 2, 1, &payload, &[]);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[0..4], [0, 255, 0, 255]);
        assert_eq!(decoded.rgba[4..8], [255, 0, 0, 255]);
    }

    #[test]
    fn decode_32bpp_uses_alpha() {
        let payload = [10, 20, 30, 255, 40, 50, 60, 128];
        let bmp = build(32, 2, 1, &payload, &[]);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[0..4], [30, 20, 10, 255]);
        assert_eq!(decoded.rgba[4..8], [60, 50, 40, 128]);
    }

    #[test]
    fn decode_32bpp_zero_alpha_is_opaque() {
        let payload = [10, 20, 30, 0, 40, 50, 60, 0];
        let bmp = build(32, 2, 1, &payload, &[]);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[3], 255);
        assert_eq!(decoded.rgba[7], 255);
    }

    #[test]
    fn decode_8bpp_palette() {
        let palette = palette_bgra(&[[255, 0, 0], [0, 255, 0], [0, 0, 255]]);
        let payload = [1, 2, 0, 0]; // 3 pixels + padding to 4 bytes
        let bmp = build(8, 3, 1, &payload, &palette);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[0..4], [0, 255, 0, 255]);
        assert_eq!(decoded.rgba[4..8], [0, 0, 255, 255]);
        assert_eq!(decoded.rgba[8..12], [255, 0, 0, 255]);
    }

    #[test]
    fn decode_4bpp_palette() {
        let palette = palette_bgra(&[
            [1, 0, 0],
            [2, 0, 0],
            [3, 0, 0],
            [4, 0, 0],
            [5, 0, 0],
            [6, 0, 0],
            [7, 0, 0],
            [8, 0, 0],
            [9, 0, 0],
            [10, 0, 0],
            [11, 0, 0],
            [12, 0, 0],
            [13, 0, 0],
            [14, 0, 0],
            [15, 0, 0],
            [16, 0, 0],
        ]);
        // One byte: high nibble = pixel 0 (index 1), low nibble = pixel 1 (index 2).
        let payload = [0x12, 0x00, 0x00, 0x00];
        let bmp = build(4, 2, 1, &payload, &palette);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[0], 2);
        assert_eq!(decoded.rgba[4], 3);
    }

    #[test]
    fn decode_1bpp_palette() {
        let palette = palette_bgra(&[[255, 0, 0], [0, 255, 0]]);
        let payload = [0b1010_0000, 0, 0, 0];
        let bmp = build(1, 4, 1, &payload, &palette);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[0..4], [0, 255, 0, 255]); // bit 7 = 1
        assert_eq!(decoded.rgba[4..8], [255, 0, 0, 255]); // bit 6 = 0
        assert_eq!(decoded.rgba[8..12], [0, 255, 0, 255]);
        assert_eq!(decoded.rgba[12..16], [255, 0, 0, 255]);
    }

    #[test]
    fn decode_bottom_up_order() {
        // 2x2 24bpp, bottom-up: bottom row first in the file. Each row is 6
        // bytes padded to 8.
        let mut payload = Vec::new();
        // bottom row (y=1): red then green
        payload.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]);
        // top row (y=0): blue then white
        payload.extend_from_slice(&[255, 0, 0, 255, 255, 255, 0, 0]);
        let bmp = build(24, 2, 2, &payload, &[]);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[0..4], [0, 0, 255, 255]); // top-left = blue
        assert_eq!(decoded.rgba[4..8], [255, 255, 255, 255]);
        assert_eq!(decoded.rgba[8..12], [255, 0, 0, 255]); // bottom-left = red
        assert_eq!(decoded.rgba[12..16], [0, 255, 0, 255]);
    }

    #[test]
    fn decode_rle8() {
        let palette = palette_bgra(&[[255, 0, 0], [0, 255, 0]]);
        // Header says compression 1.
        let mut bmp = build(8, 4, 1, &[], &palette);
        bmp[30..34].copy_from_slice(&1u32.to_le_bytes());
        // Encoded run: 2 pixels of index 1, then end of line, end of bitmap.
        bmp.extend_from_slice(&[2, 1, 0, 0, 0, 1]);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[0..4], [0, 255, 0, 255]);
        assert_eq!(decoded.rgba[4..8], [0, 255, 0, 255]);
        // The remaining pixels stay zero (unset), never panic.
        assert_eq!(decoded.rgba[8..12], [0, 0, 0, 0]);
    }

    #[test]
    fn decode_16bpp_rgb555() {
        // 0x7C00 = red, 0x03E0 = green, 0x001F = blue (plus row padding).
        let payload = [0x00, 0x7C, 0xE0, 0x03, 0x1F, 0x00, 0x00, 0x00];
        let bmp = build(16, 3, 1, &payload, &[]);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[0..4], [255, 0, 0, 255]);
        assert_eq!(decoded.rgba[4..8], [0, 255, 0, 255]);
        assert_eq!(decoded.rgba[8..12], [0, 0, 255, 255]);
    }

    #[test]
    fn reject_garbage() {
        assert!(decode(b"").is_err());
        assert!(decode(b"not a bmp at all").is_err());
        let mut bmp = build(24, 2, 2, &[0; 16], &[]);
        bmp[28..30].copy_from_slice(&12u16.to_le_bytes());
        assert!(decode(&bmp).is_err());
    }
}
