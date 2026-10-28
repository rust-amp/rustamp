//! BMP decoding for classic skin sprite sheets.
//!
//! Supported: Windows `BITMAPINFOHEADER` (and later) and OS/2
//! `BITMAPCOREHEADER`, at 1, 4, 8, 24 and 32 bits per pixel, plus the 16-bit
//! `BI_RGB` (5-5-5) images that some real skins ship, uncompressed, 8-bit
//! RLE, 4-bit RLE and 16/32-bit `BI_BITFIELDS` decoded through the masks the
//! file declares. Compression the decoder does not implement is rejected with
//! a clear reason rather than guessed at.
//!
//! The decoder works from the palette in the file; it never invents colours.
//! For 32-bit images it uses the stored alpha channel, except when the whole
//! alpha channel is zero, which classic tools write for "opaque" (a quirk that
//! spec S17 must confirm). "Whole" means the whole image: the alpha channel is
//! scanned once per image, never per row, so one transparent row cannot make
//! the rest of a sheet opaque.
//!
//! The classic sheets carry **no transparency** (S17 §4: 2.95 copies them
//! opaquely, there is no per-sheet colour key and nothing is masked). A
//! palette index therefore always yields an opaque pixel, and a pixel a
//! run-length stream does not cover is opaque black rather than a hole.

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

    // An uncompressed image is checked against the file *before* the pixel
    // buffer is allocated: a header that claims a huge image must not make
    // the decoder reserve the memory first and fail afterwards.
    if header.compression == 0 {
        check_uncompressed(&header, data)?;
    }
    // The channel masks of a bitfield image are read before the buffer is
    // allocated for the same reason.
    let bit_masks = if header.compression == 3 {
        Some(read_bit_masks(bytes, dib_size)?)
    } else {
        None
    };

    let mut rgba = vec![0u8; pixel_count as usize * 4];
    let top_down = header.top_down;

    match header.compression {
        0 => decode_uncompressed(&header, data, &palette, top_down, &mut rgba)?,
        1 => decode_rle(&header, data, &palette, top_down, &mut rgba, 8)?,
        2 => decode_rle(&header, data, &palette, top_down, &mut rgba, 4)?,
        3 => decode_bitfields(
            &header,
            data,
            bit_masks.ok_or("bitfield compression without masks")?,
            top_down,
            &mut rgba,
        )?,
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
    check_uncompressed(header, data)?;

    let width = header.width as usize;
    let height = header.height as usize;
    let bpp = header.bpp as usize;

    let row_bytes = uncompressed_row_bytes(header);
    // The "all-zero alpha means opaque" quirk is a property of the *image*,
    // so the alpha channel is scanned once here, over every row, and never
    // decided per row: a sheet whose first row is fully transparent but whose
    // body uses alpha is an image that uses its alpha channel.
    let any_alpha = bpp == 32
        && (0..height).any(|row| {
            let src_row = &data[row * row_bytes..(row + 1) * row_bytes];
            (0..width).any(|x| src_row[x * 4 + 3] != 0)
        });
    for row in 0..height {
        let src_row = &data[row * row_bytes..(row + 1) * row_bytes];
        let dst_y = if top_down { row } else { height - 1 - row };
        let dst_row = &mut out[dst_y * width * 4..(dst_y + 1) * width * 4];
        match bpp {
            1 | 4 | 8 => decode_indexed_row(bpp, src_row, palette, dst_row),
            16 => decode_rgb555(width, src_row, dst_row),
            24 => decode_bgr24(width, src_row, dst_row),
            32 => decode_bgra32(width, src_row, dst_row, any_alpha),
            _ => unreachable!(),
        }
    }
    Ok(())
}

/// The number of bytes one row of an uncompressed image occupies.
fn uncompressed_row_bytes(header: &Header) -> usize {
    let bpp = header.bpp as usize;
    (bpp * header.width as usize).div_ceil(32) * 4
}

/// Checks that an uncompressed image of `header` fits into `data`.
///
/// The bit depth is checked first, so a file with an unsupported depth is
/// reported as such even when its pixel data is short as well.
fn check_uncompressed(header: &Header, data: &[u8]) -> Result<(), String> {
    let bpp = header.bpp as usize;
    if !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) {
        return Err(format!("unsupported bit depth {bpp}"));
    }
    let row_bytes = uncompressed_row_bytes(header);
    let needed = row_bytes
        .checked_mul(header.height as usize)
        .ok_or("row size overflow")?;
    if data.len() < needed {
        return Err("truncated pixel data".into());
    }
    Ok(())
}

fn decode_indexed_row(bpp: usize, src: &[u8], palette: &[[u8; 3]], dst: &mut [u8]) {
    for (x, pixel) in dst.as_chunks_mut::<4>().0.iter_mut().enumerate() {
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
    for (x, pixel) in dst
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .take(width)
        .enumerate()
    {
        let value = u16::from_le_bytes([src[x * 2], src[x * 2 + 1]]);
        let r = expand(((value >> 10) & 0x1F) as u8);
        let g = expand(((value >> 5) & 0x1F) as u8);
        let b = expand((value & 0x1F) as u8);
        pixel.copy_from_slice(&[r, g, b, 255]);
    }
}

fn decode_bgr24(width: usize, src: &[u8], dst: &mut [u8]) {
    for (x, pixel) in dst
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .take(width)
        .enumerate()
    {
        let b = src[x * 3];
        let g = src[x * 3 + 1];
        let r = src[x * 3 + 2];
        pixel.copy_from_slice(&[r, g, b, 255]);
    }
}

/// Decodes one row of an uncompressed 32-bit image.
///
/// `any_alpha` carries the **per-image** decision whether the alpha channel is
/// in use (see [`decode_uncompressed`]). A classic tool that writes an all-zero
/// alpha channel means "opaque", so a row whose alpha bytes are all zero is
/// opaque only when the whole image's alpha channel is zero.
fn decode_bgra32(width: usize, src: &[u8], dst: &mut [u8], any_alpha: bool) {
    for (x, pixel) in dst
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .take(width)
        .enumerate()
    {
        let b = src[x * 4];
        let g = src[x * 4 + 1];
        let r = src[x * 4 + 2];
        let a = if any_alpha { src[x * 4 + 3] } else { 255 };
        pixel.copy_from_slice(&[r, g, b, a]);
    }
}

/// Decodes a run-length stream of `bpp`-bit palette indices.
///
/// `bpp` is 8 (`BI_RLE8`: one index per byte) or 4 (`BI_RLE4`: two indices per
/// byte, the high nibble first). Both encodings share the grammar — encoded
/// runs, absolute runs padded to a word, end-of-line, end-of-bitmap and delta —
/// and differ only in how a run's value bytes are unpacked.
fn decode_rle(
    header: &Header,
    data: &[u8],
    palette: &[[u8; 3]],
    top_down: bool,
    out: &mut [u8],
    bpp: usize,
) -> Result<(), String> {
    if header.bpp as usize != bpp {
        return Err(format!("BI_RLE{bpp} requires {bpp} bits per pixel"));
    }
    let width = header.width as i32;
    let height = header.height as i32;

    // S17 §4: the classic sheets are copied opaquely, there is no per-sheet
    // transparency colour key and nothing is masked. A pixel the run-length
    // stream does not cover is therefore opaque black, not a transparent hole.
    for pixel in out.as_chunks_mut::<4>().0.iter_mut() {
        pixel.copy_from_slice(&[0, 0, 0, 255]);
    }

    // The `i`-th index of an absolute run.
    let absolute_index = |run: &[u8], i: usize| -> u8 {
        if bpp == 8 {
            run[i]
        } else {
            let byte = run[i / 2];
            if i.is_multiple_of(2) {
                byte >> 4
            } else {
                byte & 0x0F
            }
        }
    };

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
            for i in 0..count as usize {
                let index = if bpp == 8 {
                    value
                } else if i.is_multiple_of(2) {
                    value >> 4
                } else {
                    value & 0x0F
                };
                set(x, y, index, out);
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
                // Absolute run of `n` palette indices, two per byte at 4 bits,
                // and the bytes are padded to a word.
                let n_usize = n as usize;
                let bytes = if bpp == 8 {
                    n_usize
                } else {
                    n_usize.div_ceil(2)
                };
                if p + bytes > data.len() {
                    return Err("truncated RLE absolute run".into());
                }
                for i in 0..n_usize {
                    set(x, y, absolute_index(&data[p..], i), out);
                    x += 1;
                }
                p += bytes;
                if !bytes.is_multiple_of(2) {
                    p += 1;
                }
            }
        }
    }
    Ok(())
}

/// The channel masks of a `BI_BITFIELDS` image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BitMasks {
    red: u32,
    green: u32,
    blue: u32,
    /// Zero when the file declares no alpha mask.
    alpha: u32,
}

/// Reads the channel masks of a `BI_BITFIELDS` image.
///
/// A `BITMAPINFOHEADER` stores three `DWORD` colour masks directly after the
/// header; a `BITMAPV4HEADER` or later stores them *inside* the header, plus
/// the alpha mask those headers add. Both start at DIB offset 40, which is
/// file offset 54 (the 14-byte file header precedes the DIB). Reading the
/// alpha mask is only valid for the V4/V5 headers (`dib_size >= 52`).
fn read_bit_masks(bytes: &[u8], dib_size: u32) -> Result<BitMasks, String> {
    let red = read_u32(bytes, 54)?;
    let green = read_u32(bytes, 58)?;
    let blue = read_u32(bytes, 62)?;
    if red | green | blue == 0 {
        return Err("bitfield image declares no colour masks".into());
    }
    let alpha = if dib_size >= 52 {
        read_u32(bytes, 66)?
    } else {
        0
    };
    Ok(BitMasks {
        red,
        green,
        blue,
        alpha,
    })
}

/// Expands the bits `mask` selects out of a packed pixel value to 8 bits.
fn bitfield_channel(value: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = u64::from(mask >> shift);
    let raw = u64::from((value & mask) >> shift);
    // Round to the nearest 8-bit level, so a full 5-bit 31 still maps to 255.
    ((raw * 255 + max / 2) / max) as u8
}

/// Decodes a 16- or 32-bit `BI_BITFIELDS` image through its channel masks.
fn decode_bitfields(
    header: &Header,
    data: &[u8],
    masks: BitMasks,
    top_down: bool,
    out: &mut [u8],
) -> Result<(), String> {
    let bpp = header.bpp as usize;
    if !matches!(bpp, 16 | 32) {
        return Err(format!(
            "bitfield compression requires 16 or 32 bits per pixel, not {bpp}"
        ));
    }
    let width = header.width as usize;
    let height = header.height as usize;
    let row_bytes = uncompressed_row_bytes(header);
    let needed = row_bytes.checked_mul(height).ok_or("row size overflow")?;
    if data.len() < needed {
        return Err("truncated bitfield pixel data".into());
    }

    let mut any_alpha = false;
    for row in 0..height {
        let src_row = &data[row * row_bytes..(row + 1) * row_bytes];
        let dst_y = if top_down { row } else { height - 1 - row };
        let dst_row = &mut out[dst_y * width * 4..(dst_y + 1) * width * 4];
        for (x, pixel) in dst_row
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .take(width)
            .enumerate()
        {
            let value = if bpp == 16 {
                u32::from(u16::from_le_bytes([src_row[x * 2], src_row[x * 2 + 1]]))
            } else {
                u32::from_le_bytes([
                    src_row[x * 4],
                    src_row[x * 4 + 1],
                    src_row[x * 4 + 2],
                    src_row[x * 4 + 3],
                ])
            };
            let alpha = if masks.alpha == 0 {
                255
            } else {
                bitfield_channel(value, masks.alpha)
            };
            any_alpha |= alpha != 0;
            pixel.copy_from_slice(&[
                bitfield_channel(value, masks.red),
                bitfield_channel(value, masks.green),
                bitfield_channel(value, masks.blue),
                alpha,
            ]);
        }
    }
    if masks.alpha != 0 && !any_alpha {
        // The same quirk an uncompressed 32-bit image follows: an alpha
        // channel that is zero everywhere means "opaque", not "invisible".
        for pixel in out.as_chunks_mut::<4>().0.iter_mut() {
            pixel[3] = 255;
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

    /// The "all-zero alpha means opaque" decision belongs to the whole image,
    /// not to one row: a sheet whose first row is fully transparent but whose
    /// body uses alpha must keep its transparency. Deciding per row turned the
    /// transparent row opaque and left the rest of the sheet half-transparent
    /// (the caller's bug: one row's answer applied to every row).
    #[test]
    fn decode_32bpp_alpha_is_decided_per_image() {
        // 2x2, bottom-up: the first row in the file (the bottom row of the
        // image) is fully transparent, the second row uses alpha 255.
        let payload = [
            10, 20, 30, 0, 40, 50, 60, 0, // file row 0 = image row 1, alpha 0
            70, 80, 90, 255, 100, 110, 120, 255, // file row 1 = image row 0
        ];
        let bmp = build(32, 2, 2, &payload, &[]);
        let decoded = decode(&bmp).unwrap();
        // The bottom row (image y = 1) stays transparent, it is not made
        // opaque by its own all-zero alpha.
        assert_eq!(decoded.rgba[8..12], [30, 20, 10, 0]);
        assert_eq!(decoded.rgba[12..16], [60, 50, 40, 0]);
        // The top row (image y = 0) keeps alpha 255.
        assert_eq!(decoded.rgba[0..4], [90, 80, 70, 255]);
        assert_eq!(decoded.rgba[4..8], [120, 110, 100, 255]);
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
        // The remaining pixels stay opaque black (S17 §4: the classic sheets
        // carry no transparency), never a transparent hole.
        assert_eq!(decoded.rgba[8..12], [0, 0, 0, 255]);
        assert_eq!(decoded.rgba[12..16], [0, 0, 0, 255]);
    }

    /// S17 §4: a run-length stream that leaves a row out must not punch a
    /// transparent hole into the sheet. The base skin's `text.bmp` is such a
    /// file: it encodes 73 of its 74 rows.
    #[test]
    fn rle8_leaves_uncovered_pixels_opaque() {
        let palette = palette_bgra(&[[255, 0, 0], [0, 255, 0]]);
        let mut bmp = build(8, 2, 2, &[], &palette);
        bmp[30..34].copy_from_slice(&1u32.to_le_bytes());
        // One row of two green pixels, then end of bitmap: the second row is
        // never encoded. The rows of a run-length stream run from the bottom
        // of the image upwards, so the encoded row is the bottom one.
        bmp.extend_from_slice(&[2, 1, 0, 1]);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[8..12], [0, 255, 0, 255]);
        assert_eq!(decoded.rgba[12..16], [0, 255, 0, 255]);
        // The row that was never encoded is opaque black.
        assert_eq!(decoded.rgba[0..4], [0, 0, 0, 255]);
        assert_eq!(decoded.rgba[4..8], [0, 0, 0, 255]);
        // No pixel of the sheet is transparent.
        assert!(
            decoded.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
            "a classic sheet must not contain transparent pixels"
        );
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
    fn decode_rle4() {
        let palette = palette_bgra(&[
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 0],
            [0, 255, 255],
        ]);
        let mut bmp = build(4, 5, 1, &[], &palette);
        bmp[30..34].copy_from_slice(&2u32.to_le_bytes());
        // An encoded run of two pixels (high nibble first: indices 1 and 2),
        // then an absolute run of three pixels (0, 3 and 2) packed two per
        // byte, then end of line and end of bitmap.
        bmp.extend_from_slice(&[2, 0x12, 0, 3, 0x03, 0x21, 0, 0, 0, 1]);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[0..4], [0, 255, 0, 255]); // index 1
        assert_eq!(decoded.rgba[4..8], [0, 0, 255, 255]); // index 2
        assert_eq!(decoded.rgba[8..12], [255, 0, 0, 255]); // index 0
        assert_eq!(decoded.rgba[12..16], [255, 255, 0, 255]); // index 3
        assert_eq!(decoded.rgba[16..20], [0, 0, 255, 255]); // index 2
    }

    /// S17 §4 applies to 4-bit RLE exactly as it does to 8-bit RLE: a pixel
    /// the stream does not cover is opaque black, never a transparent hole.
    #[test]
    fn rle4_leaves_uncovered_pixels_opaque() {
        let palette = palette_bgra(&[[255, 0, 0], [0, 255, 0]]);
        let mut bmp = build(4, 2, 2, &[], &palette);
        bmp[30..34].copy_from_slice(&2u32.to_le_bytes());
        // Two pixels of index 1 on the bottom row, then end of bitmap.
        bmp.extend_from_slice(&[2, 0x11, 0, 1]);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[8..12], [0, 255, 0, 255]);
        assert_eq!(decoded.rgba[12..16], [0, 255, 0, 255]);
        // The row that was never encoded is opaque black.
        assert_eq!(decoded.rgba[0..4], [0, 0, 0, 255]);
        assert_eq!(decoded.rgba[4..8], [0, 0, 0, 255]);
        assert!(
            decoded.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
            "a classic sheet must not contain transparent pixels"
        );
    }

    #[test]
    fn rle4_needs_four_bits_per_pixel() {
        for bpp in [1u16, 8, 16, 24, 32] {
            let mut image = build(bpp, 4, 1, &[], &palette_bgra(&[[255, 0, 0]]));
            image[30..34].copy_from_slice(&2u32.to_le_bytes());
            assert!(
                decode(&image).is_err(),
                "an RLE4 stream in a {bpp}-bit image must be rejected"
            );
        }
    }

    /// Builds a `BI_BITFIELDS` image. `dib` is 40 for the masks that follow a
    /// `BITMAPINFOHEADER`, or 108 for a V4 header that carries them (and the
    /// alpha mask) itself.
    fn build_bitfields(
        dib: u32,
        bit_depth: u16,
        width: u32,
        height: u32,
        masks: [u32; 3],
        payload: &[u8],
    ) -> Vec<u8> {
        assert!(dib == 40 || dib >= 52);
        let pixel_offset = if dib == 40 {
            14 + 40 + 12
        } else {
            14 + dib as usize
        };
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
        out[30..34].copy_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
        for (i, mask) in masks.iter().enumerate() {
            out[54 + i * 4..58 + i * 4].copy_from_slice(&mask.to_le_bytes());
        }
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn decode_16bpp_bitfields() {
        // 0xF800/0x07E0/0x001F is the classic 5-6-5 layout. The masks sit
        // inside a V4 header here.
        let payload = [0x00, 0xF8, 0x1F, 0x00];
        let bmp = build_bitfields(108, 16, 2, 1, [0xF800, 0x07E0, 0x001F], &payload);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[0..4], [255, 0, 0, 255]);
        assert_eq!(decoded.rgba[4..8], [0, 0, 255, 255]);
    }

    #[test]
    fn decode_16bpp_bitfields_after_a_40_byte_header() {
        // The same masks written after a plain BITMAPINFOHEADER.
        let payload = [0x00, 0xF8, 0x1F, 0x00];
        let bmp = build_bitfields(40, 16, 2, 1, [0xF800, 0x07E0, 0x001F], &payload);
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[0..4], [255, 0, 0, 255]);
        assert_eq!(decoded.rgba[4..8], [0, 0, 255, 255]);
    }

    #[test]
    fn decode_32bpp_bitfields_uses_the_alpha_mask() {
        // The masks are the plain BGRA byte order, plus a V4 alpha mask.
        let masks = [0x00FF_0000, 0x0000_FF00, 0x0000_00FF];
        let payload = [10, 20, 30, 255, 40, 50, 60, 0x80];
        let mut bmp = build_bitfields(108, 32, 2, 1, masks, &payload);
        bmp[66..70].copy_from_slice(&0xFF00_0000u32.to_le_bytes());
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[0..4], [30, 20, 10, 255]);
        assert_eq!(decoded.rgba[4..8], [60, 50, 40, 0x80]);
    }

    /// The all-zero-alpha quirk an uncompressed 32-bit image follows also
    /// applies to a bitfield image whose alpha mask selects zero everywhere.
    #[test]
    fn decode_32bpp_bitfields_zero_alpha_is_opaque() {
        let masks = [0x00FF_0000, 0x0000_FF00, 0x0000_00FF];
        let payload = [10, 20, 30, 0, 40, 50, 60, 0];
        let mut bmp = build_bitfields(108, 32, 2, 1, masks, &payload);
        bmp[66..70].copy_from_slice(&0xFF00_0000u32.to_le_bytes());
        let decoded = decode(&bmp).unwrap();
        assert_eq!(decoded.rgba[3], 255);
        assert_eq!(decoded.rgba[7], 255);
    }

    #[test]
    fn bitfields_needs_sixteen_or_32_bits_per_pixel() {
        let bmp = build_bitfields(
            108,
            24,
            2,
            1,
            [0x00FF_0000, 0x0000_FF00, 0x0000_00FF],
            &[0; 8],
        );
        assert!(decode(&bmp).is_err());
    }

    #[test]
    fn bitfields_without_colour_masks_is_rejected() {
        let bmp = build_bitfields(108, 16, 2, 1, [0, 0, 0], &[0; 4]);
        assert!(decode(&bmp).is_err());
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
