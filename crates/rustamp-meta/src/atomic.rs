//! Atomic file replacement.
//!
//! A tag write must never leave a half-written media file behind: a crash
//! or a failing encoder in the middle of a write would destroy the audio
//! the user still has. Every write therefore goes through
//! [`replace_atomically`], which builds the new file next to the old one and
//! puts it in place with a single rename.

use std::fs::File;
use std::io::{self, Seek, Write};
use std::path::Path;

use rustamp_core::error::{Error, Result};

/// Replaces the content of `path` with whatever `write` puts into the file
/// handle.
///
/// The new content is written into a temporary file **next to** `path` — the
/// same directory, so the final rename stays inside one volume and is
/// atomic — flushed to the disk and only then renamed over `path`. The
/// original file is therefore either the old one or the new one, never a
/// mixture, and a failing write leaves it untouched.
///
/// The temporary file inherits the permissions of the original, so replacing
/// a file does not change who may read it.
///
/// # Errors
///
/// * [`Error::Io`] if the file cannot be read, written, or replaced.
pub(crate) fn replace_atomically(
    path: &Path,
    write: impl FnOnce(&mut File) -> Result<()>,
) -> Result<()> {
    let directory = parent_directory(path);
    let permissions = std::fs::metadata(path)?.permissions();

    let mut temp = tempfile::NamedTempFile::new_in(directory)?;

    {
        let mut source = File::open(path)?;
        io::copy(&mut source, temp.as_file_mut())?;
    }

    {
        let file = temp.as_file_mut();
        file.rewind()?;
        write(file)?;
        file.flush()?;
        file.sync_all()?;
    }

    // The replacement keeps the permissions of the original file.
    std::fs::set_permissions(temp.path(), permissions)?;

    match temp.persist(path) {
        Ok(_) => Ok(()),
        // The error carries the temporary file, which is removed when it is
        // dropped; the original stays as it was.
        Err(error) => Err(Error::Io(error.error)),
    }
}

/// The directory `path` lives in.
///
/// A path without a directory component (a bare file name) belongs to the
/// current directory.
fn parent_directory(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_replaces_the_file_and_leaves_no_temporary_behind() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("song.mp3");
        std::fs::write(&path, b"old content").unwrap();

        replace_atomically(&path, |file| {
            file.write_all(b"new content")?;
            Ok(())
        })
        .unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new content");
        // Only the file itself is left in the directory.
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 1, "{entries:?}");
    }

    #[test]
    fn a_failing_write_leaves_the_original_alone() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("song.mp3");
        std::fs::write(&path, b"old content").unwrap();

        let error = replace_atomically(&path, |file| {
            file.write_all(b"half written")?;
            Err(Error::Other("the encoder gave up".into()))
        })
        .unwrap_err();

        assert!(matches!(error, Error::Other(_)), "{error:?}");
        assert_eq!(std::fs::read(&path).unwrap(), b"old content");
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 1, "{entries:?}");
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("missing.mp3");

        let error = replace_atomically(&path, |_| Ok(())).unwrap_err();
        assert!(matches!(error, Error::Io(_)), "{error:?}");
        assert!(!path.exists());
    }
}
