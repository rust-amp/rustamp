//! Skin containers: archives (`.wsz`/`.zip`), folders and in-memory sets.
//!
//! All three answer the same question — "give me the entry called `main.bmp`" —
//! with the same rules:
//!
//! * the match is case-insensitive and ignores any directory prefix, because
//!   classic skins ship with mixed case and sometimes in a subfolder;
//! * when several entries match, the last one in container order wins, which is
//!   what extracting an archive to a case-insensitive filesystem would do;
//! * entries whose path escapes the container are never indexed.

use std::collections::HashSet;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use crate::error::{Result, SkinError};

/// One entry read from a skin container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinFile {
    /// The stored entry name (for diagnostics).
    pub name: String,
    /// The decompressed bytes.
    pub bytes: Vec<u8>,
}

/// A container of named skin files.
pub trait SkinSource {
    /// Reads the first match for `base` with any of `exts` (case-insensitive,
    /// any directory prefix). The last matching entry wins.
    fn read_any(&mut self, base: &str, exts: &[&str]) -> Option<SkinFile>;

    /// The stored names, for diagnostics.
    fn list(&self) -> Vec<String>;

    /// A short human-readable description of where the skin came from.
    fn describe(&self) -> String;
}

/// Returns true when an entry name is safe to index.
fn is_safe_entry_name(name: &str) -> bool {
    let normalized = name.replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') || normalized.starts_with("//") {
        return false;
    }
    // A colon would allow a Windows drive letter or an alternate data stream.
    if normalized.contains(':') {
        return false;
    }
    normalized.split('/').all(|part| part != "..")
}

/// The lowercased file name (last path component).
fn basename_lowercase(name: &str) -> String {
    let normalized = name.replace('\\', "/");
    normalized
        .rsplit('/')
        .next()
        .unwrap_or(&normalized)
        .to_ascii_lowercase()
}

/// A skin loaded from a `.wsz` or `.zip` archive, held in memory.
pub struct ArchiveSource {
    archive: zip::ZipArchive<Cursor<Vec<u8>>>,
    /// `(lowercased basename, entry index)` in container order.
    index: Vec<(String, usize)>,
    name: String,
}

impl ArchiveSource {
    /// Opens an archive from a file path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        let name = path.display().to_string();
        Self::from_bytes(bytes, name)
    }

    /// Opens an archive from memory.
    pub fn from_bytes(bytes: Vec<u8>, name: String) -> Result<Self> {
        let archive = zip::ZipArchive::new(Cursor::new(bytes))?;
        let mut index = Vec::new();
        for i in 0..archive.len() {
            // zip 9 makes the entry name fallible: an undecodable name cannot
            // be looked up, so the entry is skipped exactly like an unsafe one.
            let Some(Ok(entry_name)) = archive.name_for_index(i) else {
                continue;
            };
            if entry_name.ends_with('/') {
                continue;
            }
            if !is_safe_entry_name(&entry_name) {
                continue;
            }
            index.push((basename_lowercase(&entry_name), i));
        }
        Ok(Self {
            archive,
            index,
            name,
        })
    }

    /// True when the archive holds no indexable entries.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

impl SkinSource for ArchiveSource {
    fn read_any(&mut self, base: &str, exts: &[&str]) -> Option<SkinFile> {
        let wanted: HashSet<String> = exts
            .iter()
            .map(|ext| format!("{}.{}", base.to_ascii_lowercase(), ext.to_ascii_lowercase()))
            .collect();
        let index = self
            .index
            .iter()
            .rev()
            .find(|(name, _)| wanted.contains(name))
            .map(|(_, i)| *i)?;
        let mut file = self
            .archive
            .by_index(index)
            .map_err(|e| SkinError::NotASkin(e.to_string()))
            .ok()?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).ok()?;
        Some(SkinFile {
            name: file.name().map(|n| n.into_owned()).unwrap_or_default(),
            bytes,
        })
    }

    fn list(&self) -> Vec<String> {
        self.archive
            .file_names()
            .filter_map(|n| n.ok())
            .map(|n| n.into_owned())
            .collect()
    }

    fn describe(&self) -> String {
        self.name.clone()
    }
}

/// A skin loaded from a folder.
pub struct FolderSource {
    root: PathBuf,
    /// `(lowercased basename, path)` in visit order.
    index: Vec<(String, PathBuf)>,
}

impl FolderSource {
    /// Opens a folder, indexing every readable file below it.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let root = std::fs::canonicalize(path)?;
        if !root.is_dir() {
            return Err(SkinError::NotASkin(path.display().to_string()));
        }
        let mut index: Vec<(String, PathBuf)> = Vec::new();
        collect_files(&root, &mut index, 0)?;
        Ok(Self { root, index })
    }

    /// The canonical folder path.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn collect_files(dir: &Path, index: &mut Vec<(String, PathBuf)>, depth: u32) -> Result<()> {
    // Guard against pathological nesting and symlink loops.
    if depth > 8 {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            // Reparse points and symlinks could escape the root; skip them.
            continue;
        }
        if metadata.is_dir() {
            collect_files(&path, index, depth + 1)?;
        } else if metadata.is_file()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            index.push((name.to_ascii_lowercase(), path));
        }
    }
    Ok(())
}

impl SkinSource for FolderSource {
    fn read_any(&mut self, base: &str, exts: &[&str]) -> Option<SkinFile> {
        let wanted: HashSet<String> = exts
            .iter()
            .map(|ext| format!("{}.{}", base.to_ascii_lowercase(), ext.to_ascii_lowercase()))
            .collect();
        let (_, path) = self
            .index
            .iter()
            .rev()
            .find(|(name, _)| wanted.contains(name))?;
        let bytes = std::fs::read(path).ok()?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        Some(SkinFile { name, bytes })
    }

    fn list(&self) -> Vec<String> {
        self.index
            .iter()
            .map(|(_, path)| path.display().to_string())
            .collect()
    }

    fn describe(&self) -> String {
        self.root.display().to_string()
    }
}

/// An in-memory skin container, used for tests and for generated defaults.
#[derive(Debug, Default, Clone)]
pub struct MemorySource {
    entries: Vec<(String, Vec<u8>)>,
    name: String,
}

impl MemorySource {
    /// Creates an empty container.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            entries: Vec::new(),
            name: name.into(),
        }
    }

    /// Adds an entry. Unsafe names are ignored.
    pub fn add(&mut self, name: impl Into<String>, bytes: Vec<u8>) {
        let name = name.into();
        if is_safe_entry_name(&name) {
            self.entries.push((name, bytes));
        }
    }

    /// Builds a container from `(name, bytes)` pairs.
    pub fn from_entries(
        name: impl Into<String>,
        entries: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) -> Self {
        let mut source = Self::new(name);
        for (entry_name, bytes) in entries {
            source.add(entry_name, bytes);
        }
        source
    }

    /// True when the container holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl SkinSource for MemorySource {
    fn read_any(&mut self, base: &str, exts: &[&str]) -> Option<SkinFile> {
        let wanted: HashSet<String> = exts
            .iter()
            .map(|ext| format!("{}.{}", base.to_ascii_lowercase(), ext.to_ascii_lowercase()))
            .collect();
        let (name, bytes) = self
            .entries
            .iter()
            .rev()
            .find(|(name, _)| wanted.contains(&basename_lowercase(name)))?;
        Some(SkinFile {
            name: name.clone(),
            bytes: bytes.clone(),
        })
    }

    fn list(&self) -> Vec<String> {
        self.entries.iter().map(|(name, _)| name.clone()).collect()
    }

    fn describe(&self) -> String {
        self.name.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_lookup_is_case_insensitive() {
        let mut source = MemorySource::new("mem");
        source.add("MAIN.BMP", vec![1, 2, 3]);
        let file = source.read_any("main", &["bmp"]).unwrap();
        assert_eq!(file.bytes, vec![1, 2, 3]);
        assert_eq!(file.name, "MAIN.BMP");
    }

    #[test]
    fn memory_duplicate_last_wins() {
        let mut source = MemorySource::new("mem");
        source.add("main.bmp", vec![1]);
        source.add("MAIN.BMP", vec![2]);
        assert_eq!(source.read_any("main", &["bmp"]).unwrap().bytes, vec![2]);
    }

    #[test]
    fn memory_matches_subfolder_basename() {
        let mut source = MemorySource::new("mem");
        source.add("some/dir/main.bmp", vec![9]);
        assert_eq!(source.read_any("main", &["bmp"]).unwrap().bytes, vec![9]);
    }

    #[test]
    fn memory_rejects_path_escapes() {
        let mut source = MemorySource::new("mem");
        source.add("../evil.bmp", vec![1]);
        source.add("/abs.bmp", vec![2]);
        source.add("c:/drive.bmp", vec![3]);
        assert!(source.read_any("evil", &["bmp"]).is_none());
        assert!(source.read_any("abs", &["bmp"]).is_none());
        assert!(source.read_any("drive", &["bmp"]).is_none());
    }
}
