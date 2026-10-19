// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-library
//!
//! Media Library (M5): a persistent, SQLite-backed index of audio tracks.
//!
//! This crate owns the storage and scan/query layer only. It does not touch
//! the player, the UI or the playlist model. A future UI lane builds on top
//! of [`Library`] by calling [`Library::scan`] and [`Library::query`].
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
//! ## Known limits
//!
//! * Change detection uses the file's stat, not its contents. A rewrite that
//!   preserves both size and modification time (to the filesystem's
//!   timestamp granularity) is not noticed until something else changes.
//! * Tag reading does not decode audio, so `duration_ms` and `bitrate_kbps`
//!   are only filled when tags themselves carry them.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use rustamp_core::error::{Error, Result};
use rustamp_core::meta::TrackMetadata;

/// The column list shared by every row-returning statement, in the order
/// [`read_raw`] expects.
const TRACK_COLUMNS: &str = "id, path, title, artist, album, album_artist, year, genre, \
     comment, track_no, track_total, disc_no, duration_ms, bitrate_kbps, mtime_ns, size";

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

/// Filter for [`Library::query`]. All fields are optional and combined with
/// AND. `None` means "no constraint".
///
/// Text filters are exact, case-sensitive matches. `path_prefix` is a literal
/// byte prefix, not a directory boundary: a prefix of `C:\music` also matches
/// `C:\music2`.
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
}

/// Persistent media library index.
pub struct Library {
    conn: Connection,
}

impl Library {
    /// Open (or create) a library database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path.as_ref()).map_err(map_sqlite)?;
        let mut lib = Self { conn };
        lib.migrate()?;
        Ok(lib)
    }

    /// Open an in-memory library (useful for tests).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(map_sqlite)?;
        let mut lib = Self { conn };
        lib.migrate()?;
        Ok(lib)
    }

    fn migrate(&mut self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS tracks (
                    id            INTEGER PRIMARY KEY AUTOINCREMENT,
                    path          TEXT NOT NULL UNIQUE,
                    title         TEXT,
                    artist        TEXT,
                    album         TEXT,
                    album_artist  TEXT,
                    year          INTEGER,
                    genre         TEXT,
                    comment       TEXT,
                    track_no      INTEGER,
                    track_total   INTEGER,
                    disc_no       INTEGER,
                    duration_ms   INTEGER,
                    bitrate_kbps  INTEGER,
                    mtime_ns      INTEGER,
                    size          INTEGER,
                    scanned_at    INTEGER
                );
                CREATE INDEX IF NOT EXISTS tracks_artist ON tracks(artist);
                CREATE INDEX IF NOT EXISTS tracks_album ON tracks(album);
                CREATE TABLE IF NOT EXISTS track_extra (
                    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                    key      TEXT NOT NULL,
                    value    TEXT NOT NULL,
                    PRIMARY KEY (track_id, key)
                );",
            )
            .map_err(map_sqlite)?;
        Ok(())
    }

    /// Scan `root`, upserting track rows. See module docs for the rule.
    ///
    /// Returns [`Error::NotFound`] when `root` does not exist.
    pub fn scan(&mut self, root: &Path) -> Result<ScanReport> {
        if fs::metadata(root).is_err() {
            return Err(Error::NotFound(root.display().to_string()));
        }

        let mut files = Vec::new();
        let mut walk_complete = true;
        walk_dir(root, &mut files, &mut walk_complete);

        // Existing rows under root, for change detection and removals.
        let existing = self.rows_under(root)?;

        let mut report = ScanReport::default();
        // Every considered file found on disk, whether or not it could be read.
        // A file that is present but unreadable must not be treated as removed.
        let mut present: HashSet<PathBuf> = HashSet::new();

        let tx = self.conn.transaction().map_err(map_sqlite)?;
        for path in files {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !rustamp_meta::supports_extension(ext) {
                continue;
            }
            present.insert(path.clone());

            let meta = match fs::metadata(&path) {
                Ok(meta) => meta,
                Err(_) => {
                    report.skipped += 1;
                    continue;
                }
            };
            let mtime_ns = mtime_of(&meta);
            let size = Some(meta.len() as i64);

            if let Some((stored_mtime, stored_size)) = existing.get(&path)
                && *stored_mtime == mtime_ns
                && *stored_size == size
            {
                report.scanned += 1;
                continue;
            }

            let tags = match rustamp_meta::read_tags(&path) {
                Ok(tags) => tags,
                Err(_) => {
                    report.skipped += 1;
                    continue;
                }
            };

            let is_new = !existing.contains_key(&path);
            upsert_row(&tx, &path, &tags, mtime_ns, size)?;
            if is_new {
                report.added += 1;
            } else {
                report.updated += 1;
            }
            report.scanned += 1;
        }

        if walk_complete {
            for path in existing.keys() {
                if !present.contains(path) {
                    delete_path(&tx, path)?;
                    report.removed += 1;
                }
            }
        }

        tx.commit().map_err(map_sqlite)?;
        Ok(report)
    }

    /// Query rows by filter, ordered by row id.
    pub fn query(&self, filter: &Query) -> Result<Vec<TrackRow>> {
        let mut sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE 1=1");
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(prefix) = &filter.path_prefix {
            sql.push_str(" AND path LIKE ? ESCAPE '\\'");
            binds.push(Box::new(like_prefix(&prefix.to_string_lossy())));
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
        sql.push_str(" ORDER BY id");

        let mut stmt = self.conn.prepare(&sql).map_err(map_sqlite)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())),
                read_raw,
            )
            .map_err(map_sqlite)?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(map_sqlite)?.into_track(&self.conn)?);
        }
        Ok(result)
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

    // ---- internals ----

    /// All stored paths under `root`, with the stat fields used for change
    /// detection.
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

/// Insert or update the row for `path` and replace its extra map.
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

/// Walk `root` recursively. Unreadable directories and entries clear
/// `complete`; symlinks are neither followed nor indexed.
fn walk_dir(root: &Path, out: &mut Vec<PathBuf>, complete: &mut bool) {
    if root.is_file() {
        out.push(root.to_path_buf());
        return;
    }
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => {
            *complete = false;
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                *complete = false;
                continue;
            }
        };
        let path = entry.path();
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => walk_dir(&path, out, complete),
            Ok(ft) if ft.is_file() => out.push(path),
            // Symlinks and other entries are not followed.
            Ok(_) => {}
            Err(_) => *complete = false,
        }
    }
}

/// Escape `%`, `_` and `\` in `prefix` and append the `%` that makes the
/// pattern a prefix match for `LIKE ... ESCAPE '\'`.
fn like_prefix(prefix: &str) -> String {
    let mut escaped = String::with_capacity(prefix.len() + 1);
    for ch in prefix.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push('%');
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
    use rustamp_core::meta::TrackMetadata;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_wav(path: &Path) {
        // Minimal 44-byte WAV header (no audio).
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
        buf.extend_from_slice(&16u16.to_le_bytes()); // bits
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&0u32.to_le_bytes());
        let mut f = File::create(path).unwrap();
        f.write_all(&buf).unwrap();
    }

    fn write_tags(path: &Path, title: &str, artist: &str, album: &str) {
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
        let mut rows = lib.query(&Query::default()).unwrap();
        assert_eq!(rows.len(), 1);
        rows.remove(0)
    }

    #[test]
    fn scan_indexes_and_queries() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("a.wav");
        let b = root.join("b.wav");
        make_wav(&a);
        make_wav(&b);
        write_tags(&a, "Song A", "Artist X", "Album 1");
        write_tags(&b, "Song B", "Artist Y", "Album 2");

        let mut lib = Library::in_memory().unwrap();
        let rep = lib.scan(root).unwrap();
        assert_eq!(rep.scanned, 2);
        assert_eq!(rep.added, 2);
        assert_eq!(rep.removed, 0);
        assert_eq!(rep.skipped, 0);
        assert_eq!(lib.count().unwrap(), 2);

        // Query by artist.
        let rows = lib
            .query(&Query {
                artist: Some("Artist X".to_owned()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].metadata.title.as_deref(), Some("Song A"));

        // Query by path prefix.
        let rows = lib
            .query(&Query {
                path_prefix: Some(root.to_path_buf()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn rescan_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = root.join("a.wav");
        make_wav(&a);
        write_tags(&a, "Song A", "Artist X", "Album 1");

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
        write_tags(&a, "Song A", "Artist X", "Album 1");

        let mut lib = Library::in_memory().unwrap();
        lib.scan(root).unwrap();
        let id_before = only_row(&lib).id;

        // Rewrite the tags. No sleep: change detection uses nanoseconds.
        write_tags(&a, "Song A2", "Artist X", "Album 1");
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
        write_tags(&a, "A", "X", "Al");
        write_tags(&b, "B", "Y", "Al2");

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
        write_tags(&a, "A", "X", "Album Z");

        let mut lib = Library::in_memory().unwrap();
        lib.scan(root).unwrap();
        let rows = lib
            .query(&Query {
                album: Some("Album Z".to_owned()),
                ..Default::default()
            })
            .unwrap();
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
            write_tags(&path, title, artist, album);
        }

        let mut lib = Library::in_memory().unwrap();
        lib.scan(root).unwrap();

        let rows = lib
            .query(&Query {
                artist: Some("X".to_owned()),
                album: Some("Album 1".to_owned()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].metadata.title.as_deref(), Some("One"));

        // A filter that matches nothing yields no rows.
        let rows = lib
            .query(&Query {
                artist: Some("X".to_owned()),
                genre: Some("Jazz".to_owned()),
                ..Default::default()
            })
            .unwrap();
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
        write_tags(&a, "A", "X", "Al");

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
        write_tags(&a, "A", "X", "Al");

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
        write_tags(&a, "A", "X", "Al");
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
        write_tags(&a, "A", "X", "Al");

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
        write_tags(&a, "A", "X", "Al");
        write_tags(&b, "B", "Y", "Al2");

        let mut lib = Library::in_memory().unwrap();
        lib.scan(&root_a).unwrap();
        lib.scan(&root_b).unwrap();
        assert_eq!(lib.count().unwrap(), 2);

        fs::remove_file(&b).unwrap();
        let rep = lib.scan(&root_b).unwrap();
        assert_eq!(rep.removed, 1);
        assert_eq!(lib.count().unwrap(), 1);
        let rows = lib
            .query(&Query {
                path_prefix: Some(root_a.clone()),
                ..Default::default()
            })
            .unwrap();
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
        write_tags(&a, "A", "X", "Al");

        let mut lib = Library::in_memory().unwrap();
        assert_eq!(lib.scan(&root).unwrap().added, 1);
        // Idempotent: a broken escape would see a changed pattern and re-add.
        assert_eq!(lib.scan(&root).unwrap().updated, 0);

        let rows = lib
            .query(&Query {
                path_prefix: Some(root.clone()),
                ..Default::default()
            })
            .unwrap();
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
}
