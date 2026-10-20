// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Property tests for the scan path.
//!
//! The tests in `lib.rs` pin one behaviour of the scan each. These pin the
//! *invariants* that have to hold for every tree, every seed and every
//! cancel point:
//!
//! * a scan is idempotent: scanning the same root twice yields the same
//!   rows and the same `total`, and no row is there twice;
//! * the index mirrors the tree: every media file has a row and every row
//!   is a media file;
//! * a file that vanished loses its row, a file that appeared gains one,
//!   and every other row keeps its id;
//! * the batch boundary never loses a row: a tree of 63, 64 or 65 files
//!   yields all of them, and the row ids have no gap;
//! * a cancelled scan commits whole batches only, says that it was
//!   cancelled, and its counters match the rows the index really holds;
//! * `last_scanned_at` is written by a finished scan and by no other;
//! * removing a track takes its `track_extra` rows with it and leaves the
//!   extras of the other tracks alone.
//!
//! The trees are synthetic: the files are small WAV headers, because the
//! scan only records what it finds. The generator is seeded, so a failing
//! case is reproducible from its seed alone.

use crate::testsupport::{Rng, last_scanned_at, make_wav, put_last_scanned_at};
use crate::{Cancel, Library, Query, SCAN_BATCH_SIZE, ScanPhase, TrackPage};
use rustamp_core::meta::TrackMetadata;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// The seeds the properties run over. Every seed builds another tree.
const SEEDS: &[u64] = &[
    0x1234_5678_9abc_def0,
    0x0f1e_2d3c_4b5a_6978,
    0xdead_beef_cafe_f00d,
    0x00c0_ffee_0bad_f00d,
    0x7fff_ffff_ffff_ffff,
];

/// The number of media files a random tree holds.
const FILES: usize = 40;

/// The number of directories a random tree holds.
const FOLDERS: usize = 4;

/// The names of the files that are not media: the scan must ignore them.
///
/// The last one has no extension at all.
const OTHERS: &[&str] = &[
    "notes.txt",
    "cover.jpg",
    "list.m3u",
    "art.png",
    "data.bin",
    "info.md",
    "README",
];

/// The extra tag the cascade property puts into the files.
const EXTRA_KEY: &str = "REPLAYGAIN_TRACK_GAIN";

/// The value the extra tag carries.
const EXTRA_VALUE: &str = "1.50 dB";

/// The files a cancelled scan is stopped at: the borders of a batch and
/// the files around them.
///
/// A batch that was complete when the cancel arrived is committed, the
/// batch that was in flight is dropped - so the border is where the two
/// answers meet.
const STOPS: &[usize] = &[1, 63, 64, 65, SCAN_BATCH_SIZE + 6];

/// A tree of files on disk.
///
/// The tree holds the media files the scan must find and the files it must
/// ignore. The `TempDir` keeps the files alive as long as the tree lives.
struct Tree {
    /// The directory the tree lives in.
    dir: TempDir,
    /// The media files, in the order they were drawn.
    media: Vec<PathBuf>,
    /// The files that are not media.
    others: Vec<PathBuf>,
}

impl Tree {
    /// A tree of `files` media files in `folders` directories, drawn by
    /// `seed`, plus a few files that are not media.
    fn new(seed: u64, files: usize, folders: usize) -> Self {
        let mut rng = Rng::new(seed);
        let dir = TempDir::new().expect("a temporary directory");
        let root = dir.path().to_path_buf();

        let mut media = Vec::with_capacity(files);
        for index in 0..files {
            let folder = root.join(format!("album{}", rng.below(folders)));
            fs::create_dir_all(&folder).expect("the folder is created");
            // Every third file has an upper-case extension: the extension
            // is matched case-insensitively, so the file is media all the
            // same.
            let extension = if index % 3 == 0 { "WAV" } else { "wav" };
            let path = folder.join(format!("t{index:03}.{extension}"));
            make_wav(&path);
            media.push(path);
        }

        let mut others = Vec::with_capacity(OTHERS.len());
        for (index, name) in OTHERS.iter().enumerate() {
            let folder = root.join(format!("album{}", rng.below(folders)));
            fs::create_dir_all(&folder).expect("the folder is created");
            let path = folder.join(format!("{index:02}-{name}"));
            fs::write(&path, b"not a media file").expect("the file is written");
            others.push(path);
        }

        Tree { dir, media, others }
    }

    /// A tree of `files` media files that carry an extra tag.
    ///
    /// The extra tag is written into the file, so the scan finds it the way
    /// it finds any other tag.
    fn with_extras(seed: u64, files: usize) -> Self {
        let tree = Tree::new(seed, files, 1);
        for path in &tree.media {
            let mut metadata = TrackMetadata {
                title: Some("T".to_owned()),
                ..Default::default()
            };
            metadata
                .extra
                .insert(EXTRA_KEY.to_owned(), EXTRA_VALUE.to_owned());
            rustamp_meta::write_tags(path, &metadata).expect("the extra tag is written");
        }
        tree
    }

    /// The directory the tree lives in.
    fn root(&self) -> &Path {
        self.dir.path()
    }

    /// The media files that live in `folder`.
    fn media_in(&self, folder: &Path) -> Vec<&PathBuf> {
        self.media
            .iter()
            .filter(|path| path.parent() == Some(folder))
            .collect()
    }
}

/// The page the index holds under `root`.
fn page_under(lib: &Library, root: &Path) -> TrackPage {
    lib.query(&Query {
        path_prefix: Some(root.to_path_buf()),
        ..Default::default()
    })
    .expect("the query succeeds")
}

/// The index mirrors the tree: every media file of the tree has a row, and
/// every row is a media file of the tree.
fn assert_mirrors(lib: &Library, tree: &Tree) {
    let page = page_under(lib, tree.root());
    let paths: BTreeSet<PathBuf> = page.rows.iter().map(|row| row.path.clone()).collect();
    let media: BTreeSet<PathBuf> = tree.media.iter().cloned().collect();
    assert_eq!(paths, media, "the index mirrors the tree");
    assert_eq!(page.total, paths.len(), "the total counts the rows");
}

/// The row id of the track at `path`.
fn id_of(lib: &Library, root: &Path, path: &Path) -> i64 {
    page_under(lib, root)
        .rows
        .iter()
        .find(|row| row.path == path)
        .expect("the row of the file")
        .id
}

/// The number of `track_extra` rows of every track, by track id.
fn extras_by_track(lib: &Library) -> BTreeMap<i64, i64> {
    let mut stmt = lib
        .conn
        .prepare("SELECT track_id, COUNT(*) FROM track_extra GROUP BY track_id")
        .expect("the statement is prepared");
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .expect("the rows are readable");
    rows.map(|row| row.expect("the row is readable")).collect()
}

/// The tracks that are not in `gone` keep the extra rows they had.
fn assert_extras_untouched(before: &BTreeMap<i64, i64>, after: &BTreeMap<i64, i64>, gone: &[i64]) {
    for (track, count) in before {
        if gone.contains(track) {
            continue;
        }
        assert_eq!(
            after.get(track),
            Some(count),
            "the track keeps its extra rows (track {track})"
        );
    }
}

/// A scan is idempotent: the second scan of the same root changes nothing.
///
/// The rows keep their ids, the `total` is the same, and no path is there
/// twice.
#[test]
fn scanning_the_same_root_twice_changes_nothing() {
    for &seed in SEEDS {
        let tree = Tree::new(seed, FILES, FOLDERS);
        let mut lib = Library::in_memory().unwrap();

        let first = lib.scan(tree.root()).expect("the scan succeeds");
        let first_page = page_under(&lib, tree.root());
        assert_eq!(
            first.added,
            tree.media.len(),
            "every media file is indexed (seed {seed})"
        );
        assert_eq!(first.scanned, tree.media.len(), "every file was read");
        assert_eq!(first.skipped, 0, "every file was readable");
        assert_eq!(first_page.rows.len(), tree.media.len());
        assert_eq!(
            first_page.total,
            first_page.rows.len(),
            "the total counts the rows"
        );
        assert_mirrors(&lib, &tree);

        // The files that are not media are not indexed.
        for row in &first_page.rows {
            assert!(
                !tree.others.contains(&row.path),
                "a file that is not media has no row (seed {seed})"
            );
        }

        let second = lib.scan(tree.root()).expect("the scan succeeds");
        let second_page = page_under(&lib, tree.root());

        assert_eq!(second.added, 0, "the second scan adds nothing");
        assert_eq!(second.updated, 0, "the second scan updates nothing");
        assert_eq!(second.removed, 0, "the second scan removes nothing");
        assert_eq!(second.skipped, 0, "the second scan skips nothing");
        assert_eq!(
            second_page.rows, first_page.rows,
            "the rows are the same, with the same ids (seed {seed})"
        );
        assert_eq!(
            second_page.total, first_page.total,
            "the total is the same (seed {seed})"
        );

        // No path is there twice.
        let paths: BTreeSet<&Path> = second_page
            .rows
            .iter()
            .map(|row| row.path.as_path())
            .collect();
        assert_eq!(
            paths.len(),
            second_page.rows.len(),
            "no row is there twice (seed {seed})"
        );
        assert_mirrors(&lib, &tree);
    }
}

/// A file that vanished loses its row, a file that appeared gains one, and
/// every other row keeps its id.
#[test]
fn a_vanished_file_loses_its_row_and_a_new_file_gains_one() {
    for &seed in SEEDS {
        let tree = Tree::new(seed, FILES, FOLDERS);
        let mut lib = Library::in_memory().unwrap();
        lib.scan(tree.root()).expect("the scan succeeds");
        let before = page_under(&lib, tree.root()).rows;

        // A file vanishes.
        let gone = tree.media[0].clone();
        fs::remove_file(&gone).expect("the file is removed");
        let report = lib.scan(tree.root()).expect("the scan succeeds");
        assert_eq!(report.removed, 1, "the row of the vanished file is gone");
        assert_eq!(report.added, 0, "nothing was added");
        let after = page_under(&lib, tree.root()).rows;
        assert_eq!(after.len(), before.len() - 1, "one row is less");
        assert!(
            !after.iter().any(|row| row.path == gone),
            "the row of the vanished file is gone (seed {seed})"
        );
        for row in &after {
            assert!(
                before.contains(row),
                "the other rows keep their ids (seed {seed})"
            );
        }

        // A file appears.
        let fresh = tree.root().join("fresh").join("t999.wav");
        fs::create_dir_all(fresh.parent().expect("the file has a parent"))
            .expect("the folder is created");
        make_wav(&fresh);
        let report = lib.scan(tree.root()).expect("the scan succeeds");
        assert_eq!(report.added, 1, "the new file has a row");
        assert_eq!(report.removed, 0, "nothing was removed");
        let rows = page_under(&lib, tree.root()).rows;
        assert_eq!(rows.len(), before.len(), "the tree is whole again");
        assert!(
            rows.iter().any(|row| row.path == fresh),
            "the row of the new file is there (seed {seed})"
        );
        for row in &rows {
            if row.path == fresh {
                continue;
            }
            assert!(
                before.contains(row),
                "the other rows keep their ids (seed {seed})"
            );
        }
    }
}

/// A folder that vanished takes the rows of its files with it.
#[test]
fn a_vanished_folder_takes_its_rows_with_it() {
    for &seed in SEEDS {
        let tree = Tree::new(seed, FILES, FOLDERS);
        let mut lib = Library::in_memory().unwrap();
        lib.scan(tree.root()).expect("the scan succeeds");
        let before = page_under(&lib, tree.root()).rows;

        // The folder of the first file disappears, with everything in it.
        let folder = tree.media[0]
            .parent()
            .expect("the file has a parent")
            .to_path_buf();
        let in_the_folder = tree.media_in(&folder);
        assert!(!in_the_folder.is_empty(), "the folder holds files");
        fs::remove_dir_all(&folder).expect("the folder is removed");

        let report = lib.scan(tree.root()).expect("the scan succeeds");
        assert_eq!(
            report.removed,
            in_the_folder.len(),
            "every file of the folder lost its row (seed {seed})"
        );
        assert_eq!(report.added, 0, "nothing was added");

        let after = page_under(&lib, tree.root()).rows;
        assert_eq!(after.len(), before.len() - in_the_folder.len());
        for row in &after {
            assert!(
                before.contains(row),
                "the other rows keep their ids (seed {seed})"
            );
            assert!(
                !row.path.starts_with(&folder),
                "no row of the vanished folder is left (seed {seed})"
            );
        }
    }
}

/// The batch boundary never loses a row.
///
/// The scan commits its work in batches of [`SCAN_BATCH_SIZE`] files, so a
/// tree that holds one batch more or less than the boundary must still
/// yield every file - and the row ids must have no gap, because a lost row
/// would leave one.
#[test]
fn the_batch_boundary_never_loses_a_row() {
    for count in [0usize, 1, 63, 64, 65, 127, 128, 129] {
        let tree = Tree::new(SEEDS[0], count, FOLDERS);
        let mut lib = Library::in_memory().unwrap();

        let report = lib.scan(tree.root()).expect("the scan succeeds");
        assert_eq!(report.added, count, "every file is added ({count} files)");
        assert_eq!(report.scanned, count, "every file is read ({count} files)");
        assert_eq!(report.removed, 0, "nothing is removed ({count} files)");

        let page = page_under(&lib, tree.root());
        assert_eq!(
            page.rows.len(),
            count,
            "every file has a row ({count} files)"
        );
        assert_eq!(
            page.total, count,
            "the total counts the rows ({count} files)"
        );

        // The row ids are 1..=count: a lost row would leave a gap.
        let ids: Vec<i64> = page.rows.iter().map(|row| row.id).collect();
        let expected: Vec<i64> = (1..=count as i64).collect();
        assert_eq!(ids, expected, "the row ids have no gap ({count} files)");

        // Every file of the tree is in the index.
        let paths: BTreeSet<&Path> = page.rows.iter().map(|row| row.path.as_path()).collect();
        for media in &tree.media {
            assert!(
                paths.contains(media.as_path()),
                "the file is indexed ({count} files)"
            );
        }
        assert_mirrors(&lib, &tree);
    }
}

/// A cancelled scan commits whole batches only, says that it was
/// cancelled, and its counters match the rows the index really holds.
///
/// The scan checks the cancel between two files and drops the batch that
/// was in flight, so what the index holds is a whole number of batches -
/// never half of one. The work that was seen but not committed is at most
/// one batch, the bound the crate's docs promise.
#[test]
fn a_cancelled_scan_commits_whole_batches_only() {
    // More files than one batch holds, so the scan has to commit a batch
    // before the cancel can stop it.
    let files = SCAN_BATCH_SIZE + 6;
    for &seed in SEEDS {
        let tree = Tree::new(seed, files, FOLDERS);

        // The stop points: the borders of a batch and one that is drawn.
        // The border matters: a batch that was complete when the cancel
        // arrived is committed, the batch that was in flight is not.
        let mut rng = Rng::new(seed ^ 0x5eed_5eed);
        let drawn = 1 + rng.below(files);
        let mut stops: Vec<usize> = STOPS.to_vec();
        stops.push(drawn);

        for stop in stops {
            let mut lib = Library::in_memory().unwrap();

            // The first scan: every committed file is a new row.
            let cancel = Cancel::new();
            let for_the_scan = cancel.clone();
            let report = lib
                .scan_with_progress(tree.root(), &for_the_scan, |event| {
                    if event.path.is_some() && event.seen == stop {
                        for_the_scan.cancel();
                    }
                })
                .expect("the scan succeeds");

            assert!(
                report.cancelled,
                "the scan says that it was cancelled (seed {seed}, stop {stop})"
            );
            assert_eq!(report.skipped, 0, "every file was readable");
            assert_eq!(
                report.removed, 0,
                "a cancelled scan removes nothing (seed {seed}, stop {stop})"
            );

            // The committed work is a whole number of batches.
            let committed = report.scanned + report.skipped;
            assert_eq!(
                committed % SCAN_BATCH_SIZE,
                0,
                "a cancelled scan commits whole batches only (seed {seed}, stop {stop})"
            );
            assert!(
                committed <= stop,
                "the committed work is not more than the work seen (seed {seed}, stop {stop})"
            );
            assert!(
                stop - committed <= SCAN_BATCH_SIZE,
                "at most one batch of work is lost (seed {seed}, stop {stop})"
            );
            assert_eq!(
                report.added, committed,
                "every committed file is a row (seed {seed}, stop {stop})"
            );
            assert_eq!(
                lib.count().unwrap(),
                committed,
                "the index holds the committed rows (seed {seed}, stop {stop})"
            );

            // The rows are files of the tree.
            let rows = page_under(&lib, tree.root()).rows;
            assert_eq!(rows.len(), committed);
            for row in &rows {
                assert!(
                    tree.media.contains(&row.path),
                    "the row is a file of the tree (seed {seed})"
                );
            }

            // A cancelled rescan changes nothing: the removal pass did not
            // run, and the files it saw are the ones it already holds.
            let before = rows;
            let cancel = Cancel::new();
            let for_the_scan = cancel.clone();
            let report = lib
                .scan_with_progress(tree.root(), &for_the_scan, |event| {
                    if event.path.is_some() && event.seen == stop {
                        for_the_scan.cancel();
                    }
                })
                .expect("the scan succeeds");
            assert!(
                report.cancelled,
                "the rescan says that it was cancelled (seed {seed}, stop {stop})"
            );
            assert_eq!(report.added, 0, "the rescan adds nothing");
            assert_eq!(report.updated, 0, "the rescan updates nothing");
            assert_eq!(report.removed, 0, "the rescan removes nothing");
            assert_eq!(
                page_under(&lib, tree.root()).rows.len(),
                before.len(),
                "the index holds the same rows (seed {seed}, stop {stop})"
            );
            assert_eq!(
                page_under(&lib, tree.root()).rows,
                before,
                "a cancelled scan changes nothing (seed {seed}, stop {stop})"
            );

            // The next scan picks up the work the cancel dropped.
            let report = lib.scan(tree.root()).expect("the scan succeeds");
            assert!(!report.cancelled, "the scan ran to the end");
            assert_eq!(
                report.added,
                files - committed,
                "the dropped files are indexed (seed {seed}, stop {stop})"
            );
            assert_mirrors(&lib, &tree);
        }
    }
}

/// A file that changed is updated in place: its row keeps its id and
/// follows the file, and the other rows are not touched.
#[test]
fn a_changed_file_is_updated_in_place() {
    for &seed in SEEDS {
        let tree = Tree::new(seed, FILES, FOLDERS);
        let mut lib = Library::in_memory().unwrap();
        lib.scan(tree.root()).expect("the scan succeeds");
        let before = page_under(&lib, tree.root()).rows;

        // One file changes: it gets tags, and with them another size.
        let changed = tree.media[0].clone();
        let metadata = TrackMetadata {
            title: Some("Changed".to_owned()),
            ..Default::default()
        };
        rustamp_meta::write_tags(&changed, &metadata).expect("the tags are written");

        let report = lib.scan(tree.root()).expect("the scan succeeds");
        assert_eq!(report.updated, 1, "the changed file is updated");
        assert_eq!(report.added, 0, "nothing was added");
        assert_eq!(report.removed, 0, "nothing was removed");

        let after = page_under(&lib, tree.root()).rows;
        assert_eq!(after.len(), before.len(), "the tree is the same");
        let row = after
            .iter()
            .find(|row| row.path == changed)
            .expect("the row of the file");
        let old = before
            .iter()
            .find(|row| row.path == changed)
            .expect("the row of the file");
        assert_eq!(row.id, old.id, "the row keeps its id");
        assert_eq!(
            row.metadata.title.as_deref(),
            Some("Changed"),
            "the row follows the file (seed {seed})"
        );
        assert_ne!(row.size, old.size, "the row carries the new size");
        for row in &after {
            if row.path == changed {
                continue;
            }
            assert!(
                before.contains(row),
                "the other rows are untouched (seed {seed})"
            );
        }
        assert_mirrors(&lib, &tree);
    }
}

/// A cancel that arrives in the removal pass deletes nothing.
///
/// The removal pass is the one step that can make the index smaller than
/// the disk, so a cancel in it must roll the whole pass back: the stale
/// row stays, and the scan says that it was cancelled.
#[test]
fn a_cancel_in_the_removal_pass_deletes_nothing() {
    for &seed in SEEDS {
        let tree = Tree::new(seed, FILES, FOLDERS);
        let mut lib = Library::in_memory().unwrap();
        lib.scan(tree.root()).expect("the scan succeeds");
        let before = page_under(&lib, tree.root()).rows;

        // A file vanishes, so the removal pass has something to do.
        let gone = tree.media[0].clone();
        fs::remove_file(&gone).expect("the file is removed");

        let cancel = Cancel::new();
        let for_the_scan = cancel.clone();
        let report = lib
            .scan_with_progress(tree.root(), &for_the_scan, |event| {
                if event.phase == ScanPhase::Removing {
                    for_the_scan.cancel();
                }
            })
            .expect("the scan succeeds");

        assert!(
            report.cancelled,
            "the scan says that it was cancelled (seed {seed})"
        );
        assert_eq!(report.removed, 0, "the removal pass deleted nothing");
        assert_eq!(
            page_under(&lib, tree.root()).rows,
            before,
            "the index is untouched (seed {seed})"
        );
    }
}

/// A scan of a file indexes that file and nothing else.
#[test]
fn a_scan_of_a_file_indexes_that_file() {
    for &seed in SEEDS {
        let tree = Tree::new(seed, 4, 1);
        let file = tree.media[0].clone();
        let mut lib = Library::in_memory().unwrap();

        let report = lib.scan(&file).expect("the scan succeeds");
        assert_eq!(report.added, 1, "the file is indexed");
        assert_eq!(lib.count().unwrap(), 1, "the index holds one row");

        // A second scan of the same file changes nothing.
        let report = lib.scan(&file).expect("the scan succeeds");
        assert_eq!(report.added, 0, "the file is indexed already");
        assert_eq!(report.updated, 0, "the file did not change");
        assert_eq!(report.removed, 0, "nothing was removed");
        assert_eq!(lib.count().unwrap(), 1, "the index still holds one row");
    }
}

/// `last_scanned_at` is written by a finished scan and by no other.
///
/// A cancelled scan did not finish, so it must not claim that the root was
/// scanned: the time of the last finished scan is kept.
#[test]
fn the_last_scan_time_is_written_by_a_finished_scan_only() {
    for &seed in SEEDS {
        let tree = Tree::new(seed, 4, 1);
        let mut lib = Library::in_memory().unwrap();
        let entry = lib.add_root(tree.root()).expect("the root is added");
        assert_eq!(
            entry.last_scanned_at, None,
            "a root that was never scanned has no time"
        );

        // A scan that was cancelled before it started: no time at all.
        let cancel = Cancel::new();
        cancel.cancel();
        let report = lib
            .scan_root_with_progress(entry.id, &cancel, |_| {})
            .expect("the scan succeeds");
        assert!(report.cancelled, "the scan was cancelled");
        assert_eq!(
            last_scanned_at(&lib, entry.id),
            None,
            "a cancelled scan claims no time (seed {seed})"
        );

        // A finished scan records its time.
        lib.scan_root(entry.id).expect("the scan succeeds");
        assert!(
            last_scanned_at(&lib, entry.id).is_some(),
            "a finished scan records the time (seed {seed})"
        );

        // A cancelled scan keeps the time of the last finished scan.
        put_last_scanned_at(&lib, entry.id, 4242);
        let cancel = Cancel::new();
        let for_the_scan = cancel.clone();
        let report = lib
            .scan_root_with_progress(entry.id, &for_the_scan, |event| {
                if event.path.is_some() {
                    for_the_scan.cancel();
                }
            })
            .expect("the scan succeeds");
        assert!(report.cancelled, "the scan was cancelled");
        assert_eq!(
            last_scanned_at(&lib, entry.id),
            Some(4242),
            "a cancelled scan keeps the time of the last finished scan (seed {seed})"
        );

        // A finished scan writes the time again.
        lib.scan_root(entry.id).expect("the scan succeeds");
        assert_ne!(
            last_scanned_at(&lib, entry.id),
            Some(4242),
            "a finished scan writes the time (seed {seed})"
        );
    }
}

/// Removing a track takes its `track_extra` rows with it, and the extras of
/// the other tracks are untouched.
///
/// Both removal paths do that: the removal pass of a scan and the
/// `remove_missing` of the library.
#[test]
fn removing_a_track_takes_its_extras_and_nothing_else() {
    for &seed in SEEDS {
        let tree = Tree::with_extras(seed, 8);
        let mut lib = Library::in_memory().unwrap();
        lib.add_root(tree.root()).expect("the root is added");
        lib.scan(tree.root()).expect("the scan succeeds");

        let before = extras_by_track(&lib);
        assert_eq!(
            before.len(),
            tree.media.len(),
            "every track has its extra rows (seed {seed})"
        );
        for row in page_under(&lib, tree.root()).rows {
            assert_eq!(
                row.metadata.extra.get(EXTRA_KEY).map(String::as_str),
                Some(EXTRA_VALUE),
                "the extra tag is in the row (seed {seed})"
            );
        }

        // One file vanishes: the scan drops its row and its extras.
        let gone = tree.media[0].clone();
        let first_gone = id_of(&lib, tree.root(), &gone);
        fs::remove_file(&gone).expect("the file is removed");
        let report = lib.scan(tree.root()).expect("the scan succeeds");
        assert_eq!(report.removed, 1, "the row of the vanished file is gone");
        let after = extras_by_track(&lib);
        assert_eq!(
            after.len(),
            before.len() - 1,
            "the extra rows of the removed track are gone (seed {seed})"
        );
        assert!(
            !after.contains_key(&first_gone),
            "the removed track has no extra rows any more (seed {seed})"
        );
        assert_extras_untouched(&before, &after, &[first_gone]);

        // The other removal path does the same.
        let gone_again = tree.media[1].clone();
        let second_gone = id_of(&lib, tree.root(), &gone_again);
        fs::remove_file(&gone_again).expect("the file is removed");
        let report = lib.remove_missing(None).expect("the removal succeeds");
        assert_eq!(report.removed, 1, "the row of the vanished file is gone");
        let after = extras_by_track(&lib);
        assert_eq!(
            after.len(),
            before.len() - 2,
            "the extra rows of the removed track are gone (seed {seed})"
        );
        assert!(
            !after.contains_key(&second_gone),
            "the removed track has no extra rows any more (seed {seed})"
        );
        assert_extras_untouched(&before, &after, &[first_gone, second_gone]);
    }
}
