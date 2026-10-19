// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The artwork cache: the decoded cover of a track, kept in memory.
//!
//! A library view asks for the cover of a track every frame it draws, and
//! the cover is embedded in the track file. Answering the question means
//! reading the file and decoding the picture - far too much work for every
//! frame. [`ArtworkCache`] does that work once and keeps the result.
//!
//! See the crate documentation for the key, the bound and the decode rule.

use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rustamp_core::error::{Error, Result};

/// The longest edge of a cached cover, in pixels.
///
/// A picture whose longest edge is longer is scaled down so that it fits
/// into a square of this size; the aspect ratio is kept. A picture that
/// already fits is stored as it is.
pub const THUMBNAIL_MAX_EDGE: u32 = 64;

/// The bytes a decoder may allocate for one picture.
///
/// A picture that needs more is not decoded at all and counts as "no cover".
/// The limit keeps a single oversized picture from taking the memory the
/// whole cache is meant to bound.
pub const MAX_DECODE_BYTES: u64 = 128 * 1024 * 1024;

/// The number of covers a cache with the default bounds holds.
pub const DEFAULT_MAX_ENTRIES: usize = 256;

/// The byte budget of a cache with the default bounds.
///
/// A cover costs at most 16 KiB (a 64x64 RGBA image), so the entry count
/// binds first with the default thumbnail size. The budget is the second
/// bound and holds a cache that is configured with many entries.
pub const DEFAULT_MAX_BYTES: usize = 8 * 1024 * 1024;

/// A decoded cover image.
///
/// The pixels are 8-bit non-premultiplied RGBA, row-major, the first row on
/// top: the same layout `rustamp-gfx`'s `Image` uses, so a caller can blit
/// them without converting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverImage {
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The RGBA bytes: `width * height * 4` of them.
    pub pixels: Vec<u8>,
}

/// What the cache has done so far.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArtworkCacheStats {
    /// The number of times the cache went to the file to read its cover.
    pub reads: u64,
    /// The number of times a cached answer was served.
    pub hits: u64,
}

/// A bounded cache of decoded cover images.
///
/// The cache is keyed by the path of the track. Every entry carries the
/// `(mtime_ns, size)` pair its cover was decoded from, so a file that
/// changed loses its cover. The cache is bounded: it holds at most a
/// number of entries and at most a number of bytes, and it drops the least
/// recently used cover when a new one does not fit.
///
/// The cache can be shared between threads: it holds its entries behind a
/// mutex and never blocks on anything but itself.
pub struct ArtworkCache {
    inner: Mutex<Inner>,
}

impl ArtworkCache {
    /// A cache with the default bounds: [`DEFAULT_MAX_ENTRIES`] entries and
    /// [`DEFAULT_MAX_BYTES`].
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_MAX_ENTRIES, DEFAULT_MAX_BYTES)
    }

    /// A cache that holds at most `max_entries` answers and at most
    /// `max_bytes` of pixels.
    ///
    /// A bound of zero is allowed and means that nothing is cached: every
    /// lookup reads the file. An answer that does not fit into the budget
    /// is answered but not kept.
    pub fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                bytes: 0,
                clock: 0,
                max_entries,
                max_bytes,
                reads: 0,
                hits: 0,
            }),
        }
    }

    /// The cover of the track at `path`.
    ///
    /// * `Ok(Some(cover))` - the file has a cover that could be decoded.
    /// * `Ok(None)` - the file has no cover, or its cover is not an image
    ///   this cache can show. The answer is cached, so a track without a
    ///   cover is not read again.
    /// * `Err(_)` - the file could not be read at all. Nothing is cached:
    ///   a failure that may be transient must not be remembered as "no
    ///   cover".
    ///
    /// A file that cannot be stat'ed and a file whose read failed are such
    /// errors. A file whose tags cannot be parsed, and a cover that is not
    /// a decodable image, are properties of the file: they count as "no
    /// cover" and are cached like any other "no cover".
    pub fn get(&self, path: &Path) -> Result<Option<Arc<CoverImage>>> {
        let stat = match fs::metadata(path) {
            Ok(meta) => FileStat::of(&meta),
            Err(err) => return Err(map_stat_error(err, path)),
        };

        if let Some(known) = self.lookup(path, stat) {
            return Ok(known.answer());
        }

        let known = load_cover(path)?;
        self.store(path, stat, known.clone());
        Ok(known.answer())
    }

    /// The number of answers the cache holds.
    pub fn entry_count(&self) -> usize {
        self.lock().entries.len()
    }

    /// What the cache has done so far.
    pub fn stats(&self) -> ArtworkCacheStats {
        let inner = self.lock();
        ArtworkCacheStats {
            reads: inner.reads,
            hits: inner.hits,
        }
    }

    /// The cached answer for `path`, if the file still has the stat the
    /// answer was built from.
    fn lookup(&self, path: &Path, stat: FileStat) -> Option<Known> {
        let mut inner = self.lock();
        match inner.lookup(path, stat) {
            Some(known) => {
                inner.hits += 1;
                Some(known)
            }
            None => {
                // The file is not cached (or it changed): it is read.
                inner.reads += 1;
                None
            }
        }
    }

    /// Keep the answer for `path`.
    fn store(&self, path: &Path, stat: FileStat, known: Known) {
        let mut inner = self.lock();
        inner.insert(path.to_path_buf(), stat, known);
    }

    /// The entries of the cache.
    ///
    /// A panic while the lock was held must not make the cache unusable: the
    /// entries are still consistent, so the poisoned guard is taken anyway.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }
}

impl Default for ArtworkCache {
    fn default() -> Self {
        Self::new()
    }
}

/// What the cache knows about one path.
#[derive(Debug, Clone)]
enum Known {
    /// The file has a cover.
    Cover(Arc<CoverImage>),
    /// The file has no cover, or none that can be shown.
    NoCover,
}

impl Known {
    /// The answer a caller gets.
    fn answer(&self) -> Option<Arc<CoverImage>> {
        match self {
            Self::Cover(image) => Some(image.clone()),
            Self::NoCover => None,
        }
    }

    /// The pixels the answer costs.
    fn bytes(&self) -> usize {
        match self {
            Self::Cover(image) => image.pixels.len(),
            Self::NoCover => 0,
        }
    }
}

/// One entry of the cache.
struct Entry {
    /// The stat the answer was built from.
    stat: FileStat,
    /// The answer.
    known: Known,
    /// What the entry costs: the pixels plus the path it is filed under.
    bytes: usize,
    /// When the entry was last used; the smallest tick is the entry that is
    /// dropped first.
    tick: u64,
}

/// The `(mtime_ns, size)` pair a cached answer was built from.
///
/// It is the same pair the index uses to notice a changed file, so a cover
/// and a row are invalidated at the same moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStat {
    /// The modification time in nanoseconds since the Unix epoch, if the
    /// file system has one.
    mtime_ns: Option<i64>,
    /// The size of the file in bytes.
    size: u64,
}

impl FileStat {
    /// The stat of a file.
    fn of(meta: &fs::Metadata) -> Self {
        Self {
            mtime_ns: crate::mtime_of(meta),
            size: meta.len(),
        }
    }
}

/// The state of the cache.
struct Inner {
    /// The answers, by path.
    entries: HashMap<PathBuf, Entry>,
    /// The bytes the entries cost together.
    bytes: usize,
    /// The clock that orders the entries by their last use.
    clock: u64,
    /// The most entries the cache holds.
    max_entries: usize,
    /// The most bytes the cache holds.
    max_bytes: usize,
    /// The number of reads from disk.
    reads: u64,
    /// The number of answers served from the cache.
    hits: u64,
}

impl Inner {
    /// The cached answer for `path`, if the file still has `stat`.
    fn lookup(&mut self, path: &Path, stat: FileStat) -> Option<Known> {
        let tick = self.next_tick();
        let entry = self.entries.get_mut(path)?;
        if entry.stat != stat {
            // The file changed: the entry is stale and is replaced by the
            // answer for the new file.
            return None;
        }
        entry.tick = tick;
        Some(entry.known.clone())
    }

    /// Keep the answer for `path` and make room for it.
    fn insert(&mut self, path: PathBuf, stat: FileStat, known: Known) {
        let bytes = known.bytes() + path.as_os_str().len();
        if let Some(old) = self.entries.remove(&path) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }

        // The least recently used answer goes first.
        while !self.entries.is_empty()
            && (self.entries.len() >= self.max_entries || self.bytes + bytes > self.max_bytes)
        {
            let Some(victim) = self.lru() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&victim) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            }
        }

        if self.entries.len() >= self.max_entries || self.bytes + bytes > self.max_bytes {
            // The answer does not fit into the budget: the cache stays as it
            // is and the answer is not kept.
            return;
        }

        let tick = self.next_tick();
        self.bytes += bytes;
        self.entries.insert(
            path,
            Entry {
                stat,
                known,
                bytes,
                tick,
            },
        );
    }

    /// The path of the least recently used entry.
    fn lru(&self) -> Option<PathBuf> {
        self.entries
            .iter()
            .min_by_key(|(_, entry)| entry.tick)
            .map(|(path, _)| path.clone())
    }

    /// The next point in time of the LRU order.
    fn next_tick(&mut self) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.clock
    }
}

/// Read the cover of `path` and decode it.
fn load_cover(path: &Path) -> Result<Known> {
    match rustamp_meta::read_artwork(path) {
        Ok(Some(artwork)) => Ok(match decode_cover(&artwork.data) {
            Some(image) => Known::Cover(Arc::new(image)),
            None => Known::NoCover,
        }),
        Ok(None) => Ok(Known::NoCover),
        Err(Error::Io(err)) => Err(Error::Io(err)),
        Err(_other) => {
            // The file has no cover this cache can show: its tag container
            // cannot be parsed. That is a property of the file, so the
            // answer is cached like any other "no cover".
            Ok(Known::NoCover)
        }
    }
}

/// Decode the picture bytes into a cover.
///
/// Returns `None` when the bytes are not a picture this cache can show:
/// an unknown format, a broken picture, or one that needs more memory than
/// [`MAX_DECODE_BYTES`].
fn decode_cover(bytes: &[u8]) -> Option<CoverImage> {
    // The format is guessed from the picture itself, not from the MIME type
    // of the tag: the guesser knows the magic bytes, the tag does not.
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;

    // A cover that is too large is scaled down into the thumbnail size; the
    // aspect ratio is kept.
    let thumbnail = if decoded.width() > THUMBNAIL_MAX_EDGE || decoded.height() > THUMBNAIL_MAX_EDGE
    {
        decoded.thumbnail(THUMBNAIL_MAX_EDGE, THUMBNAIL_MAX_EDGE)
    } else {
        decoded
    };
    let rgba = thumbnail.to_rgba8();
    Some(CoverImage {
        width: rgba.width(),
        height: rgba.height(),
        pixels: rgba.into_raw(),
    })
}

/// The error of a file that cannot be stat'ed.
fn map_stat_error(err: std::io::Error, path: &Path) -> Error {
    if err.kind() == std::io::ErrorKind::NotFound {
        Error::NotFound(path.display().to_string())
    } else {
        Error::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::FileTimes;
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    /// A synthetic PNG of one colour.
    fn png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let buffer = image::RgbaImage::from_pixel(width, height, image::Rgba(color));
        encode(
            image::DynamicImage::ImageRgba8(buffer),
            image::ImageFormat::Png,
        )
    }

    /// A synthetic JPEG of one colour.
    fn jpeg(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
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
    fn mp3_with_cover(mime: &str, picture: &[u8]) -> Vec<u8> {
        id3v2_tag(&[apic_frame(mime, picture)])
    }

    /// An MP3 file whose only content is an ID3v2 tag without a picture.
    ///
    /// The title is a hundred bytes long, because the MPEG reader seeks
    /// back over the last bytes of the file to look for an APE tag: a file
    /// that is shorter than that cannot be read at all.
    fn mp3_without_cover() -> Vec<u8> {
        let mut data = vec![0x00]; // ISO-8859-1
        data.extend_from_slice(&[b'x'; 100]);
        id3v2_tag(&[id3v2_frame(b"TIT2", &data)])
    }

    /// Write a file into `dir`.
    fn write(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, bytes).expect("write the file");
        path
    }

    /// The modification time of a file.
    fn modified(path: &Path) -> SystemTime {
        fs::metadata(path)
            .expect("stat")
            .modified()
            .expect("modified")
    }

    /// Set the modification time of a file.
    fn set_modified(path: &Path, when: SystemTime) {
        let file = fs::File::options().write(true).open(path).expect("open");
        file.set_times(FileTimes::new().set_modified(when))
            .expect("set the modification time");
    }

    #[test]
    fn a_cover_is_decoded_into_rgba_pixels() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "cover.mp3",
            &mp3_with_cover("image/png", &png(2, 2, RED)),
        );
        let cache = ArtworkCache::new();

        let cover = cache.get(&path).unwrap().expect("the cover");
        assert_eq!(cover.width, 2);
        assert_eq!(cover.height, 2);
        assert_eq!(cover.pixels.len(), 2 * 2 * 4);
        assert_eq!(&cover.pixels[..4], &RED);
        assert_eq!(&cover.pixels[8..12], &RED);
    }

    #[test]
    fn a_jpeg_cover_is_decoded() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "cover.mp3",
            &mp3_with_cover("image/jpeg", &jpeg(8, 8, BLUE)),
        );
        let cache = ArtworkCache::new();

        let cover = cache.get(&path).unwrap().expect("the cover");
        assert_eq!((cover.width, cover.height), (8, 8));
        // JPEG is lossy, so the colour is only close to the original.
        assert!(
            cover.pixels[2] > 200 && cover.pixels[0] < 60,
            "the blue channel survives: {:?}",
            &cover.pixels[..4]
        );
    }

    #[test]
    fn a_large_cover_is_scaled_down_to_the_thumbnail_edge() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "big.mp3",
            &mp3_with_cover("image/png", &png(200, 100, RED)),
        );
        let cache = ArtworkCache::new();

        let cover = cache.get(&path).unwrap().expect("the cover");
        assert_eq!(cover.width, THUMBNAIL_MAX_EDGE);
        assert_eq!(
            cover.height,
            THUMBNAIL_MAX_EDGE / 2,
            "the aspect ratio is kept"
        );
    }

    #[test]
    fn an_unchanged_file_is_served_from_the_cache() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "cover.mp3",
            &mp3_with_cover("image/png", &png(2, 2, RED)),
        );
        let cache = ArtworkCache::new();

        let first = cache.get(&path).unwrap().expect("the cover");
        let second = cache.get(&path).unwrap().expect("the cover");
        assert!(
            Arc::ptr_eq(&first, &second),
            "the cached cover is the same image"
        );
        assert_eq!(cache.stats().reads, 1, "the file is read once");
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.entry_count(), 1);
    }

    #[test]
    fn a_changed_file_is_read_again() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "cover.mp3",
            &mp3_with_cover("image/png", &png(4, 4, RED)),
        );
        let cache = ArtworkCache::new();

        let first = cache.get(&path).unwrap().expect("the cover");

        // The picture changes, so the file changes with it.
        write(
            &dir,
            "cover.mp3",
            &mp3_with_cover("image/png", &png(8, 8, BLUE)),
        );
        let second = cache.get(&path).unwrap().expect("the cover");

        assert_eq!(cache.stats().reads, 2, "the changed file is read again");
        assert_eq!(second.width, 8);
        assert_ne!(first.pixels, second.pixels);
        assert_eq!(cache.entry_count(), 1, "the stale cover is replaced");
    }

    #[test]
    fn a_new_modification_time_invalidates_the_cover() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "cover.mp3",
            &mp3_with_cover("image/png", &png(4, 4, RED)),
        );
        let cache = ArtworkCache::new();
        cache.get(&path).unwrap().expect("the cover");

        // The file keeps its size, but its modification time changes.
        set_modified(&path, SystemTime::now() + Duration::from_secs(60));
        cache.get(&path).unwrap().expect("the cover");

        assert_eq!(cache.stats().reads, 2, "the file is read again");
    }

    #[test]
    fn a_rewrite_with_the_same_stat_keeps_the_cached_cover() {
        let dir = TempDir::new().unwrap();
        let bytes = mp3_with_cover("image/png", &png(4, 4, RED));
        let path = write(&dir, "cover.mp3", &bytes);
        let cache = ArtworkCache::new();
        let first = cache.get(&path).unwrap().expect("the cover");
        let stamp = modified(&path);

        // The same length, different bytes, the same modification time: the
        // key of the entry did not change, so the cache keeps its cover.
        let mut changed = bytes.clone();
        let last = changed.len() - 1;
        changed[last] ^= 0xFF;
        write(&dir, "cover.mp3", &changed);
        set_modified(&path, stamp);
        assert_eq!(modified(&path), stamp, "the modification time round trips");

        let second = cache.get(&path).unwrap().expect("the cover");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.stats().reads, 1, "the file is not read again");
    }

    #[test]
    fn a_file_without_a_cover_is_cached() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "plain.mp3", &mp3_without_cover());
        let cache = ArtworkCache::new();

        assert!(cache.get(&path).unwrap().is_none());
        assert!(cache.get(&path).unwrap().is_none());
        assert_eq!(cache.stats().reads, 1, "the file is read once");
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.entry_count(), 1, "the missing cover is cached");
    }

    #[test]
    fn a_cover_that_is_not_an_image_is_cached_as_no_cover() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "broken.mp3",
            &mp3_with_cover("image/png", b"not an image"),
        );
        let cache = ArtworkCache::new();

        assert!(cache.get(&path).unwrap().is_none());
        assert!(cache.get(&path).unwrap().is_none());
        assert_eq!(cache.entry_count(), 1, "the answer is cached");
        assert_eq!(cache.stats().reads, 1, "the file is read once");
    }

    #[test]
    fn the_least_recently_used_cover_is_evicted() {
        let dir = TempDir::new().unwrap();
        let a = write(&dir, "a.mp3", &mp3_with_cover("image/png", &png(2, 2, RED)));
        let b = write(
            &dir,
            "b.mp3",
            &mp3_with_cover("image/png", &png(2, 2, BLUE)),
        );
        let c = write(&dir, "c.mp3", &mp3_with_cover("image/png", &png(2, 2, RED)));
        let cache = ArtworkCache::with_limits(2, DEFAULT_MAX_BYTES);

        cache.get(&a).unwrap().expect("the cover");
        cache.get(&b).unwrap().expect("the cover");
        // Using A again makes B the least recently used cover.
        cache.get(&a).unwrap().expect("the cover");
        assert_eq!(cache.entry_count(), 2);

        cache.get(&c).unwrap().expect("the cover");
        assert_eq!(cache.entry_count(), 2, "the cache stays at its bound");

        // B was dropped, so it is read again.
        let reads = cache.stats().reads;
        cache.get(&b).unwrap().expect("the cover");
        assert_eq!(
            cache.stats().reads,
            reads + 1,
            "the evicted cover is read again"
        );
    }

    #[test]
    fn a_cover_that_does_not_fit_the_budget_is_not_cached() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "cover.mp3",
            &mp3_with_cover("image/png", &png(2, 2, RED)),
        );
        // A 2x2 cover costs 16 bytes of pixels, plus the path it is filed
        // under: it does not fit into a budget of 16 bytes.
        let cache = ArtworkCache::with_limits(8, 16);

        assert!(cache.get(&path).unwrap().is_some());
        assert_eq!(cache.entry_count(), 0, "the cover is not kept");
        assert!(cache.get(&path).unwrap().is_some());
        assert_eq!(cache.stats().reads, 2, "the file is read every time");
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let dir = TempDir::new().unwrap();
        let cache = ArtworkCache::new();

        let err = cache.get(&dir.path().join("gone.mp3")).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{err:?}");
        assert_eq!(cache.entry_count(), 0);
    }

    #[test]
    fn the_cache_can_be_shared_between_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ArtworkCache>();
    }
}
