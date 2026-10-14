//! [`LoadedEntry`]: one entry as it comes out of a playlist file, plus the
//! helpers that move entries into the frozen core [`Playlist`].
//!
//! Loading always goes through [`Playlist::insert_many`] /
//! [`Playlist::push`], so the core keeps ownership of id allocation and entry
//! ids stay stable and are never reused. Cached title and duration are written
//! afterwards through [`Playlist::get_mut`].

use rustamp_core::playlist::{EntryId, MediaLocation, Playlist, PlaylistEntry};

/// One playlist entry read from a file, before it is inserted into a
/// [`Playlist`].
///
/// Only what the file itself contains is filled in: `#EXTINF`, PLS `TitleN`
/// and `LengthN`, XSPF `title` and `duration`. No title formatting (spec S14)
/// is applied here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedEntry {
    /// Where the media lives, already resolved to an absolute path or a URL.
    pub location: MediaLocation,
    /// Title from the file, if any.
    pub title: Option<String>,
    /// Duration from the file, in milliseconds, if any.
    pub duration_ms: Option<u64>,
}

impl LoadedEntry {
    /// An entry without title or duration.
    pub fn new(location: MediaLocation) -> Self {
        Self {
            location,
            title: None,
            duration_ms: None,
        }
    }

    /// Sets the title; an empty or whitespace-only title is stored as `None`.
    pub fn with_title(mut self, title: &str) -> Self {
        self.title = normalize_title(title);
        self
    }

    /// Sets the duration.
    pub fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    /// Copies location, title and duration out of a core playlist entry.
    pub fn from_playlist_entry(entry: &PlaylistEntry) -> Self {
        Self {
            location: entry.location.clone(),
            title: entry.title.clone(),
            duration_ms: entry.duration_ms,
        }
    }
}

/// Trims a title and maps the empty result to `None`.
///
/// Every reader uses this, and the writers trim too, so an empty title never
/// survives a write → read round-trip as `Some("")`.
pub fn normalize_title(title: &str) -> Option<String> {
    let trimmed = title.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Snapshots a playlist into [`LoadedEntry`] values, in playlist order.
pub fn loaded_entries(playlist: &Playlist) -> Vec<LoadedEntry> {
    playlist
        .iter()
        .map(LoadedEntry::from_playlist_entry)
        .collect()
}

/// Appends entries at the end of the playlist, returning their new ids.
pub fn append_loaded(playlist: &mut Playlist, entries: &[LoadedEntry]) -> Vec<EntryId> {
    insert_loaded(playlist, playlist.len(), entries)
}

/// Inserts entries at `index` (clamped), returning their new ids.
///
/// The locations go in through [`Playlist::insert_many`]; the cached title and
/// duration are then written with [`Playlist::get_mut`].
pub fn insert_loaded(
    playlist: &mut Playlist,
    index: usize,
    entries: &[LoadedEntry],
) -> Vec<EntryId> {
    let ids = playlist.insert_many(index, entries.iter().map(|e| e.location.clone()));
    for (id, entry) in ids.iter().zip(entries) {
        if entry.title.is_none() && entry.duration_ms.is_none() {
            continue;
        }
        if let Some(slot) = playlist.get_mut(*id) {
            slot.title = entry.title.clone();
            slot.duration_ms = entry.duration_ms;
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn entry(name: &str) -> LoadedEntry {
        LoadedEntry::new(MediaLocation::File(PathBuf::from(name)))
    }

    #[test]
    fn normalize_title_trims_and_drops_empty() {
        assert_eq!(normalize_title("  a  "), Some("a".to_owned()));
        assert_eq!(normalize_title(""), None);
        assert_eq!(normalize_title("   "), None);
        assert_eq!(normalize_title("\t"), None);
    }

    #[test]
    fn append_uses_the_core_id_allocation() {
        let mut p = Playlist::new();
        let a = p.push(MediaLocation::File(PathBuf::from("a.mp3")));
        let entries = vec![
            entry("b.mp3").with_title("B").with_duration_ms(1000),
            entry("c.mp3"),
        ];
        let ids = append_loaded(&mut p, &entries);
        assert_eq!(ids.len(), 2);
        assert!(ids[0] > a && ids[1] > ids[0], "ids keep growing");
        assert_eq!(p.len(), 3);
        assert_eq!(p.get(ids[0]).unwrap().title.as_deref(), Some("B"));
        assert_eq!(p.get(ids[0]).unwrap().duration_ms, Some(1000));
        assert_eq!(p.get(ids[1]).unwrap().title, None);
    }

    #[test]
    fn insert_places_entries_and_sets_cached_metadata() {
        let mut p = Playlist::new();
        p.push(MediaLocation::File(PathBuf::from("first.mp3")));
        p.push(MediaLocation::File(PathBuf::from("last.mp3")));
        let entries = vec![entry("mid.mp3").with_title("Mid")];
        let ids = insert_loaded(&mut p, 1, &entries);
        let names: Vec<_> = p.iter().map(|e| e.location.file_name()).collect();
        assert_eq!(names, ["first.mp3", "mid.mp3", "last.mp3"]);
        assert_eq!(p.index_of(ids[0]), Some(1));
        assert_eq!(p.get(ids[0]).unwrap().title.as_deref(), Some("Mid"));
    }

    #[test]
    fn insert_clamps_an_out_of_range_index() {
        let mut p = Playlist::new();
        let ids = insert_loaded(&mut p, 99, &[entry("a.mp3")]);
        assert_eq!(p.index_of(ids[0]), Some(0));
    }

    #[test]
    fn loaded_entries_snapshots_a_playlist() {
        let mut p = Playlist::new();
        append_loaded(&mut p, &[entry("a.mp3").with_title("A")]);
        let back = loaded_entries(&p);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].title.as_deref(), Some("A"));
        assert_eq!(
            back[0].location,
            MediaLocation::File(PathBuf::from("a.mp3"))
        );
    }
}
