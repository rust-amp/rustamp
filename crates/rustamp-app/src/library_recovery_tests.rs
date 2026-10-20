// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The recovery of a damaged media library database.
//!
//! Every test is headless: a file in a temporary folder, no window, no
//! device, no child process.

use std::path::Path;
use std::time::{Duration, SystemTime};

use rustamp_library::OpenError;
use tempfile::TempDir;

use super::*;

/// The bytes of a file that is not a database.
const DAMAGED: &[u8] = b"this is not a database, not even a little one";

/// A database file with the given bytes in it.
fn damaged_file(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.path().join("library.sqlite3");
    std::fs::write(&path, DAMAGED).expect("write the damaged file");
    path
}

/// Every damage class, with the answer the app gives.
fn every_damage(path: &Path) -> Vec<(&'static str, OpenError, bool)> {
    vec![
        (
            "not a database",
            OpenError::NotADatabase {
                path: path.to_path_buf(),
                detail: "the file does not start with the SQLite header".to_owned(),
            },
            true,
        ),
        (
            "truncated",
            OpenError::Truncated {
                path: path.to_path_buf(),
                detail: "the header is cut short".to_owned(),
            },
            true,
        ),
        (
            "damaged",
            OpenError::Corrupt {
                path: path.to_path_buf(),
                detail: "database disk image is malformed".to_owned(),
            },
            true,
        ),
        (
            "foreign schema",
            OpenError::ForeignSchema {
                path: path.to_path_buf(),
                detail: "the table tracks is missing".to_owned(),
            },
            true,
        ),
        (
            "future version",
            OpenError::FutureVersion {
                path: path.to_path_buf(),
                found: 2,
                supported: 1,
            },
            true,
        ),
        (
            "not a file",
            OpenError::NotAFile {
                path: path.to_path_buf(),
            },
            false,
        ),
        (
            "read-only",
            OpenError::ReadOnly {
                path: path.to_path_buf(),
            },
            false,
        ),
        (
            "locked",
            OpenError::Locked {
                path: path.to_path_buf(),
                detail: "database is locked".to_owned(),
            },
            false,
        ),
        (
            "io",
            OpenError::Io {
                path: path.to_path_buf(),
                detail: "access is denied".to_owned(),
            },
            false,
        ),
        (
            "failed",
            OpenError::Failed {
                path: path.to_path_buf(),
                detail: "no such table: tracks".to_owned(),
            },
            false,
        ),
    ]
}

// ---------------------------------------------------------------------------
// The branch
// ---------------------------------------------------------------------------

#[test]
fn only_the_damage_of_the_file_itself_is_recovered() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("library.sqlite3");
    for (name, error, recovered) in every_damage(&path) {
        assert_eq!(
            is_recoverable(&error),
            recovered,
            "the damage {name} is answered as told"
        );
    }
}

// ---------------------------------------------------------------------------
// The backup
// ---------------------------------------------------------------------------

#[test]
fn the_damaged_file_is_renamed_and_keeps_its_bytes() {
    let dir = TempDir::new().expect("temp dir");
    let database = damaged_file(&dir);
    let error = OpenError::NotADatabase {
        path: database.clone(),
        detail: "the file does not start with the SQLite header".to_owned(),
    };

    let recovery = quarantine(&database, &error).expect("the file is set aside");

    assert!(
        !database.exists(),
        "the database is not where it was any more"
    );
    assert_eq!(
        std::fs::read(&recovery.backup).expect("read the backup"),
        DAMAGED,
        "the backup keeps the bytes of the damaged file"
    );
    let name = recovery
        .backup
        .file_name()
        .expect("the backup has a name")
        .to_string_lossy()
        .to_string();
    assert!(
        name.starts_with("library.sqlite3.broken-"),
        "the backup is named after the database: {name}"
    );
    assert!(name.ends_with('Z'), "the stamp is a UTC time: {name}");
    assert_eq!(
        recovery.backup.parent(),
        database.parent(),
        "the backup lies next to the database"
    );
}

#[test]
fn a_second_recovery_in_the_same_second_gets_its_own_name() {
    let dir = TempDir::new().expect("temp dir");
    let database = damaged_file(&dir);
    let stamp = "20261020T075412Z";

    assert_eq!(free_suffix(&database, stamp), ".broken-20261020T075412Z");

    // The backup of the first recovery is there: the second one needs
    // another name.
    std::fs::write(sibling(&database, ".broken-20261020T075412Z"), b"old")
        .expect("write the first backup");
    assert_eq!(free_suffix(&database, stamp), ".broken-20261020T075412Z-2");
}

#[test]
fn the_write_ahead_log_goes_with_the_database() {
    let dir = TempDir::new().expect("temp dir");
    let database = damaged_file(&dir);
    let log = sibling(&database, WAL_SUFFIX);
    std::fs::write(&log, b"the pages the database file does not have").expect("write the log");
    let error = OpenError::Corrupt {
        path: database.clone(),
        detail: "database disk image is malformed".to_owned(),
    };

    let recovery = quarantine(&database, &error).expect("the file is set aside");

    // The log is not left behind: it would be applied to the fresh index.
    assert!(!log.exists(), "the log is not beside the database any more");
    let kept = sibling(&recovery.backup, WAL_SUFFIX);
    assert!(kept.is_file(), "the log lies beside the backup");
    assert_eq!(
        std::fs::read(&kept).expect("read the log"),
        b"the pages the database file does not have",
        "the log keeps its bytes"
    );
}

// ---------------------------------------------------------------------------
// The reason
// ---------------------------------------------------------------------------

#[test]
fn the_reason_is_written_next_to_the_backup() {
    let dir = TempDir::new().expect("temp dir");
    let database = damaged_file(&dir);
    let error = OpenError::Corrupt {
        path: database.clone(),
        detail: "database disk image is malformed".to_owned(),
    };

    let recovery = quarantine(&database, &error).expect("the file is set aside");
    let reason = recovery.reason.expect("the reason was written");

    let name = recovery
        .backup
        .file_name()
        .expect("the backup has a name")
        .to_string_lossy()
        .to_string();
    assert_eq!(
        reason
            .file_name()
            .expect("the reason has a name")
            .to_string_lossy(),
        format!("{name}.txt"),
        "the reason lies next to the backup"
    );

    let text = std::fs::read_to_string(&reason).expect("read the reason");
    assert!(
        text.contains(&error.to_string()),
        "the reason holds the typed message: {text}"
    );
    assert!(
        text.contains(&database.display().to_string()),
        "the reason holds the path of the database: {text}"
    );
    assert!(
        text.contains(&recovery.backup.display().to_string()),
        "the reason holds the path of the backup: {text}"
    );
    assert!(
        text.contains(&time_of(&name)),
        "the reason holds the time of the recovery: {text}"
    );
}

/// The time of a recovery, as the stamp of the backup's name spells it.
fn time_of(name: &str) -> String {
    let stamp = name
        .trim_start_matches("library.sqlite3.broken-")
        .trim_end_matches(".txt");
    format!(
        "{}-{}-{}T{}:{}:{}Z",
        &stamp[0..4],
        &stamp[4..6],
        &stamp[6..8],
        &stamp[9..11],
        &stamp[11..13],
        &stamp[13..15],
    )
}

// ---------------------------------------------------------------------------
// The note
// ---------------------------------------------------------------------------

#[test]
fn the_note_says_what_was_wrong_where_the_file_went_and_that_the_library_starts_empty() {
    let dir = TempDir::new().expect("temp dir");
    let database = damaged_file(&dir);
    let error = OpenError::Truncated {
        path: database.clone(),
        detail: "the header is cut short".to_owned(),
    };

    let recovery = quarantine(&database, &error).expect("the file is set aside");
    let note = note(&error, &recovery);

    assert!(
        note.contains(&error.to_string()),
        "the note says what was wrong: {note}"
    );
    assert!(
        note.contains(&recovery.backup.display().to_string()),
        "the note says where the old file went: {note}"
    );
    assert!(
        note.contains("starts empty"),
        "the note says that the library starts empty: {note}"
    );
    assert!(
        note.contains("Ctrl+O"),
        "the note says how the library is filled again: {note}"
    );
}

// ---------------------------------------------------------------------------
// The time
// ---------------------------------------------------------------------------

#[test]
fn the_utc_time_is_the_calendar_date() {
    assert_eq!(
        utc_time(SystemTime::UNIX_EPOCH),
        "1970-01-01T00:00:00Z",
        "the epoch is the first of January 1970"
    );
    assert_eq!(
        utc_time(SystemTime::UNIX_EPOCH + Duration::from_secs(946_684_800)),
        "2000-01-01T00:00:00Z",
        "the year 2000 is a leap year"
    );
    assert_eq!(
        utc_time(SystemTime::UNIX_EPOCH + Duration::from_secs(1_709_164_800)),
        "2024-02-29T00:00:00Z",
        "the leap day of a leap year is a day"
    );
    assert_eq!(
        utc_time(SystemTime::UNIX_EPOCH + Duration::from_secs(86_399)),
        "1970-01-01T23:59:59Z",
        "the last second of a day is in it"
    );
}

#[test]
fn the_calendar_knows_that_a_century_is_only_a_leap_year_in_the_fourth_century() {
    assert_eq!(
        calendar_date(-25_508),
        (1900, 3, 1),
        "1900 was no leap year, so the day after its February is the first of March"
    );
    assert_eq!(
        calendar_date(-25_567),
        (1900, 1, 1),
        "the first of January 1900"
    );
    assert_eq!(calendar_date(0), (1970, 1, 1), "the first of January 1970");
}

#[test]
fn the_stamp_is_the_time_without_its_marks() {
    assert_eq!(stamp_of("2026-10-20T07:54:12Z"), "20261020T075412Z");
}
