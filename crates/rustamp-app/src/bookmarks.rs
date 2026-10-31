// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The bookmark store (S03 ids 40319-40321, S13 §1 `/BOOKMARK`, RFC 0005 O12).
//!
//! No spec owns bookmarks, so the store is an own choice: an ordinary
//! `bookmarks.m3u8` beside the configuration file, which any playlist reader
//! can open. A second launch's `/BOOKMARK` writes the same file and the running
//! player reads it again before it shows it, so the file is the only state.
//!
//! An unreadable file is never overwritten: the store then lives in memory for
//! this run and the player says so.

use std::path::{Path, PathBuf};

use rustamp_audio::TrackSource;
use rustamp_core::command::Command;
use rustamp_core::playlist::MediaLocation;
use rustamp_playlist::{LoadedEntry, PlaylistFormat, read_playlist_path, write_playlist};

use crate::app::App;

/// The store's file name, beside the configuration file.
const FILE_NAME: &str = "bookmarks.m3u8";

/// The bookmarks the menu shows at most; the editor shows them all.
const MENU_ROWS: usize = 30;

/// The longest bookmark label in the menu, in characters.
const MENU_LABEL_CHARS: usize = 60;

/// Where the store of the configuration at `config` lives.
pub fn store_path(config: &Path) -> PathBuf {
    config.with_file_name(FILE_NAME)
}

/// The bookmarks in memory and the file that keeps them.
#[derive(Debug, Default)]
pub(crate) struct Bookmarks {
    path: Option<PathBuf>,
    entries: Vec<LoadedEntry>,
}

impl Bookmarks {
    /// Opens the store at `path`. A missing file is an empty store; an
    /// unreadable one comes back as the error, and the store keeps no path, so
    /// the file stays as it is.
    pub(crate) fn open(path: PathBuf) -> (Self, Option<String>) {
        match read_entries(&path) {
            Ok(entries) => (
                Self {
                    path: Some(path),
                    entries,
                },
                None,
            ),
            Err(error) => (Self::default(), Some(error)),
        }
    }

    /// Reads the file again: a second launch may have added to it.
    pub(crate) fn reload(&mut self) -> Result<(), String> {
        if let Some(path) = &self.path {
            self.entries = read_entries(path)?;
        }
        Ok(())
    }

    pub(crate) fn entries(&self) -> &[LoadedEntry] {
        &self.entries
    }

    /// Adds a bookmark at the end. A location that is already there stays
    /// where it is and keeps its title.
    pub(crate) fn add(
        &mut self,
        location: MediaLocation,
        title: Option<String>,
    ) -> Result<(), String> {
        if self.entries.iter().any(|entry| entry.location == location) {
            return Ok(());
        }
        self.entries.push(LoadedEntry {
            location,
            title,
            duration_ms: None,
        });
        self.save()
    }

    pub(crate) fn remove(&mut self, index: usize) -> Result<(), String> {
        if index < self.entries.len() {
            self.entries.remove(index);
            self.save()?;
        }
        Ok(())
    }

    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let bytes = write_playlist(PlaylistFormat::M3u8, &self.entries)
            .map_err(|error| format!("the bookmarks could not be written: {error}"))?;
        write_replacing(path, &bytes)
            .map_err(|error| format!("{} could not be written: {error}", path.display()))
    }
}

/// Adds `location` to the store at `path`, for a launch that hands the rest of
/// its command line to the running player.
pub fn add_to_store(path: PathBuf, location: MediaLocation) -> Result<(), String> {
    let (mut store, problem) = Bookmarks::open(path);
    if let Some(problem) = problem {
        return Err(problem);
    }
    store.add(location, None)
}

fn read_entries(path: &Path) -> Result<Vec<LoadedEntry>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{} could not be read: {error}", path.display())),
    };
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    read_playlist_path(&bytes, path)
        .map_err(|error| format!("{} could not be read: {error}", path.display()))
}

/// Writes through a neighbour, so a failed write leaves the old file whole.
fn write_replacing(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let scratch = path.with_extension("m3u8.tmp");
    std::fs::write(&scratch, bytes)?;
    std::fs::rename(&scratch, path)
}

/// What a bookmark is called on screen.
fn entry_label(entry: &LoadedEntry) -> String {
    match &entry.title {
        Some(title) if !title.is_empty() => title.clone(),
        _ => entry.location.to_display_string(),
    }
}

fn location_of(source: &TrackSource) -> MediaLocation {
    match source {
        TrackSource::Path(path) => MediaLocation::File(path.clone()),
        TrackSource::Url(url) => MediaLocation::Url(url.clone()),
    }
}

impl App {
    /// Opens the store beside the configuration file.
    pub(crate) fn open_bookmarks(&mut self, config: Option<&Path>) {
        let Some(config) = config else {
            self.bookmarks = Bookmarks::default();
            return;
        };
        let (store, problem) = Bookmarks::open(store_path(config));
        self.bookmarks = store;
        if let Some(problem) = problem {
            self.note(format!(
                "bookmarks are kept for this run only, because {problem}"
            ));
        }
    }

    /// Reads the store again before it is shown.
    pub(crate) fn refresh_bookmarks(&mut self) {
        if let Err(problem) = self.bookmarks.reload() {
            self.note(problem);
        }
    }

    /// Adds `location` to the bookmarks. A failure to save is noted: the
    /// bookmark still counts for this run.
    pub fn add_bookmark(&mut self, location: MediaLocation, title: Option<String>) {
        if let Err(problem) = self.bookmarks.add(location, title) {
            self.note(problem);
        }
    }

    /// "Add current track as a bookmark" (S03 id 40321). Without a current
    /// track there is nothing to add.
    pub(crate) fn bookmark_current(&mut self) {
        let Some(index) = self.engine().current_index().map(|index| index as usize) else {
            return;
        };
        self.bookmark_queue_entry(index);
    }

    /// "Bookmark the selected item(s)" (S03 id 40319), in playlist order.
    pub(crate) fn bookmark_selection(&mut self) {
        let indices: Vec<usize> = self
            .playlist
            .selected_ids()
            .into_iter()
            .filter_map(|id| self.entry_ids.iter().position(|entry| *entry == id))
            .collect();
        for index in indices {
            self.bookmark_queue_entry(index);
        }
    }

    fn bookmark_queue_entry(&mut self, index: usize) {
        let Some(source) = self.engine().queue().get(index) else {
            return;
        };
        let location = location_of(source);
        let title = self.entry_name(index);
        self.add_bookmark(location, title);
    }

    /// The labels the Bookmarks submenu lists, each safe for a system menu.
    pub(crate) fn bookmark_menu_labels(&self) -> Vec<String> {
        self.bookmarks
            .entries()
            .iter()
            .take(MENU_ROWS)
            .map(|entry| {
                let label = entry_label(entry);
                let short: String = label.chars().take(MENU_LABEL_CHARS).collect();
                // A system menu reads `&` as the next letter's mnemonic.
                short.replace('&', "&&")
            })
            .collect()
    }

    /// Plays bookmark `index`: it joins the end of the playlist and starts,
    /// because the playlist keeps what the user already has queued (own
    /// choice, no spec owns bookmarks).
    pub(crate) fn play_bookmark(&mut self, index: usize) {
        let Some(entry) = self.bookmarks.entries().get(index) else {
            return;
        };
        let command = Command::AddItems {
            items: vec![entry.location.clone()],
            at: None,
            play: true,
        };
        if let Err(error) = self.submit(command) {
            self.note(format!("the bookmark could not be played: {error}"));
        }
    }

    /// The store indices the editor lists: those whose label contains the
    /// search text, ignoring case.
    pub(crate) fn bookmark_matches(&self) -> Vec<usize> {
        let needle = self.dialogs.jump_text().to_lowercase();
        self.bookmarks
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                needle.is_empty()
                    || entry_label(entry).to_lowercase().contains(&needle)
                    || entry
                        .location
                        .to_display_string()
                        .to_lowercase()
                        .contains(&needle)
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// The editor's body: the search field, a window of matches that keeps
    /// the selected row in view, then a line with the count and the keys.
    pub(crate) fn bookmark_editor_lines(&self) -> Vec<String> {
        let matches = self.bookmark_matches();
        let selected = self.dialogs.jump_index();
        let rows = rustamp_ui::JUMP_FILE_ROWS;
        let first = selected.saturating_sub(rows - 1);
        let entries = self.bookmarks.entries();
        let mut lines = vec![format!("Search: {}", self.dialogs.jump_text())];
        if matches.is_empty() {
            lines.push(if entries.is_empty() {
                "(no bookmarks yet: Alt+I adds the current track)".to_string()
            } else {
                "(no match)".to_string()
            });
        }
        for (row, &index) in matches.iter().enumerate().skip(first).take(rows) {
            let marker = if row == selected { "> " } else { "  " };
            lines.push(format!("{marker}{}", entry_label(&entries[index])));
        }
        lines.push(format!(
            "{} of {} - Enter plays, Delete removes",
            matches.len(),
            entries.len()
        ));
        lines
    }

    /// `Enter` in the editor: plays the selected bookmark and closes.
    pub(crate) fn confirm_bookmark_editor(&mut self) {
        let matches = self.bookmark_matches();
        let Some(&index) = matches.get(self.dialogs.jump_index()) else {
            return;
        };
        self.play_bookmark(index);
        self.dialogs.close_any();
    }

    /// `Delete` in the editor: removes the selected bookmark.
    pub(crate) fn remove_selected_bookmark(&mut self) {
        let matches = self.bookmark_matches();
        let Some(&index) = matches.get(self.dialogs.jump_index()) else {
            return;
        };
        if let Err(problem) = self.bookmarks.remove(index) {
            self.note(problem);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> MediaLocation {
        MediaLocation::File(PathBuf::from(name))
    }

    #[test]
    fn a_store_survives_a_reopen_with_titles_and_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let (mut store, problem) = Bookmarks::open(path.clone());
        assert_eq!(problem, None);
        let first = dir.path().join("ä first.mp3");
        store
            .add(
                MediaLocation::File(first.clone()),
                Some("Ä & First".to_string()),
            )
            .unwrap();
        store
            .add(
                MediaLocation::Url("http://example.test/live".to_string()),
                None,
            )
            .unwrap();

        let (again, problem) = Bookmarks::open(path);
        assert_eq!(problem, None);
        assert_eq!(again.entries().len(), 2);
        assert_eq!(again.entries()[0].location, MediaLocation::File(first));
        assert_eq!(again.entries()[0].title.as_deref(), Some("Ä & First"));
        assert_eq!(
            again.entries()[1].location,
            MediaLocation::Url("http://example.test/live".to_string())
        );
    }

    #[test]
    fn a_location_is_bookmarked_once() {
        let (mut store, _) = Bookmarks::open(PathBuf::new());
        store.path = None;
        store.add(file("a.mp3"), Some("one".to_string())).unwrap();
        store.add(file("a.mp3"), Some("two".to_string())).unwrap();
        assert_eq!(store.entries().len(), 1);
        assert_eq!(store.entries()[0].title.as_deref(), Some("one"));
    }

    #[test]
    fn an_unreadable_store_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let garbage = [0u8, 159, 146, 150, 0, 1, 2, 3];
        std::fs::write(&path, garbage).unwrap();
        let (mut store, problem) = Bookmarks::open(path.clone());
        assert!(problem.is_some());
        store.add(file("a.mp3"), None).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), garbage);
        assert_eq!(store.entries().len(), 1);
    }

    #[test]
    fn a_second_launch_reaches_the_running_players_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let (mut running, _) = Bookmarks::open(path.clone());
        add_to_store(path, file("later.mp3")).unwrap();
        assert!(running.entries().is_empty());
        running.reload().unwrap();
        assert_eq!(running.entries().len(), 1);
    }

    #[test]
    fn removing_saves_and_a_stale_index_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let (mut store, _) = Bookmarks::open(path.clone());
        store.add(file("a.mp3"), None).unwrap();
        store.add(file("b.mp3"), None).unwrap();
        store.remove(7).unwrap();
        assert_eq!(store.entries().len(), 2);
        store.remove(0).unwrap();
        let (again, _) = Bookmarks::open(path);
        assert_eq!(again.entries().len(), 1);
    }
}
