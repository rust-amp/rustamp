// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The artwork cache: the decoded cover of a track, kept in memory.
//!
//! A library view asks for the cover of a track every frame it draws, and
//! the cover is either embedded in the track file or a picture that sits
//! next to it. Answering the question means reading the file and decoding
//! the picture - far too much work for every frame. [`ArtworkCache`] does
//! that work once and keeps the result.
//!
//! # Where the cover comes from
//!
//! The **embedded** front cover of the track wins: it belongs to the file
//! and travels with it. A track whose file carries no picture falls back
//! to the **sidecar** image in the same directory - the `cover.jpg` of the
//! album folder, for example. The names that are looked for, and the
//! order they are tried in, are [`SIDECAR_NAMES`].
//!
//! # When the answer is stale
//!
//! An answer is valid while the track keeps its `(mtime_ns, size)` and,
//! unless it is the embedded cover, while the sidecar next to the track
//! keeps its `(mtime_ns, size)`. A rewritten `cover.jpg` therefore
//! invalidates the cached cover, and a `cover.jpg` that appears next to a
//! track that had none invalidates the cached "no cover".
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

/// The names of the sidecar images, in the order they are tried.
///
/// A track whose file carries no picture falls back to a cover image in
/// the same directory. These are the names that are looked for, and this
/// is the order they are tried in: the first name that is a regular file
/// wins, so `cover.jpg` beats `folder.png`. A file that wins but cannot
/// be decoded is "no cover": the next name is not tried.
///
/// The list is fixed, so a caller knows what the cache looks for. It is
/// deliberately short and covers the names that are in wide use; a name
/// outside the list is not looked for.
pub const SIDECAR_NAMES: &[&str] = &[
    "cover.jpg",
    "cover.png",
    "folder.jpg",
    "folder.png",
    "front.jpg",
    "album.jpg",
];

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
/// changed loses its cover. A cover that came from a sidecar image also
/// carries the `(mtime_ns, size)` pair of that image, so a changed
/// `cover.jpg` loses the cover as well. The cache is bounded: it holds at
/// most a number of entries and at most a number of bytes, and it drops
/// the least recently used cover when a new one does not fit.
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
    /// The cover is the **embedded** picture of the track; a track whose
    /// file carries none falls back to the **sidecar** image in the same
    /// directory (see [`SIDECAR_NAMES`]).
    ///
    /// * `Ok(Some(cover))` - the track has a cover that could be decoded.
    /// * `Ok(None)` - the track has no cover, or none that this cache can
    ///   show. The answer is cached, so a track without a cover is not
    ///   read again.
    /// * `Err(_)` - the track could not be read at all. Nothing is cached:
    ///   a failure that may be transient must not be remembered as "no
    ///   cover".
    ///
    /// A file that cannot be stat'ed and a file whose read failed are such
    /// errors. A file whose tags cannot be parsed, a cover that is not a
    /// decodable image and a sidecar that cannot be read or decoded are
    /// properties of the files: they count as "no cover" and are cached
    /// like any other "no cover".
    pub fn get(&self, path: &Path) -> Result<Option<Arc<CoverImage>>> {
        let stat = match fs::metadata(path) {
            Ok(meta) => FileStat::of(&meta),
            Err(err) => return Err(map_stat_error(err, path)),
        };

        if let Some(known) = self.lookup(path, stat) {
            self.lock().hits += 1;
            return Ok(known.answer());
        }

        self.lock().reads += 1;
        let loaded = load_cover(path)?;
        let answer = loaded.known.answer();
        self.store(path, stat, loaded);
        Ok(answer)
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

    /// The cached answer for `path`, if it is still valid.
    ///
    /// The answer is valid while the track keeps its stat and - unless it
    /// is the embedded cover of the track - while the sidecar next to the
    /// track keeps its stat.
    fn lookup(&self, path: &Path, stat: FileStat) -> Option<Known> {
        let entry = {
            let inner = self.lock();
            let entry = inner.entries.get(path)?;
            if entry.stat != stat {
                // The track changed: the answer is stale.
                return None;
            }
            entry.clone()
        };

        if !entry.embedded {
            // The answer depends on the sidecar next to the track: the
            // file system has to say whether it is still the same file
            // with the same content.
            if find_sidecar(path) != entry.sidecar {
                return None;
            }
        }

        // The entry is taken again: another thread may have replaced it
        // while the sidecar was looked up.
        let mut inner = self.lock();
        let tick = inner.next_tick();
        let current = inner.entries.get_mut(path)?;
        if current.stat != entry.stat
            || current.sidecar != entry.sidecar
            || current.embedded != entry.embedded
        {
            // The entry changed under us: the answer is built again.
            return None;
        }
        current.tick = tick;
        Some(current.known.clone())
    }

    /// Keep the answer for `path`.
    fn store(&self, path: &Path, stat: FileStat, loaded: Loaded) {
        let mut inner = self.lock();
        inner.insert(path.to_path_buf(), stat, loaded);
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

/// The sidecar image next to a track.
///
/// The path and the stat of the file: the stat is what makes a changed
/// sidecar invalidate the cached answer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sidecar {
    /// The path of the sidecar file.
    path: PathBuf,
    /// The stat the picture was decoded from.
    stat: FileStat,
}

/// What the file system said about the cover of a track.
struct Loaded {
    /// The answer.
    known: Known,
    /// The sidecar the answer depends on: the file next to the track, or
    /// `None` when the directory holds none.
    ///
    /// The field is only meaningful when `embedded` is `false`: an
    /// embedded cover wins over every sidecar, so the sidecar is not even
    /// looked for.
    sidecar: Option<Sidecar>,
    /// Whether the answer is the embedded cover of the track.
    embedded: bool,
}

/// One entry of the cache.
#[derive(Clone)]
struct Entry {
    /// The stat of the track the answer was built from.
    stat: FileStat,
    /// The sidecar the answer was built from.
    sidecar: Option<Sidecar>,
    /// Whether the answer is the embedded cover of the track.
    ///
    /// An embedded cover wins over a sidecar image, so the sidecar cannot
    /// change the answer and does not have to be looked for again.
    embedded: bool,
    /// The answer.
    known: Known,
    /// What the entry costs: the pixels plus the paths it is filed under.
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
    /// Keep the answer for `path` and make room for it.
    fn insert(&mut self, path: PathBuf, stat: FileStat, loaded: Loaded) {
        let bytes = loaded.known.bytes()
            + path.as_os_str().len()
            + loaded
                .sidecar
                .as_ref()
                .map_or(0, |sidecar| sidecar.path.as_os_str().len());
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
                sidecar: loaded.sidecar,
                embedded: loaded.embedded,
                known: loaded.known,
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

/// Read the cover of the track at `path`.
///
/// The embedded front cover of the track wins; a track whose file carries
/// no picture falls back to the sidecar image in the same directory.
fn load_cover(path: &Path) -> Result<Loaded> {
    match rustamp_meta::read_artwork(path) {
        Ok(Some(artwork)) => {
            if let Some(image) = decode_cover(&artwork.data) {
                // The embedded cover wins over every sidecar, so the
                // sidecar is not even looked for.
                return Ok(Loaded {
                    known: Known::Cover(Arc::new(image)),
                    sidecar: None,
                    embedded: true,
                });
            }
            // The embedded picture is not a picture this cache can show:
            // the sidecar is the fallback.
        }
        Ok(None) => {
            // The track carries no picture: the sidecar is the fallback.
        }
        Err(Error::Io(err)) => return Err(Error::Io(err)),
        Err(_other) => {
            // The tag container cannot be parsed: the track has no
            // embedded cover this cache can show, so the sidecar is the
            // fallback.
        }
    }
    Ok(load_sidecar(path))
}

/// The sidecar cover of the track at `path`.
///
/// A sidecar that cannot be read or cannot be decoded is "no cover": it
/// must not break the row it belongs to. The sidecar itself is part of
/// the answer, so a sidecar that is fixed later invalidates it.
fn load_sidecar(path: &Path) -> Loaded {
    let sidecar = find_sidecar(path);
    let known = match &sidecar {
        Some(sidecar) => match read_picture(&sidecar.path) {
            Some(image) => Known::Cover(Arc::new(image)),
            None => Known::NoCover,
        },
        None => Known::NoCover,
    };
    Loaded {
        known,
        sidecar,
        embedded: false,
    }
}

/// The sidecar image next to the track at `path`.
///
/// The names of [`SIDECAR_NAMES`] are tried in order in the directory of
/// the track; the first one that is a regular file wins. `None` when the
/// directory holds no sidecar at all.
fn find_sidecar(path: &Path) -> Option<Sidecar> {
    let dir = path.parent()?;
    for name in SIDECAR_NAMES {
        let candidate = dir.join(name);
        let Ok(meta) = fs::metadata(&candidate) else {
            continue;
        };
        if !meta.is_file() {
            // A directory or a special file under a cover name is not a
            // cover: the next name is tried.
            continue;
        }
        return Some(Sidecar {
            path: candidate,
            stat: FileStat::of(&meta),
        });
    }
    None
}

/// Read the picture file at `path` and decode it.
fn read_picture(path: &Path) -> Option<CoverImage> {
    let bytes = fs::read(path).ok()?;
    decode_cover(&bytes)
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
    use crate::testsupport::{
        jpeg, modified, mp3_with_cover, mp3_without_cover, png, set_modified, write_in,
    };
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    /// Write a file into `dir`.
    fn write(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        crate::testsupport::write_in(dir.path(), name, bytes)
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

    // ---- sidecar cover art ----

    #[test]
    fn an_embedded_cover_wins_over_a_sidecar() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "track.mp3",
            &mp3_with_cover("image/png", &png(2, 2, RED)),
        );
        // The sidecar is right next to the track, but the embedded cover
        // wins.
        write(&dir, "cover.jpg", &jpeg(8, 8, BLUE));
        let cache = ArtworkCache::new();

        let cover = cache.get(&path).unwrap().expect("the cover");
        assert_eq!((cover.width, cover.height), (2, 2), "the embedded cover");
        assert_eq!(&cover.pixels[..4], &RED);
    }

    #[test]
    fn a_sidecar_cover_is_used_when_the_track_has_none() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "track.mp3", &mp3_without_cover());
        write(&dir, "cover.jpg", &jpeg(8, 8, BLUE));
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
    fn a_sidecar_cover_is_scaled_down_like_an_embedded_one() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "track.mp3", &mp3_without_cover());
        write(&dir, "cover.png", &png(200, 100, RED));
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
    fn the_sidecar_names_are_the_documented_list() {
        assert_eq!(
            SIDECAR_NAMES,
            &[
                "cover.jpg",
                "cover.png",
                "folder.jpg",
                "folder.png",
                "front.jpg",
                "album.jpg",
            ]
        );
    }

    #[test]
    fn the_first_sidecar_name_wins() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "track.mp3", &mp3_without_cover());
        // Every name of the list is there, each with its own size: the
        // first name of the list is the one that is used.
        for (index, name) in SIDECAR_NAMES.iter().enumerate() {
            write(&dir, name, &png(2 + index as u32, 2, RED));
        }
        let cache = ArtworkCache::new();

        let cover = cache.get(&path).unwrap().expect("the cover");
        assert_eq!(cover.width, 2, "the first name wins: {SIDECAR_NAMES:?}");
    }

    #[test]
    fn a_later_sidecar_name_is_used_when_the_earlier_ones_are_missing() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "track.mp3", &mp3_without_cover());
        // Only the last name of the list is there.
        let last = SIDECAR_NAMES.last().expect("the list is not empty");
        write(&dir, last, &png(3, 3, RED));
        let cache = ArtworkCache::new();

        let cover = cache.get(&path).unwrap().expect("the cover");
        assert_eq!((cover.width, cover.height), (3, 3), "{last} is used");
    }

    #[test]
    fn the_sidecar_is_looked_for_next_to_the_track() {
        let dir = TempDir::new().unwrap();
        let album = dir.path().join("album");
        fs::create_dir(&album).unwrap();
        // The cover sits one directory above the track: it is not the
        // cover of that track.
        write(&dir, "cover.jpg", &jpeg(8, 8, BLUE));
        let path = write_in(&album, "track.mp3", &mp3_without_cover());
        let cache = ArtworkCache::new();

        assert!(
            cache.get(&path).unwrap().is_none(),
            "the cover of the directory above is not used"
        );
    }

    #[test]
    fn a_changed_sidecar_invalidates_the_cached_cover() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "track.mp3", &mp3_without_cover());
        write(&dir, "cover.jpg", &jpeg(8, 8, BLUE));
        let cache = ArtworkCache::new();

        let first = cache.get(&path).unwrap().expect("the cover");
        assert_eq!(cache.stats().reads, 1);

        // The sidecar changes: the cached cover is stale.
        write(&dir, "cover.jpg", &png(4, 4, RED));
        let second = cache.get(&path).unwrap().expect("the cover");

        assert_eq!(cache.stats().reads, 2, "the changed sidecar is read again");
        assert_eq!((second.width, second.height), (4, 4));
        assert_ne!(first.pixels, second.pixels);
        assert_eq!(cache.entry_count(), 1, "the stale cover is replaced");
    }

    #[test]
    fn a_new_modification_time_of_the_sidecar_invalidates_the_cover() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "track.mp3", &mp3_without_cover());
        let sidecar = write(&dir, "cover.png", &png(4, 4, RED));
        let size = fs::metadata(&sidecar).unwrap().len();
        let cache = ArtworkCache::new();

        let first = cache.get(&path).unwrap().expect("the cover");
        assert_eq!(&first.pixels[..4], &RED);

        // The sidecar keeps its size, but its content and its modification
        // time change.
        write(&dir, "cover.png", &png(4, 4, BLUE));
        set_modified(&sidecar, SystemTime::now() + Duration::from_secs(60));
        assert_eq!(
            fs::metadata(&sidecar).unwrap().len(),
            size,
            "the sidecar keeps its size"
        );

        let second = cache.get(&path).unwrap().expect("the cover");
        assert_eq!(cache.stats().reads, 2, "the changed sidecar is read again");
        assert_eq!(&second.pixels[..4], &BLUE);
    }

    #[test]
    fn a_sidecar_that_appears_later_is_picked_up() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "track.mp3", &mp3_without_cover());
        let cache = ArtworkCache::new();

        assert!(cache.get(&path).unwrap().is_none(), "there is no cover yet");
        assert_eq!(cache.stats().reads, 1);

        // The cover appears next to the track: the cached "no cover" is
        // stale.
        write(&dir, "cover.jpg", &jpeg(8, 8, BLUE));
        let cover = cache.get(&path).unwrap().expect("the cover");
        assert_eq!((cover.width, cover.height), (8, 8));
        assert_eq!(cache.stats().reads, 2, "the track is read again");
    }

    #[test]
    fn a_removed_sidecar_invalidates_the_cached_cover() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "track.mp3", &mp3_without_cover());
        write(&dir, "cover.jpg", &jpeg(8, 8, BLUE));
        let cache = ArtworkCache::new();

        assert!(cache.get(&path).unwrap().is_some(), "the cover");

        fs::remove_file(dir.path().join("cover.jpg")).unwrap();
        assert!(
            cache.get(&path).unwrap().is_none(),
            "the cover is gone, so there is none"
        );
        assert_eq!(cache.stats().reads, 2, "the track is read again");
    }

    #[test]
    fn a_directory_without_a_cover_keeps_answering_no_cover() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "track.mp3", &mp3_without_cover());
        let cache = ArtworkCache::new();

        assert!(cache.get(&path).unwrap().is_none());
        assert!(cache.get(&path).unwrap().is_none());
        assert_eq!(cache.stats().reads, 1, "the track is read once");
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.entry_count(), 1, "the missing cover is cached");
    }

    #[test]
    fn an_unchanged_sidecar_is_served_from_the_cache() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "track.mp3", &mp3_without_cover());
        write(&dir, "cover.jpg", &jpeg(8, 8, BLUE));
        let cache = ArtworkCache::new();

        let first = cache.get(&path).unwrap().expect("the cover");
        let second = cache.get(&path).unwrap().expect("the cover");
        assert!(
            Arc::ptr_eq(&first, &second),
            "the cached cover is the same image"
        );
        assert_eq!(cache.stats().reads, 1, "the track is read once");
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn a_corrupt_sidecar_is_treated_as_no_cover() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "track.mp3", &mp3_without_cover());
        write(&dir, "cover.jpg", b"not an image");
        let cache = ArtworkCache::new();

        // The broken sidecar is not an error: the row keeps working.
        assert!(cache.get(&path).unwrap().is_none());
        assert!(cache.get(&path).unwrap().is_none());
        assert_eq!(cache.entry_count(), 1, "the answer is cached");
        assert_eq!(cache.stats().reads, 1, "the track is read once");
    }

    #[test]
    fn a_fixed_sidecar_is_picked_up() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "track.mp3", &mp3_without_cover());
        write(&dir, "cover.jpg", b"not an image");
        let cache = ArtworkCache::new();

        assert!(cache.get(&path).unwrap().is_none(), "the sidecar is broken");

        // The sidecar is repaired: the cached "no cover" is stale.
        write(&dir, "cover.jpg", &png(4, 4, RED));
        let cover = cache.get(&path).unwrap().expect("the cover");
        assert_eq!((cover.width, cover.height), (4, 4));
        assert_eq!(&cover.pixels[..4], &RED);
        assert_eq!(cache.stats().reads, 2, "the track is read again");
    }
}
