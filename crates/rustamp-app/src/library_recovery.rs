// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The way out of a damaged media library database.
//!
//! The index of the media library is the one thing the user cannot rebuild by
//! hand, and a damaged index is a dead end: the open is refused with a typed
//! reason and the file is left exactly as it was. That is the right rule for
//! the crate that holds the file - but it is useless on its own, because
//! nothing else happens: the user reads the message and the library stays
//! dead.
//!
//! So the app offers a way out, with three rules:
//!
//! * **The file is renamed, never deleted.** It becomes
//!   `<database>.broken-<UTC stamp>` and keeps its bytes: the user can look
//!   at it, keep it, or hand it to whoever repairs databases.
//! * **The reason is written next to it**, in `<database>.broken-<stamp>.txt`:
//!   what was wrong, which file it was, and when it was set aside.
//! * **A fresh index takes the place**, and the user is told once: what was
//!   wrong, where the old file went, and that the library starts empty.
//!
//! A write-ahead log beside the database goes with it: it holds the pages
//! the database file does not have yet, and a log that is left behind would
//! be applied to the fresh index.
//!
//! Only the damage that is about the **file** is recovered. The damage that
//! is about the **environment** - a file another process holds, a read-only
//! file, a file that cannot be read at all, a path that is not a file, a
//! database that could not be prepared - is refused: a fresh index would
//! hide those, so the file is left exactly as it was and the reason is
//! reported instead.
//!
//! The recovery happens **once**: the fresh index is a good database, so the
//! next open finds nothing to recover. Nothing here loops, and nothing here
//! is silent - every recovery leaves a note for the user.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rustamp_library::OpenError;

#[cfg(test)]
#[path = "library_recovery_tests.rs"]
mod library_recovery_tests;

/// The suffix of the name a damaged database is renamed to.
const BROKEN: &str = ".broken";

/// The suffix of the name of the file that holds the reason.
const REASON_SUFFIX: &str = ".txt";

/// The suffix of the write-ahead log of a database, beside it.
///
/// The log belongs to the database: it holds the pages the database file does
/// not have yet. It is set aside with the database, because a log that is
/// left behind would be applied to the fresh index that takes its place.
/// The shared memory beside the log is a cache that SQLite rebuilds, so it
/// can stay where it is.
const WAL_SUFFIX: &str = "-wal";

/// How many names are tried before the recovery gives up.
///
/// Two recoveries in the same second must not fight over the same name: the
/// second one gets `-2`, the third `-3`. The bound only exists so that the
/// search ends; a directory with a thousand backups of the same second is
/// not a thing a user has.
const MAX_NAMES: u32 = 1000;

/// What the recovery did.
pub(crate) struct Recovery {
    /// The path the damaged database was renamed to.
    ///
    /// The file keeps its bytes: the user has it, and the app never writes
    /// to it again.
    pub(crate) backup: PathBuf,
    /// The path of the file that holds the reason, if it could be written.
    ///
    /// `None` when the directory would not take the file. The recovery then
    /// still happened - the old file is safe and the library works - and the
    /// note says that the reason is only in the note.
    pub(crate) reason: Option<PathBuf>,
}

/// Whether a fresh index is the right answer for this damage.
///
/// The damage that is about the **file** is recovered: the file is set aside
/// and a fresh index takes its place. The damage that is about the
/// **environment** is not: another process holds the file, the file is
/// read-only, the file cannot be read, the path is not a file at all, the
/// database could not be prepared - a fresh index would hide all of them, so
/// the open is refused and the file is left alone.
pub(crate) fn is_recoverable(error: &OpenError) -> bool {
    matches!(
        error,
        OpenError::NotADatabase { .. }
            | OpenError::Truncated { .. }
            | OpenError::Corrupt { .. }
            | OpenError::ForeignSchema { .. }
            | OpenError::FutureVersion { .. }
    )
}

/// Sets the damaged database at `database` aside.
///
/// The file is **renamed, never deleted**: it becomes
/// `<database>.broken-<UTC stamp>` and keeps its bytes. The reason is
/// written next to it, in `<database>.broken-<stamp>.txt`. A write-ahead log
/// beside the database goes with it, so that it cannot reach the fresh
/// index.
///
/// The rename is the step that can fail: if it does, the file is still where
/// it was, and the caller reports the failure. The reason file is written
/// afterwards, and a directory that would not take it does not stop the
/// recovery - the note then says that the reason is only in the note.
pub(crate) fn quarantine(database: &Path, error: &OpenError) -> io::Result<Recovery> {
    let time = utc_time(SystemTime::now());
    let suffix = free_suffix(database, &stamp_of(&time));
    let backup = sibling(database, &suffix);
    let reason = sibling(database, &format!("{suffix}{REASON_SUFFIX}"));

    // The write-ahead log goes with the database. It is moved first: if it
    // cannot be moved, nothing has changed yet, and the file that is still
    // there is the one the user knows.
    let log = sibling(database, WAL_SUFFIX);
    if log.exists() {
        fs::rename(&log, sibling(&backup, WAL_SUFFIX))?;
    }

    // The rename is the point of the whole thing: the old file keeps its
    // bytes, under a name that says what happened to it.
    fs::rename(database, &backup)?;

    let written = fs::write(&reason, reason_text(database, error, &backup, &time)).is_ok();
    Ok(Recovery {
        backup,
        reason: written.then_some(reason),
    })
}

/// The note the user sees after a recovery.
///
/// The note says three things: what was wrong, where the old file went, and
/// that the library starts empty. It is the only place the user learns about
/// the recovery, so it carries the whole story.
pub(crate) fn note(error: &OpenError, recovery: &Recovery) -> String {
    let mut note = format!(
        "the media library database could not be used and has been set aside: {error}. \
         The old file was kept as {backup}.",
        error = error,
        backup = recovery.backup.display(),
    );
    match &recovery.reason {
        Some(reason) => note.push_str(&format!(" The reason is in {}.", reason.display())),
        None => note.push_str(" The reason could not be written next to it."),
    }
    note.push_str(" The library starts empty: add your folders again (Ctrl+O) to fill it.");
    note
}

/// The text of the file that holds the reason.
///
/// The file carries the typed message of the error, the path of the database
/// that was set aside, the path it was renamed to, and the time of the
/// recovery.
fn reason_text(database: &Path, error: &OpenError, backup: &Path, time: &str) -> String {
    format!(
        "Rustamp media library: the database could not be used and was set aside.\n\
         \n\
         reason: {error}\n\
         database: {database}\n\
         backup: {backup}\n\
         time (UTC): {time}\n",
        error = error,
        database = database.display(),
        backup = backup.display(),
    )
}

/// The suffix of the name the backup gets, free of any earlier backup.
///
/// The name is `.broken-<stamp>`: the stamp keeps the backups in the order
/// they were made, and the name says that the file is not to be used any
/// more. A name that is taken (a second recovery in the same second) gets a
/// number behind it.
fn free_suffix(database: &Path, stamp: &str) -> String {
    let base = format!("{BROKEN}-{stamp}");
    if !sibling(database, &base).exists() {
        return base;
    }
    for count in 2..MAX_NAMES {
        let candidate = format!("{base}-{count}");
        if !sibling(database, &candidate).exists() {
            return candidate;
        }
    }
    base
}

/// The path of the file next to `database` whose name ends in `suffix`.
fn sibling(database: &Path, suffix: &str) -> PathBuf {
    let mut name = database
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    database.with_file_name(name)
}

/// The UTC time of `now`, as `2026-10-20T07:54:12Z`.
///
/// The time is the UTC one, so the name of a backup says the same thing
/// everywhere.
fn utc_time(now: SystemTime) -> String {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (year, month, day) = calendar_date(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60,
    )
}

/// The stamp of a UTC time in a file name: `20261020T075412Z`.
///
/// The stamp is the time without the marks that a file name does not like.
fn stamp_of(time: &str) -> String {
    time.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect()
}

/// The calendar date of the `days`-th day since 1970-01-01.
///
/// The proleptic Gregorian calendar: the leap years are the ones the rule of
/// the 400, 100 and 4 years gives, and the first of January 1970 is day
/// zero. The years are walked instead of computed, because the recovery runs
/// once per damaged database and the walk cannot be wrong.
fn calendar_date(days: i64) -> (i64, u32, u32) {
    let mut year = 1970;
    let mut rest = days;
    while rest >= year_length(year) {
        rest -= year_length(year);
        year += 1;
    }
    while rest < 0 {
        year -= 1;
        rest += year_length(year);
    }
    let mut month = 1;
    for length in month_lengths(year) {
        if rest < length {
            break;
        }
        rest -= length;
        month += 1;
    }
    (year, month, (rest + 1) as u32)
}

/// The number of days of the year `year`.
fn year_length(year: i64) -> i64 {
    if is_leap(year) { 366 } else { 365 }
}

/// Whether `year` is a leap year.
fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// The number of days of the months of `year`.
fn month_lengths(year: i64) -> [i64; 12] {
    [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ]
}
