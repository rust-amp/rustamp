// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Property tests for the artwork cache.
//!
//! The tests in `artwork.rs` pin one behaviour each. These pin the
//! *invariants* that have to hold for every file, every sidecar and every
//! order of changes:
//!
//! * **coherence**: the answer the cache serves is the answer a fresh
//!   read of the file gives - a cached cover is never stale;
//! * **exact invalidation**: an answer is thrown away exactly when the
//!   stat of the file it was built from changes - the file of the track
//!   always, the sidecar next to it only when the answer is not the
//!   embedded cover;
//! * **the documented order**: the embedded cover wins over the sidecar,
//!   and a sidecar that is missing or broken is "no cover" and never an
//!   error;
//! * **no stale cache after a rewrite**: a file that is written again
//!   shows its new cover;
//! * **bounded growth**: the cache stays inside its bounds, and the entry
//!   of a file that is gone is never served;
//! * **a corrupt picture is no cover**: it is never cached as a cover,
//!   and a file that is repaired later is picked up;
//! * **nothing panics**: a missing directory, an unreadable file, an empty
//!   picture and a picture that declares a huge size are all answered.
//!
//! The files are synthetic and live in a temporary directory: a track is
//! an MP3 file with an ID3v2 tag, a sidecar is a picture file. The
//! generator is seeded, so a failing case is reproducible from its seed
//! alone.

use crate::THUMBNAIL_MAX_EDGE;
use crate::testsupport::{
    Rng, jpeg, modified, mp3_with_cover, mp3_without_cover, png, set_modified, write_in,
};
use crate::{ArtworkCache, CoverImage, DEFAULT_MAX_BYTES, DEFAULT_MAX_ENTRIES, SIDECAR_NAMES};
use rustamp_core::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

/// The seeds the properties run over. Every seed builds another scenario.
const SEEDS: &[u64] = &[
    0x1234_5678_9abc_def0,
    0x0f1e_2d3c_4b5a_6978,
    0xdead_beef_cafe_f00d,
    0x00c0_ffee_0bad_f00d,
    0x7fff_ffff_ffff_ffff,
];

/// The number of tracks a scenario holds.
const TRACKS: usize = 4;

/// The number of changes a scenario runs.
const CHANGES: usize = 16;

/// The red the tests paint a cover with.
const RED: [u8; 4] = [255, 0, 0, 255];

/// The blue the tests paint a cover with.
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// One track of a scenario.
struct Track {
    /// The folder of the track: the sidecar is looked for here.
    dir: PathBuf,
    /// The file of the track.
    path: PathBuf,
    /// The bytes the file was last written with.
    bytes: Vec<u8>,
    /// Whether the file carries a cover of its own.
    ///
    /// The field says which file the answer was built from: the
    /// embedded cover wins over every sidecar, so the sidecar next to
    /// the track is only part of the answer when the track has no
    /// embedded cover of its own.
    covered: bool,
    /// Whether the file is on the disk.
    alive: bool,
    /// The name of the sidecar next to the track, if there is one.
    sidecar: Option<String>,
}

/// A folder of tracks that a scenario changes while the cache watches.
struct Scenario {
    /// The folder the scenario lives in.
    dir: TempDir,
    /// The tracks of the scenario.
    tracks: Vec<Track>,
    /// The clock the modification times come from.
    clock: u64,
    /// The second the modification times start at.
    base: SystemTime,
}

impl Scenario {
    /// A scenario with `tracks` tracks, drawn by `seed`.
    ///
    /// Every track lives in a folder of its own, so the sidecar of one
    /// track cannot change the answer of another one.
    fn new(seed: u64, tracks: usize) -> Self {
        let mut rng = Rng::new(seed);
        let dir = TempDir::new().expect("a temporary directory");
        let base = SystemTime::now() - Duration::from_secs(3600);
        let mut scenario = Scenario {
            dir,
            tracks: Vec::new(),
            clock: 0,
            base,
        };
        for index in 0..tracks {
            let folder = scenario.dir.path().join(format!("album{index}"));
            fs::create_dir(&folder).expect("the album folder is created");
            let covered = rng.flip();
            let bytes = if covered {
                mp3_with_cover("image/png", &picture(&mut rng))
            } else {
                mp3_without_cover()
            };
            scenario.tracks.push(Track {
                dir: folder.clone(),
                path: folder.join("track.mp3"),
                bytes: Vec::new(),
                covered,
                alive: false,
                sidecar: None,
            });
            scenario.write_track(index, &bytes);
            // Every second track has a sidecar next to it.
            if index % 2 == 1 {
                let name = *rng.pick(SIDECAR_NAMES);
                let bytes = picture(&mut rng);
                scenario.write_sidecar(index, name, &bytes);
            }
        }
        scenario
    }

    /// Write `bytes` into the file of the track `index` and give the file
    /// a modification time of its own.
    fn write_track(&mut self, index: usize, bytes: &[u8]) {
        let path = self.tracks[index].path.clone();
        fs::write(&path, bytes).expect("the track is written");
        let stamp = self.stamp();
        set_modified(&path, stamp);
        let track = &mut self.tracks[index];
        track.bytes = bytes.to_vec();
        track.alive = true;
    }

    /// Give the file of the track `index` a new modification time.
    fn touch_track(&mut self, index: usize) {
        if !self.tracks[index].alive {
            // The file is not there: there is nothing to touch.
            return;
        }
        let path = self.tracks[index].path.clone();
        let stamp = self.stamp();
        set_modified(&path, stamp);
    }

    /// Remove the file of the track `index`.
    fn delete_track(&mut self, index: usize) {
        if !self.tracks[index].alive {
            // The file is not there: there is nothing to remove.
            return;
        }
        let path = self.tracks[index].path.clone();
        fs::remove_file(&path).expect("the track is removed");
        self.tracks[index].alive = false;
    }

    /// Write a sidecar image next to the track `index`.
    fn write_sidecar(&mut self, index: usize, name: &str, bytes: &[u8]) {
        let dir = self.tracks[index].dir.clone();
        let path = write_in(&dir, name, bytes);
        let stamp = self.stamp();
        set_modified(&path, stamp);
        self.tracks[index].sidecar = Some(name.to_string());
    }

    /// Remove the sidecar image next to the track `index`.
    fn remove_sidecar(&mut self, index: usize) {
        if let Some(name) = self.tracks[index].sidecar.take() {
            let dir = self.tracks[index].dir.clone();
            let _ = fs::remove_file(dir.join(name));
        }
    }

    /// The next modification time.
    ///
    /// Every write gets a second of its own, so a change is always
    /// visible: the cache compares the modification time down to the
    /// nanosecond, and a file system that only counts seconds would give
    /// two writes of the same second the same stamp.
    fn stamp(&mut self) -> SystemTime {
        self.clock += 1;
        self.base + Duration::from_secs(self.clock)
    }
}

/// A change the scenario applies to one of its tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    /// The file is written again with a cover of its own.
    NewCover,
    /// The file is written again without a cover.
    StripCover,
    /// The file keeps its bytes and gets a new modification time.
    Touch,
    /// The file is written again with the bytes it had.
    Restore,
    /// The file is removed.
    Delete,
    /// The sidecar next to the file is written again.
    ReplaceSidecar,
    /// The sidecar next to the file is removed.
    RemoveSidecar,
}

impl Change {
    /// The changes a scenario draws from.
    const ALL: &'static [Change] = &[
        Change::NewCover,
        Change::StripCover,
        Change::Touch,
        Change::Restore,
        Change::Delete,
        Change::ReplaceSidecar,
        Change::RemoveSidecar,
    ];

    /// The name of the change, for the failure messages.
    fn name(self) -> &'static str {
        match self {
            Change::NewCover => "a new cover",
            Change::StripCover => "no cover",
            Change::Touch => "a touch",
            Change::Restore => "a restore",
            Change::Delete => "a deletion",
            Change::ReplaceSidecar => "a new sidecar",
            Change::RemoveSidecar => "no sidecar",
        }
    }
}

/// Apply `change` to the track `index` of the scenario.
///
/// The answer of the track is invalidated when the stat of the file it
/// was built from changed: the file of the track always, the sidecar
/// next to it only when the answer is not the embedded cover. The
/// function says whether the answer was invalidated.
fn apply(scenario: &mut Scenario, index: usize, change: Change, rng: &mut Rng) -> bool {
    match change {
        Change::NewCover => {
            let bytes = mp3_with_cover("image/png", &picture(rng));
            scenario.write_track(index, &bytes);
            scenario.tracks[index].covered = true;
            true
        }
        Change::StripCover => {
            let bytes = mp3_without_cover();
            scenario.write_track(index, &bytes);
            scenario.tracks[index].covered = false;
            true
        }
        Change::Touch => {
            scenario.touch_track(index);
            scenario.tracks[index].alive
        }
        Change::Restore => {
            let bytes = scenario.tracks[index].bytes.clone();
            scenario.write_track(index, &bytes);
            true
        }
        Change::Delete => {
            scenario.delete_track(index);
            false
        }
        Change::ReplaceSidecar => {
            let name = match scenario.tracks[index].sidecar.clone() {
                Some(name) => name,
                None => (*rng.pick(SIDECAR_NAMES)).to_string(),
            };
            let bytes = picture(rng);
            scenario.write_sidecar(index, &name, &bytes);
            !scenario.tracks[index].covered
        }
        Change::RemoveSidecar => {
            let had = scenario.tracks[index].sidecar.is_some();
            scenario.remove_sidecar(index);
            had && !scenario.tracks[index].covered
        }
    }
}

/// The answer the cache gives for `track`, compared with the answer a
/// fresh read of the file gives.
///
/// `invalidated` says whether the file the answer was built from changed
/// since the last lookup: a file that did not change is served from the
/// cache, a file that changed is read again.
fn check(cache: &ArtworkCache, track: &Track, invalidated: bool, what: &str) {
    let reads = cache.stats().reads;
    let hits = cache.stats().hits;
    let cached = cache.get(&track.path);
    let stats = cache.stats();

    if !track.alive {
        assert!(
            matches!(cached, Err(Error::NotFound(_))),
            "a file that is gone is an error, never a cover ({what})"
        );
        assert_eq!(
            stats.reads, reads,
            "a file that is gone is not read ({what})"
        );
        assert_eq!(
            stats.hits, hits,
            "a file that is gone is not a hit ({what})"
        );
        return;
    }

    match (cached, ArtworkCache::new().get(&track.path)) {
        (Ok(cached), Ok(fresh)) => assert!(
            cached.as_deref() == fresh.as_deref(),
            "the cached answer is not the answer of a fresh read ({what}): \
             the cache says {}, the file says {}",
            describe(cached.as_deref()),
            describe(fresh.as_deref())
        ),
        (Err(_), Err(_)) => {}
        (cached, fresh) => {
            panic!("the cached answer {cached:?} differs from the fresh answer {fresh:?} ({what})")
        }
    }

    if invalidated {
        assert_eq!(
            stats.reads,
            reads + 1,
            "a changed file is read again ({what})"
        );
    } else {
        assert_eq!(
            stats.reads, reads,
            "an unchanged file is not read again ({what})"
        );
        assert_eq!(stats.hits, hits + 1, "an unchanged file is a hit ({what})");
    }
}

/// What an answer looks like, for the failure messages.
///
/// The pixels of a cover are thousands of numbers: the message names the
/// size and the first pixel, and the comparison itself is exact.
fn describe(answer: Option<&CoverImage>) -> String {
    match answer {
        None => "no cover".to_string(),
        Some(image) => {
            let first = image
                .pixels
                .get(..4)
                .map_or([0; 4], |p| [p[0], p[1], p[2], p[3]]);
            format!(
                "{}x{} ({} {} {})",
                image.width, image.height, first[0], first[1], first[2]
            )
        }
    }
}

/// A cover that is bigger than the thumbnail edge is scaled down into
/// it, and the aspect ratio is kept.
#[test]
fn a_big_cover_is_scaled_into_the_thumbnail() {
    for &seed in SEEDS {
        let mut rng = Rng::new(seed);
        let dir = TempDir::new().expect("a temporary directory");
        let width = THUMBNAIL_MAX_EDGE + 1 + rng.below(40) as u32;
        let height = THUMBNAIL_MAX_EDGE + 1 + rng.below(40) as u32;
        let path = write_in(
            dir.path(),
            "track.mp3",
            &mp3_with_cover("image/png", &png(width, height, RED)),
        );
        let cache = ArtworkCache::new();

        let cover = cache.get(&path).expect("the answer").expect("the cover");
        assert!(
            cover.width <= THUMBNAIL_MAX_EDGE && cover.height <= THUMBNAIL_MAX_EDGE,
            "the cover fits into the thumbnail: {}x{}",
            cover.width,
            cover.height
        );
        assert_eq!(
            cover.width.max(cover.height),
            THUMBNAIL_MAX_EDGE,
            "the longest edge is the thumbnail edge"
        );
        // The aspect ratio is kept, within a pixel of the rounding.
        let expected = f64::from(height) * f64::from(cover.width) / f64::from(width);
        assert!(
            (f64::from(cover.height) - expected).abs() <= 1.0,
            "the aspect ratio is kept: {}x{} instead of {width}x{height}",
            cover.width,
            cover.height
        );

        // A cover that fits is kept as it is.
        let path = write_in(
            dir.path(),
            "small.mp3",
            &mp3_with_cover("image/png", &png(4, 4, RED)),
        );
        let cover = cache.get(&path).expect("the answer").expect("the cover");
        assert_eq!(
            (cover.width, cover.height),
            (4, 4),
            "a cover that fits is not scaled"
        );
    }
}

/// The bound of the cache: an answer that does not fit into the budget
/// is handed to the caller but not kept.
#[test]
fn an_answer_that_does_not_fit_the_budget_is_not_kept() {
    for &seed in SEEDS {
        let mut rng = Rng::new(seed);
        let dir = TempDir::new().expect("a temporary directory");
        let path = write_in(
            dir.path(),
            "track.mp3",
            &mp3_with_cover("image/png", &picture(&mut rng)),
        );

        // A budget of zero bytes: no answer fits into it.
        let cache = ArtworkCache::with_limits(DEFAULT_MAX_ENTRIES, 0);
        assert!(
            cache.get(&path).expect("the answer").is_some(),
            "the answer is handed to the caller"
        );
        assert_eq!(cache.entry_count(), 0, "the answer is not kept");
        assert!(
            cache.get(&path).expect("the answer").is_some(),
            "the answer is handed to the caller again"
        );
        assert_eq!(
            cache.stats().reads,
            2,
            "the file is read again, because nothing was kept"
        );
    }
}

/// The cache is coherent: the answer it serves is the answer a fresh
/// read gives, and an answer is thrown away exactly when the file it
/// was built from changed.
#[test]
fn the_cached_answer_is_the_answer_of_a_fresh_read() {
    for &seed in SEEDS {
        let mut scenario = Scenario::new(seed, TRACKS);
        let cache = ArtworkCache::new();
        // Every track is looked up once, so the cache holds an answer for
        // each of them.
        for track in &scenario.tracks {
            assert!(
                cache.get(&track.path).is_ok(),
                "a track that is there is answered"
            );
        }

        let mut rng = Rng::new(seed ^ 0x5a5a_5a5a);
        for round in 0..CHANGES {
            let index = rng.below(scenario.tracks.len());
            let change = *rng.pick(Change::ALL);
            let invalidated = apply(&mut scenario, index, change, &mut rng);
            for other in 0..scenario.tracks.len() {
                let track = &scenario.tracks[other];
                let what = format!(
                    "seed {seed}, round {round}, track {other}, {}",
                    change.name()
                );
                check(&cache, track, invalidated && other == index, &what);
            }
        }
    }
}

/// A cached answer is invalidated exactly when the stat of the file it
/// was built from changes.
#[test]
fn a_cached_answer_is_invalidated_exactly_when_the_stat_changes() {
    let dir = TempDir::new().expect("a temporary directory");
    let path = write_in(
        dir.path(),
        "track.mp3",
        &mp3_with_cover("image/png", &png(4, 4, RED)),
    );
    let cache = ArtworkCache::new();

    // The first lookup reads the file.
    let first = cache.get(&path).expect("the answer").expect("the cover");
    assert_eq!(cache.stats().reads, 1);
    assert_eq!(cache.stats().hits, 0);

    // The stat did not change: the answer comes from the cache.
    let second = cache.get(&path).expect("the answer").expect("the cover");
    assert!(
        Arc::ptr_eq(&first, &second),
        "the cached cover is the same image"
    );
    assert_eq!(cache.stats().reads, 1, "the file is not read again");
    assert_eq!(cache.stats().hits, 1);

    // The file was touched: the answer is stale.
    set_modified(&path, later_than(&path));
    let third = cache.get(&path).expect("the answer").expect("the cover");
    assert_eq!(cache.stats().reads, 2, "the touched file is read again");
    assert_eq!(third.pixels, first.pixels, "it is the same picture");
    assert!(
        !Arc::ptr_eq(&first, &third),
        "the picture was decoded again"
    );

    // The stat did not change again: the answer comes from the cache.
    let fourth = cache.get(&path).expect("the answer").expect("the cover");
    assert!(Arc::ptr_eq(&third, &fourth));
    assert_eq!(cache.stats().reads, 2);
    assert_eq!(cache.stats().hits, 2);

    // The file was written again: the size changed, so does the answer.
    write_in(
        dir.path(),
        "track.mp3",
        &mp3_with_cover("image/png", &png(8, 8, BLUE)),
    );
    let fifth = cache.get(&path).expect("the answer").expect("the cover");
    assert_eq!(cache.stats().reads, 3, "the rewritten file is read again");
    assert_eq!((fifth.width, fifth.height), (8, 8));
    assert_eq!(&fifth.pixels[..4], &BLUE);

    // The file is gone: the cover is gone with it.
    fs::remove_file(&path).expect("the track is removed");
    assert!(
        matches!(cache.get(&path), Err(Error::NotFound(_))),
        "a file that is gone is an error, never a cover"
    );
}

/// A file that is written again shows its new cover: the cache never
/// keeps the old one.
#[test]
fn a_rewritten_file_shows_its_new_cover() {
    for &seed in SEEDS {
        let mut rng = Rng::new(seed);
        let dir = TempDir::new().expect("a temporary directory");
        let path = write_in(
            dir.path(),
            "track.mp3",
            &mp3_with_cover("image/png", &png(4, 4, RED)),
        );
        let cache = ArtworkCache::new();
        let first = cache.get(&path).expect("the answer").expect("the cover");
        assert_eq!(&first.pixels[..4], &RED);

        // The same path, new bytes, a new modification time.
        let width = 20 + rng.below(8) as u32;
        let height = 20 + rng.below(8) as u32;
        let picture = png(width, height, BLUE);
        write_in(
            dir.path(),
            "track.mp3",
            &mp3_with_cover("image/png", &picture),
        );
        set_modified(&path, later_than(&path));

        let second = cache.get(&path).expect("the answer").expect("the cover");
        assert_eq!((second.width, second.height), (width, height));
        assert_eq!(&second.pixels[..4], &BLUE, "the new cover is shown");
        assert_eq!(cache.stats().reads, 2, "the file is read again");
    }
}

/// The embedded cover wins over the sidecar, and the sidecar is not part
/// of an answer that comes from the embedded cover.
#[test]
fn the_embedded_cover_wins_over_the_sidecar() {
    for &seed in SEEDS {
        let mut rng = Rng::new(seed);
        let dir = TempDir::new().expect("a temporary directory");
        let path = write_in(
            dir.path(),
            "track.mp3",
            &mp3_with_cover("image/png", &png(4, 4, RED)),
        );
        let name = *rng.pick(SIDECAR_NAMES);
        let sidecar = write_in(dir.path(), name, &picture_named(name, 16, 16, BLUE));
        let cache = ArtworkCache::new();

        let cover = cache.get(&path).expect("the answer").expect("the cover");
        assert_eq!(
            (cover.width, cover.height),
            (4, 4),
            "the embedded cover wins over the sidecar"
        );
        assert_eq!(&cover.pixels[..4], &RED);

        // The sidecar changes: the answer does not depend on it, so the
        // cached answer is still the right one.
        let bytes = picture_named(name, 8, 8, BLUE);
        fs::write(&sidecar, &bytes).expect("the sidecar is written");
        set_modified(&sidecar, later_than(&sidecar));

        let reads = cache.stats().reads;
        let cover = cache.get(&path).expect("the answer").expect("the cover");
        assert_eq!(
            cache.stats().reads,
            reads,
            "the sidecar does not invalidate the embedded cover"
        );
        assert_eq!(&cover.pixels[..4], &RED);

        // A track without a cover of its own falls back to the sidecar.
        let path = write_in(dir.path(), "plain.mp3", &mp3_without_cover());
        let cover = cache.get(&path).expect("the answer").expect("the cover");
        assert_eq!(
            (cover.width, cover.height),
            (8, 8),
            "the sidecar is the fallback"
        );
    }
}

/// A track without a cover of its own and without a sidecar has no
/// cover, and a broken sidecar does not break the row it belongs to.
#[test]
fn a_missing_or_broken_sidecar_is_no_cover() {
    for &seed in SEEDS {
        let mut rng = Rng::new(seed);
        let dir = TempDir::new().expect("a temporary directory");
        let path = write_in(dir.path(), "track.mp3", &mp3_without_cover());
        let name = *rng.pick(SIDECAR_NAMES);
        let cache = ArtworkCache::new();

        // No sidecar at all: no cover, and no error.
        assert!(cache.get(&path).expect("the answer").is_none());

        // A sidecar that is not a picture: still no cover, and no error.
        let sidecar = write_in(dir.path(), name, b"not a picture");
        assert!(cache.get(&path).expect("the answer").is_none());
        assert_eq!(cache.stats().reads, 2, "the broken sidecar is read again");

        // The sidecar is repaired: the cover is picked up.
        let bytes = picture_named(name, 8, 8, BLUE);
        fs::write(&sidecar, &bytes).expect("the sidecar is written");
        set_modified(&sidecar, later_than(&sidecar));
        let cover = cache.get(&path).expect("the answer").expect("the cover");
        assert_eq!((cover.width, cover.height), (8, 8));
    }
}

/// The sidecar names are tried in the documented order: the first name
/// that is a regular file wins.
#[test]
fn the_sidecar_names_are_tried_in_the_documented_order() {
    for &seed in SEEDS {
        let mut rng = Rng::new(seed);
        let dir = TempDir::new().expect("a temporary directory");
        let path = write_in(dir.path(), "track.mp3", &mp3_without_cover());

        // A random set of the names is there, each with a size of its
        // own, so the name that won can be told from the others.
        let mut first = None;
        for (index, name) in SIDECAR_NAMES.iter().enumerate() {
            if !rng.flip() {
                continue;
            }
            let size = 8 + index as u32;
            let bytes = picture_named(name, size, size, RED);
            write_in(dir.path(), name, &bytes);
            if first.is_none() {
                first = Some((size, *name));
            }
        }

        let cache = ArtworkCache::new();
        let answer = cache.get(&path).expect("the answer");
        match first {
            Some((size, name)) => {
                let cover = answer.expect("the cover");
                assert_eq!(
                    cover.width, size,
                    "{name} is the first name of the list that is there"
                );
            }
            None => assert!(answer.is_none(), "there is no sidecar at all"),
        }
    }
}

/// A picture that is not a picture is "no cover", and a file that is
/// repaired later is picked up.
#[test]
fn a_corrupt_picture_is_never_cached_as_a_cover() {
    let dir = TempDir::new().expect("a temporary directory");
    let path = write_in(
        dir.path(),
        "track.mp3",
        &mp3_with_cover("image/png", b"not a picture"),
    );
    let cache = ArtworkCache::new();

    // The broken picture is no cover.
    assert!(
        cache.get(&path).expect("the answer").is_none(),
        "a broken picture is no cover"
    );
    // The answer is cached like any other answer.
    assert!(cache.get(&path).expect("the answer").is_none());
    assert_eq!(cache.stats().reads, 1, "the file is read once");
    assert_eq!(cache.entry_count(), 1, "the answer is cached");

    // The file is repaired: the cover is picked up.
    write_in(
        dir.path(),
        "track.mp3",
        &mp3_with_cover("image/png", &png(4, 4, RED)),
    );
    set_modified(&path, later_than(&path));
    let cover = cache.get(&path).expect("the answer").expect("the cover");
    assert_eq!(&cover.pixels[..4], &RED);
    assert_eq!(cache.stats().reads, 2, "the repaired file is read again");
}

/// A broken picture of a track falls back to the sidecar next to it.
#[test]
fn a_broken_embedded_picture_falls_back_to_the_sidecar() {
    let dir = TempDir::new().expect("a temporary directory");
    let path = write_in(
        dir.path(),
        "track.mp3",
        &mp3_with_cover("image/png", b"not a picture"),
    );
    write_in(dir.path(), "cover.png", &png(4, 4, BLUE));
    let cache = ArtworkCache::new();

    let cover = cache.get(&path).expect("the answer").expect("the cover");
    assert_eq!(&cover.pixels[..4], &BLUE, "the sidecar is the fallback");
}

/// The entry of a file that is gone is never served, and it is the first
/// one the LRU drops.
#[test]
fn the_entry_of_a_deleted_file_is_never_served() {
    let dir = TempDir::new().expect("a temporary directory");
    let a = write_in(
        dir.path(),
        "a.mp3",
        &mp3_with_cover("image/png", &png(4, 4, RED)),
    );
    let b = write_in(
        dir.path(),
        "b.mp3",
        &mp3_with_cover("image/png", &png(4, 4, BLUE)),
    );
    let c = write_in(
        dir.path(),
        "c.mp3",
        &mp3_with_cover("image/png", &png(4, 4, RED)),
    );
    let cache = ArtworkCache::with_limits(2, DEFAULT_MAX_BYTES);

    cache.get(&a).expect("the answer");
    // The file is gone: the answer is an error, never a stale cover.
    fs::remove_file(&a).expect("the track is removed");
    assert!(
        matches!(cache.get(&a), Err(Error::NotFound(_))),
        "a file that is gone is an error, never a cover"
    );

    cache.get(&b).expect("the answer");
    assert_eq!(cache.entry_count(), 2);

    // A third file does not fit: the least recently used answer goes
    // first, and that is the answer of the file that is gone.
    cache.get(&c).expect("the answer");
    assert_eq!(cache.entry_count(), 2, "the cache stays at its bound");

    // The answer of C is in the cache, so C took the place of the
    // answer that was dropped.
    let reads = cache.stats().reads;
    cache.get(&c).expect("the answer");
    assert_eq!(cache.stats().reads, reads, "C is cached");

    // The answer of B is still there: the answer of the file that is
    // gone was the one that was dropped.
    let reads = cache.stats().reads;
    cache.get(&b).expect("the answer");
    assert_eq!(cache.stats().reads, reads, "B is still cached");
}

/// The awkward files are answered, not panicked about.
#[test]
fn the_awkward_files_are_answered() {
    let dir = TempDir::new().expect("a temporary directory");
    let cache = ArtworkCache::new();

    // A folder that is not there.
    let missing = dir.path().join("gone").join("track.mp3");
    assert!(
        matches!(cache.get(&missing), Err(Error::NotFound(_))),
        "a file in a folder that is not there is an error"
    );

    // A path that is a folder.
    assert!(cache.get(dir.path()).is_err(), "a folder is not a track");

    // A file without a byte in it: the reader cannot read it, so it is
    // an error and not a cover.
    let empty = write_in(dir.path(), "empty.mp3", b"");
    assert!(
        cache.get(&empty).is_err(),
        "a file that cannot be read is an error"
    );

    // An embedded picture without a byte in it.
    let without = write_in(dir.path(), "without.mp3", &mp3_with_cover("image/png", b""));
    assert!(
        cache.get(&without).expect("the answer").is_none(),
        "a picture without a byte in it is no cover"
    );

    // A sidecar without a byte in it.
    let folder = dir.path().join("album");
    fs::create_dir(&folder).expect("the album folder is created");
    let track = write_in(&folder, "track.mp3", &mp3_without_cover());
    write_in(&folder, "cover.jpg", b"");
    assert!(
        cache.get(&track).expect("the answer").is_none(),
        "a sidecar without a byte in it is no cover"
    );

    // A picture that claims to be huge. The decoder must refuse it
    // before it allocates anything: the picture would need 40 GB.
    let huge = dir.path().join("huge");
    fs::create_dir(&huge).expect("the album folder is created");
    let track = write_in(&huge, "track.mp3", &mp3_without_cover());
    write_in(
        &huge,
        "cover.png",
        &png_with_declared_size(100_000, 100_000),
    );
    assert!(
        cache.get(&track).expect("the answer").is_none(),
        "a picture that claims to be huge is no cover"
    );

    // The same picture, embedded in a track.
    let track = write_in(
        &huge,
        "embedded.mp3",
        &mp3_with_cover("image/png", &png_with_declared_size(100_000, 100_000)),
    );
    assert!(
        cache.get(&track).expect("the answer").is_none(),
        "an embedded picture that claims to be huge is no cover"
    );
}

/// A file that cannot be read is an error, and the failure is not
/// remembered as "no cover".
#[test]
fn an_unreadable_file_is_an_error_and_is_not_cached() {
    let dir = TempDir::new().expect("a temporary directory");
    let cache = ArtworkCache::new();

    #[cfg(windows)]
    let (path, _lock) = {
        use std::os::windows::fs::OpenOptionsExt;
        let path = write_in(dir.path(), "locked.mp3", b"locked");
        // The file is opened without any sharing: every other handle -
        // the reader of the cache included - is refused.
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .expect("the file is locked");
        (path, lock)
    };
    #[cfg(unix)]
    let (path, _lock) = {
        // A symlink that points at itself: no user can read it.
        let path = dir.path().join("loop.mp3");
        std::os::unix::fs::symlink(&path, &path).expect("the symlink is created");
        (path, ())
    };

    let answer = cache.get(&path);
    assert!(
        answer.is_err(),
        "a file that cannot be read is an error: {answer:?}"
    );
    assert_eq!(cache.entry_count(), 0, "a failure is not cached");
}

/// A random picture: a size and a colour.
///
/// Every fourth picture is bigger than the thumbnail size, so the
/// scaling into the thumbnail is part of the property.
fn picture(rng: &mut Rng) -> Vec<u8> {
    let (width, height) = if rng.below(4) == 0 {
        (100 + rng.below(40) as u32, 60 + rng.below(40) as u32)
    } else {
        (1 + rng.below(20) as u32, 1 + rng.below(20) as u32)
    };
    png(width, height, color(rng))
}

/// A random colour.
fn color(rng: &mut Rng) -> [u8; 4] {
    [
        rng.below(256) as u8,
        rng.below(256) as u8,
        rng.below(256) as u8,
        255,
    ]
}

/// A picture in the format its name promises.
fn picture_named(name: &str, width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    if name.ends_with(".jpg") {
        jpeg(width, height, color)
    } else {
        png(width, height, color)
    }
}

/// A modification time that is later than the one the file has now.
fn later_than(path: &Path) -> SystemTime {
    modified(path) + Duration::from_secs(60)
}

/// A PNG that claims to be `width` by `height` pixels.
///
/// The picture is never decoded: the header alone is enough to make the
/// decoder refuse it.
fn png_with_declared_size(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bit, RGBA, no interlace
    chunk(&mut bytes, b"IHDR", &header);
    chunk(&mut bytes, b"IDAT", &[]);
    chunk(&mut bytes, b"IEND", &[]);
    bytes
}

/// A PNG chunk: the length, the type, the data and the checksum.
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut checksum = Vec::new();
    checksum.extend_from_slice(kind);
    checksum.extend_from_slice(data);
    out.extend_from_slice(&crc32(&checksum).to_be_bytes());
}

/// The CRC-32 of the PNG chunks: the standard CRC-32 of IEEE 802.3.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
