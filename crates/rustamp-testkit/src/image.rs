use std::fmt;
use std::fs::File;
use std::io::{BufReader, BufWriter, Cursor};
use std::path::Path;

/// Errors from image construction and PNG I/O.
#[derive(Debug)]
pub enum ImageError {
    /// Pixel buffer length does not equal `width * height * 4`.
    BadLength {
        /// Expected byte length.
        expected: usize,
        /// Actual byte length.
        actual: usize,
    },
    /// Filesystem error.
    Io(std::io::Error),
    /// PNG decode error.
    Decode(png::DecodingError),
    /// PNG encode error.
    Encode(png::EncodingError),
    /// Unsupported PNG format.
    Unsupported(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadLength { expected, actual } => {
                write!(
                    f,
                    "bad RGBA buffer length: expected {expected}, got {actual}"
                )
            }
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Decode(e) => write!(f, "png decode error: {e}"),
            Self::Encode(e) => write!(f, "png encode error: {e}"),
            Self::Unsupported(s) => write!(f, "unsupported png: {s}"),
        }
    }
}

impl std::error::Error for ImageError {}

impl From<std::io::Error> for ImageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<png::DecodingError> for ImageError {
    fn from(e: png::DecodingError) -> Self {
        Self::Decode(e)
    }
}
impl From<png::EncodingError> for ImageError {
    fn from(e: png::EncodingError) -> Self {
        Self::Encode(e)
    }
}

/// An 8-bit non-premultiplied RGBA image, row-major, top row first.
#[derive(Clone, PartialEq, Eq)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl fmt::Debug for RgbaImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RgbaImage({}x{})", self.width, self.height)
    }
}

impl RgbaImage {
    /// Creates an image filled with one RGBA colour.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let n = width as usize * height as usize;
        let mut data = Vec::with_capacity(n * 4);
        for _ in 0..n {
            data.extend_from_slice(&rgba);
        }
        Self {
            width,
            height,
            data,
        }
    }

    /// Wraps a raw RGBA buffer, validating its length.
    pub fn from_raw(width: u32, height: u32, data: Vec<u8>) -> Result<Self, ImageError> {
        let expected = width as usize * height as usize * 4;
        if data.len() != expected {
            return Err(ImageError::BadLength {
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }
    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Raw RGBA bytes.
    pub fn as_raw(&self) -> &[u8] {
        &self.data
    }
    /// Consumes the image, returning `(width, height, rgba)`.
    pub fn into_raw(self) -> (u32, u32, Vec<u8>) {
        (self.width, self.height, self.data)
    }

    /// Returns the pixel at `(x, y)`. Panics when out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = self.index(x, y);
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }

    /// Sets the pixel at `(x, y)`. Panics when out of bounds.
    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        let i = self.index(x, y);
        self.data[i..i + 4].copy_from_slice(&rgba);
    }

    fn index(&self, x: u32, y: u32) -> usize {
        assert!(
            x < self.width && y < self.height,
            "pixel ({x},{y}) out of bounds"
        );
        (y as usize * self.width as usize + x as usize) * 4
    }

    /// Decodes a PNG from memory. Any 8/16-bit grey/RGB/indexed variant is
    /// converted to RGBA8.
    pub fn from_png_bytes(bytes: &[u8]) -> Result<Self, ImageError> {
        Self::decode(Cursor::new(bytes))
    }

    /// Loads a PNG file.
    pub fn load_png(path: impl AsRef<Path>) -> Result<Self, ImageError> {
        Self::decode(BufReader::new(File::open(path)?))
    }

    fn decode<R: std::io::BufRead + std::io::Seek>(r: R) -> Result<Self, ImageError> {
        let mut decoder = png::Decoder::new(r);
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info()?;
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| ImageError::Unsupported("image too large".into()))?;
        let mut buf = vec![0; size];
        let info = reader.next_frame(&mut buf)?;
        if info.bit_depth != png::BitDepth::Eight {
            return Err(ImageError::Unsupported(format!(
                "bit depth {:?}",
                info.bit_depth
            )));
        }
        let src = &buf[..info.buffer_size()];
        let data = match info.color_type {
            png::ColorType::Rgba => src.to_vec(),
            png::ColorType::Rgb => src
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::Grayscale => src.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::GrayscaleAlpha => src
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Indexed => {
                return Err(ImageError::Unsupported("indexed after expand".into()));
            }
        };
        Self::from_raw(info.width, info.height, data)
    }

    /// Encodes the image as PNG bytes.
    pub fn to_png_bytes(&self) -> Result<Vec<u8>, ImageError> {
        let mut out = Vec::new();
        self.encode(&mut out)?;
        Ok(out)
    }

    /// Saves the image as a PNG file, creating parent directories.
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<(), ImageError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.encode(BufWriter::new(File::create(path)?))
    }

    fn encode<W: std::io::Write>(&self, w: W) -> Result<(), ImageError> {
        let mut enc = png::Encoder::new(w, self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header()?;
        writer.write_image_data(&self.data)?;
        writer.finish()?;
        Ok(())
    }
}
