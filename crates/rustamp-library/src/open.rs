// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Opening a library database without destroying a damaged one.
//!
//! A media library is an index the user cannot rebuild by hand: the tags are
//! in the files, but the *index* - the rows, the ids, the roots - is only in
//! the database. Losing it means losing the library view, and a player that
//! silently "repairs" a damaged file by writing a fresh schema over it
//! destroys the index without a word.
//!
//! So the open of this crate has one rule: **a file that could not be read
//! as this crate's index is never written to.** The file is inspected before
//! SQLite sees it, and the schema is read and checked before anything is
//! written, so a refused file stays byte for byte what it was.
//!
//! What is refused, and how:
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
//! first run: the schema is created in them.

use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use rusqlite::ffi::ErrorCode;
use rusqlite::{Connection, params};

/// The schema version of the library index.
///
/// The version is written into the database header (`PRAGMA user_version`),
/// so a file that a newer version of the player wrote can be told apart from
/// a file of this version: it is refused instead of being migrated backwards.
const SCHEMA_VERSION: u32 = 1;

/// The bytes every SQLite database file starts with.
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

/// The length of the SQLite file header.
const HEADER_LEN: usize = 100;

/// The read version of a database that keeps its changes in a write-ahead
/// log. Such a database may be shorter than its header announces, because
/// the pages that are missing are in the log next to it.
const WAL_READ_VERSION: u8 = 2;

/// The schema of the library index.
const SCHEMA_DDL: &str = "CREATE TABLE IF NOT EXISTS tracks (
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
);
CREATE TABLE IF NOT EXISTS roots (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    path     TEXT NOT NULL UNIQUE,
    added_at INTEGER NOT NULL,
    last_scanned_at INTEGER
);";

/// The tables of the library index and the columns they must have.
///
/// The list is the fingerprint of "a database of this crate": a file that
/// holds none of these tables is another program's database and is refused.
/// The `roots` table does not list the last-scan column: that column is
/// added by the migration, so a database of an older version is still ours.
const EXPECTED_TABLES: &[(&str, &[&str])] = &[
    (
        "tracks",
        &[
            "id",
            "path",
            "title",
            "artist",
            "album",
            "album_artist",
            "year",
            "genre",
            "comment",
            "track_no",
            "track_total",
            "disc_no",
            "duration_ms",
            "bitrate_kbps",
            "mtime_ns",
            "size",
            "scanned_at",
        ],
    ),
    ("track_extra", &["track_id", "key", "value"]),
    ("roots", &["id", "path", "added_at"]),
];

/// Why a library database could not be opened.
///
/// Every variant names one way the file on disk can be damaged. A file that
/// is refused is left exactly as it was: the index is never dropped,
/// truncated, migrated over or replaced because of a damaged file.
#[derive(Debug)]
pub enum OpenError {
    /// The path is not a regular file (a directory, a device, ...).
    NotAFile {
        /// The path of the database.
        path: PathBuf,
    },
    /// The file is not a SQLite database: its header is not one.
    NotADatabase {
        /// The path of the database.
        path: PathBuf,
        /// What the header said.
        detail: String,
    },
    /// The file is a SQLite database that was cut short.
    Truncated {
        /// The path of the database.
        path: PathBuf,
        /// What was missing.
        detail: String,
    },
    /// The file is a SQLite database that is damaged.
    Corrupt {
        /// The path of the database.
        path: PathBuf,
        /// What SQLite reported.
        detail: String,
    },
    /// The file is a valid database, but not a library index.
    ForeignSchema {
        /// The path of the database.
        path: PathBuf,
        /// Why it is not a library index.
        detail: String,
    },
    /// The file was written by a newer version of the player.
    FutureVersion {
        /// The path of the database.
        path: PathBuf,
        /// The version the file carries.
        found: u32,
        /// The version this crate understands.
        supported: u32,
    },
    /// The file cannot be written to.
    ReadOnly {
        /// The path of the database.
        path: PathBuf,
    },
    /// Another process holds the file.
    Locked {
        /// The path of the database.
        path: PathBuf,
        /// What the operating system said.
        detail: String,
    },
    /// The file could not be read at all.
    Io {
        /// The path of the database.
        path: PathBuf,
        /// What the operating system said.
        detail: String,
    },
    /// The database passed the checks, but could not be prepared.
    Failed {
        /// The path of the database.
        path: PathBuf,
        /// What SQLite said.
        detail: String,
    },
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::NotAFile { path } => write!(
                f,
                "{}: the media library database is not a file",
                path.display()
            ),
            OpenError::NotADatabase { path, detail } => write!(
                f,
                "{}: the media library database is not a database ({detail})",
                path.display()
            ),
            OpenError::Truncated { path, detail } => write!(
                f,
                "{}: the media library database is truncated ({detail})",
                path.display()
            ),
            OpenError::Corrupt { path, detail } => write!(
                f,
                "{}: the media library database is damaged ({detail})",
                path.display()
            ),
            OpenError::ForeignSchema { path, detail } => write!(
                f,
                "{}: the media library database is not a Rustamp library index ({detail})",
                path.display()
            ),
            OpenError::FutureVersion {
                path,
                found,
                supported,
            } => write!(
                f,
                "{}: the media library database was written by a newer version of the player \
                 (schema {found}, this version understands {supported})",
                path.display()
            ),
            OpenError::ReadOnly { path } => write!(
                f,
                "{}: the media library database is read-only",
                path.display()
            ),
            OpenError::Locked { path, detail } => write!(
                f,
                "{}: the media library database is held by another process ({detail})",
                path.display()
            ),
            OpenError::Io { path, detail } => write!(
                f,
                "{}: the media library database could not be read ({detail})",
                path.display()
            ),
            OpenError::Failed { path, detail } => write!(
                f,
                "{}: the media library database could not be prepared ({detail})",
                path.display()
            ),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<OpenError> for rustamp_core::Error {
    fn from(error: OpenError) -> Self {
        rustamp_core::Error::Other(error.to_string())
    }
}

/// The connection to the library database at `path`, checked and ready.
///
/// The order of the steps is the rule of this module: the file is looked at
/// before SQLite is allowed near it, and the schema is read and checked
/// before anything is written. A file that is refused is not touched.
pub(crate) fn connect(path: &Path) -> Result<Connection, OpenError> {
    inspect(path)?;
    let conn = Connection::open(path).map_err(|error| map_sqlite_error(path, error))?;
    check_version(&conn, path)?;
    check_schema(&conn, path)?;
    Ok(conn)
}

/// The schema of the database, brought in line with this version.
///
/// The statements are idempotent: a table that is already there keeps its
/// shape, and the migration that adds a column runs on every open. The
/// schema version is written at the end, so a database of this version is
/// recognized on the next open.
///
/// The whole preparation is **one transaction**: a preparation that fails
/// leaves the database as it was, so a file that is refused is never left
/// half migrated.
pub(crate) fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch(SCHEMA_DDL)?;
    add_column_if_missing(&tx, "roots", "last_scanned_at", "INTEGER")?;
    set_user_version(&tx, SCHEMA_VERSION)?;
    tx.commit()
}

/// Look at the file before SQLite sees it.
///
/// The check answers one question: is this a database of this crate, or at
/// least a database at all? A file that is not a database is never handed
/// to SQLite, so no write can reach it.
fn inspect(path: &Path) -> Result<(), OpenError> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        // A file that is not there yet is the first run: the database is
        // created.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(file_error(path, error)),
    };

    if !meta.is_file() {
        return Err(OpenError::NotAFile {
            path: path.to_path_buf(),
        });
    }

    // A file without a byte in it is a database that was never written to:
    // the first run creates the schema in it.
    if meta.len() == 0 {
        return Ok(());
    }

    // A file that cannot be written to cannot hold the index: the scan
    // would fail later, and the open would promise more than it can keep.
    if meta.permissions().readonly() {
        return Err(OpenError::ReadOnly {
            path: path.to_path_buf(),
        });
    }

    let header = read_header(path)?;
    if !header.starts_with(SQLITE_MAGIC) {
        return Err(OpenError::NotADatabase {
            path: path.to_path_buf(),
            detail: "the file does not start with the SQLite header".to_owned(),
        });
    }
    if header.len() < HEADER_LEN {
        return Err(OpenError::Truncated {
            path: path.to_path_buf(),
            detail: format!(
                "the {HEADER_LEN}-byte header is cut short after {} bytes",
                header.len()
            ),
        });
    }

    let Some(page_size) = page_size(&header) else {
        return Err(OpenError::NotADatabase {
            path: path.to_path_buf(),
            detail: "the header announces an impossible page size".to_owned(),
        });
    };

    // The file is a whole number of pages. A file that ends in the middle
    // of a page was cut short.
    let page_size = u64::from(page_size);
    if meta.len() % page_size != 0 {
        return Err(OpenError::Truncated {
            path: path.to_path_buf(),
            detail: format!("the file ends in the middle of a {page_size}-byte page"),
        });
    }

    // The header announces how many pages the database has. A file with
    // fewer pages than the header announces lost pages: it was cut short.
    // (A file with *more* pages is not cut short: that is what a database
    // with an unplayed rollback journal looks like, and SQLite brings it
    // back on its own.) A database with a write-ahead log is skipped: the
    // pages that are missing from the file are in the log next to it.
    if header[19] != WAL_READ_VERSION {
        let pages = u32::from_be_bytes([header[28], header[29], header[30], header[31]]);
        let announced = u64::from(pages) * page_size;
        if meta.len() < announced {
            return Err(OpenError::Truncated {
                path: path.to_path_buf(),
                detail: format!(
                    "the file holds {} of the {pages} pages the header announces",
                    meta.len() / page_size
                ),
            });
        }
    }

    Ok(())
}

/// The header of the file, or the bytes of it that are there.
fn read_header(path: &Path) -> Result<Vec<u8>, OpenError> {
    let mut file = fs::File::open(path).map_err(|error| file_error(path, error))?;
    let mut header = vec![0u8; HEADER_LEN];
    let mut read = 0;
    while read < header.len() {
        match file.read(&mut header[read..]) {
            Ok(0) => break,
            Ok(count) => read += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(file_error(path, error)),
        }
    }
    header.truncate(read);
    Ok(header)
}

/// The page size the header announces.
///
/// The value 1 stands for the largest page size. A size that is not a
/// power of two in the range SQLite allows means the file is not a
/// database.
fn page_size(header: &[u8]) -> Option<u32> {
    let raw = u16::from_be_bytes([header[16], header[17]]);
    let size = if raw == 1 { 65536 } else { u32::from(raw) };
    if size.is_power_of_two() && (512..=65536).contains(&size) {
        Some(size)
    } else {
        None
    }
}

/// The schema version of the database, refused when it is from the future.
fn check_version(conn: &Connection, path: &Path) -> Result<(), OpenError> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| map_sqlite_error(path, error))?;
    if version > i64::from(SCHEMA_VERSION) {
        return Err(OpenError::FutureVersion {
            path: path.to_path_buf(),
            found: version as u32,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// The schema of the database, refused when it is not a library index.
///
/// A database without a table in it is the first run: the schema is created
/// in it. A database that holds one of the library tables is ours, even when
/// it is older than this version - the migration brings it in line. Every
/// other database belongs to another program and is refused, so the schema
/// is never written over it.
fn check_schema(conn: &Connection, path: &Path) -> Result<(), OpenError> {
    let tables = table_names(conn).map_err(|error| map_sqlite_error(path, error))?;
    if tables.is_empty() {
        return Ok(());
    }

    let mut ours = 0;
    for (table, required) in EXPECTED_TABLES {
        if !tables.iter().any(|name| name == table) {
            continue;
        }
        ours += 1;
        let columns = column_names(conn, table).map_err(|error| map_sqlite_error(path, error))?;
        let missing: Vec<&str> = required
            .iter()
            .copied()
            .filter(|column| !columns.iter().any(|name| name == column))
            .collect();
        if !missing.is_empty() {
            // The first few names are enough to tell the tables apart; the
            // rest would only make the message unreadable.
            let shown = missing.len().min(4);
            let mut names: Vec<&str> = missing[..shown].to_vec();
            if shown < missing.len() {
                names.push("...");
            }
            return Err(OpenError::ForeignSchema {
                path: path.to_path_buf(),
                detail: format!("the table `{table}` has no column {}", names.join(", ")),
            });
        }
    }

    if ours == 0 {
        return Err(OpenError::ForeignSchema {
            path: path.to_path_buf(),
            detail: format!(
                "the database holds the table(s) {} and no library table",
                tables.join(", ")
            ),
        });
    }
    Ok(())
}

/// The tables of the database, without SQLite's own ones.
fn table_names(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// The columns of the table `name`.
fn column_names(conn: &Connection, name: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let rows = stmt.query_map(params![name], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// Add the column `column` to the table `table` when it is not there yet.
///
/// The crate has no migration framework: the schema is created with
/// `CREATE TABLE IF NOT EXISTS`, which leaves a table that already exists
/// exactly as it is. A database written by an older version therefore keeps
/// its old shape and misses the columns a newer version expects.
///
/// The guard is the table's own column list (`pragma_table_info`), so the
/// migration is idempotent: it runs on every open and does nothing once the
/// column is there. The new column is nullable and has no default, so the
/// rows of the old database keep their values and read as "never scanned"
/// until a scan fills them in.
fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
        params![table, column],
        |row| row.get(0),
    )?;
    if count > 0 {
        return Ok(());
    }
    // The table and the column are constants of this crate, never caller
    // input, so the statement cannot carry an injection.
    conn.execute_batch(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition}"
    ))
}

/// The schema version the database carries.
fn set_user_version(conn: &Connection, version: u32) -> rusqlite::Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current == i64::from(version) {
        return Ok(());
    }
    conn.pragma_update(None, "user_version", i64::from(version))
}

/// The damage a SQLite error names.
fn map_sqlite_error(path: &Path, error: rusqlite::Error) -> OpenError {
    let rusqlite::Error::SqliteFailure(failure, message) = &error else {
        return OpenError::Io {
            path: path.to_path_buf(),
            detail: error.to_string(),
        };
    };
    let detail = message
        .clone()
        .unwrap_or_else(|| format!("{:?}", failure.code));
    match failure.code {
        ErrorCode::NotADatabase => OpenError::NotADatabase {
            path: path.to_path_buf(),
            detail,
        },
        ErrorCode::DatabaseCorrupt => OpenError::Corrupt {
            path: path.to_path_buf(),
            detail,
        },
        ErrorCode::ReadOnly => OpenError::ReadOnly {
            path: path.to_path_buf(),
        },
        ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => OpenError::Locked {
            path: path.to_path_buf(),
            detail,
        },
        ErrorCode::CannotOpen | ErrorCode::PermissionDenied => OpenError::Io {
            path: path.to_path_buf(),
            detail,
        },
        _ => OpenError::Failed {
            path: path.to_path_buf(),
            detail,
        },
    }
}

/// The error of a file the crate could not read.
fn file_error(path: &Path, error: io::Error) -> OpenError {
    if is_held_by_another_process(&error) {
        return OpenError::Locked {
            path: path.to_path_buf(),
            detail: "another process holds the file open".to_owned(),
        };
    }
    OpenError::Io {
        path: path.to_path_buf(),
        detail: error.to_string(),
    }
}

/// Whether the error means "another process holds the file".
#[cfg(windows)]
fn is_held_by_another_process(error: &io::Error) -> bool {
    // ERROR_SHARING_VIOLATION: the file is open somewhere else, and the
    // other process did not share the access.
    error.raw_os_error() == Some(32)
}

/// Whether the error means "another process holds the file".
#[cfg(not(windows))]
fn is_held_by_another_process(_error: &io::Error) -> bool {
    // A POSIX advisory lock does not keep a reader out: there the lock is
    // only visible to SQLite itself.
    false
}
