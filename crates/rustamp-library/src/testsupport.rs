// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The helpers the crate's tests share.
//!
//! The property tests of the crate draw their cases from a seeded
//! generator, and the fixtures of the crate are WAV files and pictures.
//! Both live here, so every test module of the crate agrees on what a seed
//! means and on what a WAV or a cover fixture looks like.

use std::fs;
use std::fs::FileTimes;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::Library;

/// A deterministic pseudo random generator (xorshift64*).
///
/// The tests need data that is random enough to hit the corners and
/// reproducible enough to replay a failure: the seed is the whole case.
/// The generator is a few lines long and needs no dependency, and it gives
/// the same sequence on every machine.
pub(crate) struct Rng(u64);

impl Rng {
    /// A generator for `seed`. The state is never zero: xorshift needs a
    /// non-zero state to leave.
    pub(crate) fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    /// The next number of the sequence.
    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut state = self.0;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.0 = state;
        state.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A number in `0..bound`.
    pub(crate) fn below(&mut self, bound: usize) -> usize {
        assert!(bound > 0, "the bound must not be empty");
        (self.next_u64() % bound as u64) as usize
    }

    /// `true` in half of the cases.
    pub(crate) fn flip(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    /// One of `items`.
    pub(crate) fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// Writes a minimal WAV file (a header without audio) to `path`.
///
/// The scan only records what it finds, so a fixture does not have to carry
/// tags: a valid header is enough for the tag reader to accept the file.
/// The file is written in one go and closed when the call returns, so the
/// modification time the scan sees is the one of this write.
pub(crate) fn make_wav(path: &Path) {
    let mut buf = Vec::with_capacity(44);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&36u32.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // channels
    buf.extend_from_slice(&44100u32.to_le_bytes()); // sample rate
    buf.extend_from_slice(&88200u32.to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&0u32.to_le_bytes());
    fs::write(path, buf).expect("the WAV fixture is written");
}

/// The last-scan time of the root `id`, as the database holds it.
pub(crate) fn last_scanned_at(lib: &Library, id: i64) -> Option<i64> {
    lib.roots()
        .expect("the roots are readable")
        .into_iter()
        .find(|root| root.id == id)
        .expect("the root is listed")
        .last_scanned_at
}

/// Puts a known time into the root's row.
///
/// A test that wants to tell "untouched" from "written again in the same
/// second" needs a time that no scan can produce: the scan writes the
/// second it runs in, so a time far away from now is unmistakable.
pub(crate) fn put_last_scanned_at(lib: &Library, id: i64, at: i64) {
    lib.conn
        .execute(
            "UPDATE roots SET last_scanned_at = ?1 WHERE id = ?2",
            rusqlite::params![at, id],
        )
        .expect("the time is written");
}

// ---- the cover fixtures ----

/// A synthetic PNG of one colour.
pub(crate) fn png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    let buffer = image::RgbaImage::from_pixel(width, height, image::Rgba(color));
    encode(
        image::DynamicImage::ImageRgba8(buffer),
        image::ImageFormat::Png,
    )
}

/// A synthetic JPEG of one colour.
pub(crate) fn jpeg(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    let buffer = image::RgbaImage::from_pixel(width, height, image::Rgba(color));
    encode(
        image::DynamicImage::ImageRgba8(buffer),
        image::ImageFormat::Jpeg,
    )
}

/// Encode a picture.
fn encode(picture: image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
    let mut out = Vec::new();
    picture
        .write_to(&mut Cursor::new(&mut out), format)
        .expect("encode the picture");
    out
}

/// The size of an ID3v2 tag, in synchsafe bytes.
fn synchsafe(size: u32) -> [u8; 4] {
    [
        ((size >> 21) & 0x7F) as u8,
        ((size >> 14) & 0x7F) as u8,
        ((size >> 7) & 0x7F) as u8,
        (size & 0x7F) as u8,
    ]
}

/// One ID3v2.3 frame: the id, the size, the flags and the data.
fn id3v2_frame(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend_from_slice(id);
    frame.extend_from_slice(&(data.len() as u32).to_be_bytes());
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(data);
    frame
}

/// An ID3v2.3 tag with the given frames.
fn id3v2_tag(frames: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = frames.concat();
    let mut tag = Vec::new();
    tag.extend_from_slice(b"ID3");
    tag.extend_from_slice(&[0x03, 0x00, 0x00]);
    tag.extend_from_slice(&synchsafe(body.len() as u32));
    tag.extend_from_slice(&body);
    tag
}

/// An `APIC` frame: the front cover of the tag.
fn apic_frame(mime: &str, picture: &[u8]) -> Vec<u8> {
    let mut data = vec![0x00]; // ISO-8859-1
    data.extend_from_slice(mime.as_bytes());
    data.push(0x00);
    data.push(3); // the front cover
    data.extend_from_slice(b"cover");
    data.push(0x00);
    data.extend_from_slice(picture);
    id3v2_frame(b"APIC", &data)
}

/// An MP3 file whose only content is an ID3v2 tag with a front cover.
pub(crate) fn mp3_with_cover(mime: &str, picture: &[u8]) -> Vec<u8> {
    id3v2_tag(&[apic_frame(mime, picture)])
}

/// An MP3 file whose only content is an ID3v2 tag without a picture.
///
/// The title is a hundred bytes long, because the MPEG reader seeks
/// back over the last bytes of the file to look for an APE tag: a file
/// that is shorter than that cannot be read at all.
pub(crate) fn mp3_without_cover() -> Vec<u8> {
    let mut data = vec![0x00]; // ISO-8859-1
    data.extend_from_slice(&[b'x'; 100]);
    id3v2_tag(&[id3v2_frame(b"TIT2", &data)])
}

/// Write a file into the directory `dir` and return its path.
pub(crate) fn write_in(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).expect("write the file");
    path
}

/// The modification time of a file.
pub(crate) fn modified(path: &Path) -> SystemTime {
    fs::metadata(path)
        .expect("stat")
        .modified()
        .expect("modified")
}

/// Set the modification time of a file.
///
/// The time is set exactly, so a test can tell two files apart that the
/// file system would otherwise give the same stamp.
pub(crate) fn set_modified(path: &Path, when: SystemTime) {
    let file = fs::File::options().write(true).open(path).expect("open");
    file.set_times(FileTimes::new().set_modified(when))
        .expect("set the modification time");
}
