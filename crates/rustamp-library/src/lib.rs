// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-library
//!
//! Media Library (M5): a persistent, SQLite-backed index of audio tracks.
//!
//! This crate owns the storage, the scan/query layer and the tag writing
//! only. It does not touch the player, the UI or the playlist model. A
//! future UI lane builds on top of [`Library`] by calling [`Library::scan`],
//! [`Library::query`] and [`Library::write_tags`].
//!
//! ## Schema
//!
//! ```text
//! tracks(
//!   id            INTEGER PRIMARY KEY AUTOINCREMENT,  -- stable row id
//!   path          TEXT NOT NULL UNIQUE,              -- file path as scanned
//!   title, artist, album, album_artist, genre, comment TEXT,
//!   year          INTEGER,
//!   track_no, track_total, disc_no INTEGER,
//!   duration_ms   INTEGER,   -- from tags when available (rustamp-meta sets None)
//!   bitrate_kbps  INTEGER,
//!   mtime_ns      INTEGER,   -- file mtime (unix nanoseconds), for change detection
//!   size          INTEGER,   -- file size in bytes
//!   scanned_at    INTEGER    -- unix seconds of last scan
//! )
//! track_extra(track_id, key, value)  -- the TrackMetadata::extra map
//! roots(
//!   id            INTEGER PRIMARY KEY AUTOINCREMENT,
//!   path          TEXT NOT NULL UNIQUE,              -- root path as added
//!   added_at      INTEGER NOT NULL,                  -- unix seconds
//!   last_scanned_at INTEGER                          -- unix seconds, NULL = never
//! )
//! ```
//!
//! The stable row id is the SQLite `rowid` (`id`); it survives re-scans
//! because rows are upserted by `path` rather than deleted and re-inserted.
//! Paths are stored as the scanner saw them (the root's own spelling); the
//! index treats a path as an opaque string, so a file reached through a
//! differently spelled path is a different row. Non-UTF-8 paths are stored
//! through a lossy conversion and cannot be distinguished from their
//! replacement-character form.
//!
//! ## Roots
//!
//! [`Library::add_root`], [`Library::remove_root`] and [`Library::roots`]
//! manage the list of indexed directories. A root is stored as the path string
//! it was added with (no canonicalisation, no drive-letter folding), so
//! callers should use one spelling per directory; adding the same spelling
//! twice is a no-op that returns the existing row. [`Library::scan_root`]
//! scans one root by its id, and [`Library::scan`] keeps working on an
//! arbitrary path, so a scan does not have to be registered first.
//! [`Library::remove_root`] forgets the root and deletes every indexed track
//! under it; the rest of the library is untouched.
//!
//! ## When a root was last scanned
//!
//! [`Library::scan_root`] records the time of its scan in the root's row:
//! `last_scanned_at` is the unix second the scan ran to the end. A root
//! that was never scanned has no time (`NULL`), and neither has a root whose
//! last scan was **cancelled** or **failed**: a scan that did not finish
//! must not claim that the root was scanned. The previous time is kept in
//! that case, so the field answers the question "when did the index of
//! this root last get a complete look at the disk".
//!
//! The time says that the scan ran to the end; the [`ScanReport`] says what
//! it found. A scan that skipped files (a file that could not be read, a
//! directory that could not be walked) is still a scan that ran to the end,
//! and its skips are in the report.
//!
//! The column is added to an existing database when it is opened: the
//! migration is a `pragma_table_info` check followed by an `ALTER TABLE`, so
//! it is idempotent and keeps the rows of an older database.
//!
//! ## Query
//!
//! [`Library::query`] takes a [`Query`]: the optional filters (`path_prefix`,
//! `artist`, `album`, `title`, `genre`, `text`) are combined with AND, then the
//! matches are sorted, paged and counted. It returns a [`TrackPage`] whose
//! `total` is the number of matches *before* `limit`/`offset` is applied, so a
//! UI can size a scrollbar without issuing a second query.
//!
//! `path_prefix` is a literal byte prefix: the comparison is exact, so a
//! spelling that differs in case matches nothing, and a prefix that is not a
//! directory boundary still matches (`C:\music` also matches `C:\music2`).
//!
//! `text` is the free-text search: one string matched as a literal
//! case-insensitive substring of `title`, `artist` or `album` (`LIKE` with
//! `ESCAPE`), so `%`, `_` and `\` are searched literally. Both the columns and
//! the search term are case folded with Rust's Unicode lower-casing (a
//! registered `rustamp_lower` SQL function) rather than SQLite's ASCII-only
//! `NOCASE`, so `Ärzte` matches `ärzte` and `ÄRZTE`. Whitespace-only text means
//! "no text filter".
//!
//! Sorting picks [`SortKey`] `Id`, `Title`, `Artist`, `Album` or `Duration`
//! and can be reversed with `descending`; text columns sort case-insensitively
//! with the same Unicode folding, `NULL` values sort last in both directions,
//! and ties are always broken by ascending row id. That total order is what
//! makes `limit`/`offset` paging stable across queries. `limit: Some(0)`
//! returns no rows but still reports the full `total`.
//!
//! ## Scan rule
//!
//! `Library::scan(root)` walks `root` recursively (symlinks and junctions are
//! not followed, which also rules out cycles) and considers every regular
//! file whose extension [`rustamp_meta::supports_extension`] accepts. Other
//! files (artwork, playlists, text) are ignored and are not counted in any
//! field of the report.
//!
//! For each considered file it stats the file and compares `(mtime_ns, size)`
//! against the stored row:
//!
//! * no stored row, or a differing `(mtime_ns, size)` -> read tags with
//!   [`rustamp_meta::read_tags`] and upsert the row by `path` (insert or
//!   update). A genuinely new path counts as `added`; an existing path whose
//!   stat changed counts as `updated`. The row id is preserved.
//! * identical `(mtime_ns, size)` -> the row is left untouched (not counted).
//! * a file that cannot be stat'ed, read or whose tags cannot be parsed is
//!   counted as `skipped` and never aborts the scan. A previous row for that
//!   path is kept: the file still exists, so a transient read failure must not
//!   silently delete index data.
//!
//! After the walk, every stored row whose `path` lies under `root` and whose
//! file was not found on disk during this scan is deleted (`removed`). Rows
//! for paths outside `root` are never touched, so scanning one folder does not
//! wipe another folder's index. If any directory could not be read, the
//! removal pass is skipped for the whole scan: an unreadable subtree cannot
//! prove that its files are gone.
//!
//! A scan is therefore idempotent: re-scanning an unchanged tree produces
//! zero `added`/`updated`/`removed` and identical row ids. Scanning a path
//! that does not exist is an error ([`Error::NotFound`]) rather than an empty
//! report.
//!
//! ## Removing dead entries
//!
//! [`Library::remove_missing`] drops the rows whose file is gone, without a
//! full rescan: it walks the **rows** the index holds under the registered
//! roots and asks the file system about each path. It is the library's
//! counterpart of the playlist's "remove dead files".
//!
//! The **check** is one stat per row, and it has three answers:
//!
//! * the path exists -> the row stays;
//! * the path does not exist -> the row is dropped;
//! * the path cannot be checked (a permission error, a path the system
//!   refuses to name) -> the row **stays** and is reported.
//!
//! The third answer is the point of the operation. A row that cannot be
//! checked is not a row that is known to be dead: an unreadable directory
//! says nothing about the files below it, so dropping its rows would throw
//! away index data over a permission that may be gone tomorrow.
//!
//! The **scope** is the registered roots: `remove_missing(None)` walks every
//! registered root, `remove_missing(Some(&[id]))` only the roots with those
//! ids. The rows of the roots that were not selected are never touched, and
//! a row that lies outside every registered root is never touched either.
//!
//! The **report** ([`RemoveReport`]) says how many rows were checked, how
//! many were removed, and which rows were kept because their file could not
//! be checked.
//!
//! The deletion is one transaction, and the `track_extra` rows of a removed
//! track go with it: the foreign key cascades. The artwork cache is not
//! touched - it is keyed by path and checks the file's stat on every lookup,
//! so the cover of a removed track is gone the next time it is asked for.
//!
//! ## Scan progress and cancellation
//!
//! [`Library::scan_with_progress`] is the scan for a background thread: it
//! reports what it does through a callback and it can be stopped through a
//! [`Cancel`] flag. The crate owns no thread - the caller decides where the
//! scan runs and how the events reach the user interface.
//!
//! The event is a [`ScanProgress`]: the coarse [`ScanPhase`], the file the
//! scan is working on, and the counters (files seen, added, updated,
//! skipped, removed). The callback runs on the thread that drives the scan,
//! once before the first file, once per considered file and once per phase
//! change, so a caller can forward the events over a channel without
//! polling.
//!
//! The scan commits its work in batches of at most [`SCAN_BATCH_SIZE`]
//! considered files, and every batch is one transaction. The index
//! therefore never holds half a batch: it holds the batches that were
//! committed, in order. A cancelled scan keeps the batches it committed and
//! drops the batch in flight, so at most `SCAN_BATCH_SIZE` files of work
//! are lost.
//!
//! Cancellation is checked between two files and before the removal pass.
//! A cancelled scan never deletes a row: the removal pass is the one step
//! that can make the index smaller than the disk, and it does not run after
//! a cancel. It also does not run when a directory could not be read, for
//! the same reason.
//!
//! The [`ScanReport`] a scan returns describes the *committed* state: its
//! `added`, `updated` and `removed` are the changes the index really
//! holds, and `cancelled` says whether the scan was stopped. The counters
//! of a progress event are the live ones and can be ahead of the index by
//! the batch in flight.
//!
//! ## Tag writing
//!
//! [`Library::write_tags`] edits the tags of an indexed track. The values
//! are written into the **real file** with the `rustamp-meta` writer
//! ([`rustamp_meta::write_tags_with_report`]), which replaces the file
//! atomically: after the call the file is either the old one or the new
//! one, never a half-written mixture. A field that is `None` is left as the
//! file has it (the `rustamp-meta` contract), so one field can be edited
//! without the others being lost.
//!
//! [`Library::write_tags_with`] is the same write with the **removals**: a
//! field of the removal list ([`TagField`]) is taken out of the file, while
//! a field that is `None` in the metadata keeps the value the file has. A
//! removal wins over a value: a field that is in both is removed.
//!
//! The row is then rebuilt from the file: the tags are **read back** after
//! the write and it is *those* values that land in the index, never the
//! values the caller passed in. A format that stores a value differently, or
//! cannot store it at all, therefore shows up in the index as the file
//! really is - the index can never claim a value the file does not carry.
//!
//! The row id does not change (the row is updated in place), and the stored
//! `mtime_ns`/`size` are refreshed from the written file, so the next scan
//! sees no change and does not read the file a second time.
//!
//! A failed write leaves the row exactly as it was. A file that vanished
//! between the scan and the write is an error ([`Error::NotFound`]) and not
//! a silent row update: the index must not describe a file that is not
//! there any more.
//!
//! ## What the format refused
//!
//! The writer only writes the *primary* tag of a file. A value in a second
//! tag - an ID3v1 tag next to an ID3v2 tag, the `RIFF INFO` of a WAV, the
//! text chunks of an AIFF - is not written and stays in the file, so a
//! removal of that value does not take it out of the file.
//!
//! [`Library::write_tags_with`] says so: the [`WriteReport`] of its
//! [`TagWrite`] names every removal the file still carries
//! ([`WriteReport::unremoved`]), and the row shows the value the file still
//! has. A caller that only looks at the row cannot tell a removed value
//! from a value that survived - which is why the report is part of the
//! result.
//!
//! ## Artwork cache
//!
//! [`ArtworkCache`] keeps the decoded cover of a track in memory, so a
//! library view can draw the album art without reading and decoding the
//! track file on every frame.
//!
//! **Where the cover comes from**: the **embedded** front cover of the
//! track wins - it belongs to the file and travels with it. A track whose
//! file carries no picture falls back to the **sidecar** image in the same
//! directory: the `cover.jpg` of the album folder, for example. The names
//! that are looked for, and the order they are tried in, are
//! [`SIDECAR_NAMES`]; the first name that is a regular file wins. The
//! sidecar is looked for in the directory of the track only, never above
//! it.
//!
//! **The key** is the path of the track. Every entry carries the
//! `(mtime_ns, size)` pair its cover was decoded from: a lookup stats the
//! file, and the same pair is a hit while a different pair is a miss that
//! replaces the entry. That is the same pair the index uses to notice a
//! changed file, so a rewritten track loses its cover exactly when the
//! index notices the rewrite.
//!
//! **The sidecar key**: an answer that does not come from the embedded
//! cover also carries the `(mtime_ns, size)` pair of the sidecar it was
//! built from. A changed `cover.jpg` therefore invalidates the cached
//! cover, and a `cover.jpg` that appears next to a track that had none
//! invalidates the cached "no cover". The pair of the sidecar is only
//! looked at when the answer can depend on it: an embedded cover wins
//! over every sidecar, so its presence changes nothing.
//!
//! **The bound** is an LRU with two limits: at most
//! [`DEFAULT_MAX_ENTRIES`] answers and at most [`DEFAULT_MAX_BYTES`] of
//! pixels. The least recently used cover is dropped when a new one does not
//! fit; an answer that does not fit at all is handed to the caller but not
//! kept. The cache holds its entries behind a mutex and can be shared
//! between threads.
//!
//! **The decode rule**: the embedded front cover is read with
//! [`rustamp_meta::read_artwork`] and decoded with the `image` crate (JPEG
//! and PNG, the formats cover art comes in). The format is guessed from
//! the picture itself, not from the MIME type of the tag. The picture
//! becomes a non-premultiplied RGBA image; one that is larger than
//! [`THUMBNAIL_MAX_EDGE`] on its longest edge is scaled down into that
//! size, with the aspect ratio kept. A picture that needs more than
//! [`MAX_DECODE_BYTES`] to decode is not decoded and counts as "no cover".
//! A sidecar image goes through the same decoder and obeys the same
//! thumbnail and size rules.
//!
//! A file without a cover is answered with "no cover", and that answer is
//! cached: a track without album art is not read again. A file that cannot
//! be read at all is an error, and nothing is cached - a failure that may
//! be transient must not be remembered as "no cover". A sidecar that
//! cannot be read or decoded is "no cover" as well: a broken `cover.jpg`
//! must not break the row it belongs to.
//!
//! ## A damaged database
//!
//! [`Library::open`] never writes to a file it could not read as *this*
//! crate's index. A damaged file is refused with a typed [`OpenError`] and
//! left exactly as it was: the index is never dropped, truncated, migrated
//! over or replaced because a file was damaged. A player that "repairs" a
//! damaged library by writing a fresh schema over it destroys the index
//! without a word, and the rows, the ids and the roots are gone for good.
//!
//! The damage classes and the error each of them gets:
//!
//! | Damage | The error |
//! |---|---|
//! | the path is not a regular file | [`OpenError::NotAFile`] |
//! | the file does not start with the SQLite header | [`OpenError::NotADatabase`] |
//! | the file was cut short | [`OpenError::Truncated`] |
//! | SQLite reports the database as damaged | [`OpenError::Corrupt`] |
//! | the file holds another program's tables | [`OpenError::ForeignSchema`] |
//! | a newer version of the player wrote the file | [`OpenError::FutureVersion`] |
//! | the file cannot be written to | [`OpenError::ReadOnly`] |
//! | another process holds the file | [`OpenError::Locked`] |
//!
//! A file that is not there yet, and a file without a byte in it, are the
//! first run: the schema is created in them. A file that holds the tables
//! of this crate is ours, even when it is older than this version: it is
//! migrated, and its rows are kept.
//!
//! ## Known limits
//!
//! * Change detection uses the file's stat, not its contents. A rewrite that
//!   preserves both size and modification time (to the filesystem's
//!   timestamp granularity) is not noticed until something else changes.
//! * Tag reading does not decode audio, so `duration_ms` and `bitrate_kbps`
//!   are only filled when tags themselves carry them. Sorting by duration is
//!   therefore mostly a stable order over `NULL`s today.
//! * Roots may nest: removing an outer root also drops the tracks of an inner
//!   root, while the inner root stays listed.
//! * `text` matches substrings of the three columns only; it is a `LIKE` scan,
//!   not a tokenised full-text index, and cannot match across columns
//!   (`title + artist` as one haystack) or by word prefix ranking.
//! * Only [`Library::scan_root`] records the last-scan time. A plain
//!   [`Library::scan`] works on an arbitrary path and records nothing,
//!   because it does not know which root row it belongs to.
//! * A cancelled scan drops the batch that was in flight (at most
//!   [`SCAN_BATCH_SIZE`] files of work). The files it dropped are indexed
//!   by the next scan.
//! * A removal reaches the primary tag of the file only. A value in a
//!   second tag (an ID3v1 next to an ID3v2, the `RIFF INFO` of a WAV)
//!   survives it, and the write report names it
//!   ([`WriteReport::unremoved`]).
//! * The artwork cache notices a changed file through its stat, like the
//!   index does: a rewrite that keeps both the size and the modification
//!   time keeps the old cover. The same holds for a sidecar image.
//! * The sidecar fallback looks in the directory of the track only, and
//!   only for the names of [`SIDECAR_NAMES`]. A cover that sits further up
//!   the tree is not found.
//! * `remove_missing` cannot tell a deleted file from a file on a share
//!   that is not reachable: both answer "not found". The rows of a root on
//!   an offline drive are therefore dropped.
//! * `remove_missing` only reaches the rows under a registered root. A row
//!   that a scan of an unregistered path left behind is never cleaned up.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, OptionalExtension, params};
use rustamp_core::error::{Error, Result};
use rustamp_core::meta::TrackMetadata;

mod artwork;
mod open;
#[cfg(test)]
mod query_props;
#[cfg(test)]
mod scan_props;
#[cfg(test)]
mod testsupport;

pub use artwork::{
    ArtworkCache, ArtworkCacheStats, CoverImage, DEFAULT_MAX_BYTES, DEFAULT_MAX_ENTRIES,
    MAX_DECODE_BYTES, SIDECAR_NAMES, THUMBNAIL_MAX_EDGE,
};
// The damage a database file can have is part of this crate's own surface:
// a caller opens the index and has to know *why* it could not be opened.
pub use open::OpenError;
// The types of a tag write are part of this crate's own surface: a caller
// names the fields it wants removed and reads the report of the write
// without depending on `rustamp-meta` itself.
pub use rustamp_meta::{TagField, WriteReport};

/// The column list shared by every row-returning statement, in the order
/// [`read_raw`] expects.
const TRACK_COLUMNS: &str = "id, path, title, artist, album, album_artist, year, genre, \
     comment, track_no, track_total, disc_no, duration_ms, bitrate_kbps, mtime_ns, size";

/// The column list of the `roots` table, in the order [`read_root`] expects.
const ROOT_COLUMNS: &str = "id, path, added_at, last_scanned_at";

/// SQL function that lower-cases a value with Rust's Unicode rules, so the
/// crate is case-insensitive beyond SQLite's ASCII-only `NOCASE`. Registered on
/// every connection in [`configure`].
const LOWER_FN: &str = "rustamp_lower";

/// The stored `(mtime_ns, size)` pair used for change detection.
type StoredStat = (Option<i64>, Option<i64>);

/// A row in the media library index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackRow {
    /// Stable row id (SQLite rowid).
    pub id: i64,
    /// Absolute or as-scanned file path.
    pub path: PathBuf,
    /// Tag metadata.
    pub metadata: TrackMetadata,
    /// File mtime (unix nanoseconds), if known.
    pub mtime_ns: Option<i64>,
    /// File size in bytes, if known.
    pub size: Option<i64>,
}

/// Filter, sort and window for [`Library::query`]. The filter fields are
/// combined with AND; `None` means "no constraint".
///
/// Text filters are exact, case-sensitive matches. `path_prefix` is a literal
/// byte prefix, not a directory boundary and not a case-insensitive one: a
/// prefix of `C:\music` also matches `C:\music2`, but `c:\music` matches
/// nothing. The remaining fields shape the result (see the module docs):
/// [`Query::text`] searches, [`Query::sort`] and [`Query::descending`] order,
/// [`Query::limit`] and [`Query::offset`] page.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Query {
    /// Only rows whose path starts with this prefix.
    pub path_prefix: Option<PathBuf>,
    /// Exact artist match.
    pub artist: Option<String>,
    /// Exact album match.
    pub album: Option<String>,
    /// Exact title match.
    pub title: Option<String>,
    /// Exact genre match.
    pub genre: Option<String>,
    /// Free-text search: a literal, case-insensitive substring of `title`,
    /// `artist` or `album`. `%`, `_` and `\` are literal, not wildcards.
    /// Whitespace-only text is ignored.
    pub text: Option<String>,
    /// Sort column. Ties are broken by ascending row id.
    pub sort: SortKey,
    /// Reverse the sort column's order. `NULL` values still sort last.
    pub descending: bool,
    /// Maximum number of rows to return; `None` returns every match.
    pub limit: Option<u32>,
    /// Number of matches to skip before the first returned row.
    pub offset: u32,
}

/// Sort column for [`Query::sort`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    /// Insertion order (the stable row id). The default.
    #[default]
    Id,
    /// Track title, case-insensitively.
    Title,
    /// Track artist, case-insensitively.
    Artist,
    /// Album title, case-insensitively.
    Album,
    /// Track duration in milliseconds, shortest first.
    Duration,
}

/// One page of [`Library::query`] results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackPage {
    /// The requested window of matches, in sort order.
    pub rows: Vec<TrackRow>,
    /// Number of matches for the filter, ignoring `limit` and `offset`.
    pub total: usize,
}

/// A registered library root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    /// Stable root id.
    pub id: i64,
    /// Root path as it was added.
    pub path: PathBuf,
    /// Unix seconds when the root was added.
    pub added_at: i64,
    /// Unix seconds when the last scan of this root ran to the end.
    ///
    /// `None` when the root was never scanned, and also when its last scan
    /// was cancelled or failed: a scan that did not finish must not claim
    /// that the root was scanned. The time of the scan before it is kept.
    pub last_scanned_at: Option<i64>,
}

/// Summary returned by [`Library::scan`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScanReport {
    /// Considered files whose stat was readable (unchanged, added or updated).
    pub scanned: usize,
    /// New paths inserted.
    pub added: usize,
    /// Existing paths whose stat changed and were re-read.
    pub updated: usize,
    /// Stored rows under the root whose file no longer exists on disk.
    pub removed: usize,
    /// Considered files that could not be read or whose tags failed to parse.
    pub skipped: usize,
    /// Whether the scan was cancelled before it was done.
    ///
    /// A cancelled scan reports the state it committed, so `added`,
    /// `updated` and `removed` are the changes the index really holds.
    pub cancelled: bool,
}

/// Summary returned by [`Library::remove_missing`].
///
/// The report says what the operation did to the index: how many rows it
/// looked at, how many it dropped, and which rows it kept because their file
/// could not be checked.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RemoveReport {
    /// Rows whose file was looked for on disk.
    pub checked: usize,
    /// Rows whose file is gone and were dropped from the index.
    pub removed: usize,
    /// Rows whose file could not be checked; they were kept.
    ///
    /// The paths are sorted, so the list is the same for the same index.
    pub kept_unreadable: Vec<PathBuf>,
}

/// The outcome of a tag write: the refreshed row and what the format refused.
///
/// The row is what the **file** says after the write, never what the caller
/// asked for (see [`Library::write_tags_with`]). The report names the
/// values the target format could not take out of the file: a removal that
/// the format refused is in [`WriteReport::unremoved`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagWrite {
    /// The row, rebuilt from the file after the write.
    pub row: TrackRow,
    /// What the target tag format could not do.
    pub report: WriteReport,
}

/// How many considered files a scan commits in one transaction.
///
/// A scan writes its rows in batches, and every batch is one transaction, so
/// the index never holds half a batch. The batch size bounds two things at
/// once: the work a cancelled scan drops (at most one batch) and the time a
/// write transaction is open. Sixty-four files per transaction is the
/// compromise: a large library commits few times, and a cancelled scan does
/// not lose much.
pub const SCAN_BATCH_SIZE: usize = 64;

/// A cancellation flag a running scan checks between files.
///
/// The flag is shared and cheap to clone: the caller keeps one clone and
/// hands another to the scan, so a scan that runs on a worker thread can be
/// stopped from the thread that owns the user interface.
///
/// ```
/// use rustamp_library::Cancel;
///
/// // The flag is shared: the scan gets one clone, the interface thread
/// // keeps the other.
/// let cancel = Cancel::new();
/// let for_the_worker = cancel.clone();
/// assert!(!for_the_worker.is_cancelled());
/// cancel.cancel();
/// assert!(for_the_worker.is_cancelled());
/// ```
#[derive(Debug, Clone, Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
}

impl Cancel {
    /// A fresh flag that is not cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the running scan to stop.
    ///
    /// The scan stops at the next file boundary: it finishes the file it is
    /// on and returns. The batch in flight is not written, but the batches
    /// that were committed before the cancel stay in the index. Setting the
    /// flag twice is harmless, and setting it after the scan has ended
    /// changes nothing.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Whether the flag is set.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// The coarse phase a running scan is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPhase {
    /// The scan walks the tree and indexes the files it finds.
    Scanning,
    /// The removal pass deletes the rows whose file is gone.
    Removing,
    /// The scan is over, whether it finished or it was cancelled.
    ///
    /// Whether the scan was cancelled is in [`ScanReport::cancelled`], not
    /// here: the phase says that the scan ended, the report says how.
    Finished,
}

/// One progress event of a running scan.
///
/// The counters are the ones of the scan so far. They include the batch that
/// is in flight, so they can be ahead of what the index holds by up to
/// [`SCAN_BATCH_SIZE`] files; the [`ScanReport`] a scan returns counts only
/// the committed batches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanProgress {
    /// The phase the scan is in.
    pub phase: ScanPhase,
    /// The file the scan is working on, if any.
    pub path: Option<PathBuf>,
    /// Considered files the scan has seen.
    pub seen: usize,
    /// Rows the scan has added.
    pub added: usize,
    /// Rows the scan has updated.
    pub updated: usize,
    /// Files the scan could not read.
    pub skipped: usize,
    /// Rows the scan has removed.
    pub removed: usize,
}

/// Persistent media library index.
pub struct Library {
    conn: Connection,
}

impl Library {
    /// Open (or create) a library database at `path`.
    ///
    /// The file is checked before it is used, and a file that could not be
    /// read as *this* crate's index is **never written to**: it is refused
    /// with a typed [`OpenError`] and left exactly as it was. The index of a
    /// user is not lost because a file was damaged, and a damaged file is
    /// not "repaired" by writing a fresh schema over it - that would throw
    /// the index away without a word.
    ///
    /// The damage classes and the error each of them gets:
    ///
    /// | Damage | The error |
    /// |---|---|
    /// | the path is not a regular file | [`OpenError::NotAFile`] |
    /// | the file does not start with the SQLite header | [`OpenError::NotADatabase`] |
    /// | the file was cut short | [`OpenError::Truncated`] |
    /// | SQLite reports the database as damaged | [`OpenError::Corrupt`] |
    /// | the file holds another program's tables | [`OpenError::ForeignSchema`] |
    /// | a newer version of the player wrote the file | [`OpenError::FutureVersion`] |
    /// | the file cannot be written to | [`OpenError::ReadOnly`] |
    /// | another process holds the file | [`OpenError::Locked`] |
    ///
    /// A file that is not there yet, and a file without a byte in it, are
    /// the first run: the schema is created in them.
    ///
    /// # Errors
    ///
    /// [`OpenError`] when the file is not a database of this crate. The
    /// error names the damage, so a caller can tell the user what happened
    /// to the file.
    pub fn open(path: impl AsRef<Path>) -> std::result::Result<Self, OpenError> {
        let path = path.as_ref();
        let conn = open::connect(path)?;
        prepared(conn).map_err(|error| OpenError::Failed {
            path: path.to_path_buf(),
            detail: error.to_string(),
        })
    }

    /// Open an in-memory library (useful for tests).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(map_sqlite)?;
        prepared(conn).map_err(map_sqlite)
    }

    /// Scan `root`, upserting track rows. See module docs for the rule.
    ///
    /// Returns [`Error::NotFound`] when `root` does not exist.
    pub fn scan(&mut self, root: &Path) -> Result<ScanReport> {
        self.scan_with_progress(root, &Cancel::new(), |_| {})
    }

    /// Scan `root`, upserting track rows and reporting the progress.
    ///
    /// This is [`Library::scan`] with two extras: the scan reports what it
    /// does through `on_progress`, and it can be stopped through `cancel`.
    /// The crate owns no thread - the caller decides whether the scan runs
    /// on the interface thread, on a worker thread, or wherever it likes.
    ///
    /// The scan follows the rules of [`Library::scan`] (see the module docs):
    /// the same change detection on `(mtime_ns, size)`, the same treatment
    /// of an unreadable file, and the same removal pass.
    ///
    /// `on_progress` runs on the thread that drives the scan, once before
    /// the first file, once per considered file and once per phase change.
    /// It should return quickly: a caller that forwards the events to an
    /// interface thread blocks here until the channel takes them, and a
    /// caller that cannot keep up with one event per file should throttle
    /// the events it forwards.
    ///
    /// # Cancellation
    ///
    /// The scan checks `cancel` between two files. A cancelled scan stops
    /// where it is: the batch in flight is not written, so the index keeps
    /// the batches that were committed before the cancel and never holds
    /// half a batch. The removal pass does not run after a cancel, so a
    /// cancelled scan never deletes a row.
    ///
    /// The returned [`ScanReport`] describes the committed state: `added`,
    /// `updated` and `removed` are the changes the index really holds, and
    /// `cancelled` says whether the scan was stopped.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] when `root` does not exist, and the error of the
    /// database when a batch cannot be written.
    pub fn scan_with_progress(
        &mut self,
        root: &Path,
        cancel: &Cancel,
        mut on_progress: impl FnMut(ScanProgress),
    ) -> Result<ScanReport> {
        if fs::metadata(root).is_err() {
            return Err(Error::NotFound(root.display().to_string()));
        }

        // The stored rows under the root, taken before the scan writes
        // anything: they are the baseline of the change detection and the
        // set the removal pass works on.
        let existing = self.rows_under(root)?;

        let mut state = ScanState {
            conn: &mut self.conn,
            existing: &existing,
            cancel,
            on_progress: &mut on_progress,
            present: HashSet::new(),
            committed: Counters::default(),
            pending: Counters::default(),
            batch: Vec::with_capacity(SCAN_BATCH_SIZE),
            files_in_batch: 0,
            walk_complete: true,
            cancelled: false,
        };

        state.emit(ScanPhase::Scanning, None);

        if root.is_file() {
            // A file root indexes that one file.
            let _ = state.visit(root)?;
        } else {
            walk_files(&mut state, root)?;
        }

        if state.cancel.is_cancelled() {
            // The batch in flight is dropped: its rows were never written.
            state.cancelled = true;
            state.discard();
        } else {
            // The last batch, then the removal pass.
            state.flush()?;
            if state.walk_complete {
                state.removal_pass()?;
            }
        }

        state.emit(ScanPhase::Finished, None);
        Ok(state.report())
    }

    /// Scan the root registered under `root_id`. See [`Library::scan`].
    ///
    /// A scan that ran to the end records its time in the root's row; a
    /// cancelled or failed scan records nothing. See
    /// [`Library::scan_root_with_progress`].
    ///
    /// Returns [`Error::NotFound`] when no root has that id, and when the
    /// registered directory no longer exists.
    pub fn scan_root(&mut self, root_id: i64) -> Result<ScanReport> {
        self.scan_root_with_progress(root_id, &Cancel::new(), |_| {})
    }

    /// Scan the root registered under `root_id`, reporting the progress.
    /// See [`Library::scan_with_progress`].
    ///
    /// A scan that ran to the end records its time in the root's row
    /// (`last_scanned_at`), so the index knows when the root was last
    /// looked at. A scan that was **cancelled** did not finish and records
    /// nothing: the previous time is kept, because a cancelled scan must
    /// not claim that the root was scanned. A scan that **failed** returns
    /// an error and records nothing either.
    ///
    /// Returns [`Error::NotFound`] when no root has that id, and when the
    /// registered directory no longer exists.
    pub fn scan_root_with_progress(
        &mut self,
        root_id: i64,
        cancel: &Cancel,
        on_progress: impl FnMut(ScanProgress),
    ) -> Result<ScanReport> {
        let root = self
            .root_by_id(root_id)?
            .ok_or_else(|| Error::NotFound(format!("root {root_id}")))?;
        let report = self.scan_with_progress(&root.path, cancel, on_progress)?;
        if !report.cancelled {
            self.set_last_scanned_at(root_id)?;
        }
        Ok(report)
    }

    /// Query rows by filter, sorted and paged. See the module docs for the
    /// search, sort and paging rules.
    pub fn query(&self, filter: &Query) -> Result<TrackPage> {
        let (where_sql, binds) = filter_sql(filter);
        let total: i64 = self
            .conn
            .query_row(
                &format!("SELECT COUNT(*) FROM tracks{where_sql}"),
                rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())),
                |row| row.get(0),
            )
            .map_err(map_sqlite)?;

        let mut sql = format!(
            "SELECT {TRACK_COLUMNS} FROM tracks{where_sql} ORDER BY {}",
            order_sql(filter)
        );
        let limit = filter.limit.map(i64::from).unwrap_or(-1);
        let offset = i64::from(filter.offset);
        let mut bounds: Vec<&dyn rusqlite::ToSql> = binds.iter().map(|b| b.as_ref()).collect();
        if filter.limit.is_some() {
            sql.push_str(" LIMIT ? OFFSET ?");
            bounds.push(&limit);
            bounds.push(&offset);
        } else if filter.offset > 0 {
            sql.push_str(" LIMIT -1 OFFSET ?");
            bounds.push(&offset);
        }

        let mut stmt = self.conn.prepare(&sql).map_err(map_sqlite)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(bounds), read_raw)
            .map_err(map_sqlite)?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(map_sqlite)?.into_track(&self.conn)?);
        }
        Ok(TrackPage {
            rows: result,
            total: total as usize,
        })
    }

    /// Total number of indexed tracks.
    pub fn count(&self) -> Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))
            .map_err(map_sqlite)?;
        Ok(n as usize)
    }

    /// Fetch a single row by id.
    pub fn get(&self, id: i64) -> Result<Option<TrackRow>> {
        let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE id = ?1");
        let raw: Option<RawRow> = self
            .conn
            .query_row(&sql, params![id], read_raw)
            .optional()
            .map_err(map_sqlite)?;
        match raw {
            None => Ok(None),
            Some(raw) => Ok(Some(raw.into_track(&self.conn)?)),
        }
    }

    // ---- tag writing ----

    /// Write `metadata` into the tags of the indexed track `id` and refresh
    /// the row from the file.
    ///
    /// This is the write without removals: a field that is `None` in
    /// `metadata` is left as the file has it (the `rustamp-meta` contract),
    /// so one field can be edited without losing the others. A caller that
    /// wants a field to *go away* uses [`Library::write_tags_with`]; a
    /// caller that wants to know which values the format refused uses it
    /// too, with an empty removal list.
    ///
    /// The values are written into the **real file** with
    /// [`rustamp_meta::write_tags_with_report`], which replaces it
    /// atomically: after the call the file is either the old one or the new
    /// one, never a half-written mixture.
    ///
    /// The row is then rebuilt from the file: the tags are **read back**
    /// after the write, and it is *those* values that land in the index,
    /// never the values the caller passed in. A format that stores a value
    /// differently, or cannot store it at all, therefore shows up in the
    /// index as the file really is - the index can never claim a value the
    /// file does not carry.
    ///
    /// The row id does not change (the row is updated in place), and the
    /// stored `mtime_ns`/`size` are refreshed from the written file, so the
    /// next scan sees no change and does not read the file a second time.
    /// The refreshed row is returned, so a caller can show it without
    /// querying again.
    ///
    /// # Errors
    ///
    /// * [`Error::NotFound`] if no row has the id `id`, or if the file the
    ///   row points at is gone. A file that vanished between the scan and
    ///   the write is reported, never written into the row silently.
    /// * The error of the write itself: an unreadable file, a format whose
    ///   tags cannot be written, a year a tag cannot hold.
    /// * The error of reading the tags back, if the written file cannot be
    ///   read any more.
    ///
    /// A failed write leaves the row exactly as it was. A write that
    /// succeeded but could not be read back leaves the row as it was, too:
    /// the row is then stale, and the next scan repairs it, because the
    /// file's stat no longer matches the stored one.
    pub fn write_tags(&mut self, id: i64, metadata: &TrackMetadata) -> Result<TrackRow> {
        self.write_tags_with(id, metadata, &[])
            .map(|write| write.row)
    }

    /// Write `metadata` into the tags of the indexed track `id`, remove the
    /// fields of `removals` and refresh the row from the file.
    ///
    /// This is [`Library::write_tags`] with the removals: a field of
    /// `metadata` that is `None` keeps the value the file has, and a field
    /// of `removals` ([`TagField`]) is taken out of the file. A removal wins
    /// over a value: a field that is in both is removed.
    ///
    /// The write is the atomic one of the `rustamp-meta` writer
    /// ([`rustamp_meta::write_tags_with_report`]): after the call the file
    /// is either the old one or the new one, never a half-written mixture.
    ///
    /// The row is then rebuilt from the file: the tags are **read back**
    /// after the write, and it is *those* values that land in the index,
    /// never the values the caller passed in. A format that stores a value
    /// differently, or cannot store it at all, therefore shows up in the
    /// index as the file really is.
    ///
    /// ## What the format refused
    ///
    /// The writer only writes the *primary* tag of the file. A value in a
    /// second tag - an ID3v1 tag next to an ID3v2 tag, the `RIFF INFO` of a
    /// WAV, the text chunks of an AIFF - is not written and stays in the
    /// file, so a removal of that value does not take it out of the file.
    /// The returned [`TagWrite`] names every removal that is in that state
    /// ([`WriteReport::unremoved`]), and the row carries the value the file
    /// still has.
    ///
    /// # Errors
    ///
    /// * [`Error::NotFound`] if no row has the id `id`, or if the file the
    ///   row points at is gone. A file that vanished between the scan and
    ///   the write is reported, never written into the row silently.
    /// * The error of the write itself: an unreadable file, a format whose
    ///   tags cannot be written, a year a tag cannot hold.
    /// * The error of reading the tags back, if the written file cannot be
    ///   read any more.
    ///
    /// A failed write leaves the row exactly as it was. A write that
    /// succeeded but could not be read back leaves the row as it was, too:
    /// the row is then stale, and the next scan repairs it, because the
    /// file's stat no longer matches the stored one.
    pub fn write_tags_with(
        &mut self,
        id: i64,
        metadata: &TrackMetadata,
        removals: &[TagField],
    ) -> Result<TagWrite> {
        let row = self
            .get(id)?
            .ok_or_else(|| Error::NotFound(format!("track {id}")))?;
        let path = row.path;

        // The file must still be there. A write into a file that vanished
        // would fail halfway, and a row that describes a file which is not
        // there any more must not be updated.
        if fs::metadata(&path).is_err() {
            return Err(Error::NotFound(path.display().to_string()));
        }

        // The report says what the format refused: a removal the file
        // still carries in a second tag, an `extra` entry the target format
        // has no field for.
        let report = rustamp_meta::write_tags_with_report(&path, metadata, removals)?;

        // What the file says now, not what the caller asked for.
        let tags = rustamp_meta::read_tags(&path)?;

        // The write replaced the file, so the stat is taken again: the row
        // must carry the size and the mtime of the new file, or the next
        // scan would see a change and read the file a second time. A stat
        // that fails leaves both unknown, which the next scan repairs.
        let stat = fs::metadata(&path).ok();
        let mtime_ns = stat.as_ref().and_then(mtime_of);
        let size = stat.map(|stat| stat.len() as i64);

        let tx = self.conn.transaction().map_err(map_sqlite)?;
        // The row is updated by its path, so the id it had before the write
        // is the id it has now.
        let row_id = upsert_row(&tx, &path, &tags, mtime_ns, size)?;
        tx.commit().map_err(map_sqlite)?;

        let row = self
            .get(row_id)?
            .ok_or_else(|| Error::NotFound(format!("track {row_id}")))?;
        Ok(TagWrite { row, report })
    }

    // ---- roots ----

    /// Add `path` as a library root and return it.
    ///
    /// Idempotent: a path that is already a root is returned unchanged. The
    /// path is stored exactly as given (no canonicalisation) and does not have
    /// to exist yet, so a library can list an offline drive; scanning it while
    /// it is missing reports [`Error::NotFound`].
    pub fn add_root(&mut self, path: &Path) -> Result<Root> {
        let path_str = path.to_string_lossy().to_string();
        if let Some(existing) = self.root_by_path(&path_str)? {
            return Ok(existing);
        }
        let added_at = now_unix();
        self.conn
            .execute(
                "INSERT INTO roots (path, added_at) VALUES (?1, ?2)",
                params![path_str, added_at],
            )
            .map_err(map_sqlite)?;
        Ok(Root {
            id: self.conn.last_insert_rowid(),
            path: path.to_path_buf(),
            added_at,
            last_scanned_at: None,
        })
    }

    /// Forget the root `path` and delete every indexed track under it.
    ///
    /// Returns the number of deleted tracks, which may be zero for a root that
    /// was never scanned. Tracks under other roots are untouched. Returns
    /// [`Error::NotFound`] when `path` is not a registered root.
    pub fn remove_root(&mut self, path: &Path) -> Result<usize> {
        let path_str = path.to_string_lossy().to_string();
        if self.root_by_path(&path_str)?.is_none() {
            return Err(Error::NotFound(path.display().to_string()));
        }
        let stored = self.rows_under(path)?;
        let tx = self.conn.transaction().map_err(map_sqlite)?;
        for track_path in stored.keys() {
            delete_path(&tx, track_path)?;
        }
        tx.execute("DELETE FROM roots WHERE path = ?1", params![path_str])
            .map_err(map_sqlite)?;
        tx.commit().map_err(map_sqlite)?;
        Ok(stored.len())
    }

    /// The registered roots, oldest first.
    pub fn roots(&self) -> Result<Vec<Root>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {ROOT_COLUMNS} FROM roots ORDER BY id"))
            .map_err(map_sqlite)?;
        let rows = stmt.query_map([], read_root).map_err(map_sqlite)?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(map_sqlite)?);
        }
        Ok(result)
    }

    /// Look up a root by its stored path string.
    fn root_by_path(&self, path: &str) -> Result<Option<Root>> {
        self.conn
            .query_row(
                &format!("SELECT {ROOT_COLUMNS} FROM roots WHERE path = ?1"),
                params![path],
                read_root,
            )
            .optional()
            .map_err(map_sqlite)
    }

    /// Look up a root by id.
    fn root_by_id(&self, id: i64) -> Result<Option<Root>> {
        self.conn
            .query_row(
                &format!("SELECT {ROOT_COLUMNS} FROM roots WHERE id = ?1"),
                params![id],
                read_root,
            )
            .optional()
            .map_err(map_sqlite)
    }

    /// Set the time of the last scan of the root `id`.
    ///
    /// The time is the unix second of the moment, not the time of the scan's
    /// start: the scan has already run when this is called.
    fn set_last_scanned_at(&mut self, id: i64) -> Result<()> {
        self.conn
            .execute(
                "UPDATE roots SET last_scanned_at = ?1 WHERE id = ?2",
                params![now_unix(), id],
            )
            .map_err(map_sqlite)?;
        Ok(())
    }

    // ---- removing dead entries ----

    /// Drop the rows whose file is gone, under the selected roots.
    ///
    /// This is the library's "remove dead entries": it walks the **rows** the
    /// index holds under the registered roots and asks the file system about
    /// each path, so it does not need a full rescan. See the module docs for
    /// the rule.
    ///
    /// `roots` selects the roots the operation works on: `None` means every
    /// registered root, a list of root ids means those roots only (the ids
    /// come from [`Library::roots`]). A root id that is not registered is an
    /// error ([`Error::NotFound`]), and nothing is removed in that case.
    ///
    /// The rows of the roots that were not selected are never touched.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] when a given root id is not registered, and the
    /// error of the database when the deletion cannot be written.
    pub fn remove_missing(&mut self, roots: Option<&[i64]>) -> Result<RemoveReport> {
        let selected = self.selected_roots(roots)?;

        // The rows under the selected roots. A row under two roots (nested
        // roots) is looked at once.
        let mut candidates: BTreeSet<PathBuf> = BTreeSet::new();
        for root in &selected {
            for (path, _) in self.rows_under(&root.path)? {
                candidates.insert(path);
            }
        }

        let mut gone: Vec<PathBuf> = Vec::new();
        let mut kept_unreadable: Vec<PathBuf> = Vec::new();
        for path in &candidates {
            match fs::metadata(path) {
                // The file is there: the row stays.
                Ok(_) => {}
                // The file is gone: the row is dropped.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    gone.push(path.clone());
                }
                // The file could not be checked: the row stays, and the
                // caller learns which rows these are.
                Err(_) => kept_unreadable.push(path.clone()),
            }
        }

        let removed = self.delete_paths(&gone)?;
        Ok(RemoveReport {
            checked: candidates.len(),
            removed,
            kept_unreadable,
        })
    }

    /// The roots an operation works on: the given ids, or every registered
    /// root when no ids are given.
    fn selected_roots(&self, roots: Option<&[i64]>) -> Result<Vec<Root>> {
        let Some(ids) = roots else {
            return self.roots();
        };
        let mut selected = Vec::with_capacity(ids.len());
        for id in ids {
            let root = self
                .root_by_id(*id)?
                .ok_or_else(|| Error::NotFound(format!("root {id}")))?;
            selected.push(root);
        }
        Ok(selected)
    }

    /// Delete the rows with the given paths, in one transaction.
    ///
    /// The paths are unique in the index, so every path is exactly one row.
    /// The `track_extra` rows of a deleted track go with it: the foreign key
    /// cascades.
    fn delete_paths(&mut self, paths: &[PathBuf]) -> Result<usize> {
        if paths.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.transaction().map_err(map_sqlite)?;
        for path in paths {
            delete_path(&tx, path)?;
        }
        tx.commit().map_err(map_sqlite)?;
        Ok(paths.len())
    }

    // ---- internals ----

    /// All stored paths under `root`, with the stat fields used for change
    /// detection. This is also the row set [`Library::remove_root`] deletes,
    /// so both operations agree on what "under this root" means.
    fn rows_under(&self, root: &Path) -> Result<HashMap<PathBuf, StoredStat>> {
        let pattern = like_prefix(&root.to_string_lossy());
        let mut stmt = self
            .conn
            .prepare("SELECT path, mtime_ns, size FROM tracks WHERE path LIKE ?1 ESCAPE '\\'")
            .map_err(map_sqlite)?;
        let rows = stmt
            .query_map(params![pattern], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })
            .map_err(map_sqlite)?;

        let mut result = HashMap::new();
        for row in rows {
            let (path, mtime_ns, size) = row.map_err(map_sqlite)?;
            let path = PathBuf::from(path);
            // The LIKE pattern is only a cheap pre-filter; the component-wise
            // check rejects paths that merely share the byte prefix.
            if path.starts_with(root) {
                result.insert(path, (mtime_ns, size));
            }
        }
        Ok(result)
    }
}

/// The connection, prepared for use: the SQL function registered and the
/// schema in place.
fn prepared(mut conn: Connection) -> rusqlite::Result<Library> {
    configure(&conn)?;
    open::migrate(&mut conn)?;
    Ok(Library { conn })
}

/// The counters of a scan.
#[derive(Debug, Default, Clone, Copy)]
struct Counters {
    /// Considered files that were looked at.
    seen: usize,
    /// Rows that were written as a new path.
    added: usize,
    /// Rows that were written for a known path.
    updated: usize,
    /// Files that could not be read.
    skipped: usize,
    /// Rows that were deleted.
    removed: usize,
}

impl std::ops::Add for Counters {
    type Output = Counters;

    fn add(self, other: Counters) -> Counters {
        Counters {
            seen: self.seen + other.seen,
            added: self.added + other.added,
            updated: self.updated + other.updated,
            skipped: self.skipped + other.skipped,
            removed: self.removed + other.removed,
        }
    }
}

/// A row the scan has read and is going to write.
struct PendingRow {
    /// The file the row belongs to.
    path: PathBuf,
    /// The tags read from the file.
    tags: TrackMetadata,
    /// The file's mtime, for the change detection of the next scan.
    mtime_ns: Option<i64>,
    /// The file's size, for the change detection of the next scan.
    size: Option<i64>,
}

/// What the scan did with one considered file.
enum FileOutcome {
    /// A path the index did not know yet.
    Added,
    /// A known path whose stat changed.
    Updated,
    /// A known path with the same stat: the row is left untouched.
    Unchanged,
    /// A file that could not be stat'ed, read or parsed.
    Skipped,
}

/// The state of one running scan.
///
/// The scan keeps two sets of counters: the committed ones describe what the
/// index holds, the pending ones describe the batch that is in flight. A
/// progress event reports the sum of both, the report of the scan the
/// committed ones only.
struct ScanState<'a> {
    /// The database the rows are written to.
    conn: &'a mut Connection,
    /// The stored rows under the root, taken before the scan writes.
    existing: &'a HashMap<PathBuf, StoredStat>,
    /// The cancellation flag.
    cancel: &'a Cancel,
    /// Where the progress events go.
    on_progress: &'a mut dyn FnMut(ScanProgress),
    /// The considered files that were found on disk, whether or not they
    /// could be read. A file that is present but unreadable must not be
    /// treated as removed.
    present: HashSet<PathBuf>,
    /// The counters of the committed batches.
    committed: Counters,
    /// The counters of the batch in flight.
    pending: Counters,
    /// The rows of the batch in flight, not written yet.
    batch: Vec<PendingRow>,
    /// Considered files in the batch in flight.
    files_in_batch: usize,
    /// Whether the tree was walked completely.
    walk_complete: bool,
    /// Whether the scan was cancelled.
    cancelled: bool,
}

impl ScanState<'_> {
    /// Index one considered file.
    ///
    /// Returns `false` when the scan must stop, which is what a cancel
    /// does.
    fn visit(&mut self, path: &Path) -> Result<bool> {
        if self.cancel.is_cancelled() {
            return Ok(false);
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !rustamp_meta::supports_extension(ext) {
            // Not a media file: ignored, and counted in no field.
            return Ok(true);
        }
        self.present.insert(path.to_path_buf());
        self.pending.seen += 1;
        self.files_in_batch += 1;

        match self.index_file(path) {
            FileOutcome::Added => self.pending.added += 1,
            FileOutcome::Updated => self.pending.updated += 1,
            FileOutcome::Skipped => self.pending.skipped += 1,
            FileOutcome::Unchanged => {}
        }
        self.emit(ScanPhase::Scanning, Some(path));

        if self.files_in_batch >= SCAN_BATCH_SIZE {
            self.flush()?;
        }
        Ok(true)
    }

    /// Decide what to do with one considered file and stage its row.
    fn index_file(&mut self, path: &Path) -> FileOutcome {
        let meta = match fs::metadata(path) {
            Ok(meta) => meta,
            Err(_) => return FileOutcome::Skipped,
        };
        let mtime_ns = mtime_of(&meta);
        let size = Some(meta.len() as i64);

        if let Some((stored_mtime, stored_size)) = self.existing.get(path)
            && *stored_mtime == mtime_ns
            && *stored_size == size
        {
            // Same stat: the row is left untouched.
            return FileOutcome::Unchanged;
        }

        let tags = match rustamp_meta::read_tags(path) {
            Ok(tags) => tags,
            Err(_) => return FileOutcome::Skipped,
        };

        let is_new = !self.existing.contains_key(path);
        self.batch.push(PendingRow {
            path: path.to_path_buf(),
            tags,
            mtime_ns,
            size,
        });
        if is_new {
            FileOutcome::Added
        } else {
            FileOutcome::Updated
        }
    }

    /// Write the rows of the batch in one transaction and commit them.
    ///
    /// The batch becomes visible as a whole: the index never holds half of
    /// it. A scan that is cancelled before the flush never writes the batch
    /// at all.
    fn flush(&mut self) -> Result<()> {
        if !self.batch.is_empty() {
            let tx = self.conn.transaction().map_err(map_sqlite)?;
            for row in &self.batch {
                upsert_row(&tx, &row.path, &row.tags, row.mtime_ns, row.size)?;
            }
            tx.commit().map_err(map_sqlite)?;
            self.batch.clear();
        }
        self.committed = self.committed + self.pending;
        self.pending = Counters::default();
        self.files_in_batch = 0;
        Ok(())
    }

    /// Throw the batch in flight away: its rows were never written.
    fn discard(&mut self) {
        self.batch.clear();
        self.pending = Counters::default();
        self.files_in_batch = 0;
    }

    /// Delete the rows under the root whose file is gone.
    ///
    /// The pass only runs when the tree was walked completely and the scan
    /// was not cancelled. All deletions go into one transaction, so a cancel
    /// during the pass rolls it back and no row is removed.
    fn removal_pass(&mut self) -> Result<()> {
        self.emit(ScanPhase::Removing, None);
        let existing = self.existing;
        let tx = self.conn.transaction().map_err(map_sqlite)?;
        let mut removed = 0usize;
        let mut aborted = false;
        for path in existing.keys() {
            if self.cancel.is_cancelled() {
                aborted = true;
                break;
            }
            if !self.present.contains(path) {
                delete_path(&tx, path)?;
                removed += 1;
            }
        }
        if aborted {
            // The transaction is dropped without a commit: the rollback
            // leaves every row in place.
            self.cancelled = true;
            return Ok(());
        }
        tx.commit().map_err(map_sqlite)?;
        self.committed.removed += removed;
        Ok(())
    }

    /// Send one progress event.
    fn emit(&mut self, phase: ScanPhase, path: Option<&Path>) {
        let live = self.committed + self.pending;
        let event = ScanProgress {
            phase,
            path: path.map(Path::to_path_buf),
            seen: live.seen,
            added: live.added,
            updated: live.updated,
            skipped: live.skipped,
            removed: live.removed,
        };
        (self.on_progress)(event);
    }

    /// The report of the committed state.
    fn report(&self) -> ScanReport {
        ScanReport {
            scanned: self.committed.seen.saturating_sub(self.committed.skipped),
            added: self.committed.added,
            updated: self.committed.updated,
            removed: self.committed.removed,
            skipped: self.committed.skipped,
            cancelled: self.cancelled,
        }
    }
}

/// Configure a fresh connection: foreign keys on, so deleting a track also
/// drops its `track_extra` rows, plus the Unicode lower-casing SQL function.
fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.create_scalar_function(
        LOWER_FN,
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let value: Option<String> = ctx.get(0)?;
            Ok(value.map(|v| v.to_lowercase()))
        },
    )
}

/// The `WHERE` clause for `filter` and its bound values, in bind order.
fn filter_sql(filter: &Query) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
    let mut sql = String::from(" WHERE 1=1");
    let mut binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(prefix) = &filter.path_prefix {
        // A literal byte prefix: `instr` finds the prefix at the start of
        // the path. A `LIKE` would not do: SQLite's `LIKE` ignores the
        // case of ASCII letters, and the prefix is a byte prefix, not a
        // case-insensitive one.
        sql.push_str(" AND instr(path, ?) = 1");
        binds.push(Box::new(prefix.to_string_lossy().to_string()));
    }
    if let Some(v) = &filter.artist {
        sql.push_str(" AND artist = ?");
        binds.push(Box::new(v.clone()));
    }
    if let Some(v) = &filter.album {
        sql.push_str(" AND album = ?");
        binds.push(Box::new(v.clone()));
    }
    if let Some(v) = &filter.title {
        sql.push_str(" AND title = ?");
        binds.push(Box::new(v.clone()));
    }
    if let Some(v) = &filter.genre {
        sql.push_str(" AND genre = ?");
        binds.push(Box::new(v.clone()));
    }
    if let Some(text) = &filter.text {
        let term = text.trim();
        if !term.is_empty() {
            let pattern = like_contains(term);
            sql.push_str(&format!(
                " AND ({LOWER_FN}(title) LIKE {LOWER_FN}(?) ESCAPE '\\' \
                 OR {LOWER_FN}(artist) LIKE {LOWER_FN}(?) ESCAPE '\\' \
                 OR {LOWER_FN}(album) LIKE {LOWER_FN}(?) ESCAPE '\\')"
            ));
            for _ in 0..3 {
                binds.push(Box::new(pattern.clone()));
            }
        }
    }
    (sql, binds)
}

/// The `ORDER BY` clause for `filter`: the requested column, then the row id as
/// a tie-break, so paging is stable. `NULL` values sort last in both
/// directions.
fn order_sql(filter: &Query) -> String {
    let dir = if filter.descending { "DESC" } else { "ASC" };
    match filter.sort {
        SortKey::Id => format!("id {dir}"),
        SortKey::Title => text_order("title", dir),
        SortKey::Artist => text_order("artist", dir),
        SortKey::Album => text_order("album", dir),
        SortKey::Duration => format!("(duration_ms IS NULL), duration_ms {dir}, id ASC"),
    }
}

/// Order by a text column, case-folded with [`LOWER_FN`].
fn text_order(column: &str, dir: &str) -> String {
    format!("({column} IS NULL), {LOWER_FN}({column}) {dir}, id ASC")
}

/// Read one `roots` row.
fn read_root(row: &rusqlite::Row<'_>) -> rusqlite::Result<Root> {
    Ok(Root {
        id: row.get(0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
        added_at: row.get(2)?,
        last_scanned_at: row.get(3)?,
    })
}

fn upsert_row(
    conn: &Connection,
    path: &Path,
    tags: &TrackMetadata,
    mtime_ns: Option<i64>,
    size: Option<i64>,
) -> Result<i64> {
    let path_str = path.to_string_lossy().to_string();
    let now = now_unix();
    let existing_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM tracks WHERE path = ?1",
            params![path_str],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sqlite)?;

    let id = if let Some(id) = existing_id {
        conn.execute(
            "UPDATE tracks SET title=?1, artist=?2, album=?3, album_artist=?4, year=?5, \
             genre=?6, comment=?7, track_no=?8, track_total=?9, disc_no=?10, \
             duration_ms=?11, bitrate_kbps=?12, mtime_ns=?13, size=?14, scanned_at=?15 \
             WHERE id=?16",
            params![
                tags.title,
                tags.artist,
                tags.album,
                tags.album_artist,
                tags.year.map(|y| y as i64),
                tags.genre,
                tags.comment,
                tags.track_no.map(|n| n as i64),
                tags.track_total.map(|n| n as i64),
                tags.disc_no.map(|n| n as i64),
                tags.duration_ms.map(|n| n as i64),
                tags.bitrate_kbps.map(|n| n as i64),
                mtime_ns,
                size,
                now,
                id
            ],
        )
        .map_err(map_sqlite)?;
        id
    } else {
        conn.execute(
            "INSERT INTO tracks (path, title, artist, album, album_artist, year, genre, \
             comment, track_no, track_total, disc_no, duration_ms, bitrate_kbps, mtime_ns, \
             size, scanned_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![
                path_str,
                tags.title,
                tags.artist,
                tags.album,
                tags.album_artist,
                tags.year.map(|y| y as i64),
                tags.genre,
                tags.comment,
                tags.track_no.map(|n| n as i64),
                tags.track_total.map(|n| n as i64),
                tags.disc_no.map(|n| n as i64),
                tags.duration_ms.map(|n| n as i64),
                tags.bitrate_kbps.map(|n| n as i64),
                mtime_ns,
                size,
                now
            ],
        )
        .map_err(map_sqlite)?;
        conn.last_insert_rowid()
    };

    // Replace extras.
    conn.execute("DELETE FROM track_extra WHERE track_id = ?1", params![id])
        .map_err(map_sqlite)?;
    for (k, v) in &tags.extra {
        conn.execute(
            "INSERT INTO track_extra (track_id, key, value) VALUES (?1, ?2, ?3)",
            params![id, k, v],
        )
        .map_err(map_sqlite)?;
    }
    Ok(id)
}

fn delete_path(conn: &Connection, path: &Path) -> Result<()> {
    let path_str = path.to_string_lossy().to_string();
    conn.execute("DELETE FROM tracks WHERE path = ?1", params![path_str])
        .map_err(map_sqlite)?;
    Ok(())
}

fn extras_for(conn: &Connection, track_id: i64) -> Result<BTreeMap<String, String>> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM track_extra WHERE track_id = ?1")
        .map_err(map_sqlite)?;
    let rows = stmt
        .query_map(params![track_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(map_sqlite)?;

    let mut map = BTreeMap::new();
    for row in rows {
        let (key, value) = row.map_err(map_sqlite)?;
        map.insert(key, value);
    }
    Ok(map)
}

/// The single definition of the row column order; keep in sync with
/// [`TRACK_COLUMNS`].
fn read_raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRow> {
    Ok(RawRow {
        id: row.get(0)?,
        path: row.get(1)?,
        title: row.get(2)?,
        artist: row.get(3)?,
        album: row.get(4)?,
        album_artist: row.get(5)?,
        year: row.get(6)?,
        genre: row.get(7)?,
        comment: row.get(8)?,
        track_no: row.get(9)?,
        track_total: row.get(10)?,
        disc_no: row.get(11)?,
        duration_ms: row.get(12)?,
        bitrate_kbps: row.get(13)?,
        mtime_ns: row.get(14)?,
        size: row.get(15)?,
    })
}

/// Raw database row, before the extra map is attached.
struct RawRow {
    id: i64,
    path: String,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    year: Option<i64>,
    genre: Option<String>,
    comment: Option<String>,
    track_no: Option<i64>,
    track_total: Option<i64>,
    disc_no: Option<i64>,
    duration_ms: Option<i64>,
    bitrate_kbps: Option<i64>,
    mtime_ns: Option<i64>,
    size: Option<i64>,
}

impl RawRow {
    fn into_track(self, conn: &Connection) -> Result<TrackRow> {
        let extra = extras_for(conn, self.id)?;
        let metadata = TrackMetadata {
            title: self.title,
            artist: self.artist,
            album: self.album,
            album_artist: self.album_artist,
            year: self.year.map(|y| y as u32),
            genre: self.genre,
            comment: self.comment,
            track_no: self.track_no.map(|n| n as u32),
            track_total: self.track_total.map(|n| n as u32),
            disc_no: self.disc_no.map(|n| n as u32),
            duration_ms: self.duration_ms.map(|n| n as u64),
            bitrate_kbps: self.bitrate_kbps.map(|n| n as u32),
            extra,
        };
        Ok(TrackRow {
            id: self.id,
            path: PathBuf::from(self.path),
            metadata,
            mtime_ns: self.mtime_ns,
            size: self.size,
        })
    }
}

/// Walk `dir` recursively and hand every regular file to `state`.
///
/// The walk stops as soon as the scan is cancelled. `state.walk_complete`
/// is cleared when a directory or an entry cannot be read, which is what
/// keeps the removal pass from running over a half-walked tree. Symlinks
/// are neither followed nor indexed.
fn walk_files(state: &mut ScanState<'_>, dir: &Path) -> Result<bool> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => {
            state.walk_complete = false;
            return Ok(true);
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                state.walk_complete = false;
                continue;
            }
        };
        let path = entry.path();
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => {
                if !walk_files(state, &path)? {
                    return Ok(false);
                }
            }
            Ok(ft) if ft.is_file() => {
                if !state.visit(&path)? {
                    return Ok(false);
                }
            }
            // Symlinks and other entries are not followed.
            Ok(_) => {}
            Err(_) => state.walk_complete = false,
        }
    }
    Ok(true)
}

/// Escape `%`, `_` and `\` in `prefix` and append the `%` that makes the
/// pattern a prefix match for `LIKE ... ESCAPE '\'`.
fn like_prefix(prefix: &str) -> String {
    let mut escaped = escape_like(prefix);
    escaped.push('%');
    escaped
}

/// `%escaped%`: a literal substring match for `LIKE ... ESCAPE '\'`.
fn like_contains(text: &str) -> String {
    format!("%{}%", escape_like(text))
}

/// Escape the `LIKE` metacharacters in `text`, keeping it literal.
fn escape_like(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// Modification time in nanoseconds since the Unix epoch.
fn mtime_of(meta: &fs::Metadata) -> Option<i64> {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn map_sqlite<E: std::fmt::Display>(e: E) -> Error {
    Error::Other(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsupport::{last_scanned_at, make_wav, put_last_scanned_at};
    use rustamp_core::meta::TrackMetadata;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    /// A WAV file with a RIFF `LIST INFO` chunk.
    ///
    /// The chunk is a **second** tag: the writer writes the ID3v2 chunk of
    /// a WAV and leaves the `INFO` chunk alone, so a value the `INFO` chunk
    /// holds survives a removal.
    fn make_wav_with_info(path: &Path, title: &str, artist: &str) {
        /// A RIFF chunk: the id, the size, the data, padded to an even size.
        fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut out = Vec::with_capacity(8 + data.len() + 1);
            out.extend_from_slice(id);
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(data);
            if data.len() % 2 == 1 {
                out.push(0);
            }
            out
        }

        // The `fmt ` chunk: 8-bit mono PCM.
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
        fmt.extend_from_slice(&1u16.to_le_bytes()); // mono
        fmt.extend_from_slice(&44100u32.to_le_bytes()); // sample rate
        fmt.extend_from_slice(&44100u32.to_le_bytes()); // byte rate
        fmt.extend_from_slice(&1u16.to_le_bytes()); // block align
        fmt.extend_from_slice(&8u16.to_le_bytes()); // bits per sample

        // The `LIST INFO` chunk: the title and the artist.
        let mut info = b"INFO".to_vec();
        let mut inam = title.as_bytes().to_vec();
        inam.push(0);
        let mut iart = artist.as_bytes().to_vec();
        iart.push(0);
        info.extend_from_slice(&chunk(b"INAM", &inam));
        info.extend_from_slice(&chunk(b"IART", &iart));

        let mut body = b"WAVE".to_vec();
        body.extend_from_slice(&chunk(b"fmt ", &fmt));
        body.extend_from_slice(&chunk(b"data", &[0u8; 16]));
        body.extend_from_slice(&chunk(b"LIST", &info));

        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend_from_slice(&body);

        let mut f = File::create(path).unwrap();
        f.write_all(&file).unwrap();
    }

    /// Writes a title/artist/album fixture into `path`. Named apart from
    /// [`Library::write_tags`] so the two are not confused in a test.
    fn write_fixture_tags(path: &Path, title: &str, artist: &str, album: &str) {
        let md = TrackMetadata {
            title: Some(title.to_owned()),
            artist: Some(artist.to_owned()),
            album: Some(album.to_owned()),
            genre: Some("Rock".to_owned()),
            track_no: Some(3),
            ..Default::default()
        };
        rustamp_meta::write_tags(path, &md).unwrap();
    }

    fn only_row(lib: &Library) -> TrackRow {
        let mut rows = lib.query(&Query::default()).unwrap().rows;
        assert_eq!(rows.len(), 1);
        rows.remove(0)
    }

    /// Rows for `filter` with the default sort and no paging.
    fn matches(lib: &Library, filter: &Query) -> Vec<TrackRow> {
        let page = lib.query(filter).unwrap();
        assert_eq!(
            page.total,
            page.rows.len(),
            "unpaged query: total is the row count"
        );
        page.rows
    }

    /// Titles of `filter`'s rows, `None` where a title is absent.
    fn titles(lib: &Library, filter: &Query) -> Vec<Option<String>> {
        matches(lib, filter)
            .into_iter()
            .map(|row| row.metadata.title)
            .collect()
    }

    #[test]
    fn scan_indexes_and_queries() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("a.wav");
        let b = root.join("b.wav");
        make_wav(&a);
        make_wav(&b);
        write_fixture_tags(&a, "Song A", "Artist X", "Album 1");
        write_fixture_tags(&b, "Song B", "Artist Y", "Album 2");

        let mut lib = Library::in_memory().unwrap();
        let rep = lib.scan(root).unwrap();
        assert_eq!(rep.scanned, 2);
        assert_eq!(rep.added, 2);
        assert_eq!(rep.removed, 0);
        assert_eq!(rep.skipped, 0);
        assert_eq!(lib.count().unwrap(), 2);

        // Query by artist.
        let rows = matches(
            &lib,
            &Query {
                artist: Some("Artist X".to_owned()),
                ..Default::default()
            },
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].metadata.title.as_deref(), Some("Song A"));

        // Query by path prefix.
        let rows = matches(
            &lib,
            &Query {
                path_prefix: Some(root.to_path_buf()),
                ..Default::default()
            },
        );
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn rescan_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "Song A", "Artist X", "Album 1");

        let mut lib = Library::in_memory().unwrap();
        lib.scan(root).unwrap();
        let first = only_row(&lib);

        let r2 = lib.scan(root).unwrap();
        assert_eq!(r2.scanned, 1);
        assert_eq!(r2.added, 0);
        assert_eq!(r2.updated, 0);
        assert_eq!(r2.removed, 0);
        assert_eq!(r2.skipped, 0);
        let second = only_row(&lib);
        assert_eq!(first.id, second.id, "stable id preserved");
        assert_eq!(first.mtime_ns, second.mtime_ns);
    }

    #[test]
    fn update_on_change() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "Song A", "Artist X", "Album 1");

        let mut lib = Library::in_memory().unwrap();
        lib.scan(root).unwrap();
        let id_before = only_row(&lib).id;

        // Rewrite the tags. No sleep: change detection uses nanoseconds.
        write_fixture_tags(&a, "Song A2", "Artist X", "Album 1");
        let rep = lib.scan(root).unwrap();
        assert_eq!(rep.updated, 1);
        assert_eq!(rep.added, 0);
        assert_eq!(rep.removed, 0);

        let row = only_row(&lib);
        assert_eq!(row.id, id_before, "stable id preserved");
        assert_eq!(row.metadata.title.as_deref(), Some("Song A2"));
    }

    #[test]
    fn removed_files_dropped() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("a.wav");
        let b = root.join("b.wav");
        make_wav(&a);
        make_wav(&b);
        write_fixture_tags(&a, "A", "X", "Al");
        write_fixture_tags(&b, "B", "Y", "Al2");

        let mut lib = Library::in_memory().unwrap();
        lib.scan(root).unwrap();
        assert_eq!(lib.count().unwrap(), 2);

        fs::remove_file(&b).unwrap();
        let rep = lib.scan(root).unwrap();
        assert_eq!(rep.removed, 1);
        assert_eq!(lib.count().unwrap(), 1);
    }

    #[test]
    fn query_by_album() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Album Z");

        let mut lib = Library::in_memory().unwrap();
        lib.scan(root).unwrap();
        let rows = matches(
            &lib,
            &Query {
                album: Some("Album Z".to_owned()),
                ..Default::default()
            },
        );
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn query_filters_combine_with_and() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        for (name, title, artist, album) in [
            ("a.wav", "One", "X", "Album 1"),
            ("b.wav", "Two", "X", "Album 2"),
            ("c.wav", "Three", "Y", "Album 1"),
        ] {
            let path = root.join(name);
            make_wav(&path);
            write_fixture_tags(&path, title, artist, album);
        }

        let mut lib = Library::in_memory().unwrap();
        lib.scan(root).unwrap();

        let rows = matches(
            &lib,
            &Query {
                artist: Some("X".to_owned()),
                album: Some("Album 1".to_owned()),
                ..Default::default()
            },
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].metadata.title.as_deref(), Some("One"));

        // A filter that matches nothing yields no rows.
        let rows = matches(
            &lib,
            &Query {
                artist: Some("X".to_owned()),
                genre: Some("Jazz".to_owned()),
                ..Default::default()
            },
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn extras_round_trip() {
        // Ensure extra map persists.
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("a.wav");
        make_wav(&a);
        let mut md = TrackMetadata {
            title: Some("A".to_owned()),
            ..Default::default()
        };
        md.extra
            .insert("REPLAYGAIN_TRACK_GAIN".to_owned(), "1.50 dB".to_owned());
        rustamp_meta::write_tags(&a, &md).unwrap();

        let mut lib = Library::in_memory().unwrap();
        lib.scan(root).unwrap();
        let row = only_row(&lib);
        assert_eq!(
            row.metadata
                .extra
                .get("REPLAYGAIN_TRACK_GAIN")
                .map(String::as_str),
            Some("1.50 dB")
        );
    }

    #[test]
    fn persistent_db_file() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("lib.db");
        let root = dir.path().join("music");
        fs::create_dir(&root).unwrap();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");

        {
            let mut lib = Library::open(&db).unwrap();
            lib.scan(&root).unwrap();
            assert_eq!(lib.count().unwrap(), 1);
        }
        // Reopen: data persists.
        let lib = Library::open(&db).unwrap();
        assert_eq!(lib.count().unwrap(), 1);
    }

    #[test]
    fn get_by_id() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        lib.scan(root).unwrap();
        let id = only_row(&lib).id;
        let row = lib.get(id).unwrap().unwrap();
        assert_eq!(row.id, id);
        assert_eq!(row.metadata.title.as_deref(), Some("A"));

        assert!(lib.get(id + 1).unwrap().is_none());
    }

    #[test]
    fn non_audio_files_are_ignored() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");
        fs::write(root.join("cover.jpg"), b"not an image").unwrap();
        fs::write(root.join("notes.txt"), b"liner notes").unwrap();
        fs::write(root.join("list.m3u"), b"a.wav").unwrap();

        let mut lib = Library::in_memory().unwrap();
        let rep = lib.scan(root).unwrap();
        assert_eq!(rep.scanned, 1);
        assert_eq!(rep.added, 1);
        // Ignored files are neither considered nor skipped.
        assert_eq!(rep.skipped, 0);
        assert_eq!(lib.count().unwrap(), 1);
    }

    #[test]
    fn unreadable_file_keeps_its_row_and_counts_as_skipped() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        lib.scan(root).unwrap();
        let before = only_row(&lib);

        // Corrupt the file: the stat changes, the tag read fails.
        fs::write(&a, b"this is not a media file at all").unwrap();
        let rep = lib.scan(root).unwrap();
        assert_eq!(rep.skipped, 1);
        assert_eq!(rep.updated, 0);
        assert_eq!(rep.removed, 0);
        assert_eq!(lib.count().unwrap(), 1, "the file still exists");

        let after = only_row(&lib);
        assert_eq!(before.id, after.id);
        assert_eq!(after.metadata.title.as_deref(), Some("A"));
    }

    #[test]
    fn scan_outside_root_does_not_touch_other_roots() {
        let dir = TempDir::new().unwrap();
        let root_a = dir.path().join("a");
        let root_b = dir.path().join("b");
        fs::create_dir(&root_a).unwrap();
        fs::create_dir(&root_b).unwrap();
        let a = root_a.join("a.wav");
        let b = root_b.join("b.wav");
        make_wav(&a);
        make_wav(&b);
        write_fixture_tags(&a, "A", "X", "Al");
        write_fixture_tags(&b, "B", "Y", "Al2");

        let mut lib = Library::in_memory().unwrap();
        lib.scan(&root_a).unwrap();
        lib.scan(&root_b).unwrap();
        assert_eq!(lib.count().unwrap(), 2);

        fs::remove_file(&b).unwrap();
        let rep = lib.scan(&root_b).unwrap();
        assert_eq!(rep.removed, 1);
        assert_eq!(lib.count().unwrap(), 1);
        let rows = matches(
            &lib,
            &Query {
                path_prefix: Some(root_a.clone()),
                ..Default::default()
            },
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].metadata.title.as_deref(), Some("A"));
    }

    #[test]
    fn like_wildcards_in_paths_are_literal() {
        let dir = TempDir::new().unwrap();
        // Both `%` and `_` are LIKE wildcards and must be escaped.
        let root = dir.path().join("100%_music");
        fs::create_dir(&root).unwrap();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        assert_eq!(lib.scan(&root).unwrap().added, 1);
        // Idempotent: a broken escape would see a changed pattern and re-add.
        assert_eq!(lib.scan(&root).unwrap().updated, 0);

        let rows = matches(
            &lib,
            &Query {
                path_prefix: Some(root.clone()),
                ..Default::default()
            },
        );
        assert_eq!(rows.len(), 1);

        fs::remove_file(&a).unwrap();
        assert_eq!(lib.scan(&root).unwrap().removed, 1);
        assert_eq!(lib.count().unwrap(), 0);
    }

    #[test]
    fn scanning_a_missing_root_is_an_error() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        let mut lib = Library::in_memory().unwrap();
        assert!(matches!(lib.scan(&missing), Err(Error::NotFound(_))));
        assert_eq!(lib.count().unwrap(), 0);
    }

    /// Index one WAV per `(file name, title, artist, album)` entry. The
    /// `TempDir` is returned so the caller keeps the files alive.
    fn library_with(entries: &[(&str, &str, &str, &str)]) -> (TempDir, Library) {
        let dir = TempDir::new().unwrap();
        for (name, title, artist, album) in entries {
            let path = dir.path().join(name);
            make_wav(&path);
            write_fixture_tags(&path, title, artist, album);
        }
        let mut lib = Library::in_memory().unwrap();
        lib.scan(dir.path()).unwrap();
        (dir, lib)
    }

    #[test]
    fn roots_are_added_listed_and_idempotent() {
        let mut lib = Library::in_memory().unwrap();
        assert!(lib.roots().unwrap().is_empty());

        let first = lib.add_root(Path::new("C:/music")).unwrap();
        let second = lib.add_root(Path::new("D:/more")).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(first.path, PathBuf::from("C:/music"));

        // Adding the same spelling again returns the existing row.
        let again = lib.add_root(Path::new("C:/music")).unwrap();
        assert_eq!(again, first);

        let roots = lib.roots().unwrap();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0], first);
        assert_eq!(roots[1], second);
    }

    #[test]
    fn scan_root_scans_the_registered_directory() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("music");
        fs::create_dir(&root).unwrap();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        let entry = lib.add_root(&root).unwrap();
        let rep = lib.scan_root(entry.id).unwrap();
        assert_eq!(rep.added, 1);
        assert_eq!(lib.count().unwrap(), 1);

        // An unknown root id is an error, not an empty report.
        assert!(matches!(
            lib.scan_root(entry.id + 1),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn removing_a_root_drops_its_tracks_and_extras() {
        let dir = TempDir::new().unwrap();
        let root_a = dir.path().join("a");
        let root_b = dir.path().join("b");
        fs::create_dir(&root_a).unwrap();
        fs::create_dir(&root_b).unwrap();
        for (root, title) in [(&root_a, "A"), (&root_b, "B")] {
            let path = root.join("track.wav");
            make_wav(&path);
            let mut md = TrackMetadata {
                title: Some(title.to_owned()),
                ..Default::default()
            };
            // A key the WAV writer round-trips (see `extras_round_trip`).
            md.extra
                .insert("REPLAYGAIN_TRACK_GAIN".to_owned(), "1.50 dB".to_owned());
            rustamp_meta::write_tags(&path, &md).unwrap();
        }

        let mut lib = Library::in_memory().unwrap();
        let entry_a = lib.add_root(&root_a).unwrap();
        let entry_b = lib.add_root(&root_b).unwrap();
        lib.scan_root(entry_a.id).unwrap();
        lib.scan_root(entry_b.id).unwrap();
        assert_eq!(lib.count().unwrap(), 2);
        assert_eq!(extra_rows(&lib), 2);

        assert_eq!(lib.remove_root(&root_a).unwrap(), 1, "one track dropped");
        // The root that is left is the one that was registered: its id and
        // its path are the ones `add_root` handed out.
        let listed = lib.roots().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, entry_b.id);
        assert_eq!(listed[0].path, entry_b.path);
        assert_eq!(lib.count().unwrap(), 1);
        let rows = matches(&lib, &Query::default());
        assert_eq!(rows[0].metadata.title.as_deref(), Some("B"));
        assert_eq!(extra_rows(&lib), 1, "the extra rows cascade with the track");

        // A root that is listed and scanned drops its track as well.
        assert_eq!(lib.remove_root(&root_b).unwrap(), 1);
        assert_eq!(extra_rows(&lib), 0);
        assert!(lib.roots().unwrap().is_empty());
    }

    #[test]
    fn removing_a_listed_but_unscanned_root_drops_nothing() {
        let mut lib = Library::in_memory().unwrap();
        lib.add_root(Path::new("C:/empty")).unwrap();
        assert_eq!(lib.remove_root(Path::new("C:/empty")).unwrap(), 0);
        assert!(lib.roots().unwrap().is_empty());
    }

    #[test]
    fn removing_an_unlisted_root_is_an_error() {
        let mut lib = Library::in_memory().unwrap();
        assert!(matches!(
            lib.remove_root(Path::new("C:/never-added")),
            Err(Error::NotFound(_))
        ));
    }

    /// Number of `track_extra` rows, to check the delete cascade.
    fn extra_rows(lib: &Library) -> i64 {
        lib.conn
            .query_row("SELECT COUNT(*) FROM track_extra", [], |row| row.get(0))
            .unwrap()
    }

    // ---- the last scan of a root ----

    /// The column names of the `roots` table, as the database sees them.
    fn root_columns(lib: &Library) -> Vec<String> {
        let mut stmt = lib
            .conn
            .prepare("SELECT name FROM pragma_table_info('roots')")
            .unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.map(|row| row.unwrap()).collect()
    }

    /// Write a database with the schema of an older version: the `roots`
    /// table without the last-scan column, and one row in it.
    fn write_old_database(path: &Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE roots (
                 id       INTEGER PRIMARY KEY AUTOINCREMENT,
                 path     TEXT NOT NULL UNIQUE,
                 added_at INTEGER NOT NULL
             );
             INSERT INTO roots (path, added_at) VALUES ('C:/old-music', 1000);",
        )
        .unwrap();
    }

    #[test]
    fn a_fresh_database_has_the_last_scan_column() {
        let lib = Library::in_memory().unwrap();
        assert!(
            root_columns(&lib)
                .iter()
                .any(|name| name == "last_scanned_at"),
            "the roots table has the last-scan column"
        );
    }

    #[test]
    fn an_old_database_migrates_and_keeps_its_rows() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("old.db");
        write_old_database(&db);

        let lib = Library::open(&db).unwrap();

        assert!(
            root_columns(&lib)
                .iter()
                .any(|name| name == "last_scanned_at"),
            "the migration added the column"
        );

        let roots = lib.roots().unwrap();
        assert_eq!(roots.len(), 1, "the row of the old database is still there");
        assert_eq!(roots[0].path, PathBuf::from("C:/old-music"));
        assert_eq!(roots[0].added_at, 1000);
        assert_eq!(
            roots[0].last_scanned_at, None,
            "a root that was never scanned has no time"
        );
    }

    #[test]
    fn the_migration_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("old.db");
        write_old_database(&db);

        // The first open migrates, the second one finds the column and
        // leaves the table alone.
        let lib = Library::open(&db).unwrap();
        drop(lib);
        let lib = Library::open(&db).unwrap();
        assert_eq!(lib.roots().unwrap().len(), 1);
        assert!(
            root_columns(&lib)
                .iter()
                .any(|name| name == "last_scanned_at"),
            "the column is still there"
        );
    }

    #[test]
    fn a_scan_records_the_time_of_the_root() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("music");
        fs::create_dir(&root).unwrap();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        let entry = lib.add_root(&root).unwrap();
        assert_eq!(
            last_scanned_at(&lib, entry.id),
            None,
            "a root that was never scanned has no time"
        );

        let before = now_unix();
        lib.scan_root(entry.id).unwrap();
        let after = now_unix();

        let at = last_scanned_at(&lib, entry.id).expect("the scan recorded its time");
        assert!(
            (before..=after).contains(&at),
            "the time is the second the scan ran in"
        );
    }

    #[test]
    fn a_cancelled_scan_records_no_time() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("music");
        fs::create_dir(&root).unwrap();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        let entry = lib.add_root(&root).unwrap();

        let cancel = Cancel::new();
        cancel.cancel();
        let report = lib
            .scan_root_with_progress(entry.id, &cancel, |_| {})
            .unwrap();
        assert!(report.cancelled);
        assert_eq!(
            last_scanned_at(&lib, entry.id),
            None,
            "a cancelled scan did not finish and claims no time"
        );
    }

    #[test]
    fn a_cancelled_scan_keeps_the_time_of_the_last_finished_scan() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("music");
        fs::create_dir(&root).unwrap();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        let entry = lib.add_root(&root).unwrap();
        lib.scan_root(entry.id).unwrap();
        put_last_scanned_at(&lib, entry.id, 4242);

        let cancel = Cancel::new();
        cancel.cancel();
        lib.scan_root_with_progress(entry.id, &cancel, |_| {})
            .unwrap();

        assert_eq!(
            last_scanned_at(&lib, entry.id),
            Some(4242),
            "the time of the last finished scan is kept"
        );
    }

    #[test]
    fn a_failed_scan_records_no_time() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("music");
        fs::create_dir(&root).unwrap();

        let mut lib = Library::in_memory().unwrap();
        let entry = lib.add_root(&root).unwrap();
        put_last_scanned_at(&lib, entry.id, 4242);

        // The directory is gone: the scan fails.
        fs::remove_dir(&root).unwrap();
        assert!(matches!(lib.scan_root(entry.id), Err(Error::NotFound(_))));

        assert_eq!(
            last_scanned_at(&lib, entry.id),
            Some(4242),
            "a failed scan records no time"
        );
    }

    #[test]
    fn a_scan_of_an_unregistered_path_records_no_time() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("music");
        fs::create_dir(&root).unwrap();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        let entry = lib.add_root(&root).unwrap();

        // The plain scan works on an arbitrary path: it does not know which
        // root row it belongs to, so it records no time.
        lib.scan(&root).unwrap();
        assert_eq!(last_scanned_at(&lib, entry.id), None);

        // The registered scan does record it.
        lib.scan_root(entry.id).unwrap();
        assert!(last_scanned_at(&lib, entry.id).is_some());
    }

    #[test]
    fn the_time_of_a_root_is_kept_when_the_root_is_removed_and_added_again() {
        let mut lib = Library::in_memory().unwrap();
        let root = Path::new("C:/music");
        let entry = lib.add_root(root).unwrap();
        put_last_scanned_at(&lib, entry.id, 4242);
        lib.remove_root(root).unwrap();

        // A root that is added again is a new row: it was never scanned.
        let again = lib.add_root(root).unwrap();
        assert_eq!(again.last_scanned_at, None);
        assert_eq!(last_scanned_at(&lib, again.id), None);
    }

    #[test]
    fn freetext_searches_title_artist_and_album_case_insensitively() {
        let (_dir, lib) = library_with(&[
            ("a.wav", "Sonne", "Rammstein", "Mutter"),
            ("b.wav", "Mein Herz Brennt", "Rammstein", "Mutter"),
            ("c.wav", "Wunder", "Ärzte", "Jazz ist anders"),
            ("d.wav", "Jazz Club", "Someone", "Other"),
        ]);

        // Title, lowercase query against a capitalised title.
        assert_eq!(
            titles(&lib, &text_query("sonne")),
            vec![Some("Sonne".to_owned())]
        );
        // Artist, twice.
        assert_eq!(titles(&lib, &text_query("ramm")).len(), 2, "both Rammstein");
        // Album.
        assert_eq!(titles(&lib, &text_query("mutter")).len(), 2);
        // A term in either of two columns.
        assert_eq!(titles(&lib, &text_query("jazz")).len(), 2);
        // Unicode case folding: `Ärzte` matches `ärzte` and `ÄRZTE`.
        assert_eq!(
            titles(&lib, &text_query("ärzte")),
            vec![Some("Wunder".to_owned())]
        );
        assert_eq!(titles(&lib, &text_query("ÄRZTE")).len(), 1);
        // Whitespace-only text is no filter at all.
        assert_eq!(titles(&lib, &text_query("   ")).len(), 4);
        // No match: empty rows and a zero total.
        let page = lib.query(&text_query("nothing here")).unwrap();
        assert!(page.rows.is_empty());
        assert_eq!(page.total, 0);

        // Free text and a field filter combine with AND.
        let filter = Query {
            artist: Some("Rammstein".to_owned()),
            ..text_query("mutter")
        };
        assert_eq!(titles(&lib, &filter).len(), 2);
        let filter = Query {
            artist: Some("Someone".to_owned()),
            ..text_query("mutter")
        };
        assert!(titles(&lib, &filter).is_empty());
    }

    #[test]
    fn freetext_treats_like_wildcards_literally() {
        let (_dir, lib) = library_with(&[
            ("a.wav", "100% Hits", "X", "Al"),
            ("b.wav", "100 Hits", "X", "Al"),
            ("c.wav", "a_b", "Y", "Al"),
            ("d.wav", "axb", "Y", "Al"),
        ]);

        assert_eq!(
            titles(&lib, &text_query("100%")),
            vec![Some("100% Hits".to_owned())]
        );
        assert_eq!(
            titles(&lib, &text_query("a_b")),
            vec![Some("a_b".to_owned())]
        );
        // A lone `%` searches for a literal `%` instead of matching anything.
        assert_eq!(
            titles(&lib, &text_query("%")),
            vec![Some("100% Hits".to_owned())]
        );
    }

    #[test]
    fn sort_orders_by_column_case_insensitively_with_nulls_last() {
        let (dir, lib) = library_with(&[
            ("a.wav", "banana", "zoe", "Zebra"),
            ("b.wav", "Apple", "Ada", "apple"),
            ("c.wav", "cherry", "mike", "Mango"),
        ]);

        let by_title = Query {
            sort: SortKey::Title,
            ..Default::default()
        };
        assert_eq!(
            titles(&lib, &by_title),
            vec![
                Some("Apple".to_owned()),
                Some("banana".to_owned()),
                Some("cherry".to_owned())
            ]
        );

        let artists = |query: &Query| -> Vec<Option<String>> {
            matches(&lib, query)
                .into_iter()
                .map(|row| row.metadata.artist)
                .collect()
        };
        let by_artist = Query {
            sort: SortKey::Artist,
            ..Default::default()
        };
        assert_eq!(
            artists(&by_artist),
            vec![
                Some("Ada".to_owned()),
                Some("mike".to_owned()),
                Some("zoe".to_owned())
            ]
        );

        let albums = |query: &Query| -> Vec<Option<String>> {
            matches(&lib, query)
                .into_iter()
                .map(|row| row.metadata.album)
                .collect()
        };
        let by_album = Query {
            sort: SortKey::Album,
            ..Default::default()
        };
        assert_eq!(
            albums(&by_album),
            vec![
                Some("apple".to_owned()),
                Some("Mango".to_owned()),
                Some("Zebra".to_owned())
            ]
        );

        // Durations come out of tags, so set them directly: `a` has none, `b`
        // is the shortest. `NULL` sorts last even though it is the first row
        // by insert order.
        set_duration(&lib, &dir.path().join("b.wav"), 1000);
        set_duration(&lib, &dir.path().join("c.wav"), 5000);
        let by_duration = Query {
            sort: SortKey::Duration,
            ..Default::default()
        };
        let ids: Vec<i64> = matches(&lib, &by_duration)
            .into_iter()
            .map(|r| r.id)
            .collect();
        let full = matches(&lib, &by_title);
        let id_of = |title: &str| -> i64 {
            full.iter()
                .find(|row| row.metadata.title.as_deref() == Some(title))
                .unwrap()
                .id
        };
        assert_eq!(ids, vec![id_of("Apple"), id_of("cherry"), id_of("banana")]);

        // Descending reverses the durations but keeps the `NULL` last.
        let by_duration_desc = Query {
            descending: true,
            ..by_duration.clone()
        };
        let ids: Vec<i64> = matches(&lib, &by_duration_desc)
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec![id_of("cherry"), id_of("Apple"), id_of("banana")]);
    }

    #[test]
    fn sort_descending_reverses_order_with_stable_ties() {
        let (_dir, lib) = library_with(&[
            ("a.wav", "one", "X", "Al"),
            ("b.wav", "two", "X", "Al"),
            ("c.wav", "three", "Y", "Al"),
        ]);

        // Insert order, for the row ids.
        let ascending = matches(&lib, &Query::default());
        assert_eq!(ascending.len(), 3);
        let (id_a, id_b, id_c) = (ascending[0].id, ascending[1].id, ascending[2].id);
        assert_eq!(ascending[0].metadata.title.as_deref(), Some("one"));

        let by_artist_desc = Query {
            sort: SortKey::Artist,
            descending: true,
            ..Default::default()
        };
        let ids: Vec<i64> = matches(&lib, &by_artist_desc)
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(
            ids,
            vec![id_c, id_a, id_b],
            "Y first, X ties by ascending id"
        );

        let by_title_desc = Query {
            sort: SortKey::Title,
            descending: true,
            ..Default::default()
        };
        assert_eq!(
            titles(&lib, &by_title_desc),
            vec![
                Some("two".to_owned()),
                Some("three".to_owned()),
                Some("one".to_owned())
            ],
            "descending undoes the ascending title order"
        );
    }

    #[test]
    fn pagination_reports_the_unpaged_total() {
        let (_dir, lib) = library_with(&[
            ("a.wav", "T1", "X", "Al"),
            ("b.wav", "T2", "X", "Al"),
            ("c.wav", "T3", "X", "Al"),
            ("d.wav", "T4", "X", "Al"),
            ("e.wav", "T5", "X", "Al"),
        ]);
        let sorted = |limit: Option<u32>, offset: u32| -> Query {
            Query {
                sort: SortKey::Title,
                limit,
                offset,
                ..Default::default()
            }
        };

        for (limit, offset, expected) in [
            (Some(2), 0, vec!["T1", "T2"]),
            (Some(2), 2, vec!["T3", "T4"]),
            (Some(2), 4, vec!["T5"]),
            (Some(2), 5, vec![]),
            (None, 3, vec!["T4", "T5"]),
            (None, 5, vec![]),
            (Some(0), 0, vec![]),
            (Some(10), 0, vec!["T1", "T2", "T3", "T4", "T5"]),
        ] {
            let page = lib.query(&sorted(limit, offset)).unwrap();
            assert_eq!(
                page.rows
                    .iter()
                    .map(|row| row.metadata.title.clone().unwrap())
                    .collect::<Vec<_>>(),
                expected,
                "limit {limit:?} offset {offset}"
            );
            assert_eq!(page.total, 5, "limit {limit:?} offset {offset}");
        }

        // The total follows the filter, not the window.
        let filter = Query {
            text: Some("T3".to_owned()),
            limit: Some(1),
            ..Default::default()
        };
        let page = lib.query(&filter).unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.total, 1);
    }

    /// A [`Query`] with only the free-text term set.
    fn text_query(text: &str) -> Query {
        Query {
            text: Some(text.to_owned()),
            ..Default::default()
        }
    }

    /// `TrackMetadata` cannot carry `duration_ms` into a WAV, so set it
    /// straight in the index for the duration sort test.
    fn set_duration(lib: &Library, path: &Path, duration_ms: i64) {
        let changed = lib
            .conn
            .execute(
                "UPDATE tracks SET duration_ms = ?1 WHERE path = ?2",
                rusqlite::params![duration_ms, path.to_string_lossy()],
            )
            .unwrap();
        assert_eq!(changed, 1);
    }

    // ---- tag writing ----

    /// A library with one indexed WAV file, and the row it holds.
    fn library_with_one_track() -> (TempDir, Library, PathBuf, TrackRow) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.wav");
        make_wav(&path);
        write_fixture_tags(&path, "Song A", "Artist X", "Album 1");
        let mut lib = Library::in_memory().unwrap();
        lib.scan(dir.path()).unwrap();
        let row = only_row(&lib);
        (dir, lib, path, row)
    }

    #[test]
    fn writing_tags_updates_the_row_without_a_rescan() {
        let (_dir, mut lib, path, before) = library_with_one_track();

        let mut edited = before.metadata.clone();
        edited.title = Some("Song A2".to_owned());
        let after = lib.write_tags(before.id, &edited).unwrap();

        // The row carries the new value, and the id is the one it had.
        assert_eq!(after.id, before.id, "stable id preserved");
        assert_eq!(after.metadata.title.as_deref(), Some("Song A2"));
        assert_eq!(after.metadata.artist.as_deref(), Some("Artist X"));

        // The stored row carries it too, not just the returned one.
        let stored = lib.get(before.id).unwrap().unwrap();
        assert_eq!(stored.metadata.title.as_deref(), Some("Song A2"));

        // The file really was written: it is the file that says so.
        let on_disk = rustamp_meta::read_tags(&path).unwrap();
        assert_eq!(on_disk.title.as_deref(), Some("Song A2"));
        assert_eq!(after.metadata, on_disk, "the row is what the file says");

        // The stat was refreshed with the write, so the next scan finds no
        // change and does not read the file a second time.
        let report = lib.scan(path.parent().unwrap()).unwrap();
        assert_eq!(report.added, 0);
        assert_eq!(report.updated, 0);
        assert_eq!(report.removed, 0);
        assert_eq!(report.skipped, 0);
        assert_eq!(only_row(&lib).metadata.title.as_deref(), Some("Song A2"));
    }

    #[test]
    fn the_row_follows_the_file_and_not_the_caller() {
        let (_dir, mut lib, path, before) = library_with_one_track();

        let mut edited = before.metadata.clone();
        edited.title = Some("Song A2".to_owned());
        // A field no tag container carries: the writer drops it and the
        // reader never reports it.
        edited.duration_ms = Some(1234);
        let after = lib.write_tags(before.id, &edited).unwrap();

        assert_eq!(after.metadata.title.as_deref(), Some("Song A2"));
        assert_eq!(
            after.metadata.duration_ms, None,
            "the file carries no duration, so the row must not claim one"
        );
        assert_eq!(after.metadata, rustamp_meta::read_tags(&path).unwrap());
    }

    #[test]
    fn a_partial_write_keeps_the_tags_the_caller_did_not_set() {
        let (_dir, mut lib, _path, before) = library_with_one_track();

        // Only the title is set: the writer leaves the other tags as the
        // file has them, and the read-back shows exactly that.
        let edited = TrackMetadata {
            title: Some("Song A2".to_owned()),
            ..Default::default()
        };
        let after = lib.write_tags(before.id, &edited).unwrap();

        assert_eq!(after.metadata.title.as_deref(), Some("Song A2"));
        assert_eq!(after.metadata.artist.as_deref(), Some("Artist X"));
        assert_eq!(after.metadata.album.as_deref(), Some("Album 1"));
    }

    #[test]
    fn a_failed_write_leaves_the_row_unchanged() {
        let (_dir, mut lib, path, before) = library_with_one_track();

        // A year no tag can hold: the write is refused before the file is
        // opened, so neither the file nor the row changes.
        let mut edited = before.metadata.clone();
        edited.title = Some("Song A2".to_owned());
        edited.year = Some(70_000);
        let error = lib.write_tags(before.id, &edited).unwrap_err();
        assert!(matches!(error, Error::InvalidArgument(_)), "{error:?}");

        let after = only_row(&lib);
        assert_eq!(after, before, "the row is untouched");
        let on_disk = rustamp_meta::read_tags(&path).unwrap();
        assert_eq!(on_disk.title.as_deref(), Some("Song A"));
    }

    #[test]
    fn a_write_to_a_file_that_is_no_media_leaves_the_row_unchanged() {
        let (_dir, mut lib, path, before) = library_with_one_track();

        // The file is broken behind the library's back: the write fails on
        // the file, and the row keeps the values it had.
        fs::write(&path, b"this is not a media file at all").unwrap();
        let mut edited = before.metadata.clone();
        edited.title = Some("Song A2".to_owned());
        let error = lib.write_tags(before.id, &edited).unwrap_err();
        assert!(matches!(error, Error::UnsupportedFormat(_)), "{error:?}");

        let after = only_row(&lib);
        assert_eq!(after.id, before.id);
        assert_eq!(after.metadata.title.as_deref(), Some("Song A"));
    }

    #[test]
    fn a_vanished_file_is_reported_and_leaves_the_row_alone() {
        let (_dir, mut lib, path, before) = library_with_one_track();

        // The file is gone between the scan and the write.
        fs::remove_file(&path).unwrap();

        let mut edited = before.metadata.clone();
        edited.title = Some("Song A2".to_owned());
        let error = lib.write_tags(before.id, &edited).unwrap_err();
        assert!(matches!(error, Error::NotFound(_)), "{error:?}");

        // The row is not updated with values no file has.
        let after = only_row(&lib);
        assert_eq!(after.id, before.id);
        assert_eq!(after.metadata.title.as_deref(), Some("Song A"));
    }

    #[test]
    fn writing_to_an_unknown_track_is_an_error() {
        let mut lib = Library::in_memory().unwrap();
        let error = lib.write_tags(42, &TrackMetadata::default()).unwrap_err();
        assert!(matches!(error, Error::NotFound(_)), "{error:?}");
        assert_eq!(lib.count().unwrap(), 0, "no row was created");
    }

    #[test]
    fn a_write_keeps_the_extra_tags_of_the_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.wav");
        make_wav(&path);
        let mut md = TrackMetadata {
            title: Some("Song A".to_owned()),
            ..Default::default()
        };
        md.extra
            .insert("REPLAYGAIN_TRACK_GAIN".to_owned(), "1.50 dB".to_owned());
        rustamp_meta::write_tags(&path, &md).unwrap();

        let mut lib = Library::in_memory().unwrap();
        lib.scan(dir.path()).unwrap();
        let before = only_row(&lib);

        let mut edited = before.metadata.clone();
        edited.title = Some("Song A2".to_owned());
        let after = lib.write_tags(before.id, &edited).unwrap();

        assert_eq!(after.metadata.title.as_deref(), Some("Song A2"));
        assert_eq!(
            after
                .metadata
                .extra
                .get("REPLAYGAIN_TRACK_GAIN")
                .map(String::as_str),
            Some("1.50 dB"),
            "the extra tag survived the write"
        );
    }

    // ---- tag removal ----

    #[test]
    fn a_removal_clears_the_tag_and_the_row_follows_the_file() {
        let (_dir, mut lib, path, before) = library_with_one_track();

        // Nothing is written, one field is removed.
        let write = lib
            .write_tags_with(before.id, &TrackMetadata::default(), &[TagField::Title])
            .unwrap();

        // The row is what the file says: the title is gone.
        assert_eq!(write.row.metadata.title, None);
        assert_eq!(rustamp_meta::read_tags(&path).unwrap().title, None);

        // The fields the removal did not name keep their value: a `None`
        // still means "leave the value the file has".
        assert_eq!(write.row.metadata.artist.as_deref(), Some("Artist X"));
        assert_eq!(write.row.metadata.album.as_deref(), Some("Album 1"));
        assert_eq!(write.row.metadata.genre.as_deref(), Some("Rock"));

        // The id is the one the row had, and the stored row is the one the
        // write returned.
        assert_eq!(write.row.id, before.id, "stable id preserved");
        assert_eq!(lib.get(before.id).unwrap().unwrap(), write.row);

        // The format had nothing to refuse.
        assert!(write.report.unremoved.is_empty());
        assert!(write.report.is_complete());
    }

    #[test]
    fn a_value_in_a_second_tag_is_reported_as_not_removed() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.wav");
        // The `INFO` chunk is a second tag: the writer writes the ID3v2
        // chunk and leaves the `INFO` chunk alone.
        make_wav_with_info(&path, "Info Title", "Info Artist");
        write_fixture_tags(&path, "Id3 Title", "Artist X", "Album 1");

        let mut lib = Library::in_memory().unwrap();
        lib.scan(dir.path()).unwrap();
        let before = only_row(&lib);
        assert_eq!(before.metadata.title.as_deref(), Some("Id3 Title"));

        let write = lib
            .write_tags_with(before.id, &TrackMetadata::default(), &[TagField::Title])
            .unwrap();

        // The title of the ID3v2 tag is gone, but the file still carries the
        // value in the tag the writer does not touch: the report says so.
        assert_eq!(write.report.unremoved, vec![TagField::Title]);
        assert!(!write.report.is_complete());

        // The row follows the file, so it still shows the value the file
        // carries.
        assert_eq!(write.row.metadata.title.as_deref(), Some("Info Title"));
        assert_eq!(
            rustamp_meta::read_tags(&path).unwrap().title.as_deref(),
            Some("Info Title")
        );
    }

    #[test]
    fn a_failed_removal_leaves_the_row_unchanged() {
        let (_dir, mut lib, path, before) = library_with_one_track();

        // A year no tag can hold: the write is refused before the file is
        // opened, so neither the file nor the row changes.
        let mut edited = before.metadata.clone();
        edited.title = Some("Song A2".to_owned());
        edited.year = Some(70_000);
        let error = lib
            .write_tags_with(before.id, &edited, &[TagField::Artist])
            .unwrap_err();
        assert!(matches!(error, Error::InvalidArgument(_)), "{error:?}");

        let after = only_row(&lib);
        assert_eq!(after, before, "the row is untouched");
        let on_disk = rustamp_meta::read_tags(&path).unwrap();
        assert_eq!(on_disk.title.as_deref(), Some("Song A"));
        assert_eq!(on_disk.artist.as_deref(), Some("Artist X"));
    }

    #[test]
    fn a_removal_to_a_vanished_file_is_reported() {
        let (_dir, mut lib, path, before) = library_with_one_track();

        // The file is gone between the scan and the write.
        fs::remove_file(&path).unwrap();

        let error = lib
            .write_tags_with(before.id, &TrackMetadata::default(), &[TagField::Title])
            .unwrap_err();
        assert!(matches!(error, Error::NotFound(_)), "{error:?}");

        // The row is not updated with values no file has.
        let after = only_row(&lib);
        assert_eq!(after.id, before.id);
        assert_eq!(after.metadata.title.as_deref(), Some("Song A"));
    }

    // ---- scan progress and cancellation ----

    #[test]
    fn progress_events_arrive_in_order() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        for name in ["a.wav", "b.wav", "c.wav"] {
            let path = root.join(name);
            make_wav(&path);
            write_fixture_tags(&path, name, "X", "Al");
        }
        // A file that is not media is ignored: it produces no event.
        fs::write(root.join("notes.txt"), b"liner notes").unwrap();

        let mut lib = Library::in_memory().unwrap();
        let cancel = Cancel::new();
        let mut events: Vec<ScanProgress> = Vec::new();
        let report = lib
            .scan_with_progress(root, &cancel, |event| events.push(event))
            .unwrap();

        // The phases arrive in order: the scan, the removal pass, the end.
        let phases: Vec<ScanPhase> = events.iter().map(|event| event.phase).collect();
        assert_eq!(
            phases,
            vec![
                ScanPhase::Scanning,
                ScanPhase::Scanning,
                ScanPhase::Scanning,
                ScanPhase::Scanning,
                ScanPhase::Removing,
                ScanPhase::Finished,
            ]
        );

        // The file events carry the file and a rising counter.
        let file_events: Vec<&ScanProgress> =
            events.iter().filter(|event| event.path.is_some()).collect();
        assert_eq!(file_events.len(), 3);
        for (i, event) in file_events.iter().enumerate() {
            assert_eq!(event.seen, i + 1, "the counter rises with every file");
            assert_eq!(event.added, i + 1);
            assert_eq!(event.updated, 0);
            assert_eq!(event.skipped, 0);
        }
        let mut seen: Vec<String> = file_events
            .iter()
            .map(|event| event.path.as_ref().unwrap().display().to_string())
            .collect();
        seen.sort();
        let mut expected: Vec<String> = ["a.wav", "b.wav", "c.wav"]
            .iter()
            .map(|name| root.join(name).display().to_string())
            .collect();
        expected.sort();
        assert_eq!(seen, expected);

        // The last event carries the summary of the scan.
        let last = events.last().unwrap();
        assert_eq!(last.phase, ScanPhase::Finished);
        assert_eq!(last.seen, 3);
        assert_eq!(last.added, 3);
        assert_eq!(last.removed, 0);
        assert_eq!(report.added, 3);
        assert_eq!(report.scanned, 3);
        assert_eq!(report.removed, 0);
        assert!(!report.cancelled);
    }

    #[test]
    fn a_file_that_cannot_be_read_is_reported_as_skipped() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let good = root.join("good.wav");
        let broken = root.join("broken.wav");
        make_wav(&good);
        write_fixture_tags(&good, "A", "X", "Al");
        // The file is not media: the tag read fails.
        fs::write(&broken, b"this is not a media file at all").unwrap();

        let mut lib = Library::in_memory().unwrap();
        let cancel = Cancel::new();
        let mut events: Vec<ScanProgress> = Vec::new();
        let report = lib
            .scan_with_progress(root, &cancel, |event| events.push(event))
            .unwrap();

        assert_eq!(report.added, 1);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.scanned, 1);
        // The skipped file is reported with its path.
        let skipped: Vec<&ScanProgress> = events
            .iter()
            .filter(|event| event.path.as_deref() == Some(broken.as_path()))
            .collect();
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].skipped, 1);
    }

    #[test]
    fn a_cancelled_scan_stops_early_and_keeps_the_committed_rows() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // More files than one batch holds, so the scan has to commit a
        // batch before the cancel can stop it.
        for i in 0..(SCAN_BATCH_SIZE + 6) {
            make_wav(&root.join(format!("t{i:03}.wav")));
        }

        let mut lib = Library::in_memory().unwrap();
        let cancel = Cancel::new();
        let for_the_scan = cancel.clone();
        let report = lib
            .scan_with_progress(root, &for_the_scan, |event| {
                if event.path.is_some() && event.seen == SCAN_BATCH_SIZE + 3 {
                    for_the_scan.cancel();
                }
            })
            .unwrap();

        assert!(report.cancelled, "the scan was stopped");
        assert!(cancel.is_cancelled());
        // The first batch was committed as a whole, the batch in flight was
        // dropped: the index never holds half a batch.
        assert_eq!(lib.count().unwrap(), SCAN_BATCH_SIZE);
        assert_eq!(report.added, SCAN_BATCH_SIZE);
        assert_eq!(report.scanned, SCAN_BATCH_SIZE);
        assert_eq!(report.removed, 0);
    }

    #[test]
    fn a_cancel_before_the_delete_pass_leaves_the_index_untouched() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("a.wav");
        let b = root.join("b.wav");
        make_wav(&a);
        make_wav(&b);
        write_fixture_tags(&a, "A", "X", "Al");
        write_fixture_tags(&b, "B", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        lib.scan(root).unwrap();
        assert_eq!(lib.count().unwrap(), 2);

        // One file is gone, so a scan that reaches the removal pass would
        // drop its row. The other one changes, so a scan that reaches the
        // index would rewrite its row.
        fs::remove_file(&b).unwrap();
        write_fixture_tags(&a, "A2", "X", "Al");

        let cancel = Cancel::new();
        let for_the_scan = cancel.clone();
        let report = lib
            .scan_with_progress(root, &for_the_scan, |event| {
                if event.path.is_some() {
                    for_the_scan.cancel();
                }
            })
            .unwrap();

        assert!(report.cancelled);
        assert_eq!(report.removed, 0, "the removal pass did not run");
        assert_eq!(lib.count().unwrap(), 2, "the stale row is still there");
        let rows = matches(&lib, &Query::default());
        let a_row = rows.iter().find(|row| row.path == a).unwrap();
        assert_eq!(
            a_row.metadata.title.as_deref(),
            Some("A"),
            "the batch in flight was dropped"
        );
    }

    #[test]
    fn a_normal_scan_still_behaves_as_before() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        for (name, title) in [("a.wav", "A"), ("b.wav", "B"), ("c.wav", "C")] {
            let path = root.join(name);
            make_wav(&path);
            write_fixture_tags(&path, title, "X", "Al");
        }
        // A file that is not media is ignored.
        fs::write(root.join("cover.jpg"), b"not an image").unwrap();

        // The plain scan.
        let mut plain = Library::in_memory().unwrap();
        let plain_report = plain.scan(root).unwrap();

        // The same scan, with progress.
        let mut with_progress = Library::in_memory().unwrap();
        let cancel = Cancel::new();
        let report = with_progress
            .scan_with_progress(root, &cancel, |_| {})
            .unwrap();

        assert_eq!(plain_report, report);
        assert!(!report.cancelled);
        assert_eq!(report.scanned, 3);
        assert_eq!(report.added, 3);
        assert_eq!(report.updated, 0);
        assert_eq!(report.removed, 0);
        assert_eq!(report.skipped, 0);
        assert_eq!(with_progress.count().unwrap(), 3);
    }

    #[test]
    fn a_scan_cancelled_before_it_starts_does_nothing() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        let cancel = Cancel::new();
        cancel.cancel();
        let report = lib.scan_with_progress(root, &cancel, |_| {}).unwrap();

        assert!(report.cancelled);
        assert_eq!(report.scanned, 0);
        assert_eq!(report.added, 0);
        assert_eq!(lib.count().unwrap(), 0);
    }

    // ---- removing dead entries ----

    #[test]
    fn remove_missing_drops_the_row_whose_file_is_gone() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("music");
        fs::create_dir(&root).unwrap();
        let a = root.join("a.wav");
        let b = root.join("b.wav");
        make_wav(&a);
        make_wav(&b);
        write_fixture_tags(&a, "A", "X", "Al");
        write_fixture_tags(&b, "B", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        let entry = lib.add_root(&root).unwrap();
        lib.scan_root(entry.id).unwrap();
        assert_eq!(lib.count().unwrap(), 2);

        fs::remove_file(&b).unwrap();
        let report = lib.remove_missing(None).unwrap();
        assert_eq!(report.checked, 2, "both rows were looked at");
        assert_eq!(report.removed, 1);
        assert!(report.kept_unreadable.is_empty());

        // The row of the file that is still there stays.
        let rows = matches(&lib, &Query::default());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, a);
        assert_eq!(rows[0].metadata.title.as_deref(), Some("A"));
    }

    #[test]
    fn remove_missing_keeps_the_rows_whose_file_is_there() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("music");
        fs::create_dir(&root).unwrap();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        let entry = lib.add_root(&root).unwrap();
        lib.scan_root(entry.id).unwrap();

        let report = lib.remove_missing(None).unwrap();
        assert_eq!(report.checked, 1);
        assert_eq!(report.removed, 0);
        assert!(report.kept_unreadable.is_empty());
        assert_eq!(lib.count().unwrap(), 1);
        assert_eq!(only_row(&lib).metadata.title.as_deref(), Some("A"));
    }

    #[test]
    fn remove_missing_leaves_the_other_roots_alone() {
        let dir = TempDir::new().unwrap();
        let root_a = dir.path().join("a");
        let root_b = dir.path().join("b");
        fs::create_dir(&root_a).unwrap();
        fs::create_dir(&root_b).unwrap();
        let a = root_a.join("a.wav");
        let b = root_b.join("b.wav");
        make_wav(&a);
        make_wav(&b);
        write_fixture_tags(&a, "A", "X", "Al");
        write_fixture_tags(&b, "B", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        let entry_a = lib.add_root(&root_a).unwrap();
        let entry_b = lib.add_root(&root_b).unwrap();
        lib.scan_root(entry_a.id).unwrap();
        lib.scan_root(entry_b.id).unwrap();
        assert_eq!(lib.count().unwrap(), 2);

        // Only the chosen root is walked: the row of the other root is not
        // even looked at.
        fs::remove_file(&a).unwrap();
        let report = lib.remove_missing(Some(&[entry_a.id])).unwrap();
        assert_eq!(report.checked, 1, "only the rows of the chosen root");
        assert_eq!(report.removed, 1);
        assert_eq!(lib.count().unwrap(), 1);
        assert_eq!(only_row(&lib).path, b);

        // Without a choice every root is walked.
        fs::remove_file(&b).unwrap();
        let report = lib.remove_missing(None).unwrap();
        assert_eq!(report.checked, 1);
        assert_eq!(report.removed, 1);
        assert_eq!(lib.count().unwrap(), 0);
    }

    #[test]
    fn remove_missing_keeps_a_row_whose_path_cannot_be_checked() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("music");
        fs::create_dir(&root).unwrap();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        let entry = lib.add_root(&root).unwrap();
        lib.scan_root(entry.id).unwrap();

        // A row whose path the system refuses to name: the check fails with
        // something else than "not found", so the row must not be dropped.
        let odd = root.join("odd\u{0}name.wav");
        lib.conn
            .execute(
                "INSERT INTO tracks (path, title) VALUES (?1, ?2)",
                rusqlite::params![odd.to_string_lossy().to_string(), "Odd"],
            )
            .unwrap();

        let report = lib.remove_missing(None).unwrap();
        assert_eq!(report.checked, 2);
        assert_eq!(
            report.removed, 0,
            "a row that cannot be checked is not dead"
        );
        assert_eq!(report.kept_unreadable, vec![odd.clone()]);
        assert_eq!(
            lib.count().unwrap(),
            2,
            "the row that cannot be checked stays"
        );
    }

    #[test]
    fn remove_missing_drops_the_extras_with_the_track() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("music");
        fs::create_dir(&root).unwrap();
        let a = root.join("a.wav");
        make_wav(&a);
        let mut md = TrackMetadata {
            title: Some("A".to_owned()),
            ..Default::default()
        };
        // A key the WAV writer round-trips (see `extras_round_trip`).
        md.extra
            .insert("REPLAYGAIN_TRACK_GAIN".to_owned(), "1.50 dB".to_owned());
        rustamp_meta::write_tags(&a, &md).unwrap();

        let mut lib = Library::in_memory().unwrap();
        let entry = lib.add_root(&root).unwrap();
        lib.scan_root(entry.id).unwrap();
        assert_eq!(lib.count().unwrap(), 1);
        assert_eq!(extra_rows(&lib), 1);

        fs::remove_file(&a).unwrap();
        let report = lib.remove_missing(None).unwrap();
        assert_eq!(report.removed, 1);
        assert_eq!(lib.count().unwrap(), 0);
        assert_eq!(extra_rows(&lib), 0, "the extra rows cascade with the track");
    }

    #[test]
    fn remove_missing_with_an_unknown_root_is_an_error() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("music");
        fs::create_dir(&root).unwrap();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        let entry = lib.add_root(&root).unwrap();
        lib.scan_root(entry.id).unwrap();

        let error = lib.remove_missing(Some(&[entry.id + 1])).unwrap_err();
        assert!(matches!(error, Error::NotFound(_)), "{error:?}");
        assert_eq!(lib.count().unwrap(), 1, "nothing was removed");
    }

    #[test]
    fn remove_missing_on_an_empty_library_does_nothing() {
        let mut lib = Library::in_memory().unwrap();
        let report = lib.remove_missing(None).unwrap();
        assert_eq!(report, RemoveReport::default());
        assert_eq!(lib.count().unwrap(), 0);
    }

    /// A PNG of one colour, as a sidecar cover.
    fn png_cover() -> Vec<u8> {
        let picture = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(picture)
            .write_to(&mut io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("encode the picture");
        out
    }

    #[test]
    fn remove_missing_does_not_touch_the_artwork_cache() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("music");
        fs::create_dir(&root).unwrap();
        let a = root.join("a.wav");
        make_wav(&a);
        write_fixture_tags(&a, "A", "X", "Al");
        // A sidecar cover next to the track: the cache answers from it.
        fs::write(root.join("cover.png"), png_cover()).unwrap();

        let cache = ArtworkCache::new();
        assert!(
            cache.get(&a).unwrap().is_some(),
            "the sidecar cover is cached"
        );

        let mut lib = Library::in_memory().unwrap();
        let entry = lib.add_root(&root).unwrap();
        lib.scan_root(entry.id).unwrap();
        fs::remove_file(&a).unwrap();
        let report = lib.remove_missing(None).unwrap();
        assert_eq!(report.removed, 1);

        // The cache belongs to the caller: the operation did not touch it.
        assert_eq!(cache.entry_count(), 1);
    }

    // ---- a damaged database ----

    /// The bytes of the file at `path`.
    fn bytes_of(path: &Path) -> Vec<u8> {
        fs::read(path).expect("the file is readable")
    }

    /// A library database with a root in it.
    fn write_library_database(path: &Path) {
        let mut library = Library::open(path).expect("a fresh database");
        library.add_root(Path::new("C:/music")).expect("the root");
    }

    /// The error of an open that had to fail.
    fn open_error(path: &Path) -> OpenError {
        match Library::open(path) {
            Ok(_) => panic!("the database at {} was opened", path.display()),
            Err(error) => error,
        }
    }

    /// The tables the database at `path` holds, sorted by name.
    fn tables_of(path: &Path) -> Vec<String> {
        let conn = Connection::open(path).expect("the database opens");
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' \
                 AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .expect("the schema is readable");
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("the tables are readable");
        rows.map(|row| row.expect("a table name")).collect()
    }

    /// The schema version the database at `path` carries.
    fn user_version_of(path: &Path) -> i64 {
        let conn = Connection::open(path).expect("the database opens");
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("the version is readable")
    }

    /// The page size the database file announces.
    fn page_size_of(path: &Path) -> usize {
        let bytes = bytes_of(path);
        let raw = u16::from_be_bytes([bytes[16], bytes[17]]);
        if raw == 1 { 65536 } else { usize::from(raw) }
    }

    #[test]
    fn a_file_that_is_not_a_database_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        // A file with the name of a library database and content that is
        // not a database.
        fs::write(&path, vec![0x5au8; 4096]).unwrap();
        let before = bytes_of(&path);

        let error = open_error(&path);
        assert!(matches!(error, OpenError::NotADatabase { .. }), "{error:?}");
        assert_eq!(
            bytes_of(&path),
            before,
            "the file is left exactly as it was"
        );
    }

    #[test]
    fn a_truncated_database_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        write_library_database(&path);

        // The file is cut in the middle of a page.
        let full = bytes_of(&path);
        fs::write(&path, &full[..full.len() - 100]).unwrap();
        let before = bytes_of(&path);

        let error = open_error(&path);
        assert!(matches!(error, OpenError::Truncated { .. }), "{error:?}");
        assert_eq!(
            bytes_of(&path),
            before,
            "the file is left exactly as it was"
        );
    }

    #[test]
    fn a_database_that_lost_pages_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        write_library_database(&path);
        // A write that does not change the size of the file: the header and
        // the file are out of step, which is the normal state of a database
        // that was written to.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("UPDATE roots SET added_at = added_at")
                .unwrap();
        }

        // The file is cut at a page boundary: the pages are gone, and the
        // header still announces them.
        let full = bytes_of(&path);
        let cut = full.len() - page_size_of(&path);
        fs::write(&path, &full[..cut]).unwrap();
        let before = bytes_of(&path);

        let error = open_error(&path);
        assert!(matches!(error, OpenError::Truncated { .. }), "{error:?}");
        assert_eq!(
            bytes_of(&path),
            before,
            "the file is left exactly as it was"
        );
    }

    #[test]
    fn a_database_with_a_foreign_tracks_table_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        // Another program wrote a `tracks` table with other columns.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE tracks (id INTEGER PRIMARY KEY, name TEXT)")
                .unwrap();
        }
        let before = bytes_of(&path);

        let error = open_error(&path);
        assert!(
            matches!(error, OpenError::ForeignSchema { .. }),
            "{error:?}"
        );
        assert_eq!(
            bytes_of(&path),
            before,
            "the file is left exactly as it was"
        );
    }

    #[test]
    fn a_database_of_another_program_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE playlist (id INTEGER PRIMARY KEY, name TEXT)")
                .unwrap();
        }
        let before = bytes_of(&path);

        // The schema of the library is not written over the database of
        // another program: the rows of the user stay as they are.
        let error = open_error(&path);
        assert!(
            matches!(error, OpenError::ForeignSchema { .. }),
            "{error:?}"
        );
        assert_eq!(
            bytes_of(&path),
            before,
            "the file is left exactly as it was"
        );
        assert_eq!(
            tables_of(&path),
            vec!["playlist"],
            "the library tables were not created in it"
        );
    }

    #[test]
    fn a_database_of_a_newer_player_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        write_library_database(&path);
        // A newer version of the player wrote the file.
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "user_version", 99).unwrap();
        }
        let before = bytes_of(&path);

        let error = open_error(&path);
        match error {
            OpenError::FutureVersion {
                found, supported, ..
            } => {
                assert_eq!(found, 99, "the error names the version of the file");
                assert!(
                    supported < 99,
                    "the error names the version this crate knows"
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            bytes_of(&path),
            before,
            "the file is left exactly as it was"
        );
    }

    #[test]
    fn a_read_only_database_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        write_library_database(&path);
        let original = fs::metadata(&path).unwrap().permissions();
        let mut permissions = original.clone();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
        let before = bytes_of(&path);

        let error = open_error(&path);
        assert!(matches!(error, OpenError::ReadOnly { .. }), "{error:?}");
        assert_eq!(
            bytes_of(&path),
            before,
            "the file is left exactly as it was"
        );

        // The file can be removed again.
        fs::set_permissions(&path, original).unwrap();
    }

    #[test]
    fn a_locked_database_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        write_library_database(&path);
        let before = bytes_of(&path);

        // Another connection holds the database: it cannot be read.
        let holder = Connection::open(&path).unwrap();
        holder.execute_batch("BEGIN EXCLUSIVE").unwrap();

        let error = open_error(&path);
        assert!(matches!(error, OpenError::Locked { .. }), "{error:?}");

        holder.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            bytes_of(&path),
            before,
            "the file is left exactly as it was"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_file_held_by_another_process_is_refused() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        write_library_database(&path);
        let before = bytes_of(&path);

        // Another process holds the file open without sharing the access.
        let handle = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&path)
            .expect("the file is held");

        let error = open_error(&path);
        assert!(matches!(error, OpenError::Locked { .. }), "{error:?}");

        drop(handle);
        assert_eq!(
            bytes_of(&path),
            before,
            "the file is left exactly as it was"
        );
    }

    #[test]
    fn a_directory_in_place_of_the_database_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        fs::create_dir(&path).unwrap();

        let error = open_error(&path);
        assert!(matches!(error, OpenError::NotAFile { .. }), "{error:?}");
    }

    #[test]
    fn a_damaged_database_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        write_library_database(&path);

        // The header does not announce the size of the file any more, and
        // the file is half as long as it was: the pages the schema points at
        // are gone.
        let full = bytes_of(&path);
        let mut damaged = full[..full.len() / 2].to_vec();
        damaged[28..32].copy_from_slice(&0u32.to_be_bytes());
        damaged[92..96].copy_from_slice(&0u32.to_be_bytes());
        fs::write(&path, &damaged).unwrap();
        let before = bytes_of(&path);

        let error = open_error(&path);
        assert!(matches!(error, OpenError::Corrupt { .. }), "{error:?}");
        assert_eq!(
            bytes_of(&path),
            before,
            "the file is left exactly as it was"
        );
    }

    #[test]
    fn a_database_that_cannot_be_prepared_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        write_library_database(&path);
        // The `roots` table is a view: the column the migration adds cannot
        // be added to it, so the database cannot be prepared.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "DROP TABLE roots;
                 CREATE VIEW roots AS
                     SELECT 1 AS id, 'C:/music' AS path, 0 AS added_at;",
            )
            .unwrap();
        }
        let before = bytes_of(&path);

        let error = open_error(&path);
        assert!(matches!(error, OpenError::Failed { .. }), "{error:?}");
        assert_eq!(
            bytes_of(&path),
            before,
            "the file is left exactly as it was"
        );
    }

    #[test]
    fn a_database_that_cannot_be_opened_is_refused() {
        let dir = TempDir::new().unwrap();
        // The directory of the database does not exist.
        let path = dir.path().join("nowhere").join("library.db");

        let error = open_error(&path);
        assert!(matches!(error, OpenError::Io { .. }), "{error:?}");
    }

    #[test]
    fn an_empty_file_is_the_first_run() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        fs::write(&path, b"").unwrap();

        let library =
            Library::open(&path).expect("an empty file is a database that was never written to");
        assert_eq!(library.count().unwrap(), 0);
        drop(library);

        assert_eq!(
            tables_of(&path),
            vec!["roots", "track_extra", "tracks"],
            "the schema is in the file"
        );
    }

    #[test]
    fn a_missing_file_is_the_first_run() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        assert!(!path.exists());

        let library = Library::open(&path).expect("a missing file is a database that is not there");
        assert_eq!(library.count().unwrap(), 0);
        drop(library);

        assert!(path.is_file(), "the database was created");
        assert_eq!(tables_of(&path), vec!["roots", "track_extra", "tracks"]);
    }

    #[test]
    fn a_valid_database_opens_again_and_keeps_its_rows() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("library.db");
        write_library_database(&path);

        let library = Library::open(&path).expect("a valid database opens");
        let roots = library.roots().expect("the roots are readable");
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].path, PathBuf::from("C:/music"));
        drop(library);

        assert_eq!(
            user_version_of(&path),
            1,
            "the database carries the schema version of this crate"
        );
    }
}
