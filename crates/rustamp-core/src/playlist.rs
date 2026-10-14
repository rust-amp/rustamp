//! Playlist data model with stable entry ids.
//!
//! [`Playlist`] is a plain, single-threaded data structure: the app state
//! owner mutates it and publishes `PlaylistChanged` events. Behaviors such as
//! shuffle order (S15), randomize, and file formats live in `rustamp-playlist`.
//!
//! Every entry gets an [`EntryId`] that is unique within the playlist for its
//! whole lifetime and never reused, even after removal. Commands and events
//! refer to entries by id so they stay valid while indices shift.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Stable, never-reused identifier of a playlist entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntryId(pub u64);

/// Where a piece of media lives.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaLocation {
    /// A local file.
    File(PathBuf),
    /// A network stream or other URL.
    Url(String),
}

impl MediaLocation {
    /// Final path component / URL tail, for display and sorting.
    pub fn file_name(&self) -> String {
        match self {
            MediaLocation::File(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            MediaLocation::Url(u) => u.rsplit('/').next().unwrap_or(u).to_owned(),
        }
    }

    /// Lossy string form of the full location.
    pub fn to_display_string(&self) -> String {
        match self {
            MediaLocation::File(p) => p.to_string_lossy().into_owned(),
            MediaLocation::Url(u) => u.clone(),
        }
    }
}

/// One playlist row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistEntry {
    /// Stable id.
    pub id: EntryId,
    /// Media location.
    pub location: MediaLocation,
    /// Cached display title (formatted by `rustamp-meta`), if known.
    pub title: Option<String>,
    /// Cached duration in ms, if known.
    pub duration_ms: Option<u64>,
}

/// Sort keys for `Command::Sort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortKey {
    /// By displayed title (case-insensitive).
    Title,
    /// By file name only.
    FileName,
    /// By full path then file name.
    PathAndFileName,
    /// By duration (unknown first).
    Duration,
}

/// What changed in a playlist, carried by `Event::PlaylistChanged`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaylistChangeKind {
    /// Entries were inserted.
    Inserted,
    /// Entries were removed.
    Removed,
    /// Entries were moved.
    Moved,
    /// The playlist was cleared.
    Cleared,
    /// The whole order changed (sort, reverse, randomize).
    Reordered,
    /// The selection changed.
    SelectionChanged,
    /// Cached title/duration of an entry was updated.
    EntryUpdated,
}

/// The playlist model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Playlist {
    entries: Vec<PlaylistEntry>,
    next_id: u64,
    selection: BTreeSet<EntryId>,
    current: Option<EntryId>,
}

impl Playlist {
    /// Empty playlist.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate entries in order.
    pub fn iter(&self) -> std::slice::Iter<'_, PlaylistEntry> {
        self.entries.iter()
    }

    /// Entries as a slice.
    pub fn entries(&self) -> &[PlaylistEntry] {
        &self.entries
    }

    /// Entry at `index`.
    pub fn get_index(&self, index: usize) -> Option<&PlaylistEntry> {
        self.entries.get(index)
    }

    /// Entry with `id`.
    pub fn get(&self, id: EntryId) -> Option<&PlaylistEntry> {
        self.index_of(id).map(|i| &self.entries[i])
    }

    /// Mutable entry with `id` (e.g. to update cached title/duration).
    pub fn get_mut(&mut self, id: EntryId) -> Option<&mut PlaylistEntry> {
        self.index_of(id).map(|i| &mut self.entries[i])
    }

    /// Current index of `id` (O(n)).
    pub fn index_of(&self, id: EntryId) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    fn alloc_id(&mut self) -> EntryId {
        let id = EntryId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Insert a location at `index` (clamped to `len`), returning its new id.
    pub fn insert(&mut self, index: usize, location: MediaLocation) -> EntryId {
        let id = self.alloc_id();
        let index = index.min(self.entries.len());
        self.entries.insert(
            index,
            PlaylistEntry {
                id,
                location,
                title: None,
                duration_ms: None,
            },
        );
        id
    }

    /// Insert several locations contiguously at `index`.
    ///
    /// Ids are allocated in iteration order, exactly as if [`Playlist::insert`]
    /// had been called once per location, but the entries are spliced into
    /// place in one step (O(k + n) instead of O(k * n)).
    pub fn insert_many<I>(&mut self, index: usize, locations: I) -> Vec<EntryId>
    where
        I: IntoIterator<Item = MediaLocation>,
    {
        let start = index.min(self.entries.len());
        let mut new = Vec::new();
        let mut ids = Vec::new();
        for location in locations {
            let id = self.alloc_id();
            ids.push(id);
            new.push(PlaylistEntry {
                id,
                location,
                title: None,
                duration_ms: None,
            });
        }
        self.entries.splice(start..start, new);
        ids
    }

    /// Append a location.
    pub fn push(&mut self, location: MediaLocation) -> EntryId {
        self.insert(self.entries.len(), location)
    }

    /// Remove an entry. Clears it from selection; if it was current, current becomes `None`.
    pub fn remove(&mut self, id: EntryId) -> Option<PlaylistEntry> {
        let i = self.index_of(id)?;
        self.selection.remove(&id);
        if self.current == Some(id) {
            self.current = None;
        }
        Some(self.entries.remove(i))
    }

    /// Remove all entries; ids are still never reused.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.selection.clear();
        self.current = None;
    }

    /// Move one entry so that it ends up at `to` (index in the resulting list, clamped).
    /// Returns `false` if the id is unknown.
    pub fn move_entry(&mut self, id: EntryId, to: usize) -> bool {
        let Some(from) = self.index_of(id) else {
            return false;
        };
        let e = self.entries.remove(from);
        let to = to.min(self.entries.len());
        self.entries.insert(to, e);
        true
    }

    /// Move several entries as a block, keeping their relative order, so the
    /// first lands at `to` within the list *after* the moved entries have been
    /// taken out (clamped). Unknown ids are ignored.
    pub fn move_entries(&mut self, ids: &[EntryId], to: usize) {
        let set: BTreeSet<EntryId> = ids.iter().copied().collect();
        let (moved, mut rest): (Vec<_>, Vec<_>) =
            self.entries.drain(..).partition(|e| set.contains(&e.id));
        let to = to.min(rest.len());
        rest.splice(to..to, moved);
        self.entries = rest;
    }

    /// Reorder to exactly `order` (a permutation of the current ids). Ids not
    /// listed are appended in their previous order; unknown ids are ignored.
    /// Used by sort/reverse/randomize.
    ///
    /// Runs in O(n + m) for `n` entries and `m` ids in `order`; a quadratic
    /// scan here made randomizing a large playlist take minutes.
    pub fn apply_order(&mut self, order: &[EntryId]) {
        let mut old: Vec<Option<PlaylistEntry>> = self.entries.drain(..).map(Some).collect();
        let index: HashMap<EntryId, usize> = old
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.as_ref().map(|e| (e.id, i)))
            .collect();
        let mut out = Vec::with_capacity(old.len());
        for id in order {
            if let Some(&i) = index.get(id) {
                out.extend(old[i].take());
            }
        }
        out.extend(old.into_iter().flatten());
        self.entries = out;
    }

    /// Reverse the order.
    pub fn reverse(&mut self) {
        self.entries.reverse();
    }

    /// Stable sort by `key`.
    pub fn sort(&mut self, key: SortKey) {
        match key {
            SortKey::Title => self.entries.sort_by_cached_key(|e| {
                e.title
                    .clone()
                    .unwrap_or_else(|| e.location.file_name())
                    .to_lowercase()
            }),
            SortKey::FileName => self
                .entries
                .sort_by_cached_key(|e| e.location.file_name().to_lowercase()),
            SortKey::PathAndFileName => self
                .entries
                .sort_by_cached_key(|e| e.location.to_display_string().to_lowercase()),
            SortKey::Duration => self.entries.sort_by_key(|e| e.duration_ms),
        }
    }

    // --- current ---

    /// Id of the current (playing / cued) entry.
    pub fn current(&self) -> Option<EntryId> {
        self.current
    }

    /// Index of the current entry.
    pub fn current_index(&self) -> Option<usize> {
        self.current.and_then(|id| self.index_of(id))
    }

    /// Set the current entry; returns `false` (and leaves it unchanged) for an unknown id.
    pub fn set_current(&mut self, id: Option<EntryId>) -> bool {
        match id {
            Some(i) if self.index_of(i).is_none() => false,
            _ => {
                self.current = id;
                true
            }
        }
    }

    // --- selection ---

    /// Selected ids (sorted by id, not by position).
    pub fn selection(&self) -> &BTreeSet<EntryId> {
        &self.selection
    }

    /// Selected ids in playlist order.
    pub fn selected_in_order(&self) -> Vec<EntryId> {
        self.entries
            .iter()
            .filter(|e| self.selection.contains(&e.id))
            .map(|e| e.id)
            .collect()
    }

    /// Whether `id` is selected.
    pub fn is_selected(&self, id: EntryId) -> bool {
        self.selection.contains(&id)
    }

    /// Replace the selection (unknown ids dropped).
    pub fn select_only(&mut self, ids: &[EntryId]) {
        self.selection = ids
            .iter()
            .copied()
            .filter(|i| self.index_of(*i).is_some())
            .collect();
    }

    /// Toggle one entry's selection.
    pub fn toggle_selected(&mut self, id: EntryId) {
        if self.index_of(id).is_none() {
            return;
        }
        if !self.selection.remove(&id) {
            self.selection.insert(id);
        }
    }

    /// Select every entry.
    pub fn select_all(&mut self) {
        self.selection = self.entries.iter().map(|e| e.id).collect();
    }

    /// Clear the selection.
    pub fn select_none(&mut self) {
        self.selection.clear();
    }

    /// Invert the selection.
    pub fn invert_selection(&mut self) {
        self.selection = self
            .entries
            .iter()
            .map(|e| e.id)
            .filter(|i| !self.selection.contains(i))
            .collect();
    }
}

impl<'a> IntoIterator for &'a Playlist {
    type Item = &'a PlaylistEntry;
    type IntoIter = std::slice::Iter<'a, PlaylistEntry>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(n: &str) -> MediaLocation {
        MediaLocation::File(PathBuf::from(n))
    }

    fn names(p: &Playlist) -> Vec<String> {
        p.iter().map(|e| e.location.file_name()).collect()
    }

    #[test]
    fn insert_move_remove_keep_stable_ids() {
        let mut p = Playlist::new();
        let a = p.push(f("a.mp3"));
        let b = p.push(f("b.mp3"));
        let c = p.push(f("c.mp3"));
        let x = p.insert(1, f("x.mp3"));
        assert_eq!(names(&p), ["a.mp3", "x.mp3", "b.mp3", "c.mp3"]);
        assert_eq!(p.index_of(c), Some(3));

        p.set_current(Some(b));
        p.select_only(&[a, c]);
        assert!(p.move_entry(c, 0));
        assert_eq!(names(&p), ["c.mp3", "a.mp3", "x.mp3", "b.mp3"]);
        assert_eq!(p.index_of(c), Some(0));
        assert_eq!(p.current_index(), Some(3));
        assert_eq!(p.selected_in_order(), vec![c, a]);

        p.move_entries(&[a, b], 1);
        assert_eq!(names(&p), ["c.mp3", "a.mp3", "b.mp3", "x.mp3"]);

        let removed = p.remove(a).unwrap();
        assert_eq!(removed.id, a);
        assert_eq!(names(&p), ["c.mp3", "b.mp3", "x.mp3"]);
        assert!(!p.is_selected(a));
        assert_eq!(p.get(b).unwrap().location, f("b.mp3"));

        // ids are never reused
        let y = p.push(f("y.mp3"));
        assert!(y > x && y > c);
        p.remove(b);
        assert_eq!(p.current(), None);
    }

    #[test]
    fn sort_reverse_apply_order() {
        let mut p = Playlist::new();
        let ids = p.insert_many(0, [f("b"), f("C"), f("a")]);
        p.sort(SortKey::FileName);
        assert_eq!(names(&p), ["a", "b", "C"]);
        p.reverse();
        assert_eq!(names(&p), ["C", "b", "a"]);
        p.apply_order(&[ids[2], ids[0], ids[1]]);
        assert_eq!(names(&p), ["a", "b", "C"]);
    }

    #[test]
    fn selection_ops() {
        let mut p = Playlist::new();
        let ids = p.insert_many(0, [f("1"), f("2"), f("3")]);
        p.select_only(&[ids[0]]);
        p.invert_selection();
        assert_eq!(p.selected_in_order(), vec![ids[1], ids[2]]);
        assert!(!p.set_current(Some(EntryId(99))));
    }

    #[test]
    fn insert_many_matches_repeated_insert() {
        let mut batched = Playlist::new();
        batched.push(f("first"));
        let batch_ids = batched.insert_many(1, [f("a"), f("b"), f("c")]);

        let mut stepwise = Playlist::new();
        stepwise.push(f("first"));
        let step_ids: Vec<EntryId> = ["a", "b", "c"]
            .iter()
            .enumerate()
            .map(|(offset, n)| stepwise.insert(1 + offset, f(n)))
            .collect();

        assert_eq!(batch_ids, step_ids);
        assert_eq!(names(&batched), names(&stepwise));
        assert_eq!(names(&batched), ["first", "a", "b", "c"]);

        // An index past the end clamps, like `insert`.
        let ids = batched.insert_many(99, [f("z")]);
        assert_eq!(ids.len(), 1);
        assert_eq!(names(&batched), ["first", "a", "b", "c", "z"]);
    }

    #[test]
    fn apply_order_handles_duplicates_and_unknown_ids() {
        let mut p = Playlist::new();
        let ids = p.insert_many(0, [f("a"), f("b"), f("c")]);
        // `c` twice, an unknown id, and `b` missing: `b` is appended in its
        // previous relative order, the second `c` is ignored.
        p.apply_order(&[ids[2], EntryId(999), ids[2], ids[0]]);
        assert_eq!(names(&p), ["c", "a", "b"]);
        assert_eq!(p.len(), 3);
    }

    /// Guards against a quadratic `apply_order`: the previous implementation
    /// needed minutes for a playlist of this size (lane E measured it).
    #[test]
    fn apply_order_is_not_quadratic() {
        const N: usize = 20_000;
        let mut p = Playlist::new();
        let ids: Vec<EntryId> = p.insert_many(
            0,
            (0..N).map(|i| MediaLocation::File(PathBuf::from(format!("{i}.mp3")))),
        );
        assert_eq!(ids.len(), N);

        let reversed: Vec<EntryId> = ids.iter().rev().copied().collect();
        let started = std::time::Instant::now();
        p.apply_order(&reversed);
        let elapsed = started.elapsed();

        assert_eq!(p.get_index(0).unwrap().id, ids[N - 1]);
        assert_eq!(p.get_index(N - 1).unwrap().id, ids[0]);
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "apply_order took {elapsed:?} for {N} entries"
        );
    }
}
