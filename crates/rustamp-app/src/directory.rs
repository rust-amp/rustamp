// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! What a chosen directory contributes to the playlist.
//!
//! The "Open directory" action (S03 §1, command id 40187; the playlist
//! editor's "Add directory", S03 §2, id 1036, asks for the same thing) hands
//! the user the shell's folder dialog. The dialog answers with one path;
//! this module turns that path into the playlist entries the pick produces.
//!
//! # Depth: recursive (an own choice)
//!
//! No spec settles how deep the scan goes. S03 names the action and its
//! shortcut, S04 covers the menus that carry it, and S06 covers the
//! playlist editor's ADD and LIST OPTS dialogs; none of them says whether a
//! directory contributes the files it holds or the whole tree below it.
//! **Rustamp scans recursively**, because a music folder is usually a tree
//! (one folder per album, a "live" folder, a "remixes" folder) and one pick
//! should bring in the tree the user pointed at. The rule lives in one
//! function, so the decision has one place.
//!
//! # What counts as a media file
//!
//! Only the files the player's decoders claim: the player cannot play
//! anything else, and a playlist full of entries that can never be decoded is
//! a worse answer than a short one. A playlist file inside the directory is
//! **not** imported - the directory face adds entries, and loading a
//! playlist is the playlist editor's LIST OPTS path (RFC 0010).
//!
//! # Symlinks
//!
//! The walk never descends into a directory symlink (on Windows a junction
//! or a mount point counts as one), so a link that points at an ancestor
//! cannot turn the scan into an endless loop. A symlink to a *file* is a
//! media file like the file it points at and is added.
//!
//! # Cost
//!
//! The scan runs in the tick that asked for it, so a very large tree holds
//! that frame still. The pick is a modal action of the user, and a music
//! folder is small enough for that; a tree with hundreds of thousands of
//! files would show a pause.

use std::io;
use std::path::{Path, PathBuf};

use rustamp_core::command::Command;

/// The media files a directory holds, sorted by path.
///
/// The scan is recursive (see the module docs). `extensions` are the file
/// extensions the player can decode, without the leading dot; every other
/// file in the tree is left out.
///
/// The result is sorted, so the playlist order does not depend on the order
/// the file system happens to hand the entries out.
///
/// A directory that cannot be read at all is an error. A subdirectory that
/// cannot be read is reported and skipped: one unreadable corner must not
/// throw away the whole pick.
pub fn media_files(directory: &Path, extensions: &[String]) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect(directory, extensions, &mut files)?;
    files.sort();
    Ok(files)
}

/// The command that adds the chosen media files to the playlist.
///
/// `at` is `None`, so the files are **appended** to the playlist, and `play`
/// is the requesting command's flag, carried through untouched: the main
/// window's open actions start the first added entry, the playlist's ADD
/// only appends.
pub fn add_command(paths: &[PathBuf], play: bool) -> Command {
    Command::AddItems {
        items: rustamp_ui::media_items_for_paths(paths),
        at: None,
        play,
    }
}

/// Collects the media files of `directory` and of the tree below it.
fn collect(directory: &Path, extensions: &[String], files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        // An entry that cannot be read is skipped: the directory itself is
        // the caller's business, a single entry is not.
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            // A subdirectory that cannot be read is reported and skipped: one
            // unreadable corner must not throw away the whole pick.
            if let Err(error) = collect(&path, extensions, files) {
                eprintln!(
                    "rustamp: the subdirectory {} could not be read: {error}",
                    path.display()
                );
            }
            continue;
        }
        if file_type.is_symlink() {
            // A link to a file counts like the file it points at; a link to a
            // directory is not descended into, so the scan cannot loop.
            let Ok(target) = std::fs::metadata(&path) else {
                continue;
            };
            if !target.is_file() {
                continue;
            }
        } else if !file_type.is_file() {
            continue;
        }
        if is_media_file(&path, extensions) {
            files.push(path);
        }
    }
    Ok(())
}

/// Whether `path` names a file the player can decode.
///
/// The extension decides, case-insensitively; a name without an extension
/// is not a media file.
fn is_media_file(path: &Path, extensions: &[String]) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    extensions
        .iter()
        .any(|known| known.eq_ignore_ascii_case(extension))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// The extensions the tests pretend the player can decode.
    fn extensions() -> Vec<String> {
        ["mp3", "flac", "ogg"]
            .iter()
            .map(|extension| extension.to_string())
            .collect()
    }

    /// Writes an empty file, creating the directories above it.
    fn write(directory: &Path, name: &str) -> PathBuf {
        let path = directory.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, b"").unwrap();
        path
    }

    /// A directory contributes the media files of the whole tree below it.
    #[test]
    fn a_directory_contributes_the_media_files_of_the_whole_tree() {
        let dir = TempDir::new().unwrap();
        let first = write(dir.path(), "a.mp3");
        write(dir.path(), "notes.txt");
        let second = write(dir.path(), "album/b.flac");
        let third = write(dir.path(), "album/disc 2/c.ogg");
        assert_eq!(
            media_files(dir.path(), &extensions()).unwrap(),
            vec![first, second, third],
            "the scan is recursive and leaves the other files out"
        );
    }

    /// A directory without a media file contributes nothing.
    #[test]
    fn a_directory_without_media_files_contributes_nothing() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "notes.txt");
        assert!(media_files(dir.path(), &extensions()).unwrap().is_empty());
    }

    /// A folder that is named like a track is a folder, not a track.
    #[test]
    fn a_folder_named_like_a_track_is_not_a_track() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("album.mp3")).unwrap();
        assert!(media_files(dir.path(), &extensions()).unwrap().is_empty());
    }

    /// A directory that is not there is an error, not an empty pick.
    #[test]
    fn a_missing_directory_is_an_error() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("nothing-here");
        assert!(media_files(&missing, &extensions()).is_err());
    }

    /// A file is not a directory.
    #[test]
    fn a_file_is_not_a_directory() {
        let dir = TempDir::new().unwrap();
        let file = write(dir.path(), "a.mp3");
        assert!(media_files(&file, &extensions()).is_err());
    }

    /// The extension decides what counts as media.
    #[test]
    fn the_extension_decides_what_is_media() {
        let extensions = extensions();
        assert!(is_media_file(Path::new("song.MP3"), &extensions));
        assert!(!is_media_file(Path::new("song.mp3.bak"), &extensions));
        assert!(!is_media_file(Path::new("mp3"), &extensions));
        assert!(!is_media_file(Path::new("notes.txt"), &extensions));
    }

    /// The chosen files are appended, and the `play` flag is carried through.
    #[test]
    fn the_chosen_files_are_appended_and_the_play_flag_is_kept() {
        let paths = vec![
            PathBuf::from("C:/music/a.mp3"),
            PathBuf::from("C:/music/b.mp3"),
        ];
        match add_command(&paths, true) {
            Command::AddItems { items, at, play } => {
                assert_eq!(at, None, "the chosen files are appended");
                assert!(play, "the requesting command's play flag is kept");
                assert_eq!(items.len(), 2, "one entry per chosen file");
                assert!(
                    matches!(&items[0], rustamp_core::playlist::MediaLocation::File(path) if path == &paths[0])
                );
                assert!(
                    matches!(&items[1], rustamp_core::playlist::MediaLocation::File(path) if path == &paths[1])
                );
            }
            other => panic!("the pick must add the files: {other:?}"),
        }
    }
}
