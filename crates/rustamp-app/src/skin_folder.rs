// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The skins folder: the skins the player can load from disk.
//!
//! # The folder
//!
//! The skins live in a `skins` folder **beside the configuration file** — the
//! same place the media library's database and `presets.eqf` live. The player
//! never creates that folder: a missing folder is the normal "no skins yet"
//! case, and a player that wrote into the user's configuration folder behind
//! their back would be a surprise.
//!
//! # What is a skin
//!
//! Two kinds of entry in the folder are skins:
//!
//! * a **`.wsz` archive** — the classic skin file. It must be a readable zip
//!   archive that holds a `main.bmp` (the classic main-window sheet) and it
//!   must not be larger than [`MAX_ARCHIVE_BYTES`]. A file that is not one is
//!   skipped and reported, never offered.
//! * a **skin folder** — a subfolder that holds a `main.bmp`. A subfolder
//!   without one is not a skin and is ignored: the user may keep other files
//!   in subfolders.
//!
//! Everything else in the folder is ignored. The list is sorted by the name
//! of the entry, so the order does not depend on the order the file system
//! hands the entries out in.
//!
//! # The cache
//!
//! The folder is read when the skin list is asked for and kept until the
//! folder changes. The change is the folder's modification time, so a skin
//! that is added, removed or renamed shows up right away, while a folder
//! nobody touches is read once. A file's *content* changing does not move
//! the folder's time: the list says which skins exist, not what they hold.
//! A file that is replaced in place (the same name, new content) therefore
//! keeps the entry the list had — the folder has to change for the list to
//! be read again.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rustamp_skin::{ArchiveSource, FolderSource, SkinSource};

/// The name of the folder beside the configuration that holds the skins.
pub const SKINS_FOLDER: &str = "skins";

/// The classic main-window sheet that every skin carries.
const MAIN_SHEET: &str = "main.bmp";

/// The largest archive the scan reads. Own choice: a classic skin is a few
/// hundred kilobytes, and the scan reads the file into memory to look at its
/// entries — a file that is not a skin must not cost gigabytes.
pub const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

/// The skins folder beside `config_path`.
///
/// `None` when there is no configuration file (nothing to sit beside) or
/// when the path has no parent (a root path).
pub fn skins_dir(config_path: Option<&Path>) -> Option<PathBuf> {
    let parent = config_path?.parent()?;
    Some(parent.join(SKINS_FOLDER))
}

/// One read of the skins folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkinScan {
    /// The skins the folder holds, sorted by name.
    pub skins: Vec<PathBuf>,
    /// The entries that were skipped, with the reason.
    pub problems: Vec<String>,
}

/// Reads `dir` and returns the skins it holds.
///
/// A `.wsz` file that is not a readable skin archive is skipped and reported
/// in [`problems`](Self::problems); a folder that does not exist holds no
/// skins and is not a problem.
pub fn scan(dir: &Path) -> SkinScan {
    let mut result = SkinScan::default();
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        // A folder that does not exist is the normal "no skins yet".
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return result,
        Err(error) => {
            result.problems.push(format!(
                "the skins folder {} could not be read: {error}",
                dir.display()
            ));
            return result;
        }
    };
    for item in listing.flatten() {
        let path = item.path();
        // The metadata follows a link, so a linked skin is a skin like any
        // other; a link that points nowhere is not a skin.
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if metadata.is_file() {
            if is_archive(&path) {
                read_archive(&path, metadata.len(), &mut result);
            }
        } else if metadata.is_dir() && holds_main_sheet(&path) {
            read_folder(&path, &mut result);
        }
    }
    result.skins.sort_by_cached_key(|path| {
        let name = file_name(path);
        (name.to_lowercase(), name.to_owned())
    });
    result.problems.sort();
    result
}

/// Whether `path` is a `.wsz` archive.
fn is_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("wsz"))
}

/// Reads a `.wsz` archive: a skin when it is a readable archive with a
/// `main.bmp` and a size a skin can have, a problem when it is not.
fn read_archive(path: &Path, size: u64, result: &mut SkinScan) {
    // The scan reads the archive into memory to look at its entries, so a
    // file that is far too big to be a skin is refused before it is read.
    if size > MAX_ARCHIVE_BYTES {
        result.problems.push(format!(
            "{} was skipped: it is larger than {} MiB",
            path.display(),
            MAX_ARCHIVE_BYTES >> 20
        ));
        return;
    }
    match ArchiveSource::open(path) {
        Ok(source) if !holds_main_sheet_in(&source.list()) => {
            result.problems.push(format!(
                "{} was skipped: the archive carries no {MAIN_SHEET}",
                path.display()
            ));
        }
        Ok(_) => result.skins.push(path.to_path_buf()),
        Err(error) => result.problems.push(format!(
            "{} was skipped: it is not a readable skin archive ({error})",
            path.display()
        )),
    }
}

/// Reads a skin folder: a skin when it can be read, a problem when it cannot.
fn read_folder(path: &Path, result: &mut SkinScan) {
    match FolderSource::open(path) {
        Ok(_) => result.skins.push(path.to_path_buf()),
        Err(error) => result.problems.push(format!(
            "{} was skipped: the skin folder could not be read ({error})",
            path.display()
        )),
    }
}

/// Whether the folder holds a `main.bmp` at its top level.
fn holds_main_sheet(dir: &Path) -> bool {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return false;
    };
    listing.flatten().any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(MAIN_SHEET)
    })
}

/// Whether the archive holds a `main.bmp`.
///
/// The loader ignores the folder an entry sits in, so the name is compared
/// without its path.
fn holds_main_sheet_in(names: &[String]) -> bool {
    names
        .iter()
        .any(|name| basename(name).eq_ignore_ascii_case(MAIN_SHEET))
}

/// The last part of an archive entry's name.
fn basename(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// The name of the entry, as the list is sorted by it.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The cached read of the skins folder.
///
/// The folder is read when the skin list is asked for and kept until the
/// folder changes (see the module docs): the list is not read again for
/// every frame.
#[derive(Debug, Default)]
pub(crate) struct SkinFolder {
    /// The folder the cache describes.
    dir: Option<PathBuf>,
    /// The folder's modification time when it was read.
    stamp: Option<SystemTime>,
    /// The read's result; `None` while the folder was never read.
    scan: Option<SkinScan>,
    /// The problems of the last read that the app has not reported yet.
    pending: Vec<String>,
}

impl SkinFolder {
    /// The skins of `dir`, reading the folder when it changed.
    ///
    /// `dir` is `None` when the player does not know where its skins live
    /// (no configuration file): the list is then empty.
    pub(crate) fn skins(&mut self, dir: Option<&Path>) -> &[PathBuf] {
        if self.changed(dir) {
            self.read(dir);
        }
        self.scan.as_ref().map_or(&[], |scan| scan.skins.as_slice())
    }

    /// Takes the problems of the last read, so the app can report them.
    pub(crate) fn take_problems(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }

    /// Whether the folder changed since the last read.
    fn changed(&self, dir: Option<&Path>) -> bool {
        if self.scan.is_none() || self.dir != dir.map(Path::to_path_buf) {
            return true;
        }
        stamp_of(dir) != self.stamp
    }

    /// Reads the folder and keeps the result.
    fn read(&mut self, dir: Option<&Path>) {
        self.dir = dir.map(Path::to_path_buf);
        self.stamp = stamp_of(dir);
        self.scan = Some(match dir {
            Some(dir) => scan(dir),
            None => SkinScan::default(),
        });
        self.pending = self
            .scan
            .as_ref()
            .map(|scan| scan.problems.clone())
            .unwrap_or_default();
    }
}

/// The folder's modification time, when it can be read.
fn stamp_of(dir: Option<&Path>) -> Option<SystemTime> {
    let dir = dir?;
    std::fs::metadata(dir).ok()?.modified().ok()
}

/// Whether two paths name the same skin.
///
/// The paths are compared as they are first; when they differ, their
/// canonical forms decide, so a skin that was loaded through another
/// spelling of the same path is still recognised. Two spellings of one path
/// carry the same file name, and that check comes first: without it every
/// skin of the folder would cost a canonicalization, for every frame the
/// list is drawn.
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if !same_file_name(a, b) {
        return false;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Whether the two paths name the same file, ignoring the case as Windows
/// does.
fn same_file_name(a: &Path, b: &Path) -> bool {
    match (a.file_name(), b.file_name()) {
        (Some(a), Some(b)) => a
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()),
        _ => false,
    }
}

/// Writes a `.wsz` archive that holds a `main.bmp`, so the scan takes it for
/// a skin.
///
/// The tests of the app use it too: the scan must see the same files a user
/// drops into the folder.
#[cfg(test)]
pub(crate) fn write_skin_archive(path: &Path) {
    use std::io::Write;

    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file(
            MAIN_SHEET,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"not a real sheet").unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    std::fs::write(path, bytes).unwrap();
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_skins_folder_sits_beside_the_configuration() {
        let config = Path::new("C:/players/rustamp.toml");
        assert_eq!(
            skins_dir(Some(config)),
            Some(PathBuf::from("C:/players/skins"))
        );
        // A configuration file in the working directory: the folder is
        // beside it, so it is beside the working directory too.
        assert_eq!(
            skins_dir(Some(Path::new("rustamp.toml"))),
            Some(PathBuf::from("skins"))
        );
        // Without a configuration file there is nothing to sit beside.
        assert_eq!(skins_dir(None), None);
    }

    #[test]
    fn an_empty_folder_holds_no_skin() {
        let dir = tempfile::tempdir().unwrap();
        let scan = scan(dir.path());
        assert!(scan.skins.is_empty(), "{scan:?}");
        assert!(scan.problems.is_empty(), "{scan:?}");
    }

    #[test]
    fn a_folder_that_does_not_exist_is_not_a_problem() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("skins");
        let scan = scan(&missing);
        assert!(scan.skins.is_empty(), "{scan:?}");
        assert!(scan.problems.is_empty(), "{scan:?}");
    }

    #[test]
    fn two_archives_are_listed_in_name_order() {
        let dir = tempfile::tempdir().unwrap();
        write_skin_archive(&dir.path().join("Beta.wsz"));
        write_skin_archive(&dir.path().join("alpha.wsz"));
        let scan = scan(dir.path());
        let names: Vec<String> = scan.skins.iter().map(|path| file_name(path)).collect();
        assert_eq!(names, vec!["alpha.wsz", "Beta.wsz"], "{scan:?}");
        assert!(scan.problems.is_empty(), "{scan:?}");
    }

    #[test]
    fn a_broken_archive_is_skipped_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.wsz"), b"not a zip at all").unwrap();
        let scan = scan(dir.path());
        assert!(scan.skins.is_empty(), "{scan:?}");
        assert_eq!(scan.problems.len(), 1, "{scan:?}");
        assert!(scan.problems[0].contains("broken.wsz"), "{scan:?}");
    }

    #[test]
    fn an_archive_without_a_main_sheet_is_skipped_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file(
                "readme.txt",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(b"no skin here").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        std::fs::write(dir.path().join("empty.wsz"), bytes).unwrap();
        let scan = scan(dir.path());
        assert!(scan.skins.is_empty(), "{scan:?}");
        assert_eq!(scan.problems.len(), 1, "{scan:?}");
        assert!(scan.problems[0].contains("empty.wsz"), "{scan:?}");
    }

    #[test]
    fn a_skin_folder_is_listed_and_another_folder_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let skin = dir.path().join("MySkin");
        std::fs::create_dir(&skin).unwrap();
        std::fs::write(skin.join(MAIN_SHEET), b"").unwrap();
        let other = dir.path().join("notes");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("readme.txt"), b"").unwrap();
        let scan = scan(dir.path());
        assert_eq!(scan.skins, vec![skin], "{scan:?}");
        assert!(scan.problems.is_empty(), "{scan:?}");
    }

    #[test]
    fn a_file_that_is_not_a_skin_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"").unwrap();
        let scan = scan(dir.path());
        assert!(scan.skins.is_empty(), "{scan:?}");
        assert!(scan.problems.is_empty(), "{scan:?}");
    }

    #[test]
    fn an_archive_that_is_too_big_to_be_a_skin_is_skipped_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        // The file is extended, not written: the scan must refuse it before
        // it reads it.
        let file = std::fs::File::create(dir.path().join("huge.wsz")).unwrap();
        file.set_len(MAX_ARCHIVE_BYTES + 1).unwrap();
        drop(file);
        let scan = scan(dir.path());
        assert!(scan.skins.is_empty(), "{scan:?}");
        assert_eq!(scan.problems.len(), 1, "{scan:?}");
        assert!(scan.problems[0].contains("huge.wsz"), "{scan:?}");
    }

    #[test]
    fn an_unchanged_folder_is_not_read_again() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.wsz"), b"not a zip at all").unwrap();
        let mut folder = SkinFolder::default();
        assert!(folder.skins(Some(dir.path())).is_empty());
        // The problem of the read is reported once.
        assert_eq!(folder.take_problems().len(), 1);
        assert!(folder.take_problems().is_empty());
        // The folder did not change, so it is not read again and nothing is
        // reported a second time.
        assert!(folder.skins(Some(dir.path())).is_empty());
        assert!(folder.take_problems().is_empty());
    }

    #[test]
    fn a_changed_folder_is_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut folder = SkinFolder::default();
        assert!(folder.skins(Some(dir.path())).is_empty());
        // The clock's resolution is coarse, so the file is written after a
        // short pause: the folder's time must move for the read to happen.
        std::thread::sleep(Duration::from_millis(20));
        write_skin_archive(&dir.path().join("one.wsz"));
        let skins = folder.skins(Some(dir.path()));
        assert_eq!(skins.len(), 1, "{skins:?}");
        assert_eq!(skins[0], dir.path().join("one.wsz"));
    }

    #[test]
    fn another_folder_is_read_again() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        write_skin_archive(&first.path().join("one.wsz"));
        let mut folder = SkinFolder::default();
        assert_eq!(folder.skins(Some(first.path())).len(), 1);
        // The cache belongs to one folder: another folder is read.
        assert!(folder.skins(Some(second.path())).is_empty());
        assert_eq!(folder.skins(Some(first.path())).len(), 1);
    }

    #[test]
    fn the_same_skin_in_another_spelling_is_the_same_skin() {
        let dir = tempfile::tempdir().unwrap();
        let skin = dir.path().join("one.wsz");
        std::fs::write(&skin, b"").unwrap();
        assert!(same_path(&skin, &skin));
        assert!(same_path(&skin, &dir.path().join(".").join("one.wsz")));
        assert!(!same_path(&skin, &dir.path().join("two.wsz")));

        // The same file name in another folder is another skin.
        let other = dir.path().join("sub");
        std::fs::create_dir(&other).unwrap();
        let twin = other.join("one.wsz");
        std::fs::write(&twin, b"").unwrap();
        assert!(!same_path(&skin, &twin));
    }
}
