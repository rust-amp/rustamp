// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Property and malformed-input tests for the skin parsers.
//!
//! A skin's files are written by whoever made the skin, so every parser has
//! to survive garbage: a truncated file, a wrong magic, an absurd dimension,
//! a run-length stream that runs past the end of the file, text that is not
//! UTF-8, a missing section, an archive whose `main.bmp` is a directory.
//!
//! Two things are asserted for every such input:
//!
//! * **no panic** — a malformed skin is a bad skin, not a crash;
//! * **the documented fallback** — the parser either fails with a reason or
//!   returns the documented default, never a silently wrong parse.
//!
//! The pure parsers get a few invariants on top: the analyser colours are
//! always 24 entries with the defaults filled in, a region mask never leaves
//! the window, and a cursor is either present with its bytes or absent.
//!
//! The fuzz loops use a seeded generator, so a failure is reproducible and
//! the suite stays fast (a fraction of a second).

use rustamp_skin::bmp;
use rustamp_skin::source::{ArchiveSource, FolderSource, MemorySource};
use rustamp_skin::{
    BuiltinLookalikeSkin, CursorKind, CursorSlot, PlaylistStyle, Region, RegionSection, Sheet,
    Skin, SkinSource, webamp,
};

// ---------------------------------------------------------------------------
// A small deterministic generator
// ---------------------------------------------------------------------------

/// A xorshift64\* generator: deterministic, so every case is reproducible.
struct Rng(u64);

impl Rng {
    /// Creates a generator with the given seed.
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    /// The next 64 random bits.
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A random number below `n` (`n` must not be zero).
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// `len` random bytes.
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next_u64() & 0xFF) as u8).collect()
    }

    /// A random blob of up to `max` bytes.
    fn blob(&mut self, max: usize) -> Vec<u8> {
        let len = self.below(max);
        self.bytes(len)
    }
}

// ---------------------------------------------------------------------------
// BMP builders
// ---------------------------------------------------------------------------

/// A BMP file under construction.
struct Bmp {
    dib_size: u32,
    width: i32,
    height: i32,
    bpp: u16,
    compression: u32,
    clr_used: u32,
    palette: Vec<u8>,
    pixels: Vec<u8>,
    /// Overrides the pixel data offset written into the file.
    pixel_offset: Option<u32>,
}

impl Bmp {
    /// A 4x2 image header at the given bit depth.
    fn new(bpp: u16) -> Bmp {
        Bmp {
            dib_size: 40,
            width: 4,
            height: 2,
            bpp,
            compression: 0,
            clr_used: 0,
            palette: Vec::new(),
            pixels: Vec::new(),
            pixel_offset: None,
        }
    }

    /// A 4x2 image with an `OS/2` core header.
    fn core(bpp: u16) -> Bmp {
        Bmp {
            dib_size: 12,
            ..Bmp::new(bpp)
        }
    }

    /// Assembles the file.
    fn build(&self) -> Vec<u8> {
        let header_len = 14 + self.dib_size as usize;
        let pixel_offset = self
            .pixel_offset
            .unwrap_or((header_len + self.palette.len()) as u32);
        let mut out = vec![0u8; header_len];
        out[0] = b'B';
        out[1] = b'M';
        let file_size = (pixel_offset as usize + self.pixels.len()) as u32;
        out[2..6].copy_from_slice(&file_size.to_le_bytes());
        out[10..14].copy_from_slice(&pixel_offset.to_le_bytes());
        out[14..18].copy_from_slice(&self.dib_size.to_le_bytes());
        if self.dib_size >= 40 {
            out[18..22].copy_from_slice(&self.width.to_le_bytes());
            out[22..26].copy_from_slice(&self.height.to_le_bytes());
            out[26..28].copy_from_slice(&1u16.to_le_bytes());
            out[28..30].copy_from_slice(&self.bpp.to_le_bytes());
            out[30..34].copy_from_slice(&self.compression.to_le_bytes());
            out[46..50].copy_from_slice(&self.clr_used.to_le_bytes());
        } else {
            // `BITMAPCOREHEADER`: width, height, planes, bit count.
            out[18..20].copy_from_slice(&(self.width as u16).to_le_bytes());
            out[20..22].copy_from_slice(&(self.height as u16).to_le_bytes());
            out[22..24].copy_from_slice(&1u16.to_le_bytes());
            out[24..26].copy_from_slice(&self.bpp.to_le_bytes());
        }
        out.extend_from_slice(&self.palette);
        out.extend_from_slice(&self.pixels);
        out
    }
}

/// The palette entries of an indexed image, in the `BGRA` order a BMP stores.
fn palette_bgra(colors: &[[u8; 3]]) -> Vec<u8> {
    colors.iter().flat_map(|c| [c[2], c[1], c[0], 0]).collect()
}

/// The 16 colours a 4-bit image may use.
fn sixteen_colors() -> Vec<[u8; 3]> {
    (0..16u8).map(|i| [i * 16, 255 - i * 16, i * 8]).collect()
}

/// A valid 4x2 image at the given bit depth.
fn valid_image(bpp: u16) -> Vec<u8> {
    let mut bmp = Bmp::new(bpp);
    match bpp {
        1 => {
            bmp.clr_used = 2;
            bmp.palette = palette_bgra(&[[255, 0, 0], [0, 255, 0]]);
            bmp.pixels = vec![0b1010_0000, 0, 0, 0, 0b0110_0000, 0, 0, 0];
        }
        4 => {
            bmp.clr_used = 16;
            bmp.palette = palette_bgra(&sixteen_colors());
            bmp.pixels = vec![0x12, 0x34, 0, 0, 0x56, 0x78, 0, 0];
        }
        8 => {
            bmp.clr_used = 4;
            bmp.palette = palette_bgra(&[[1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12]]);
            bmp.pixels = vec![0, 1, 2, 3, 3, 2, 1, 0];
        }
        16 => bmp.pixels = vec![0; 16],
        24 => bmp.pixels = vec![0; 24],
        32 => bmp.pixels = vec![0; 32],
        other => panic!("no image for {other} bits per pixel"),
    }
    bmp.build()
}

/// A valid 8-bit image whose pixel data is a run-length stream.
fn rle8_image(stream: &[u8]) -> Vec<u8> {
    let mut bmp = Bmp::new(8);
    bmp.compression = 1;
    bmp.clr_used = 2;
    bmp.palette = palette_bgra(&[[255, 0, 0], [0, 255, 0]]);
    bmp.pixels = stream.to_vec();
    bmp.build()
}

/// The decoded image is consistent with its own header.
fn assert_consistent(decoded: &bmp::DecodedBmp) {
    assert!(decoded.width > 0 && decoded.height > 0);
    assert_eq!(
        decoded.rgba.len(),
        decoded.width as usize * decoded.height as usize * 4,
        "the pixel buffer must match the header"
    );
}

// ---------------------------------------------------------------------------
// BMP: malformed input
// ---------------------------------------------------------------------------

/// Every truncation of a valid image is rejected — a sheet that is cut short
/// is a missing sheet, never a half-decoded one.
#[test]
fn every_truncation_of_a_valid_image_is_rejected() {
    for bpp in [1u16, 4, 8, 16, 24, 32] {
        let image = valid_image(bpp);
        for len in 0..image.len() {
            let truncated = &image[..len];
            assert!(
                bmp::decode(truncated).is_err(),
                "a {bpp}-bit image cut to {len} bytes must be rejected"
            );
        }
        // The whole image still decodes.
        let decoded = bmp::decode(&image).expect("the valid image decodes");
        assert_consistent(&decoded);
    }
}

/// A file that is not a BMP is rejected with a reason.
#[test]
fn a_wrong_magic_is_rejected() {
    for magic in [
        b"ZM".as_slice(),
        b"PK\x03\x04".as_slice(),
        b"\x00\x00".as_slice(),
        b"BM".as_slice(),
    ] {
        let mut image = valid_image(24);
        image[0..2].copy_from_slice(&magic[..2]);
        if magic == b"BM" {
            // The magic is right; the file is still complete, so it decodes.
            assert!(bmp::decode(&image).is_ok());
        } else {
            assert!(
                bmp::decode(&image).is_err(),
                "the magic {magic:?} must be rejected"
            );
        }
    }
    assert!(bmp::decode(b"").is_err());
    assert!(bmp::decode(b"BM").is_err());
    assert!(bmp::decode(b"not a bmp at all").is_err());
}

/// A header that claims an impossible image is rejected.
#[test]
fn absurd_dimensions_are_rejected() {
    let cases: Vec<(i32, i32)> = vec![
        (0, 0),
        (0, 10),
        (10, 0),
        (-1, 10),
        (10, -1),
        (i32::MIN, 10),
        (i32::MAX, 1),
        (1, i32::MIN),
        (100_000, 100_000),
        (1 << 20, 1 << 20),
    ];
    for (width, height) in cases {
        let mut image = valid_image(24);
        image[18..22].copy_from_slice(&width.to_le_bytes());
        image[22..26].copy_from_slice(&height.to_le_bytes());
        assert!(
            bmp::decode(&image).is_err(),
            "a {width}x{height} image must be rejected"
        );
    }
}

/// A negative height means a top-down image, not an error.
#[test]
fn a_top_down_image_decodes() {
    let mut image = valid_image(24);
    image[22..26].copy_from_slice(&(-2i32).to_le_bytes());
    let decoded = bmp::decode(&image).expect("a top-down image decodes");
    assert_consistent(&decoded);
    assert_eq!(decoded.height, 2);
}

/// A bit depth or a compression the decoder does not implement is rejected
/// with a reason, never guessed at.
#[test]
fn unsupported_encodings_are_rejected() {
    for bpp in [0u16, 2, 3, 5, 7, 9, 15, 17, 33, 64] {
        let mut image = valid_image(24);
        image[28..30].copy_from_slice(&bpp.to_le_bytes());
        assert!(
            bmp::decode(&image).is_err(),
            "a {bpp}-bit image must be rejected"
        );
    }
    for compression in [2u32, 3] {
        // 4-bit RLE and bitfields are implemented, but only for their own bit
        // depths: in a 24-bit image they are still rejected.
        let mut image = valid_image(24);
        image[30..34].copy_from_slice(&compression.to_le_bytes());
        assert!(
            bmp::decode(&image).is_err(),
            "the compression {compression} must be rejected at 24 bits per pixel"
        );
    }
    for compression in [4u32, 5, 99] {
        let mut image = valid_image(24);
        image[30..34].copy_from_slice(&compression.to_le_bytes());
        assert!(
            bmp::decode(&image).is_err(),
            "the compression {compression} must be rejected"
        );
    }
}

/// A pixel data offset that points past the end of the file is rejected.
#[test]
fn a_pixel_offset_past_the_end_is_rejected() {
    let mut image = valid_image(24);
    image[10..14].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(bmp::decode(&image).is_err());

    let mut image = valid_image(24);
    let past_the_end = image.len() as u32 + 1;
    image[10..14].copy_from_slice(&past_the_end.to_le_bytes());
    assert!(bmp::decode(&image).is_err());
}

/// A palette that is shorter than the header announces is not an error: the
/// missing entries read as black, and the image still decodes.
#[test]
fn a_short_palette_reads_as_black() {
    let mut bmp = Bmp::new(8);
    bmp.clr_used = 256;
    bmp.palette = palette_bgra(&[[255, 0, 0], [0, 255, 0]]);
    // The bottom row of the file is the bottom row of the image.
    bmp.pixels = vec![0, 1, 2, 2, 0, 0, 0, 0];
    let image = bmp.build();
    let decoded = bmp::decode(&image).expect("a short palette still decodes");
    assert_consistent(&decoded);
    // The bottom row of the image (the first row of the file): red, green
    // and two pixels without a palette entry.
    assert_eq!(decoded.rgba[16..20], [255, 0, 0, 255]);
    assert_eq!(decoded.rgba[20..24], [0, 255, 0, 255]);
    assert_eq!(decoded.rgba[24..28], [0, 0, 0, 255]);
    assert_eq!(decoded.rgba[28..32], [0, 0, 0, 255]);
}

/// A run-length stream that runs past the end of the file is rejected.
#[test]
fn an_rle8_stream_that_runs_past_the_end_is_rejected() {
    // An absolute run of 200 pixels with only three bytes left.
    let image = rle8_image(&[0, 200, 1, 2, 3]);
    assert!(bmp::decode(&image).is_err());

    // A delta whose two operands are cut off.
    let image = rle8_image(&[0, 2, 1]);
    assert!(bmp::decode(&image).is_err());
}

/// A run-length stream of an image that is not 8-bit is rejected.
#[test]
fn rle8_needs_eight_bits_per_pixel() {
    for bpp in [1u16, 4, 16, 24, 32] {
        let mut image = valid_image(bpp);
        image[30..34].copy_from_slice(&1u32.to_le_bytes());
        assert!(
            bmp::decode(&image).is_err(),
            "an RLE8 stream in a {bpp}-bit image must be rejected"
        );
    }
}

/// A run-length stream that walks out of the image changes nothing: the
/// pixels outside the image are simply not written.
#[test]
fn an_rle8_stream_outside_the_image_is_ignored() {
    // A delta that moves the cursor far to the right and above the image.
    let image = rle8_image(&[0, 2, 200, 200, 2, 1, 0, 1]);
    let decoded = bmp::decode(&image).expect("a stream out of bounds decodes");
    assert_consistent(&decoded);
    // S17 §4: the classic sheets carry no transparency, so every pixel of a
    // decoded sheet is opaque.
    assert!(
        decoded.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
        "a classic sheet must not contain transparent pixels"
    );
}

/// A run-length stream without an end-of-bitmap marker stops at the end of
/// the file and keeps the pixels it wrote.
#[test]
fn an_rle8_stream_without_an_end_marker_keeps_its_pixels() {
    // Two pixels of index 1 and nothing else: the stream just stops. The
    // rows of a run-length stream run from the bottom of the image upwards.
    let image = rle8_image(&[2, 1]);
    let decoded = bmp::decode(&image).expect("an unterminated stream decodes");
    assert_consistent(&decoded);
    // The bottom row of the image carries the two green pixels.
    assert_eq!(decoded.rgba[16..20], [0, 255, 0, 255]);
    assert_eq!(decoded.rgba[20..24], [0, 255, 0, 255]);
    // The pixels the stream never covered stay opaque black.
    assert_eq!(decoded.rgba[0..4], [0, 0, 0, 255]);
}

/// A mutated image never panics, and what it decodes to is always
/// consistent with its own header.
#[test]
fn a_mutated_image_never_panics() {
    let mut rng = Rng::new(0x5EED_1234);
    for bpp in [1u16, 4, 8, 16, 24, 32] {
        let image = valid_image(bpp);
        for _ in 0..64 {
            let mut mutated = image.clone();
            // A few random byte flips.
            for _ in 0..1 + rng.below(4) {
                let index = rng.below(mutated.len());
                mutated[index] ^= 1 << rng.below(8);
            }
            // And a random truncation.
            let len = rng.below(mutated.len());
            let mutated = &mutated[..len];
            if let Ok(decoded) = bmp::decode(mutated) {
                assert_consistent(&decoded);
            }
        }
    }
}

/// Random bytes are not a BMP and must not panic.
#[test]
fn random_bytes_are_not_a_bmp() {
    let mut rng = Rng::new(0xC0FF_EE00);
    for _ in 0..256 {
        let bytes = rng.blob(128);
        if let Ok(decoded) = bmp::decode(&bytes) {
            assert_consistent(&decoded);
        }
    }
}

/// The `OS/2` core header is read at its own offsets.
#[test]
fn a_core_header_image_decodes() {
    let mut bmp = Bmp::core(24);
    bmp.pixels = vec![0; 24];
    let image = bmp.build();
    let decoded = bmp::decode(&image).expect("a core-header image decodes");
    assert_consistent(&decoded);
    assert_eq!((decoded.width, decoded.height), (4, 2));
}

// ---------------------------------------------------------------------------
// pledit.txt
// ---------------------------------------------------------------------------

/// A malformed colour leaves the documented default in place.
#[test]
fn a_malformed_colour_keeps_the_default() {
    let defaults = PlaylistStyle::default();
    for value in [
        "", " ", "#", "##", "zzz", "#GGGGGG", "#12345", "#1234567", "12 34 56", "0x00FF00", "-",
    ] {
        let style = PlaylistStyle::parse(&format!("[Text]\nNormal={value}\n"));
        assert_eq!(
            style.normal, defaults.normal,
            "the colour {value:?} must keep the default"
        );
    }
}

/// A colour with more digits than a colour has keeps the default; a
/// three-digit colour is expanded the classic way.
#[test]
fn a_colour_is_read_by_its_digit_count() {
    let style = PlaylistStyle::parse("[Text]\nNormal=#0f0\nCurrent=#00FF00\n");
    assert_eq!(style.normal, [0x00, 0xFF, 0x00]);
    assert_eq!(style.current, [0x00, 0xFF, 0x00]);
}

/// A missing key keeps the default of that key, and the other keys are read
/// as usual.
#[test]
fn a_missing_key_keeps_its_default() {
    let defaults = PlaylistStyle::default();
    let style = PlaylistStyle::parse("[Text]\nNormal=#123456\n");
    assert_eq!(style.normal, [0x12, 0x34, 0x56]);
    assert_eq!(style.current, defaults.current);
    assert_eq!(style.normal_bg, defaults.normal_bg);
    assert_eq!(style.selected_bg, defaults.selected_bg);
    assert_eq!(style.font, defaults.font);
}

/// The font line never yields an unusable size: a size outside the range is
/// ignored like any other unusable token.
#[test]
fn a_font_size_is_always_usable() {
    for value in [
        "Arial,0",
        "Arial,256",
        "Arial,999",
        "Arial,-1",
        "Arial,99999999999999999999",
        "Arial,1e3",
    ] {
        let style = PlaylistStyle::parse(&format!("[Text]\nFont={value}\n"));
        let font = style.font.expect("the default font");
        assert_eq!(font.family, "Arial", "the family of {value:?}");
        assert_eq!(font.size, None, "the size of {value:?} must stay unset");
    }
    // A size inside the range is read.
    let style = PlaylistStyle::parse("[Text]\nFont=Arial,255\n");
    assert_eq!(style.font.unwrap().size, Some(255));
}

/// A font line without a family name keeps the built-in default font.
#[test]
fn a_font_line_without_a_family_keeps_the_default_font() {
    for value in ["", " ", ",12", ",,,", "   ,  , "] {
        let style = PlaylistStyle::parse(&format!("[Text]\nFont={value}\n"));
        assert_eq!(
            style.font,
            PlaylistStyle::default().font,
            "the font line {value:?} must keep the default"
        );
    }
}

/// A file without a `[Text]` section keeps every default.
#[test]
fn a_file_without_the_section_keeps_every_default() {
    for text in [
        "",
        "Normal=#FF0000\n",
        "[Other]\nNormal=#FF0000\n",
        "[text\nNormal=#FF0000\n",
        "[Text\n",
        "[]\nNormal=#FF0000\n",
    ] {
        assert_eq!(
            PlaylistStyle::parse(text),
            PlaylistStyle::default(),
            "the text {text:?} must keep every default"
        );
    }
}

/// Arbitrary text never panics and never yields an unusable style.
#[test]
fn arbitrary_text_never_yields_an_unusable_style() {
    let mut rng = Rng::new(0xBEEF_0042);
    for _ in 0..256 {
        let bytes = rng.blob(96);
        let text = String::from_utf8_lossy(&bytes);
        let style = PlaylistStyle::parse(&text);
        if let Some(font) = &style.font {
            assert!(!font.family.is_empty(), "a font always names a family");
            if let Some(size) = font.size {
                assert!(
                    (1..=255).contains(&size),
                    "a font size is always in range, got {size}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// viscolor.txt
// ---------------------------------------------------------------------------

/// The analyser colours are always 24 entries, and the entries the file does
/// not mention keep their defaults.
#[test]
fn the_analyser_colours_are_always_twenty_four_entries() {
    let mut rng = Rng::new(0x1234_5678);
    for _ in 0..64 {
        let bytes = rng.blob(200);
        let text = String::from_utf8_lossy(&bytes);
        let colors = rustamp_skin::viscolor::parse_viscolors(&text);
        assert_eq!(colors.len(), 24, "there are always 24 analyser colours");
        // Every colour is a plain RGB triple.
        for color in colors {
            assert_eq!(color.len(), 3);
        }
    }
}

/// A file with fewer than 24 entries keeps the defaults of the rest.
#[test]
fn missing_entries_keep_their_defaults() {
    let colors = rustamp_skin::viscolor::parse_viscolors("1,2,3\n4,5,6\n");
    assert_eq!(colors[0], [1, 2, 3]);
    assert_eq!(colors[1], [4, 5, 6]);
    assert_eq!(colors[2], webamp::DEFAULT_VISCOLORS[2]);
    assert_eq!(colors[23], webamp::DEFAULT_VISCOLORS[23]);
}

/// A file with more than 24 entries uses the first 24 and ignores the rest.
#[test]
fn entries_beyond_the_twenty_fourth_are_ignored() {
    let mut text = String::new();
    for i in 0..40 {
        text.push_str(&format!("{i},0,0\n"));
    }
    let colors = rustamp_skin::viscolor::parse_viscolors(&text);
    assert_eq!(colors[0], [0, 0, 0]);
    assert_eq!(colors[23], [23, 0, 0]);
}

/// A line that is not a colour does not consume a slot.
#[test]
fn a_malformed_line_does_not_consume_a_slot() {
    let colors = rustamp_skin::viscolor::parse_viscolors(
        "; a note\n1,2\nnot a colour\n3,4,5\n-1,-2,-3\n1,2,x\n",
    );
    // Only the one complete line is a colour, and it lands in slot 0.
    assert_eq!(colors[0], [3, 4, 5]);
    assert_eq!(colors[1], webamp::DEFAULT_VISCOLORS[1]);
}

/// A component outside the byte range is clamped, not wrapped.
#[test]
fn out_of_range_components_are_clamped() {
    let colors = rustamp_skin::viscolor::parse_viscolors("300,400,500\n-1,0,0\n");
    assert_eq!(colors[0], [255, 255, 255]);
    // A negative component is not a number the format knows: the line is
    // ignored and does not consume a slot.
    assert_eq!(colors[1], webamp::DEFAULT_VISCOLORS[1]);
}

/// A number too large for the format is ignored, not a panic.
#[test]
fn an_absurd_component_is_ignored() {
    let colors = rustamp_skin::viscolor::parse_viscolors("99999999999999999999,0,0\n1,2,3\n");
    // The line is not a colour, so it does not consume a slot.
    assert_eq!(colors[0], [1, 2, 3]);
    assert_eq!(colors[1], webamp::DEFAULT_VISCOLORS[1]);
}

/// A line with more than three components uses the first three.
#[test]
fn a_line_with_four_components_uses_the_first_three() {
    let colors = rustamp_skin::viscolor::parse_viscolors("1,2,3,4\n");
    assert_eq!(colors[0], [1, 2, 3]);
}

// ---------------------------------------------------------------------------
// region.txt
// ---------------------------------------------------------------------------

/// A count that is not a number never mis-reads the polygons after it: the
/// point list cannot be aligned any more, so the parser stops.
#[test]
fn a_malformed_count_does_not_shift_the_other_polygons() {
    // Three polygons of four points each; the middle count is malformed.
    let text = "[Normal]\nNumPoints=4,-4,4\n\
               PointList=0,0,10,0,10,10,0,10, 20,0,30,0,30,10,20,10, 40,0,50,0,50,10,40,10\n";
    let region = Region::parse(text);
    // The first polygon is read; the rest cannot be aligned and are dropped.
    assert_eq!(region.normal().len(), 1);
    assert_eq!(
        region.normal()[0],
        vec![(0, 0), (10, 0), (10, 10), (0, 10)],
        "the polygon before the malformed count must be the one that was read"
    );
}

/// A count that is not a number at all is dropped, and a count that is a
/// number but far too large keeps the points the list has — never an
/// overflow.
#[test]
fn an_absurd_count_is_dropped() {
    // A count that is not a number: the point list cannot be aligned any
    // more, so no polygon is read at all.
    for count in [
        "-1",
        "-99999999999999999999",
        "four",
        "4.0",
        "1e3",
        "0x10",
        " ",
        "99999999999999999999999999",
    ] {
        let text = format!("[Normal]\nNumPoints={count}\nPointList=0,0,1,1,2,2,3,3\n");
        let region = Region::parse(&text);
        assert!(
            region.normal().is_empty(),
            "the count {count:?} must not yield a polygon"
        );
    }

    // A count that is a number but absurdly large: the polygon keeps the
    // points the list has, and the arithmetic does not overflow.
    for count in ["18446744073709551615", "9223372036854775807", "4294967296"] {
        let text = format!("[Normal]\nNumPoints={count}\nPointList=0,0,1,1,2,2,3,3\n");
        let region = Region::parse(&text);
        assert_eq!(region.normal().len(), 1, "the count {count:?}");
        assert_eq!(
            region.normal()[0],
            vec![(0, 0), (1, 1), (2, 2), (3, 3)],
            "the polygon keeps the points the list has"
        );
    }
}

/// A count that is a number but larger than the point list keeps the points
/// it has and drops the polygon.
#[test]
fn a_count_larger_than_the_point_list_drops_the_polygon() {
    let text = "[Normal]\nNumPoints=1000\nPointList=0,0,1,1\n";
    let region = Region::parse(text);
    assert!(region.normal().is_empty());
}

/// A polygon whose point list ends early keeps the points it has.
#[test]
fn a_polygon_that_ends_early_keeps_its_points() {
    // Four points announced, three delivered: the polygon has three points.
    let text = "[Normal]\nNumPoints=4\nPointList=0,0,10,0,10,10\n";
    let region = Region::parse(text);
    assert_eq!(region.normal().len(), 1);
    assert_eq!(region.normal()[0], vec![(0, 0), (10, 0), (10, 10)]);
}

/// A count below three is dropped, and the points it would have used are
/// skipped, so the polygons after it keep their own points.
#[test]
fn a_dropped_count_does_not_shift_the_next_polygon() {
    let text = "[Normal]\nNumPoints=4,2,4\n\
               PointList=0,0,10,0,10,10,0,10, 20,0,30,0, 40,0,50,0,50,10,40,10\n";
    let region = Region::parse(text);
    // The first polygon is read; the polygon with two points is dropped.
    assert_eq!(region.normal().len(), 2);
    assert_eq!(region.normal()[0], vec![(0, 0), (10, 0), (10, 10), (0, 10)]);
    assert_eq!(
        region.normal()[1],
        vec![(40, 0), (50, 0), (50, 10), (40, 10)],
        "the second polygon must keep its own points"
    );
}

/// A region mask never leaves the window it was built for.
#[test]
fn a_region_mask_is_inside_the_window() {
    let cases: Vec<(&str, u32, u32)> = vec![
        (
            "[Normal]\nNumPoints=4\nPointList=0,0,275,0,275,116,0,116\n",
            275,
            116,
        ),
        (
            "[Normal]\nNumPoints=4\nPointList=-50,-50,5000,-50,5000,5000,-50,5000\n",
            275,
            116,
        ),
        (
            "[Normal]\nNumPoints=4\nPointList=1000,1000,2000,1000,2000,2000,1000,2000\n",
            275,
            116,
        ),
        ("[Normal]\nNumPoints=3\nPointList=0,0,10,0,0,10\n", 32, 32),
        ("[Normal]\nNumPoints=4\nPointList=0,0,0,0,0,0,0,0\n", 16, 16),
    ];
    for (text, width, height) in cases {
        let region = Region::parse(text);
        let mask = region
            .mask(RegionSection::Normal, width, height)
            .expect("the section has a polygon");
        assert_eq!(mask.width(), width);
        assert_eq!(mask.height(), height);
        assert!(
            mask.covered_pixels() <= width as u64 * height as u64,
            "a mask never covers more than the window"
        );

        // The runs stay inside the window, do not overlap and add up to the
        // number of covered pixels.
        let spans = mask.spans();
        let mut covered = 0u64;
        for (index, span) in spans.iter().enumerate() {
            assert!(span.x_start < span.x_end, "a run is never empty");
            assert!(span.x_end <= width, "a run stays inside the window");
            assert!(span.y < height, "a run stays inside the window");
            covered += (span.x_end - span.x_start) as u64;
            if index > 0 {
                let previous = spans[index - 1];
                assert!(
                    previous.y < span.y || (previous.y == span.y && previous.x_end < span.x_start),
                    "the runs of a row do not overlap"
                );
            }
        }
        assert_eq!(covered, mask.covered_pixels());

        // A pixel outside the window is never covered.
        assert!(!mask.contains(width, 0));
        assert!(!mask.contains(0, height));
    }
}

/// A window without pixels has no mask to speak of, and a section without a
/// polygon has none either.
#[test]
fn an_empty_window_or_section_has_no_mask() {
    let region = Region::parse("[Normal]\nNumPoints=4\nPointList=0,0,10,0,10,10,0,10\n");
    assert!(region.mask(RegionSection::Normal, 0, 0).is_some());
    assert_eq!(
        region
            .mask(RegionSection::Normal, 0, 0)
            .expect("a mask")
            .covered_pixels(),
        0
    );
    assert!(region.mask(RegionSection::Equalizer, 275, 116).is_none());
}

/// Arbitrary region text never panics and never yields a polygon outside
/// the window.
#[test]
fn arbitrary_region_text_never_panics() {
    let mut rng = Rng::new(0x7777_8888);
    for _ in 0..256 {
        let bytes = rng.blob(128);
        let text = String::from_utf8_lossy(&bytes);
        let region = Region::parse(&text);
        let mask = region.mask(RegionSection::Normal, 64, 64);
        if let Some(mask) = mask {
            assert_eq!((mask.width(), mask.height()), (64, 64));
            assert!(mask.covered_pixels() <= 64 * 64);
            for span in mask.spans() {
                assert!(span.x_start < span.x_end);
                assert!(span.x_end <= 64);
                assert!(span.y < 64);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Cursors
// ---------------------------------------------------------------------------

/// A cursor is either present with the file's bytes or absent; it is never
/// present with other bytes and never half-decoded.
#[test]
fn a_cursor_is_present_with_its_bytes_or_absent() {
    let mut source = MemorySource::new("cursors");
    source.add("close.cur", b"not a cursor at all".to_vec());
    source.add("posbar.ani", b"RIFF-animated".to_vec());
    source.add("volbal.cur", Vec::new());
    let skin = Skin::from_source(&mut source, &BuiltinLookalikeSkin).expect("the skin loads");

    let close = skin.cursor_slot(CursorSlot::Close).expect("close.cur");
    assert_eq!(close.bytes, b"not a cursor at all");
    assert_eq!(close.kind, CursorKind::Cur);

    let posbar = skin.cursor_slot(CursorSlot::PosBar).expect("posbar.ani");
    assert_eq!(posbar.bytes, b"RIFF-animated");
    assert_eq!(posbar.kind, CursorKind::Ani);

    // An empty file is still a file: the bytes are handed out untouched and
    // the crate never decodes them.
    let volbal = skin.cursor_slot(CursorSlot::VolBal).expect("volbal.cur");
    assert!(volbal.bytes.is_empty());

    // Every slot the skin does not carry stays absent.
    for slot in CursorSlot::ALL {
        if matches!(
            slot,
            CursorSlot::Close | CursorSlot::PosBar | CursorSlot::VolBal
        ) {
            continue;
        }
        assert!(
            skin.cursor_slot(*slot).is_none(),
            "slot {slot:?} must be absent"
        );
    }

    // Every cursor the skin reports names a slot the model knows.
    for cursor in skin.cursors() {
        assert!(
            CursorSlot::from_file_name(&cursor.name).is_some(),
            "the cursor {} belongs to no slot",
            cursor.name
        );
    }
}

/// A cursor file that is not a cursor at all is still handed out untouched:
/// the crate never decodes a `.cur` or an `.ani` (S17 §8).
#[test]
fn a_cursor_is_never_decoded() {
    let mut source = MemorySource::new("cursors");
    source.add("normal.cur", vec![0x00, 0xFF, 0x00, 0x80]);
    let skin = Skin::from_source(&mut source, &BuiltinLookalikeSkin).expect("the skin loads");
    let normal = skin.cursor_slot(CursorSlot::Normal).expect("normal.cur");
    assert_eq!(normal.bytes, vec![0x00, 0xFF, 0x00, 0x80]);
}

/// A cursor whose name is not a classic slot is not asked for and never
/// reaches the skin.
#[test]
fn a_cursor_of_an_unknown_name_is_not_loaded() {
    let mut source = MemorySource::new("cursors");
    source.add("not-a-cursor.cur", b"whatever".to_vec());
    source.add("main.bmp", b"BM".to_vec());
    let skin = Skin::from_source(&mut source, &BuiltinLookalikeSkin).expect("the skin loads");
    assert!(skin.cursors().is_empty());
    for slot in CursorSlot::ALL {
        assert!(skin.cursor_slot(*slot).is_none(), "slot {slot:?}");
    }
}

/// A sheet in a subfolder is found: the skin file lookup ignores a
/// directory prefix.
#[test]
fn a_sheet_in_a_subfolder_is_found() {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("sheets/main.bmp", options).unwrap();
    std::io::Write::write_all(&mut writer, &valid_image(24)).unwrap();
    let bytes = writer.finish().unwrap().into_inner();

    let mut archive =
        ArchiveSource::from_bytes(bytes, "probe".to_string()).expect("the archive opens");
    assert!(archive.read_any("main", &["bmp"]).is_some());
}

/// A cursor in a subfolder fills its slot: the skin file lookup ignores a
/// directory prefix, and so does the slot lookup. The skin's own cursor is
/// not shadowed by the fallback's.
#[test]
fn a_cursor_in_a_subfolder_fills_its_slot() {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("cursors/close.cur", options).unwrap();
    std::io::Write::write_all(&mut writer, b"shipped-close-cursor").unwrap();
    let bytes = writer.finish().unwrap().into_inner();

    let mut archive =
        ArchiveSource::from_bytes(bytes, "probe".to_string()).expect("the archive opens");
    let skin = Skin::from_source(&mut archive, &BuiltinLookalikeSkin).expect("the skin loads");

    let close = skin
        .cursor_slot(CursorSlot::Close)
        .expect("the cursor in the subfolder fills the slot");
    assert_eq!(close.bytes, b"shipped-close-cursor");
    assert!(
        !skin
            .substitutions()
            .iter()
            .any(|s| s.requested == "close.cur"),
        "a shipped cursor must not record a substitution"
    );
}

// ---------------------------------------------------------------------------
// Containers: archives and folders
// ---------------------------------------------------------------------------

/// Packs `(name, bytes)` entries into a ZIP archive.
fn zip_bytes(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in entries {
        writer.start_file(name.clone(), options).unwrap();
        std::io::Write::write_all(&mut writer, bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// An archive whose `main.bmp` is a **directory** carries no sheet: the
/// entry is not a file, so the sheet falls back to the default skin.
#[test]
fn an_archive_with_main_bmp_as_a_directory_carries_no_sheet() {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer.add_directory("main.bmp", options).unwrap();
    writer.start_file("pledit.txt", options).unwrap();
    std::io::Write::write_all(&mut writer, b"[Text]\nNormal=#00FF00\n").unwrap();
    let bytes = writer.finish().unwrap().into_inner();

    let mut archive =
        ArchiveSource::from_bytes(bytes, "probe".to_string()).expect("the archive opens");
    assert!(
        archive.read_any("main", &["bmp"]).is_none(),
        "a directory is not a sheet"
    );

    let skin = Skin::from_source(&mut archive, &BuiltinLookalikeSkin).expect("the skin loads");
    // The sheet comes from the fallback, not from the directory entry.
    assert!(
        skin.substitutions()
            .iter()
            .any(|s| s.requested == "main.bmp"),
        "the missing sheet must be reported"
    );
    // The text file of the archive is still read.
    assert_eq!(skin.pledit().normal, [0x00, 0xFF, 0x00]);
}

/// A file that is not an archive is an error, not a panic.
#[test]
fn a_corrupt_archive_is_reported() {
    let mut rng = Rng::new(0x0BAD_F00D);
    for _ in 0..64 {
        let bytes = rng.blob(256);
        let result = ArchiveSource::from_bytes(bytes, "probe".to_string());
        assert!(result.is_err(), "random bytes are not an archive");
    }
}

/// An archive whose entry is corrupt is skipped: the sheet falls back
/// instead of the load failing.
#[test]
fn an_archive_with_a_corrupt_entry_falls_back() {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    // The entry is stored, so its payload is the image itself and a flipped
    // byte breaks it.
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.start_file("main.bmp", options).unwrap();
    std::io::Write::write_all(&mut writer, &valid_image(24)).unwrap();
    writer
        .start_file("pledit.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut writer, b"[Text]\nNormal=#00FF00\n").unwrap();
    let mut bytes = writer.finish().unwrap().into_inner();

    // Corrupt the stored payload of the first entry.
    let payload = first_payload_offset(&bytes).expect("the archive has a payload");
    bytes[payload + 8] ^= 0xFF;

    let mut archive =
        ArchiveSource::from_bytes(bytes, "probe".to_string()).expect("the archive opens");
    let skin = Skin::from_source(&mut archive, &BuiltinLookalikeSkin).expect("the skin loads");
    assert!(
        skin.warnings().iter().any(|w| w.contains("main.bmp"))
            || skin
                .substitutions()
                .iter()
                .any(|s| s.requested == "main.bmp"),
        "the corrupt sheet must be reported: {:?} / {:?}",
        skin.warnings(),
        skin.substitutions()
    );
}

/// The offset of the stored payload of the first entry of an archive.
fn first_payload_offset(bytes: &[u8]) -> Option<usize> {
    let start = bytes.windows(4).position(|w| w == b"PK\x03\x04")?;
    let name_len = u16::from_le_bytes([bytes[start + 26], bytes[start + 27]]) as usize;
    let extra_len = u16::from_le_bytes([bytes[start + 28], bytes[start + 29]]) as usize;
    Some(start + 30 + name_len + extra_len)
}

/// An archive that carries the same name twice uses the last entry, the way
/// extracting it to a case-insensitive file system would.
#[test]
fn the_last_duplicate_entry_wins() {
    let entries = vec![
        ("main.bmp".to_string(), valid_image(24)),
        ("MAIN.BMP".to_string(), valid_image(24)),
    ];
    let mut archive =
        ArchiveSource::from_bytes(zip_bytes(&entries), "probe".to_string()).expect("it opens");
    let file = archive.read_any("main", &["bmp"]).expect("the entry");
    assert_eq!(file.bytes, valid_image(24));
}

/// An entry whose name escapes the archive is never indexed.
#[test]
fn an_escaping_entry_is_never_indexed() {
    let entries = vec![
        ("../main.bmp".to_string(), valid_image(24)),
        ("sub/../../titlebar.bmp".to_string(), valid_image(24)),
        ("/main.bmp".to_string(), valid_image(24)),
    ];
    let mut archive =
        ArchiveSource::from_bytes(zip_bytes(&entries), "probe".to_string()).expect("it opens");
    assert!(archive.read_any("main", &["bmp"]).is_none());
    assert!(archive.read_any("titlebar", &["bmp"]).is_none());
}

/// A folder is indexed by the base name of every file below it, and a
/// subfolder's files are found as well.
#[test]
fn a_folder_is_indexed_by_base_name() {
    let dir = tempfile::tempdir().unwrap();
    let image = valid_image(24);
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("MAIN.BMP"), &image).unwrap();
    std::fs::write(dir.path().join("sub").join("titlebar.bmp"), &image).unwrap();

    let mut folder = FolderSource::open(dir.path()).expect("the folder opens");
    assert_eq!(
        folder.read_any("main", &["bmp"]).expect("main").bytes,
        image
    );
    assert_eq!(
        folder
            .read_any("titlebar", &["bmp"])
            .expect("titlebar")
            .bytes,
        image
    );
    assert!(folder.read_any("cbuttons", &["bmp"]).is_none());
}

/// A path that is not a folder is not a skin folder.
#[test]
fn a_file_is_not_a_skin_folder() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("skin.zip");
    std::fs::write(&path, b"not a zip").unwrap();
    assert!(FolderSource::open(&path).is_err());
    assert!(FolderSource::open(dir.path().join("does-not-exist")).is_err());
}

// ---------------------------------------------------------------------------
// The loader end to end
// ---------------------------------------------------------------------------

/// A skin whose every file is malformed loads with the documented fallbacks:
/// the sheets come from the default skin, the text files use the defaults
/// and the warnings name the files that failed.
#[test]
fn a_skin_of_malformed_files_loads_with_the_documented_fallbacks() {
    let entries = vec![
        ("main.bmp".to_string(), b"BM\x00\x00 not a bmp".to_vec()),
        ("titlebar.bmp".to_string(), vec![0xFF; 3]),
        ("cbuttons.bmp".to_string(), valid_image(24)),
        ("pledit.txt".to_string(), vec![0xFF, 0xFE, 0xFD]),
        ("viscolor.txt".to_string(), b"not a colour\n".to_vec()),
        (
            "region.txt".to_string(),
            b"[Normal]\nNumPoints=-1\nPointList=0,0\n".to_vec(),
        ),
        ("close.cur".to_string(), b"not a cursor".to_vec()),
    ];
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in &entries {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
    }

    let skin = Skin::load(dir.path()).expect("a skin of malformed files still loads");

    // The sheets that do not decode fall back to the default skin.
    assert!(
        skin.warnings().iter().any(|w| w.contains("main.bmp")),
        "the failed sheet must be reported: {:?}",
        skin.warnings()
    );
    for sheet in [Sheet::Main, Sheet::Titlebar] {
        assert!(
            skin.substitutions()
                .iter()
                .any(|s| s.requested == sheet.file_name()),
            "the missing sheet {sheet:?} must be reported"
        );
    }
    // The sheet that does decode comes from the skin.
    assert!(skin.sheet(Sheet::Cbuttons).is_some());

    // The text files fall back to the documented defaults.
    assert_eq!(skin.pledit(), &PlaylistStyle::default());
    assert_eq!(skin.viscolors(), &webamp::DEFAULT_VISCOLORS);
    assert!(skin.region().is_empty());

    // The cursor is handed out untouched.
    let close = skin.cursor_slot(CursorSlot::Close).expect("close.cur");
    assert_eq!(close.bytes, b"not a cursor");
}

/// A skin that carries a text file that is not UTF-8 loads with the
/// documented defaults instead of failing.
#[test]
fn a_text_file_that_is_not_utf8_uses_the_defaults() {
    let entries = vec![
        (
            "pledit.txt".to_string(),
            vec![0xFF, 0xFE, b'[', b'T', b']', 0x80],
        ),
        ("viscolor.txt".to_string(), vec![0xFF, 0xFE, 0x80]),
    ];
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in &entries {
        std::fs::write(dir.path().join(name), bytes).unwrap();
    }

    let skin = Skin::load(dir.path()).expect("the skin loads");
    assert_eq!(skin.pledit(), &PlaylistStyle::default());
    assert_eq!(skin.viscolors(), &webamp::DEFAULT_VISCOLORS);
}

/// A text file that starts with a byte-order mark keeps the defaults: the
/// mark is part of the first line, so the section is not recognised.
#[test]
fn a_text_file_with_a_byte_order_mark_uses_the_defaults() {
    let mut pledit = vec![0xEF, 0xBB, 0xBF];
    pledit.extend_from_slice(b"[Text]\nNormal=#00FF00\n");
    let entries = vec![("pledit.txt".to_string(), pledit)];
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in &entries {
        std::fs::write(dir.path().join(name), bytes).unwrap();
    }

    let skin = Skin::load(dir.path()).expect("the skin loads");
    assert_eq!(skin.pledit(), &PlaylistStyle::default());
}

/// S17 §3: a `nums_ex.bmp` that does not decode does not take the time
/// display away from the skin's own `numbers.bmp` — the choice follows the
/// file the *skin* carries.
#[test]
fn a_corrupt_nums_ex_does_not_take_over_the_time_display() {
    let entries = vec![
        ("numbers.bmp".to_string(), valid_image(24)),
        ("nums_ex.bmp".to_string(), b"BM\x00\x00 not a bmp".to_vec()),
    ];
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in &entries {
        std::fs::write(dir.path().join(name), bytes).unwrap();
    }

    let skin = Skin::load(dir.path()).expect("the skin loads");
    assert_eq!(skin.time_digit_sheet(), Sheet::Numbers);
    assert!(
        skin.warnings().iter().any(|w| w.contains("nums_ex.bmp")),
        "the failed sheet must be reported: {:?}",
        skin.warnings()
    );
}
